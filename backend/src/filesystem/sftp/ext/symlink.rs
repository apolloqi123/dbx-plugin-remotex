//! Extended SFTP operation layer.

use super::*;

// ---------------------------------------------------------------------------
// Symlinks
// ---------------------------------------------------------------------------

/// `sftp/symlink-create` — creates `link_path` pointing at `target`.
/// `target` keeps its original form (absolute or relative): relative targets
/// are the normal way to build relocatable links, so normalizing it would
/// silently change what the link resolves to.
///
/// 路径来源分工（M16）：`link_path` 是「wire 目录前缀 + 用户新输入的显示
/// 链接名」（前端 `joinRemote(currentPath, name)`）——latin-1 模式按
/// [`sftp_name::write_path_bytes`] 还原字节；`target` 保持原文（相对链接
/// 语义），按其字符串字节发送。裸包 SYMLINK 按 OpenSSH wire 次序装包。
pub async fn symlink_create(
    runtime: &SshRuntime,
    session_id: &str,
    target: &str,
    link_path: &str,
    encoding: NameEncoding,
) -> Result<(), String> {
    runtime.ensure_writable(session_id).await?;
    let target = clean_link_target(target)?;
    let link_path = normalize_remote_path(link_path)?;
    if encoding == NameEncoding::Latin1 {
        match runtime.raw_sftp_client(session_id).await {
            Ok(mut client) => {
                return client
                    .symlink(target.as_bytes(), &sftp_name::write_path_bytes(&link_path))
                    .await;
            }
            Err(error) => {
                eprintln!(
                    "[dbx-plugin-sshx] raw byte symlink-create unavailable, falling back: {error}"
                );
            }
        }
    }
    let sftp = runtime.sftp(session_id).await?;
    // Spike result (russh-sftp 3.0.0, src/client/rawsession.rs:709 +
    // protocol/symlink.rs): the wire order is (linkpath, targetpath), which is
    // the *draft* order — but OpenSSH's server reads (target, linkpath) and
    // pkg/sftp ships the swap for exactly this reason (packet.go, "the order
    // ... was inadvertently reversed"). Calling with swapped arguments makes
    // the link land on `link_path` pointing at `target` on OpenSSH and every
    // OpenSSH-compatible sftp-server.
    sftp.lock()
        .await
        .symlink(target.clone(), link_path.clone())
        .await
        .map_err(|error| format!("SFTP symlink-create failed: {error}"))?;
    Ok(())
}

/// `sftp/symlink-read` — resolves `link_path` to its target string.
///
/// 路径来源分工（M16）：`link_path` 是列表回传的整条 wire 形式，latin-1
/// 模式整条 [`sftp_name::unescape_wire`] 还原后走裸包 READLINK，指向文本按
/// latin-1 显示解码（与列表名同一套解码语义）。前端把读到的指向回传给
/// [`symlink_update`] 时按显示文本编码回字节，读↔写在 latin-1 域内闭环。
pub async fn symlink_read(
    runtime: &SshRuntime,
    session_id: &str,
    link_path: &str,
    encoding: NameEncoding,
) -> Result<Value, String> {
    let link_path = normalize_remote_path(link_path)?;
    if encoding == NameEncoding::Latin1 {
        match runtime.raw_sftp_client(session_id).await {
            Ok(mut client) => {
                let raw_link = sftp_name::unescape_wire(&link_path);
                let raw_target = client
                    .readlink(&raw_link)
                    .await
                    .map_err(|error| format!("SFTP symlink-read failed: {error}"))?;
                let target = sftp_name::decode_display_name(&raw_target, NameEncoding::Latin1).text;
                return Ok(json!({ "linkPath": link_path, "target": target }));
            }
            Err(error) => {
                eprintln!(
                    "[dbx-plugin-sshx] raw byte symlink-read unavailable, falling back: {error}"
                );
            }
        }
    }
    let sftp = runtime.sftp(session_id).await?;
    let target = sftp
        .lock()
        .await
        .read_link(link_path.clone())
        .await
        .map_err(|error| format!("SFTP symlink-read failed: {error}"))?;
    Ok(json!({ "linkPath": link_path, "target": target }))
}

/// `sftp/symlink-update` — repoints an existing symlink. SFTP v3 has no
/// re-link primitive, so the update is read-compare, then remove + recreate;
/// when the requested target already matches the call is a no-op.
///
/// 路径来源分工（M16）：`link_path` 是列表回传的整条 wire 形式（latin-1 下
/// 整条还原字节）；`target` 是 [`symlink_read`] 解出的显示文本或用户新输入，
/// latin-1 下按 [`sftp_name::latin1_encode_display`] 编码回字节——与
/// [`symlink_read`] 构成闭环。
pub async fn symlink_update(
    runtime: &SshRuntime,
    session_id: &str,
    link_path: &str,
    target: &str,
    encoding: NameEncoding,
) -> Result<(), String> {
    runtime.ensure_writable(session_id).await?;
    let target = clean_link_target(target)?;
    let link_path = normalize_remote_path(link_path)?;
    if encoding == NameEncoding::Latin1 {
        match runtime.raw_sftp_client(session_id).await {
            Ok(mut client) => {
                let raw_link = sftp_name::unescape_wire(&link_path);
                let raw_target = sftp_name::latin1_encode_display(&target);
                if client
                    .readlink(&raw_link)
                    .await
                    .map(|current| current == raw_target)
                    .unwrap_or(false)
                {
                    return Ok(());
                }
                client.remove(&raw_link).await.map_err(|error| {
                    format!("SFTP symlink-update could not remove the old link: {error}")
                })?;
                return client
                    .symlink(&raw_target, &raw_link)
                    .await
                    .map_err(|error| format!("SFTP symlink-update failed: {error}"));
            }
            Err(error) => {
                eprintln!(
                    "[dbx-plugin-sshx] raw byte symlink-update unavailable, falling back: {error}"
                );
            }
        }
    }
    let sftp = runtime.sftp(session_id).await?;
    let session = sftp.lock().await;
    if session
        .read_link(link_path.clone())
        .await
        .map(|current| current == target)
        .unwrap_or(false)
    {
        return Ok(());
    }
    session
        .remove_file(link_path.clone())
        .await
        .map_err(|error| format!("SFTP symlink-update could not remove the old link: {error}"))?;
    // Same swapped-argument order as symlink_create (OpenSSH wire order).
    session
        .symlink(target, link_path)
        .await
        .map_err(|error| format!("SFTP symlink-update failed: {error}"))
}

/// Validates a symlink target: symlink targets travel verbatim inside the
/// SFTP SYMLINK packet (no shell involved), so only NUL and emptiness are
/// rejected; leading/trailing blanks are kept meaningful on purpose.
pub(crate) fn clean_link_target(target: &str) -> Result<String, String> {
    if target.is_empty() || target.contains('\0') {
        return Err("Symlink target is empty or invalid".to_string());
    }
    Ok(target.to_string())
}
