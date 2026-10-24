use super::*;

// ---------------------------------------------------------------------------
// read_file / write_file
// ---------------------------------------------------------------------------

/// Per-call ceiling for `sudo/readFile`: the read rides `exec` output, so a
/// read-to-end over a huge root file buffers the whole thing through the
/// output String + base64 decode + re-encode (≈4× file size in RSS) before
/// anyone can react. Callers page with offset/length (the workbench always
/// passes an explicit length); a 0 length — the legacy "read to end" shape —
/// is clamped to this ceiling and reported through `truncated`, matching the
/// chunked contract of every other transfer lane.
pub const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;

/// Reads a byte range via sudo `tail -c`/`head -c` and returns it base64
/// encoded (tiny-rdm ReadFileSudo). One extra byte is requested so a short
/// read is detectable without a second round trip; the surplus is trimmed
/// locally and reported through `truncated`.
pub async fn read_file(
    runtime: &SshRuntime,
    session_id: &str,
    path: &str,
    offset: u64,
    length: u64,
) -> Result<Value, String> {
    let path = normalize_remote_path(path)?;
    // length=0（读到结尾）钳到上限；显式 length 同样钳制——传 u64::MAX 会让
    // `head -c` 变成全文件读取，经 exec String + base64 解码/再编码约 4 倍
    // 文件大小的内存。超出部分照既有 truncated 语义上报。
    let length = if length == 0 {
        MAX_READ_BYTES
    } else {
        length.min(MAX_READ_BYTES)
    };
    // tail counts from byte 1, so +1 converts a zero-based offset.
    let head = if offset == 0 {
        format!(
            "head -c {} -- {} | ",
            length.saturating_add(1),
            shell_quote(&path)
        )
    } else {
        format!(
            "tail -c +{} -- {} | head -c {} | ",
            offset.saturating_add(1),
            shell_quote(&path),
            length.saturating_add(1)
        )
    };
    let command = format!("{head}base64");
    let output = sudo_exec(runtime, session_id, &command, TIMEOUT_READ_SECS).await?;
    let decoded = BASE64_STANDARD
        .decode(
            output
                .bytes()
                .filter(|byte| !byte.is_ascii_whitespace())
                .collect::<Vec<u8>>(),
        )
        .map_err(|error| format!("Remote returned invalid base64 data: {error}"))?;
    let truncated = length > 0 && decoded.len() > length as usize;
    let mut data = decoded;
    if truncated {
        data.truncate(length as usize);
    }
    Ok(json!({
        "dataBase64": BASE64_STANDARD.encode(data),
        "truncated": truncated,
    }))
}

/// Writes a file via sudo, embedding the payload as base64 text (tiny-rdm
/// WriteFileSudo). `SshRuntime::exec` exposes no data-stdin channel (its
/// stdin carries the sudo password), so the payload is chunked to
/// [`WRITE_CHUNK_BYTES`]. Chunks land in `<path>.dbx-part` (first chunk
/// truncates via `>`, later ones append via `>>`) and a final `mv -f` puts
/// the file in place — the SFTP lanes' part-file contract, so a mid-transfer
/// failure (expired sudo timestamp, dropped connection) leaves the original
/// file untouched instead of a half-written truncation.
pub async fn write_file(
    runtime: &SshRuntime,
    session_id: &str,
    path: &str,
    data_base64: &str,
) -> Result<(), String> {
    let path = normalize_remote_path(path)?;
    let data = BASE64_STANDARD
        .decode(
            data_base64
                .bytes()
                .filter(|byte| !byte.is_ascii_whitespace())
                .collect::<Vec<u8>>(),
        )
        .map_err(|error| format!("Invalid base64 file data: {error}"))?;
    // 暂存名掺 uuid：确定性 `<path>.dbx-part` 在两个并发 sudo/writeFile 写
    // 同一路径时块级交错/互相截断，最后各自 mv 同一份 part 落位产出损坏
    // 文件。uuid 隔离后每个调用只 mv 自己的 part，失败清理也只删自己的。
    let part = format!("{}.dbx-part-{}", path, uuid::Uuid::new_v4());
    let part_quoted = shell_quote(&part);
    let path_quoted = shell_quote(&path);
    // Best-effort permission carry-over: `mv` swaps the inode, so an
    // overwrite would otherwise reset an existing file's mode to the umask
    // default. Fails harmlessly when the target does not exist yet.
    let keep_mode = format!(
        "{{ chmod --reference={path_q} {part_q} 2>/dev/null || true; }}",
        path_q = path_quoted,
        part_q = part_quoted
    );
    let finalize = format!("mv -f -- {part_quoted} {path_quoted}");
    if data.is_empty() {
        // `chunks` yields nothing for empty input; create the empty file once.
        let command = format!(": > {part_quoted} && {keep_mode} && {finalize}");
        if let Err(error) = sudo_exec(runtime, session_id, &command, TIMEOUT_WRITE_SECS).await {
            cleanup_part(runtime, session_id, &part_quoted).await;
            return Err(error);
        }
        return Ok(());
    }
    for (index, chunk) in data.chunks(WRITE_CHUNK_BYTES).enumerate() {
        let redirect = if index == 0 { ">" } else { ">>" };
        let command = format!(
            "printf %s '{}' | base64 -d {} {}",
            BASE64_STANDARD.encode(chunk),
            redirect,
            part_quoted
        );
        if let Err(error) = sudo_exec(runtime, session_id, &command, TIMEOUT_WRITE_SECS).await {
            // 清理暂存；目标文件保持原样。
            cleanup_part(runtime, session_id, &part_quoted).await;
            return Err(error);
        }
    }
    let command = format!("{keep_mode} && {finalize}");
    if let Err(error) = sudo_exec(runtime, session_id, &command, TIMEOUT_WRITE_SECS).await {
        // finalize 失败（sudo 时间戳过期、连接断开）同样清理暂存：与分块
        // 错误路径对称，不在远端残留 uuid 命名的 part 文件。
        cleanup_part(runtime, session_id, &part_quoted).await;
        return Err(error);
    }
    Ok(())
}

/// Best-effort 清理 sudo 写入的远端暂存件（清理失败静默——错误主线优先）。
async fn cleanup_part(runtime: &SshRuntime, session_id: &str, part_quoted: &str) {
    let _ = sudo_exec(
        runtime,
        session_id,
        &format!("rm -f -- {part_quoted}"),
        TIMEOUT_QUICK_SECS,
    )
    .await;
}
