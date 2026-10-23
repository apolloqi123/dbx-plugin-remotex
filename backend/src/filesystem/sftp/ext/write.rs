//! Extended SFTP operation layer.

use super::*;

/// `sftp/write` — direct write for small files: base64 payload in, temporary
/// `.dbx-part-<uuid>` file out, atomic rename onto the target.
///
/// 路径来源分工（M16）：`sftp/write` 的 `remotePath` 来自前端对列表条目 uri
/// 的整条回传（`pathFromUri(entry.uri)`）与 watcher 的 remote-edit 目标——
/// 都是 wire 形式。latin-1 模式整条 [`sftp_name::unescape_wire`] 还原服务器
/// 字节（wire 的 `%` 自转义保证字面 `%XX` 名往返不变），不走末段显示编码。
pub async fn write_file(
    runtime: &SshRuntime,
    session_id: &str,
    path: &str,
    data_base64: &str,
    encoding: NameEncoding,
) -> Result<(), String> {
    let data = decode_direct_write_payload(data_base64)?;
    // write_bytes runs the write gate (ensure_writable) — no second check here.
    write_bytes(runtime, session_id, path, &data, encoding).await
}

/// In-memory variant of [`write_file`] shared with the watcher round-trip
/// (`watch/upload`): the bytes are already sidecar-resident, so no base64
/// detour and no 4 MiB direct-write cap applies here — the caller owns the
/// size policy. Stages through `.dbx-part-<uuid>` and renames atomically,
/// preserving the target's permission bits (issue #37).
pub async fn write_bytes(
    runtime: &SshRuntime,
    session_id: &str,
    path: &str,
    data: &[u8],
    encoding: NameEncoding,
) -> Result<(), String> {
    runtime.ensure_writable(session_id).await?;
    let path = normalize_remote_path(path)?;
    if encoding == NameEncoding::Latin1 {
        // 整条 wire 路径还原为服务器字节后走裸包暂存 + 原子提交；裸包客户端
        // 建立失败回退高层路径，操作发出后的失败原样上抛（M15 先例）。
        match runtime.raw_sftp_client(session_id).await {
            Ok(mut client) => {
                let raw_target = sftp_name::unescape_wire(&path);
                let task_id = Uuid::new_v4().to_string();
                let (raw_temporary, raw_backup) = raw_paths_next_to(
                    &raw_target,
                    &format!(".dbx-part-{task_id}"),
                    &format!(".dbx-part-{task_id}.backup"),
                );
                let handle = client.open_write(&raw_temporary).await?;
                if let Err(error) = raw_write_all(&mut client, &handle, data).await {
                    drop(handle);
                    let _ = client.remove(&raw_temporary).await;
                    return Err(format!("SFTP write failed: {error}"));
                }
                if let Err(error) = client.close(&handle).await {
                    let _ = client.remove(&raw_temporary).await;
                    return Err(format!("SFTP write flush failed: {error}"));
                }
                return raw_commit_file(&mut client, &raw_temporary, &raw_target, &raw_backup)
                    .await;
            }
            Err(error) => {
                eprintln!("[dbx-plugin-sshx] raw byte write unavailable, falling back: {error}");
            }
        }
    }
    let sftp = runtime.sftp(session_id).await?;
    let task_id = Uuid::new_v4().to_string();
    let (temporary, backup) = direct_write_paths(&path, &task_id);
    {
        let session = sftp.lock().await;
        let mut file = session
            .create(temporary.clone())
            .await
            .map_err(sftp_error)?;
        if let Err(error) = file.write_all(data).await {
            drop(file);
            let _ = session.remove_file(temporary.clone()).await;
            return Err(format!("SFTP write failed: {error}"));
        }
        if let Err(error) = file.flush().await {
            drop(file);
            let _ = session.remove_file(temporary.clone()).await;
            return Err(format!("SFTP write flush failed: {error}"));
        }
    }
    commit_temporary_file(&sftp, &temporary, &path, &backup).await
}

/// Decodes a base64 payload and enforces the direct-write size cap.
pub(crate) fn decode_direct_write_payload(data_base64: &str) -> Result<Vec<u8>, String> {
    let data = BASE64_STANDARD
        .decode(data_base64)
        .map_err(|error| format!("Invalid base64 file data: {error}"))?;
    ensure_direct_write_size(data.len())?;
    Ok(data)
}

pub(crate) fn ensure_direct_write_size(size: usize) -> Result<(), String> {
    if size > MAX_DIRECT_WRITE_SIZE {
        Err(format!(
            "Direct SFTP writes are limited to {MAX_DIRECT_WRITE_SIZE} bytes; use the streaming upload slot (sftp/upload/start) instead"
        ))
    } else {
        Ok(())
    }
}

/// Temporary and backup names placed next to the write target, mirroring the
/// `.dbx-upload-<task>.part` pattern of the streaming upload path.
pub(crate) fn direct_write_paths(target: &str, task_id: &str) -> (String, String) {
    let (parent, _) = target.rsplit_once('/').unwrap_or(("/", ""));
    let parent = if parent.is_empty() { "/" } else { parent };
    let base = format!("{}/.dbx-part-{task_id}", parent.trim_end_matches('/'));
    (base.clone(), format!("{base}.backup"))
}

/// Renames a finished temporary file onto its target; an existing target is
/// moved aside first and restored if the rename fails. The target's
/// permission bits ride along onto the staged file, so an overwritten script
/// keeps its executable bit (issue #37).
pub(crate) async fn commit_temporary_file(
    sftp: &Arc<AsyncMutex<SftpSession>>,
    temporary: &str,
    target: &str,
    backup: &str,
) -> Result<(), String> {
    let target_attributes = sftp.lock().await.metadata(target.to_string()).await.ok();
    apply_preserved_permissions(sftp, temporary, target_attributes.as_ref()).await?;
    let target_exists = target_attributes.is_some();
    if target_exists {
        sftp.lock()
            .await
            .rename(target.to_string(), backup.to_string())
            .await
            .map_err(sftp_error)?;
    }
    if let Err(error) = sftp
        .lock()
        .await
        .rename(temporary.to_string(), target.to_string())
        .await
    {
        if target_exists {
            let _ = sftp
                .lock()
                .await
                .rename(backup.to_string(), target.to_string())
                .await;
        }
        let _ = sftp.lock().await.remove_file(temporary.to_string()).await;
        return Err(sftp_error(error));
    }
    if target_exists {
        let _ = sftp.lock().await.remove_file(backup.to_string()).await;
    }
    Ok(())
}

/// 在 `target`（服务器原始字节）旁边取一对暂存路径字节：ASCII 暂存名由
/// 调用方给定（write 族 `.dbx-part-<uuid>` / 上传族 `.dbx-upload-<task>.part`，
/// 与高层 [`direct_write_paths`] / ssh.rs `remote_transfer_paths` 命名对齐）。
/// 目录前缀直接复用目标字节——目录段已在调用方按 wire 还原，这里只做拼接。
pub(crate) fn raw_paths_next_to(
    target: &[u8],
    temp_name: &str,
    backup_name: &str,
) -> (Vec<u8>, Vec<u8>) {
    let parent: &[u8] = match target.iter().rposition(|&byte| byte == b'/') {
        Some(0) => b"/",
        Some(position) => &target[..position],
        None => b"/",
    };
    (
        sftp_name::join_raw_path(parent, temp_name.as_bytes()),
        sftp_name::join_raw_path(parent, backup_name.as_bytes()),
    )
}

/// 裸包分块写：按 [`RAW_WRITE_CHUNK`] 切分，offset 随写推进。
pub(crate) async fn raw_write_all(
    client: &mut RawSftpClient,
    handle: &[u8],
    mut data: &[u8],
) -> Result<(), String> {
    let mut offset = 0_u64;
    while !data.is_empty() {
        let chunk = &data[..data.len().min(RAW_WRITE_CHUNK)];
        client.write_chunk(handle, offset, chunk).await?;
        offset += chunk.len() as u64;
        data = &data[chunk.len()..];
    }
    Ok(())
}

/// 裸包原子提交（[`commit_temporary_file`] / ssh.rs `commit_remote_file` 的
/// 字节保真版）：STAT 目标取权限位 → SETSTAT 到暂存文件（保留 +x，issue #37）
/// → 目标已存在则先改名让位（失败回滚还原）→ 暂存改名落位 → 清理备份。
pub(crate) async fn raw_commit_file(
    client: &mut RawSftpClient,
    temporary: &[u8],
    target: &[u8],
    backup: &[u8],
) -> Result<(), String> {
    let target_attributes = client.stat(target).await.ok();
    if let Some(permissions) = target_attributes
        .as_ref()
        .and_then(|attrs| attrs.permissions)
    {
        if let Err(error) = client
            .setstat(
                temporary,
                &RawAttrs {
                    permissions: Some(permissions & 0o7777),
                    ..RawAttrs::default()
                },
            )
            .await
        {
            let _ = client.remove(temporary).await;
            return Err(format!(
                "SFTP save failed while preserving permissions {}: {error}; original file left unchanged",
                format_permissions(permissions),
            ));
        }
    }
    let target_exists = target_attributes.is_some();
    if target_exists {
        client
            .rename(target, backup)
            .await
            .map_err(|error| format!("SFTP operation failed: {error}"))?;
    }
    if let Err(error) = client.rename(temporary, target).await {
        if target_exists {
            let _ = client.rename(backup, target).await;
        }
        let _ = client.remove(temporary).await;
        return Err(format!("SFTP operation failed: {error}"));
    }
    if target_exists {
        let _ = client.remove(backup).await;
    }
    Ok(())
}

/// 裸包上传的取消/进度注入点（与高层 finish_upload 管线共享同一份状态）。
pub(crate) struct RawUploadContext<'a> {
    pub cancelled: &'a AtomicBool,
    pub cancel_reason: &'a std::sync::Mutex<Option<String>>,
    pub progress: Box<dyn FnMut(u64) -> Result<(), String> + Send + 'a>,
}

/// latin-1 裸包上传提交（`sftp/upload/finish` 专用）：本地 spool 文件 → raw
/// 临时文件（CREATE|WRITE|TRUNC，分块 WRITE）→ 原子 rename 落位。取消检查、
/// 进度回调与高层管线逐块对齐；取消/写失败时清理远端暂存文件。
/// `remote_path` 是上传族路径来源「wire 目录前缀 + 显示末段」，经
/// [`sftp_name::write_path_bytes`] 还原为服务器字节。
pub(crate) async fn raw_push_upload_file(
    client: &mut RawSftpClient,
    local_path: &Path,
    remote_path: &str,
    task_id: &str,
    read_chunk_size: usize,
    mut context: RawUploadContext<'_>,
) -> Result<(), String> {
    let raw_target = sftp_name::write_path_bytes(remote_path);
    let (raw_temporary, raw_backup) = raw_paths_next_to(
        &raw_target,
        &format!(".dbx-upload-{task_id}.part"),
        &format!(".dbx-upload-{task_id}.backup"),
    );
    let mut source = tokio::fs::File::open(local_path)
        .await
        .map_err(|error| format!("Failed to open upload spool file: {error}"))?;
    let handle = client.open_write(&raw_temporary).await?;
    let mut buffer = vec![0_u8; read_chunk_size];
    let mut transferred = 0_u64;
    let result: Result<(), String> = async {
        loop {
            if context.cancelled.load(Ordering::Acquire) {
                let reason = context
                    .cancel_reason
                    .lock()
                    .map(|reason| reason.clone())
                    .unwrap_or_default();
                return Err(crate::sessions::ssh::upload_cancel_error(reason.as_deref()));
            }
            let read = source
                .read(&mut buffer)
                .await
                .map_err(|error| format!("Failed to read upload spool file: {error}"))?;
            if read == 0 {
                break;
            }
            raw_write_all(client, &handle, &buffer[..read]).await?;
            transferred = transferred.saturating_add(read as u64);
            (context.progress)(transferred)?;
        }
        client.close(&handle).await?;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        let _ = client.close(&handle).await;
        let _ = client.remove(&raw_temporary).await;
        return Err(error);
    }
    raw_commit_file(client, &raw_temporary, &raw_target, &raw_backup).await
}
