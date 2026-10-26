//! Container file RPCs (`docker/files/*`): browse, stat, export/import via
//! staging paths, and local chunked uploads. Split out of `docker.rs` to keep
//! each handler file under the 600-line structural cap; dispatch stays in
//! `handle_docker` (mock registry scanning walks this directory too).

use super::*;

use std::path::Path;
use std::time::Duration;

use crate::features::docker::files::{
    cleanup_command, cleanup_local, create_local_staging_dir, export_command, export_local,
    files_list_payload, import_command, import_local, is_missing_tool, is_regular_file_type,
    join_container_path, list_command, list_entries, local_staging_regular_file_size,
    mktemp_command, normalize_container_path, parse_mktemp_output, parse_staging_probe,
    parse_stat_output, safe_staging_name, staging_probe_command, stat_command, stat_payload,
    validate_staging_path, StagingLocation, UploadRegistry, TRANSFER_TIMEOUT, UPLOAD_CHUNK_BYTES,
    UPLOAD_MAX_BYTES,
};
use crate::features::docker::{
    find_local_cli, run_local, validate_container_id, DockerTarget, EngineConfig, FILES_TIMEOUT,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;

impl Plugin {
    pub(super) fn handle_docker_files(&self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            // 容器文件浏览（容器管理分屏）。`ls -1ap` 只读采集：SSH 走
            // `ssh/exec`，本机走 argv 直呼；路径统一归一化后进命令，容器
            // 缺 `ls` 时错误文本保留 `ls` + `executable file not found` 供
            // 前端识别并禁用上传/下载入口。响应恒为
            // `{path,parent,entries}`，目录优先、名称升序。
            "docker/files/list" => {
                let container_id = required_string(&params, "containerId")?;
                let engine = crate::features::docker::parse_engine(&params)?;
                let requested = container_path_param(&params)?;
                let path = normalize_container_path(requested)?;
                let (ssh_command, local_argv) = list_command(&engine, container_id, &path)?;
                if is_local_target(&params)? {
                    let (exit_code, output) =
                        self.run_local_args(&engine, &local_argv, FILES_TIMEOUT)?;
                    if exit_code != 0 {
                        return Err(format!(
                            "{} files/list failed (exit {exit_code}): {output}",
                            engine.cli
                        ));
                    }
                    return Ok(files_list_payload(&path, list_entries(&path, &output)));
                }
                let session_id = required_string(&params, "sessionId")?;
                let (exit_code, output) =
                    self.ssh_output(session_id, &ssh_command, FILES_TIMEOUT)?;
                if exit_code != 0 {
                    return Err(format!(
                        "{} files/list failed (exit {exit_code}): {output}",
                        engine.cli
                    ));
                }
                Ok(files_list_payload(&path, list_entries(&path, &output)))
            }

            // 条目详情：`stat -c '%F|%s|%Y|%a'`。容器缺 `stat`（distroless
            // 镜像）时返回 `{unavailable:true}` 而不是报错——UI 显示“—”且
            // 不影响下载；其余失败照常带 stderr 返回。
            "docker/files/stat" => {
                let container_id = required_string(&params, "containerId")?;
                let engine = crate::features::docker::parse_engine(&params)?;
                let requested = container_path_param(&params)?;
                let path = normalize_container_path(requested)?;
                let stat = if is_local_target(&params)? {
                    self.local_stat(&engine, container_id, &path)?
                } else {
                    let session_id = required_string(&params, "sessionId")?;
                    self.remote_stat(session_id, &engine, container_id, &path)?
                };
                Ok(stat_payload(stat))
            }

            // 容器文件 → 暂存。先 stat：目录拒绝（v1 目录不可下载），且
            // 只放行 `%F == "regular file"`——`docker cp` 会保留符号链接，
            // 容器内指向 /etc/shadow 的链接会变成主机/远端链接，随后被
            // SFTP 或本机保存跟随。SSH 在远端 `mktemp -d
            // /tmp/dbx-sshx.XXXXXX` 后 `docker cp`；本机在系统临时目录建
            // `dbx-sshx-<uuid>` 后 argv `docker cp`。复制完成后再对暂存项
            // 做一次类型校验（lstat / `stat -c %F`），stat 不可用时也不会
            // 把目录或链接交出去。
            "docker/files/export" => {
                let container_id = required_string(&params, "containerId")?;
                validate_container_id(container_id)?;
                let engine = crate::features::docker::parse_engine(&params)?;
                let requested = container_path_param(&params)?;
                let path = normalize_container_path(requested)?;
                let local = is_local_target(&params)?;
                let stat = if local {
                    self.local_stat(&engine, container_id, &path)?
                } else {
                    let session_id = required_string(&params, "sessionId")?;
                    self.remote_stat(session_id, &engine, container_id, &path)?
                };
                if let Some(stat) = stat.as_ref() {
                    if stat.kind == "dir" {
                        return Err("目录暂不支持下载".to_string());
                    }
                    if !stat.is_regular_file() {
                        return Err("仅支持下载普通文件".to_string());
                    }
                }
                let name = safe_staging_name(&path);
                if local {
                    let dir = create_local_staging_dir()?;
                    let staging = dir.join(&name);
                    let staging_text = staging.to_string_lossy().into_owned();
                    let size = match self.runtime.block_on(export_local(
                        &engine,
                        container_id,
                        &path,
                        &staging_text,
                    )) {
                        Ok(size) => size,
                        Err(error) => {
                            cleanup_local(&staging_text);
                            return Err(error);
                        }
                    };
                    return Ok(json!({
                        "stagingPath": staging_text,
                        "size": size,
                        "name": name,
                        "location": "local",
                    }));
                }
                let session_id = required_string(&params, "sessionId")?;
                let (dir_code, dir_output) =
                    self.ssh_output(session_id, &mktemp_command(), FILES_TIMEOUT)?;
                if dir_code != 0 {
                    return Err(format!(
                        "Failed to create a remote staging directory: {dir_output}"
                    ));
                }
                let dir = parse_mktemp_output(&dir_output)?;
                let staging = join_container_path(&dir, &name);
                let command = export_command(&engine, container_id, &path, &staging)?;
                let (exit_code, output) =
                    self.ssh_output(session_id, &command, TRANSFER_TIMEOUT)?;
                if exit_code != 0 {
                    self.cleanup_remote_staging(session_id, &dir);
                    return Err(format!(
                        "{} cp failed (exit {exit_code}): {output}",
                        engine.cli
                    ));
                }
                let size = match self.remote_staging_regular_file(session_id, &staging) {
                    Ok(size) => size,
                    Err(error) => {
                        self.cleanup_remote_staging(session_id, &dir);
                        return Err(error);
                    }
                };
                Ok(json!({
                    "stagingPath": staging,
                    "size": size,
                    "name": name,
                    "location": "remote",
                }))
            }

            // SSH 上传暂存路径（本机上传走 upload/*，不用 SFTP 暂存）。
            "docker/files/prepareUpload" => {
                if is_local_target(&params)? {
                    return Err(
                        "docker/files/prepareUpload is only available for SSH targets"
                            .to_string(),
                    );
                }
                let session_id = required_string(&params, "sessionId")?;
                self.runtime
                    .block_on(self.ssh.ensure_writable(session_id))?;
                let name = safe_staging_name(required_string(&params, "name")?);
                let (dir_code, dir_output) =
                    self.ssh_output(session_id, &mktemp_command(), FILES_TIMEOUT)?;
                if dir_code != 0 {
                    return Err(format!(
                        "Failed to create a remote staging directory: {dir_output}"
                    ));
                }
                let dir = parse_mktemp_output(&dir_output)?;
                let staging = join_container_path(&dir, &name);
                validate_staging_path(&staging, StagingLocation::Remote)?;
                Ok(json!({ "stagingPath": staging, "name": name }))
            }

            // 暂存 → 容器。destPath 以 `/` 结尾视为目录并保留暂存文件名；
            // 否则视为完整目标路径。暂存项必须经 lstat/`stat %F` 确认为
            // 普通文件（符号链接/目录/其他类型拒绝）。审计只在复制成功后
            // 写入，记录真实字节数（容器 id/目标/大小，不含文件内容）。
            "docker/files/import" => {
                let container_id = required_string(&params, "containerId")?;
                // 先校验 id，审计行（含 id）只能在合法 id 下产生。
                validate_container_id(container_id)?;
                let engine = crate::features::docker::parse_engine(&params)?;
                let staging = required_string(&params, "stagingPath")?;
                let dest_input = required_string(&params, "destPath")?;
                let dest = crate::features::docker::resolve_import_dest(
                    dest_input,
                    &safe_staging_name(staging),
                )?;
                let local = is_local_target(&params)?;
                let size = if local {
                    validate_staging_path(staging, StagingLocation::Local)?;
                    let size = local_staging_regular_file_size(staging)?;
                    self.runtime
                        .block_on(import_local(&engine, staging, container_id, &dest))?;
                    size
                } else {
                    validate_staging_path(staging, StagingLocation::Remote)?;
                    let session_id = required_string(&params, "sessionId")?;
                    self.runtime
                        .block_on(self.ssh.ensure_writable(session_id))?;
                    let size = self.remote_staging_regular_file(session_id, staging)?;
                    let command = import_command(&engine, container_id, staging, &dest)?;
                    let (exit_code, output) =
                        self.ssh_output(session_id, &command, TRANSFER_TIMEOUT)?;
                    if exit_code != 0 {
                        return Err(format!(
                            "{} cp failed (exit {exit_code}): {output}",
                            engine.cli
                        ));
                    }
                    size
                };
                crate::features::docker::audit_import(
                    &self.ssh.data_dir(),
                    container_id,
                    &dest,
                    size,
                );
                Ok(json!({ "path": dest }))
            }

            // 删除暂存：只接受本功能生成的前缀。清理失败只返回
            // `removed:false`，不改变传输结果。成功后同一条目从上传
            // 注册表移除，避免 finished 上传在进程生命周期内泄漏。
            "docker/files/cleanup" => {
                let staging = required_string(&params, "stagingPath")?;
                if is_local_target(&params)? {
                    validate_staging_path(staging, StagingLocation::Local)?;
                    let removed = cleanup_local(staging);
                    if removed {
                        self.forget_upload(staging);
                    }
                    return Ok(json!({ "removed": removed }));
                }
                let session_id = required_string(&params, "sessionId")?;
                let command = cleanup_command(staging)?;
                let (exit_code, _) = self.ssh_output(session_id, &command, FILES_TIMEOUT)?;
                Ok(json!({ "removed": exit_code == 0 }))
            }

            // 本机暂存 → 下载目录（复用下载目录/撞名/历史语义）。
            "docker/files/saveLocal" => {
                if !is_local_target(&params)? {
                    return Err(
                        "docker/files/saveLocal is only available for local targets".to_string()
                    );
                }
                let staging = required_string(&params, "stagingPath")?;
                validate_staging_path(staging, StagingLocation::Local)?;
                let name = required_string(&params, "name")?;
                let conflict = params.get("conflict").and_then(Value::as_str);
                let saved = crate::filesystem::local::downloads::save_local_path(
                    &self.ssh.data_dir(),
                    name,
                    Path::new(staging),
                    conflict,
                )?;
                if cleanup_local(staging) {
                    self.forget_upload(staging);
                }
                Ok(saved)
            }

            // 本机分片上传（SSH 上传走 prepareUpload + SFTP）。`begin` 建
            // 临时文件，`chunk` 必须顺序、offset 等于已收字节数，单分片
            // ≤256 KiB，累计 ≤512 MiB；`finish` 关闭文件返回暂存路径，
            // 随后由前端调用 import + cleanup。
            "docker/files/upload/begin" => {
                if !is_local_target(&params)? {
                    return Err(
                        "docker/files/upload/begin is only available for local targets"
                            .to_string(),
                    );
                }
                let name = required_string(&params, "name")?;
                let upload_id = self.upload_registry()?.begin(name)?;
                Ok(json!({
                    "uploadId": upload_id,
                    "maxBytes": UPLOAD_MAX_BYTES,
                }))
            }

            "docker/files/upload/chunk" => {
                if !is_local_target(&params)? {
                    return Err(
                        "docker/files/upload/chunk is only available for local targets"
                            .to_string(),
                    );
                }
                let upload_id = required_string(&params, "uploadId")?;
                let offset = params
                    .get("offset")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "Missing offset".to_string())?;
                let encoded = required_string(&params, "dataBase64")?;
                // Reject an oversized encoded payload before allocating the
                // decoded buffer (base64 expands 3 bytes to 4 chars).
                if encoded.len() > UPLOAD_CHUNK_BYTES.div_ceil(3) * 4 {
                    return Err(format!(
                        "Upload chunk exceeds {UPLOAD_CHUNK_BYTES} bytes"
                    ));
                }
                let data = BASE64_STANDARD
                    .decode(encoded)
                    .map_err(|_| "Invalid dataBase64".to_string())?;
                if data.len() > UPLOAD_CHUNK_BYTES {
                    return Err(format!(
                        "Upload chunk exceeds {UPLOAD_CHUNK_BYTES} bytes"
                    ));
                }
                let received = self.upload_registry()?.chunk(upload_id, offset, &data)?;
                Ok(json!({ "received": received }))
            }

            "docker/files/upload/finish" => {
                if !is_local_target(&params)? {
                    return Err(
                        "docker/files/upload/finish is only available for local targets"
                            .to_string(),
                    );
                }
                let upload_id = required_string(&params, "uploadId")?;
                let (staging_path, size) = self.upload_registry()?.finish(upload_id)?;
                Ok(json!({ "stagingPath": staging_path, "size": size }))
            }

            "docker/files/upload/cancel" => {
                if !is_local_target(&params)? {
                    return Err(
                        "docker/files/upload/cancel is only available for local targets"
                            .to_string(),
                    );
                }
                let upload_id = required_string(&params, "uploadId")?;
                self.upload_registry()?.cancel(upload_id)?;
                Ok(json!({ "canceled": true }))
            }

            _ => Err(format!("Method not found: {method}")),
        }
    }

    /// Locks the shared upload registry (poisoning is a hard error).
    fn upload_registry(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, UploadRegistry>, String> {
        self.docker_uploads
            .lock()
            .map_err(|_| "Upload registry is poisoned".to_string())
    }

    /// Drops the upload entry that produced `staging_path` once its file was
    /// cleaned up. Best-effort: local export staging paths (not uploads)
    /// simply match nothing.
    fn forget_upload(&self, staging_path: &str) {
        if let Ok(mut registry) = self.upload_registry() {
            registry.remove_by_staging_path(staging_path);
        }
    }

    /// Runs one local container CLI invocation (argv only).
    fn run_local_args(
        &self,
        engine: &EngineConfig,
        argv: &[String],
        timeout: Duration,
    ) -> Result<(i32, String), String> {
        let cli = find_local_cli(&engine.cli)
            .ok_or_else(|| format!("{} CLI not found on this machine", engine.cli))?;
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.runtime.block_on(run_local(
            &cli,
            &args,
            timeout,
            engine.endpoint.as_deref(),
        ))
    }

    /// Runs one SSH command and flattens the response into `(exitCode, output)`.
    fn ssh_output(
        &self,
        session_id: &str,
        command: &str,
        timeout: Duration,
    ) -> Result<(i32, String), String> {
        let response = self.runtime.block_on(self.ssh.exec(
            session_id,
            None,
            command,
            false,
            Some(timeout.as_secs()),
        ))?;
        Ok((
            response.get("exitCode").and_then(Value::as_i64).unwrap_or(-1) as i32,
            response
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ))
    }

    /// `stat` inside a remote container, `None` when the tool is missing.
    fn remote_stat(
        &self,
        session_id: &str,
        engine: &EngineConfig,
        container_id: &str,
        path: &str,
    ) -> Result<Option<crate::features::docker::ContainerStat>, String> {
        let (command, _) = stat_command(engine, container_id, path)?;
        let (exit_code, output) = self.ssh_output(session_id, &command, FILES_TIMEOUT)?;
        if exit_code != 0 {
            if is_missing_tool(&output, "stat") {
                return Ok(None);
            }
            return Err(format!(
                "{} files/stat failed (exit {exit_code}): {output}",
                engine.cli
            ));
        }
        Ok(parse_stat_output(&output))
    }

    /// `stat` against the local daemon, `None` when the tool is missing.
    fn local_stat(
        &self,
        engine: &EngineConfig,
        container_id: &str,
        path: &str,
    ) -> Result<Option<crate::features::docker::ContainerStat>, String> {
        let (_, argv) = stat_command(engine, container_id, path)?;
        let (exit_code, output) = self.run_local_args(engine, &argv, FILES_TIMEOUT)?;
        if exit_code != 0 {
            if is_missing_tool(&output, "stat") {
                return Ok(None);
            }
            return Err(format!(
                "{} files/stat failed (exit {exit_code}): {output}",
                engine.cli
            ));
        }
        Ok(parse_stat_output(&output))
    }

    /// Best-effort removal of a generated remote staging directory (also
    /// covers non-empty dirs and symlinks a failed `docker cp` may have left).
    fn cleanup_remote_staging(&self, session_id: &str, staging_dir: &str) {
        if let Ok(command) = cleanup_command(staging_dir) {
            let _ = self.ssh_output(session_id, &command, FILES_TIMEOUT);
        }
    }

    /// `stat -c '%F|%s'` on the SSH host for a staging path, rejecting
    /// anything that is not a plain file. `docker cp` preserves symlinks, so
    /// the type must be checked after staging (and before import, where a
    /// link would be copied into the container). Returns the real size.
    fn remote_staging_regular_file(
        &self,
        session_id: &str,
        staging: &str,
    ) -> Result<u64, String> {
        let command = staging_probe_command(staging)?;
        let (exit_code, output) = self.ssh_output(session_id, &command, FILES_TIMEOUT)?;
        if exit_code != 0 {
            return Err(format!(
                "Failed to inspect staging file '{staging}': {}",
                output.trim()
            ));
        }
        match parse_staging_probe(&output) {
            Some((file_type, size)) if is_regular_file_type(&file_type) => Ok(size),
            Some((file_type, _)) => Err(format!(
                "Staging path is not a regular file ({file_type})"
            )),
            None => Err(format!("Failed to inspect staging file '{staging}'")),
        }
    }
}

/// Whether the call addresses the sidecar machine (`target: "local"`).
fn is_local_target(params: &Value) -> Result<bool, String> {
    Ok(crate::features::docker::parse_target(params)? == DockerTarget::Local)
}

/// `path` parameter: absent/null means the container root; any other
/// non-string value is a caller error instead of a silent `/` fallback.
fn container_path_param(params: &Value) -> Result<&str, String> {
    match params.get("path") {
        None | Some(Value::Null) => Ok("/"),
        Some(Value::String(value)) => Ok(value),
        Some(_) => Err("path must be a string".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_paths_default_to_root_but_reject_non_strings() {
        assert_eq!(container_path_param(&serde_json::json!({})).unwrap(), "/");
        assert_eq!(
            container_path_param(&serde_json::json!({ "path": null })).unwrap(),
            "/"
        );
        assert_eq!(
            container_path_param(&serde_json::json!({ "path": "/var/log" })).unwrap(),
            "/var/log"
        );
        assert!(container_path_param(&serde_json::json!({ "path": 7 })).is_err());
        assert!(container_path_param(&serde_json::json!({ "path": ["/etc"] })).is_err());
    }
}
