//! Extended SFTP operation layer.

use super::*;

/// `sftp/stat` — metadata for one remote path. Uses lstat semantics so
/// symlinks are reported as `symlink`, matching the `sftp/list` entries.
///
/// 路径来源分工（M17 增量③迁移）：前端传来的整条 wire 路径（`pathFromUri`）。
/// latin-1 模式整条按 [`sftp_name::unescape_wire`] 还原后走裸包 LSTAT
/// （此前高层客户端按字面量发送，转义名探不到）。裸包 attrs 没有 uid/gid，
/// 属主/属组仍走 `stat -c` shell 查询尽力而为——转义名下 shell 字节参数
/// 不可控（登记边界），查询失败回退 numeric/`-`，元数据主体不受影响。
/// 裸包客户端**建立**失败回退高层路径（M15 先例）。
pub async fn stat(
    runtime: &SshRuntime,
    session_id: &str,
    path: &str,
    encoding: NameEncoding,
) -> Result<Value, String> {
    let path = normalize_remote_path(path)?;
    if encoding == NameEncoding::Latin1 {
        match runtime.raw_sftp_client(session_id).await {
            Ok(mut client) => {
                let attrs = client
                    .lstat(&sftp_name::unescape_wire(&path))
                    .await
                    .map_err(|error| format!("SFTP stat failed: {error}"))?;
                let (owner_name, group_name) =
                    lookup_owner_group_names(runtime, session_id, &path).await;
                return Ok(json!({
                    "path": path,
                    "kind": crate::sessions::ssh::classify_raw_kind(attrs.permissions),
                    "size": attrs.size,
                    "modifiedAt": attrs.mtime.map(u64::from),
                    "mode": attrs.permissions.map(format_permissions),
                    // 裸包 v3 attrs 不携带 uid/gid：numeric 兜底缺位，属主列
                    // 以 shell 查询名为准（查不到显示 "-"，见上边界说明）。
                    "owner": owner_name.clone(),
                    "group": group_name.clone(),
                    "ownerName": owner_name,
                    "ownerUid": Option::<u32>::None,
                    "groupName": group_name,
                    "groupGid": Option::<u32>::None,
                }));
            }
            Err(error) => {
                eprintln!("[dbx-plugin-sshx] raw byte stat unavailable, falling back: {error}");
            }
        }
    }
    let sftp = runtime.sftp(session_id).await?;
    let metadata = sftp
        .lock()
        .await
        .symlink_metadata(path.clone())
        .await
        .map_err(|error| format!("SFTP stat failed: {error}"))?;
    let kind = match metadata.file_type() {
        FileType::File => "file",
        FileType::Dir => "directory",
        FileType::Symlink => "symlink",
        FileType::Other => "other",
    };
    // SFTP 协议默认只返回 uid/gid 数字，user/group 字段为 None。
    // 如果 russh-sftp 没拿到名字，远程跑 `stat -c '%U %G'` 查。
    let (owner_name, group_name) = match (metadata.user.clone(), metadata.group.clone()) {
        (Some(u), Some(g)) if !u.is_empty() && !g.is_empty() => (Some(u), Some(g)),
        _ => lookup_owner_group_names(runtime, session_id, &path).await,
    };
    let owner_display = owner_name
        .clone()
        .or_else(|| metadata.uid.map(|u| u.to_string()));
    let group_display = group_name
        .clone()
        .or_else(|| metadata.gid.map(|g| g.to_string()));
    Ok(json!({
        "path": path,
        "kind": kind,
        "size": metadata.size,
        "modifiedAt": metadata.mtime.map(u64::from),
        "mode": metadata.permissions.map(format_mode),
        "owner": owner_display,
        "group": group_display,
        "ownerName": owner_name,
        "ownerUid": metadata.uid,
        "groupName": group_name,
        "groupGid": metadata.gid,
    }))
}

/// `sftp/exists` — `true` when the path is statable (dangling symlinks count).
///
/// 错误语义（M23/R1，对齐 MCP 面 `mcp.rs::raw_sftp_exists` 的 M18 契约）：
/// 只把 SSH_FX_NO_SUCH_FILE 判「不存在」，其余 LSTAT 失败（权限拒绝、通道
/// 异常等）如实上抛——绝不误报 `exists: false`。既有调用方（前端 rename
/// 覆盖预检、粘贴预检、上传撞名预检）对预检报错均按「无法判定、不阻断，
/// 交由后续执行时报错」的既有惯例处理，预检无法判定时报错优于误判。
///
/// 路径来源分工（M16，M17 补全）：调用方分两类——
/// - rename 覆盖预检、上传撞名预检：`whole_wire = false`，路径是「wire 目录
///   前缀 + 用户新输入的显示末段」，latin-1 模式按 [`sftp_name::
///   write_path_bytes`] 还原服务器字节（前缀 `%XX` 还原、末段按显示文本
///   latin-1 编码）；
/// - 粘贴预检（M17 起）：`whole_wire = true`，整条路径都是列表回传的 wire
///   形式，latin-1 模式整条按 [`sftp_name::unescape_wire`] 还原（M16 遗留①：
///   此前整条 wire 名末段被按显示文本编码，非 UTF-8 名探不到）。
///
/// 两种形式 latin-1 下都走裸包 LSTAT；裸包客户端**建立**失败回退高层路径
/// （M15 先例）。
pub async fn exists(
    runtime: &SshRuntime,
    session_id: &str,
    path: &str,
    encoding: NameEncoding,
    whole_wire: bool,
) -> Result<bool, String> {
    let path = normalize_remote_path(path)?;
    if encoding == NameEncoding::Latin1 {
        match runtime.raw_sftp_client(session_id).await {
            Ok(mut client) => {
                let raw_path = if whole_wire {
                    sftp_name::unescape_wire(&path)
                } else {
                    sftp_name::write_path_bytes(&path)
                };
                return raw_exists_decision(client.lstat(&raw_path).await);
            }
            Err(error) => {
                eprintln!("[dbx-plugin-sshx] raw byte exists unavailable, falling back: {error}");
            }
        }
    }
    let sftp = runtime.sftp(session_id).await?;
    let outcome = sftp.lock().await.symlink_metadata(path).await;
    match outcome {
        Ok(_) => Ok(true),
        Err(error) => high_level_exists_decision(&error),
    }
}

/// Decision core of the latin-1 raw exists probe: only SSH_FX_NO_SUCH_FILE
/// counts as absent, every other LSTAT failure surfaces as an error
/// (M23/R1, same contract as `mcp.rs::raw_sftp_exists` — a permission error
/// must never masquerade as `exists: false`).
pub(crate) fn raw_exists_decision(result: Result<RawAttrs, String>) -> Result<bool, String> {
    match result {
        Ok(_) => Ok(true),
        Err(error) => match sftp_raw::error_status(&error) {
            Some(sftp_raw::SSH_FX_NO_SUCH_FILE) => Ok(false),
            _ => Err(error),
        },
    }
}

/// Decision core of the auto (high-level) exists probe: same
/// NO_SUCH_FILE-only contract as [`raw_exists_decision`] and the MCP auto
/// branch (mirror of `mcp.rs::is_no_such_file_error`).
pub(crate) fn high_level_exists_decision(
    error: &russh_sftp::client::error::Error,
) -> Result<bool, String> {
    if matches!(
        error,
        russh_sftp::client::error::Error::Status(status)
            if status.status_code == russh_sftp::protocol::StatusCode::NoSuchFile
    ) {
        Ok(false)
    } else {
        Err(sftp_error(error))
    }
}

/// `sftp/rename-unique` — suggests a non-conflicting file name inside `dir`
/// for an upload about to land there. `name` itself wins when free, otherwise
/// `name(1)` .. `name(999)` are probed (the `(n)` is inserted before the last
/// extension: `report.pdf` -> `report(1).pdf`). Returns `{name, conflict}`.
///
/// `sudo`（UploadSudo 配套）：目标目录对登录用户不可读（如 /root）时，SFTP
/// 探测全部失败会退化成原名覆盖；改走 sudo `test -e` 逐候选探测，撞名让位
/// 语义与普通车道一致。latin-1 非 UTF-8 字节名是该车道登记边界（exec 命令
/// 串是 UTF-8 String）。
///
/// 路径来源分工（M16）：`dir` 是列表回传的 wire 形式（latin-1 下整条按
/// `%XX` 还原），`name` 是上传文件的新输入显示文本（latin-1 下连同 `(n)`
/// 候选一起按 [`sftp_name::latin1_encode_display`] 编码回字节）。候选探测
/// 走裸包 LSTAT；裸包客户端**建立**失败回退高层探测路径（M15 先例）。返回
/// 的 `name` 保持显示形式——前端把它拼回上传路径后，落盘侧 [`write_path_bytes`]
/// 的分工（wire 前缀 + 显示末段）会编码出同一组服务器字节。
pub async fn rename_unique(
    runtime: &SshRuntime,
    session_id: &str,
    dir: &str,
    name: &str,
    encoding: NameEncoding,
    sudo: bool,
) -> Result<Value, String> {
    let dir = normalize_remote_path(dir)?;
    let clean = clean_unique_name(name)?;
    if sudo {
        for (index, candidate) in unique_name_candidates(&clean).into_iter().enumerate() {
            let full = join_remote_name(&dir, &candidate);
            if !crate::features::security::sudo::fs::exists(runtime, session_id, &full).await? {
                return Ok(json!({ "name": candidate, "conflict": index > 0 }));
            }
        }
        return Err(format!(
            "No unique name derived from '{clean}' within {UNIQUE_NAME_PROBE_LIMIT} attempts"
        ));
    }
    if encoding == NameEncoding::Latin1 {
        match runtime.raw_sftp_client(session_id).await {
            Ok(mut client) => {
                for (index, (candidate, raw)) in unique_name_raw_candidates(&dir, name)?
                    .into_iter()
                    .enumerate()
                {
                    // NO_SUCH_FILE-only 契约（同 raw_exists_decision）：把任何
                    // lstat 错误当"目标不存在"会在链路抖动下让原名直接撞上
                    // 既有文件、上传走 CREAT|TRUNC 静默覆盖。
                    if raw_exists_decision(client.lstat(&raw).await)? {
                        return Ok(json!({ "name": candidate, "conflict": index > 0 }));
                    }
                }
                return Err(format!(
                    "No unique name derived from '{clean}' within {UNIQUE_NAME_PROBE_LIMIT} attempts"
                ));
            }
            Err(error) => {
                eprintln!(
                    "[dbx-plugin-sshx] raw byte rename-unique unavailable, falling back: {error}"
                );
            }
        }
    }
    let sftp = runtime.sftp(session_id).await?;
    let session = sftp.lock().await;
    let probe = |candidate: &str| {
        let full = join_remote_name(&dir, candidate);
        session.symlink_metadata(full)
    };
    for (index, candidate) in unique_name_candidates(&clean).into_iter().enumerate() {
        let absent = match probe(&candidate).await {
            Ok(_) => false,
            Err(error) => high_level_exists_decision(&error)?,
        };
        if absent {
            return Ok(json!({ "name": candidate, "conflict": index > 0 }));
        }
    }
    Err(format!(
        "No unique name derived from '{clean}' within {UNIQUE_NAME_PROBE_LIMIT} attempts"
    ))
}

/// `sftp/touch` — creates an empty file when missing, otherwise refreshes
/// mtime/atime. Servers that reject SETSTAT times make the refresh a no-op,
/// mirroring tiny-rdm's create-only `Touch`.
///
/// 路径来源分工（M16）：`path` 是「wire 目录前缀 + 用户新输入的显示文件名」
/// （前端 `joinRemote(currentPath, name)`）。latin-1 模式按
/// [`sftp_name::write_path_bytes`] 还原服务器字节后走裸包 LSTAT/SETSTAT/
/// OPEN(creat|write|trunc)；裸包客户端**建立**失败回退高层路径。
pub async fn touch(
    runtime: &SshRuntime,
    session_id: &str,
    path: &str,
    encoding: NameEncoding,
) -> Result<(), String> {
    runtime.ensure_writable(session_id).await?;
    let path = normalize_remote_path(path)?;
    if encoding == NameEncoding::Latin1 {
        match runtime.raw_sftp_client(session_id).await {
            Ok(mut client) => {
                let raw_path = sftp_name::write_path_bytes(&path);
                if client.lstat(&raw_path).await.is_ok() {
                    // 时间刷新是尽力而为：SETSTAT 被拒与高层语义一致地容忍。
                    let now = current_unix_secs();
                    let _ = client
                        .setstat(
                            &raw_path,
                            &RawAttrs {
                                atime: Some(now),
                                mtime: Some(now),
                                ..RawAttrs::default()
                            },
                        )
                        .await;
                    return Ok(());
                }
                let handle = client.open_write(&raw_path).await?;
                return client.close(&handle).await;
            }
            Err(error) => {
                eprintln!("[dbx-plugin-sshx] raw byte touch unavailable, falling back: {error}");
            }
        }
    }
    let sftp = runtime.sftp(session_id).await?;
    let session = sftp.lock().await;
    if session.symlink_metadata(path.clone()).await.is_ok() {
        let now = current_unix_secs();
        let times = FileAttributes {
            atime: Some(now),
            mtime: Some(now),
            ..FileAttributes::default()
        };
        // Unsupported (or unpermitted) utime is not an error for touch.
        let _ = session.set_metadata(path, times).await;
        return Ok(());
    }
    let file = session
        .open_with_flags(path, OpenFlags::CREATE | OpenFlags::WRITE)
        .await
        .map_err(sftp_error)?;
    drop(file);
    Ok(())
}
