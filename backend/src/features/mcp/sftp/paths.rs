//! MCP sftp path, encoding and raw-SFTP helpers.

use super::*;

/// MCP 工具面的连接级文件名编码判定（M17）：arguments 携带的 connectionId
/// （保存连接；dispatch 层已把 connectionName/端点选择器归一化为该字段）
/// 命中 `sftp_name_encoding_overrides` 时优先，否则跟随全局
/// `sftp_name_encoding`，缺省 auto——与工作台
/// `Plugin::resolve_sftp_encoding_opt` 同构（内联拨号按未覆盖处理）。
pub(in crate::features::mcp) fn mcp_sftp_encoding(data_dir: &Path, arguments: &Value) -> sftp_name::NameEncoding {
    preferences::sftp_name_encoding_for(data_dir, non_empty_argument(arguments, "connectionId"))
}

/// latin-1 裸包列表条目 → MCP 工具响应条目（M17）。名字口径为**显示形式**：
/// `name`/`path` 都是 latin-1 解码文本（解码输出恒在 U+0000..=U+00FF 域内，
/// `latin1_encode_display` 是其精确逆变换——AI 把返回的 path 原样回传给
/// sftp_mkdir/sftp_remove/sftp_rename 即落回服务器原始字节，不引入 %XX
/// 转义噪声）。kind 按 v3 permissions 类型位归类（缺 permissions 退回
/// file，非标准服务器不会把普通文件误渲染成目录）；`.`/`..` 跳过。
pub(in crate::features::mcp) fn raw_list_items(dir: &str, entries: Vec<sftp_raw::RawEntry>) -> Vec<Value> {
    entries
        .into_iter()
        .filter(|entry| entry.name.as_slice() != b"." && entry.name.as_slice() != b"..")
        .map(|entry| {
            let display =
                sftp_name::decode_display_name(&entry.name, sftp_name::NameEncoding::Latin1);
            // 目录 + 显示名的拼接与 join_wire_name 同形（纯字符串 join，无转
            // 义语义），直接复用避免重复实现。
            json!({
                "name": display.text,
                "path": sftp_name::join_wire_name(dir, &display.text),
                "kind": classify_raw_kind(entry.attrs.permissions),
                "size": entry.attrs.size,
                "modifiedAt": entry.attrs.mtime,
            })
        })
        .collect()
}

/// `sftp_stat` 响应装配（auto 与 latin-1 共用）。口径与 auto 分支一致：
/// permissions 四位八进制、秒级时间戳。uid/gid 由调用方尽力而为补齐（裸包
/// v3 attrs 不携带，见 [`lookup_remote_uid_gid`]）。
pub(in crate::features::mcp) fn raw_stat_json(
    path: &str,
    attrs: sftp_raw::RawAttrs,
    uid: Option<u32>,
    gid: Option<u32>,
) -> Value {
    json!({
        "path": path,
        "size": attrs.size,
        "permissions": attrs.permissions.map(|bits| format!("{:04o}", bits & 0o7777)),
        "modifiedAt": attrs.mtime,
        "accessedAt": attrs.atime,
        "uid": uid,
        "gid": gid,
    })
}

/// latin-1（M18）裸包 attrs 无 uid/gid（工作台 `sftp/stat` M17 先例同口径）：
/// shell `stat -c '%u %g'` 尽力而为补齐属主数字。命令串是 UTF-8 String，非
/// ASCII 显示名的字节参数不可控（M17-A 登记边界），exec 失败/解析不出按
/// `(None, None)` 处理，主元数据不受影响。
pub(in crate::features::mcp) async fn lookup_remote_uid_gid(
    handle: &Handle<SshClient>,
    path: &str,
) -> (Option<u32>, Option<u32>) {
    let command = format!("stat -c '%u %g' -- {}", exec::shell_quote(path));
    let Ok(outcome) = exec::exec_plain(handle, &command, Duration::from_secs(10), &[]).await else {
        return (None, None);
    };
    let mut fields = outcome.output.split_whitespace();
    (
        fields.next().and_then(|field| field.parse().ok()),
        fields.next().and_then(|field| field.parse().ok()),
    )
}

/// latin-1（M18）`sftp_read_file` 裸包分支：显示路径还原字节后
/// OPEN(READ) + READ 分块循环（`RawSftp::read_file`，32 KiB 粒度，v3 规范
/// 建议口径）。大文件策略沿既有 MCP 边界：单次至多 `max_bytes`（上限
/// maxDownloadBytes），用 `max_bytes + 1` 探测截断——与 auto 分支 `take()`
/// 语义一致。返回 `(数据, 是否截断)`。
pub(in crate::features::mcp) async fn raw_sftp_read_file<S>(
    client: &mut sftp_raw::RawSftp<S>,
    path: &str,
    offset: u64,
    max_bytes: u64,
) -> Result<(Vec<u8>, bool), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + std::marker::Unpin,
{
    let mut data = client
        .read_file(
            &sftp_name::latin1_encode_display(path),
            offset,
            max_bytes.saturating_add(1),
        )
        .await?;
    let truncated = data.len() as u64 > max_bytes;
    if truncated {
        data.truncate(max_bytes as usize);
    }
    Ok((data, truncated))
}

/// `sftp_read_file` 响应装配（auto 与 latin-1 共用）。
pub(in crate::features::mcp) fn read_file_response(
    path: &str,
    data: &[u8],
    truncated: bool,
    as_base64: bool,
) -> Value {
    if as_base64 {
        json!({
            "path": path,
            "dataBase64": BASE64_STANDARD.encode(data),
            "truncated": truncated,
        })
    } else {
        json!({
            "path": path,
            "content": String::from_utf8_lossy(data),
            "truncated": truncated,
        })
    }
}

/// latin-1（M18）`sftp_write_file` 裸包分支：显示路径还原字节后
/// OPEN(CREAT|WRITE|TRUNC) 截断直写 + WRITE 32 KiB 分块（选型：MCP 面
/// 沿既有 sftp_write_file 直写语义，无工作台上传族的 `.dbx-part` 暂存
/// 需求）。`overwrite=false` 的覆盖预检走裸包 LSTAT，与写入同一字节口径；
/// 操作错误原样上抛不回退（与 M17-B 写工具同策略，避免重复执行）。
pub(in crate::features::mcp) async fn raw_sftp_write_file<S>(
    client: &mut sftp_raw::RawSftp<S>,
    path: &str,
    content: &[u8],
    overwrite: bool,
) -> Result<Value, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + std::marker::Unpin,
{
    let bytes = raw_sftp_write_bytes(client, path, content, overwrite).await?;
    Ok(json!({ "path": path, "bytes": bytes }))
}

/// latin-1（M19）`sftp_upload` 裸包车道共用核心：与 [`raw_sftp_write_file`]
/// 同一直写语义，返回写入字节数供调用方按工具各自的响应形状组装
/// （sftp_write_file → `{path, bytes}`；sftp_upload → `{localPath,
/// remotePath, bytes}`）。
pub(in crate::features::mcp) async fn raw_sftp_write_bytes<S>(
    client: &mut sftp_raw::RawSftp<S>,
    path: &str,
    content: &[u8],
    overwrite: bool,
) -> Result<usize, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + std::marker::Unpin,
{
    let raw_path = sftp_name::latin1_encode_display(path);
    if !overwrite && client.lstat(&raw_path).await.is_ok() {
        return Err(format!(
            "Remote path already exists: {path} (pass overwrite=true to replace)"
        ));
    }
    let handle = client.open_write(&raw_path).await?;
    let result = async {
        for (index, chunk) in content.chunks(sftp_raw::MAX_WRITE_CHUNK).enumerate() {
            client
                .write_chunk(&handle, (index * sftp_raw::MAX_WRITE_CHUNK) as u64, chunk)
                .await?;
        }
        Ok::<(), String>(())
    }
    .await;
    // 写失败也先 CLOSE 释放句柄，再上抛原错误。
    let closed = client.close(&handle).await;
    result?;
    closed?;
    Ok(content.len())
}

pub(in crate::features::mcp) fn sftp_error(error: impl std::fmt::Display) -> String {
    format!("SFTP operation failed: {error}")
}

/// Resolves the allowed local roots for agent-driven `sftp_upload` /
/// `sftp_download` transfers: the configured `localTransferRoot` when set
/// (it must resolve — an unreachable root is an error, never a silent
/// fallback), otherwise the OS temp dir plus the plugin data dir. Roots
/// are canonicalized so macOS `/var` → `/private/var` style aliasing
/// cannot dodge the `starts_with` containment check.
pub(in crate::features::mcp) fn local_transfer_roots_for(
    configured: &str,
    data_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    if !configured.is_empty() {
        return std::fs::canonicalize(configured)
            .map(|root| vec![root])
            .map_err(|error| {
                format!(
                    "Configured localTransferRoot {configured} cannot be resolved: {error} \
                     (create the directory or reset it via mcp/settings/set)"
                )
            });
    }
    Ok([std::env::temp_dir(), data_dir.to_path_buf()]
        .into_iter()
        .filter_map(|root| std::fs::canonicalize(&root).ok())
        .collect())
}

/// Containment gate for every agent-supplied local path behind
/// `sftp_upload` (read side) and `sftp_download` (write side): the
/// canonical path must stay inside an allowed root and clear the
/// sensitive-path blocklist (credential stores, shell bootstrap files) in
/// every mode, so a configured root cannot be used to reach them either.
/// `path` must already be canonical — symlink and `..` traversal artifacts
/// are resolved by the caller (`std::fs::canonicalize`, or the
/// canonical-parent rejoin on the download side).
pub(in crate::features::mcp) fn ensure_local_transfer_allowed_in(
    roots: &[PathBuf],
    configured: &str,
    path: &Path,
) -> Result<(), String> {
    if is_sensitive_local_path(&path.to_string_lossy()) {
        return Err(format!(
            "Refusing to transfer the local sensitive path {} via sftp_upload/sftp_download",
            path.display()
        ));
    }
    if roots.iter().any(|root| path.starts_with(root)) {
        return Ok(());
    }
    Err(if configured.is_empty() {
        format!(
            "Local path {} is outside the allowed transfer roots (OS temp dir and plugin data \
             dir); set localTransferRoot via mcp/settings/set to allow more",
            path.display()
        )
    } else {
        format!(
            "Local path {} is outside the configured localTransferRoot {configured}",
            path.display()
        )
    })
}
