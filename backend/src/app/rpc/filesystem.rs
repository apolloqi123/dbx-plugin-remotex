//! Host filesystem provider, file watches, and the binary watch publisher.

use super::*;

/// Forwards confirmed watcher changes to the host as a `watch/file-modified`
/// event, channel shape matching the other sidecar state broadcasts
/// (`local/session/state`, `ssh/forward/state`). Emission failures are logged
/// and otherwise ignored — the registry stays authoritative.
struct WatchEventPublisher(PluginEmitter);

impl crate::filesystem::watch::EventPublisher for WatchEventPublisher {
    fn publish(&self, payload: Value) {
        if let Err(error) = self.0.event("watch/file-modified", payload) {
            eprintln!(
                "[dbx-plugin-sshx] watch file-modified event failed: {}",
                error.message
            );
        }
    }
}

impl Plugin {
    pub(super) fn filesystem_session(&self, params: &Value) -> Result<String, String> {
        if let Some(session_id) = params.get("sessionId").and_then(Value::as_str) {
            return Ok(session_id.to_string());
        }
        let connection_id = connection_id_param(params)?;
        self.runtime
            .block_on(self.ssh.session_id_for_connection(connection_id))
    }

    pub(super) fn filesystem_list(&self, params: Value) -> Result<Value, String> {
        let session_id = self.filesystem_session(&params)?;
        let path = filesystem_path(&params)?;
        // Host filesystem-provider listings stay on the zero-round-trip path;
        // owner names are opt-in via `sftp/list` only.
        // 宿主 filesystem-provider 列表固定 auto：编码容错只面向 SFTP 面板。
        let entries = self.runtime.block_on(self.ssh.sftp_list_path(
            &session_id,
            &path,
            false,
            crate::filesystem::sftp::name::NameEncoding::Auto,
        ))?;
        Ok(json!({ "entries": entries }))
    }

    pub(super) fn filesystem_read(&self, params: Value) -> Result<Value, String> {
        let session_id = self.filesystem_session(&params)?;
        let path = filesystem_path(&params)?;
        let offset = optional_u64(&params, "offset", 0);
        let max_bytes = bounded_bytes(&params, "maxBytes", 256 * 1024);
        let (data, truncated) = self.runtime.block_on(self.ssh.sftp_read_path(
            &session_id,
            &path,
            offset,
            max_bytes,
        ))?;
        Ok(json!({
            "dataBase64": BASE64_STANDARD.encode(data),
            "contentType": content_type(&path),
            "truncated": truncated
        }))
    }

    pub(super) fn filesystem_write(&self, params: Value) -> Result<Value, String> {
        let session_id = self.filesystem_session(&params)?;
        let path = filesystem_path(&params)?;
        let data = BASE64_STANDARD
            .decode(required_string(&params, "dataBase64")?)
            .map_err(|error| format!("Invalid base64 file data: {error}"))?;
        let create = params
            .get("create")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let overwrite = params
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.runtime.block_on(self.ssh.sftp_write_path(
            &session_id,
            &path,
            &data,
            create,
            overwrite,
        ))?;
        Ok(json!({ "success": true }))
    }

    pub(super) fn filesystem_create_directory(&self, params: Value) -> Result<Value, String> {
        let session_id = self.filesystem_session(&params)?;
        let path = filesystem_path(&params)?;
        // 宿主 filesystem-provider 固定 auto：编码容错只面向 SFTP 面板。
        self.runtime.block_on(self.ssh.sftp_create_directory(
            &session_id,
            &path,
            crate::filesystem::sftp::name::NameEncoding::Auto,
        ))?;
        Ok(json!({ "success": true }))
    }

    pub(super) fn filesystem_delete(&self, params: Value) -> Result<Value, String> {
        let session_id = self.filesystem_session(&params)?;
        let path = filesystem_path(&params)?;
        let recursive = params
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.runtime.block_on(self.ssh.sftp_delete(
            &session_id,
            &path,
            recursive,
            crate::filesystem::sftp::name::NameEncoding::Auto,
        ))?;
        Ok(json!({ "success": true }))
    }

    pub(super) fn filesystem_rename(&self, params: Value) -> Result<Value, String> {
        let session_id = self.filesystem_session(&params)?;
        let source = required_string(&params, "sourceUri").and_then(path_from_sftp_uri)?;
        let target = required_string(&params, "targetUri").and_then(path_from_sftp_uri)?;
        self.runtime.block_on(self.ssh.sftp_rename(
            &session_id,
            &source,
            &target,
            crate::filesystem::sftp::name::NameEncoding::Auto,
        ))?;
        Ok(json!({ "success": true }))
    }

    pub(super) fn handle_filesystem(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            // 外部编辑器回传（仅桌面端）：前端先用 sftp/download 把文件落到
            // 本地 remote-edit 目录，这里只注册监听；确认内容真变后经
            // watch/file-modified 事件推回工作台。
            "watch/start" => {
                let request: crate::filesystem::watch::WatchStartRequest = parse(params)?;
                self.runtime.block_on(self.watcher.start(
                    request,
                    Arc::new(WatchEventPublisher(emitter.clone())),
                    self.session_probe(),
                    &self.ssh.data_dir(),
                ))
            }

            "watch/stop" => {
                let watch_id = required_string(&params, "watchId")?;
                self.runtime.block_on(self.watcher.stop(watch_id))?;
                Ok(json!({ "success": true }))
            }

            "watch/stop-all" => {
                let session_id = required_string(&params, "sessionId")?;
                let stopped = self.runtime.block_on(self.watcher.stop_session(session_id));
                Ok(json!({ "success": true, "stopped": stopped }))
            }

            // 把被监听文件的当前磁盘字节推回远端：sidecar 从 remote-edit 下载
            // 路径读字节（宿主桥没有按路径读本地文件的能力），走 sftp/write
            // 同款原子落盘；写门禁 ensure_writable 与其他 SFTP 写完全一致。
            "watch/upload" => {
                let watch_id = required_string(&params, "watchId")?;
                let session_id = self
                    .runtime
                    .block_on(self.watcher.session_for_watch(watch_id));
                let encoding = self.resolve_sftp_encoding_opt(session_id.as_deref());
                self.runtime
                    .block_on(self.watcher.upload_back(&self.ssh, watch_id, encoding))
            }

            "filesystem/list" => self.filesystem_list(params),

            "filesystem/read" => self.filesystem_read(params),

            "filesystem/write" => self.filesystem_write(params),

            "filesystem/createDirectory" => self.filesystem_create_directory(params),

            "filesystem/delete" => self.filesystem_delete(params),

            "filesystem/rename" => self.filesystem_rename(params),
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
