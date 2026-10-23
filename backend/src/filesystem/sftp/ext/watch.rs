//! Extended SFTP operation layer.

use super::*;

// `sftp/upload-local {sessionId, localPath, remotePath} -> {path, size}` —
/// Path-origin gate for the external-editor round-trip: only files this
/// plugin downloaded into `<downloads>/remote-edit/` may cross the bridge.
/// `pub(crate)` because `file_watch` reuses it for `watch/start` and
/// `watch/upload` — the watchId is a bearer token, the real boundary is the
/// path origin.
pub(crate) fn validate_remote_edit_path(
    local_path: &Path,
    data_dir: &Path,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf, String> {
    // 与前端下载目标同源（downloadDir 偏好优先）：自定义下载目录时若回落
    // env/USERPROFILE 基址，remote-edit 根在该基址下不存在，打开/回传校验
    // 全部必然失败（#175）。
    let downloads = crate::filesystem::local::downloads::effective_downloads_base(lookup, data_dir);
    validate_remote_edit_root(local_path, &downloads)
}

/// The prefix check with a pinned downloads base — the seam `file_watch`
/// tests use so validation does not depend on the runner's real Downloads
/// directory. Production callers go through [`validate_remote_edit_path`].
pub(crate) fn validate_remote_edit_root(
    local_path: &Path,
    downloads: &Path,
) -> Result<PathBuf, String> {
    if !local_path.is_absolute() {
        return Err("localPath must be an absolute path".to_string());
    }
    let canonical = local_path.canonicalize().map_err(|error| {
        format!(
            "Local file '{}' does not exist: {error}",
            local_path.display()
        )
    })?;
    let root = downloads.join("remote-edit");
    let root = root
        .canonicalize()
        .map_err(|_| "Local file is not inside the remote-edit directory".to_string())?;
    if !canonical.starts_with(&root) {
        return Err("Only files inside the remote-edit directory can be uploaded".to_string());
    }
    Ok(canonical)
}

pub async fn upload_watched_file(
    runtime: &SshRuntime,
    session_id: &str,
    local_path: &str,
    remote_path: &str,
    encoding: NameEncoding,
) -> Result<Value, String> {
    runtime.ensure_writable(session_id).await?;
    let local =
        validate_remote_edit_path(Path::new(local_path.trim()), &runtime.data_dir(), |key| {
            std::env::var_os(key)
        })?;
    let size = tokio::fs::metadata(&local)
        .await
        .map_err(|error| format!("Local file '{}' is unreadable: {error}", local.display()))?
        .len();
    if size > MAX_UPLOAD_LOCAL_SIZE {
        return Err(format!(
            "Remote-edit uploads are limited to {MAX_UPLOAD_LOCAL_SIZE} bytes"
        ));
    }
    let remote_path = normalize_remote_path(remote_path)?;
    // 路径来源分工（M16）：remotePath 是 watcher 登记的整条 wire 形式（列表
    // 回传），latin-1 模式整条还原字节后走裸包暂存 + 原子提交。
    if encoding == NameEncoding::Latin1 {
        match runtime.raw_sftp_client(session_id).await {
            Ok(mut client) => {
                let raw_target = sftp_name::unescape_wire(&remote_path);
                let task_id = Uuid::new_v4().to_string();
                let (raw_temporary, raw_backup) = raw_paths_next_to(
                    &raw_target,
                    &format!(".dbx-part-{task_id}"),
                    &format!(".dbx-part-{task_id}.backup"),
                );
                push_local_file_raw(&mut client, &local, &raw_temporary, UPLOAD_LOCAL_CHUNK)
                    .await?;
                raw_commit_file(&mut client, &raw_temporary, &raw_target, &raw_backup).await?;
                return Ok(json!({ "path": remote_path, "size": size }));
            }
            Err(error) => {
                eprintln!(
                    "[dbx-plugin-sshx] raw byte upload-local unavailable, falling back: {error}"
                );
            }
        }
    }
    let sftp = runtime.sftp(session_id).await?;
    let task_id = Uuid::new_v4().to_string();
    let (temporary, backup) = direct_write_paths(&remote_path, &task_id);
    {
        let session = sftp.lock().await;
        let mut file = session
            .create(temporary.clone())
            .await
            .map_err(sftp_error)?;
        let mut reader = match tokio::fs::File::open(&local).await {
            Ok(reader) => reader,
            Err(error) => {
                let _ = session.remove_file(temporary.clone()).await;
                return Err(format!(
                    "Local file '{}' is unreadable: {error}",
                    local.display()
                ));
            }
        };
        let mut buffer = vec![0_u8; UPLOAD_LOCAL_CHUNK];
        loop {
            let read = match reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) => {
                    drop(file);
                    let _ = session.remove_file(temporary.clone()).await;
                    return Err(format!(
                        "Local file '{}' read failed: {error}",
                        local.display()
                    ));
                }
            };
            if let Err(error) = file.write_all(&buffer[..read]).await {
                drop(file);
                let _ = session.remove_file(temporary.clone()).await;
                return Err(format!("SFTP write failed: {error}"));
            }
        }
        if let Err(error) = file.flush().await {
            drop(file);
            let _ = session.remove_file(temporary.clone()).await;
            return Err(format!("SFTP write flush failed: {error}"));
        }
    }
    commit_temporary_file(&sftp, &temporary, &remote_path, &backup).await?;
    Ok(json!({ "path": remote_path, "size": size }))
}

/// 裸包流式推一个本地文件到远端暂存路径：CREATE|WRITE|TRUNC 打开 → 按
/// `read_chunk_size` 读本地 → 按 [`RAW_WRITE_CHUNK`] 分块 WRITE → CLOSE。
/// 任何失败都会清理远端暂存文件。
pub(crate) async fn push_local_file_raw(
    client: &mut RawSftpClient,
    local_path: &Path,
    raw_temporary: &[u8],
    read_chunk_size: usize,
) -> Result<(), String> {
    let mut reader = tokio::fs::File::open(local_path).await.map_err(|error| {
        format!(
            "Local file '{}' is unreadable: {error}",
            local_path.display()
        )
    })?;
    let handle = client.open_write(raw_temporary).await?;
    let mut buffer = vec![0_u8; read_chunk_size];
    let result: Result<(), String> = async {
        loop {
            let read = reader
                .read(&mut buffer)
                .await
                .map_err(|error| format!("Local file read failed: {error}"))?;
            if read == 0 {
                break;
            }
            raw_write_all(client, &handle, &buffer[..read]).await?;
        }
        client.close(&handle).await
    }
    .await;
    if let Err(error) = result {
        let _ = client.close(&handle).await;
        let _ = client.remove(raw_temporary).await;
        return Err(error);
    }
    Ok(())
}
