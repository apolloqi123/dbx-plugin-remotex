//! Sftp Format helpers for the SSH runtime.

use super::*;

/// Classifies a READDIR entry into the wire `kind` vocabulary of `sftp/list`.
///
/// russh-sftp derives `FileType` solely from the POSIX type bits in the
/// server-supplied READDIR `permissions` field; a server that omits
/// PERMISSIONS (or sends permissions without type bits — seen on
/// virtual/disk-mount SFTP services) collapses every entry to
/// `FileType::Other`. Issue #36 family: such entries used to be reported as
/// `"other"`, which the UI rendered as a generic text-document icon instead
/// of a file icon.
///
/// Rules:
/// - `Dir` / `File` / `Symlink` pass the server declaration through;
/// - `Other` (missing/unusable type bits) degrades to `"file"`: nothing on
///   the wire distinguishes files from directories at that point, and
///   "unknown renders as a file" matches FileZilla's behaviour. Real
///   directories on conforming servers always carry the DIR type bit, so
///   they never reach this branch.
pub(super) fn classify_entry_kind(file_type: FileType) -> &'static str {
    match file_type {
        FileType::File => "file",
        FileType::Dir => "directory",
        FileType::Symlink => "symlink",
        FileType::Other => "file",
    }
}

/// 裸包客户端具体类型：每次操作独占一条 sftp 子系统通道，发一收一。
pub(crate) type RawSftpClient = sftp_raw::RawSftp<russh::ChannelStream<russh::client::Msg>>;

/// 裸包客户端路径的 kind 判定：按 v3 permissions 的 POSIX 类型位归类；
/// attrs 缺 permissions（非标准服务器）时退回 file（与高层路径的 Other
/// 语义一致），避免把普通文件误渲染成目录。pub(crate)：MCP 工具面
/// latin-1 裸包列表/删除判型复用（M17）。
pub(crate) fn classify_raw_kind(permissions: Option<u32>) -> &'static str {
    let Some(mode) = permissions else {
        return "file";
    };
    match mode & 0o170000 {
        0o040000 => "directory",
        0o120000 => "symlink",
        _ => "file",
    }
}

pub(super) fn content_type_for_path(path: &str) -> Option<String> {
    let extension = path.rsplit('.').next()?.to_ascii_lowercase();
    let content_type = match extension.as_str() {
        "txt" | "log" | "md" | "rs" | "ts" | "js" | "json" | "yaml" | "yml" | "toml" | "conf" => {
            "text/plain"
        }
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => return None,
    };
    Some(content_type.to_string())
}

pub(crate) fn format_permissions(value: u32) -> String {
    format!("{:04o}", value & 0o7777)
}

/// Budget for the one `ls -l` round trip that upgrades numeric owner ids to
/// names. Generous enough for slow links, still bounded so a wedged shell
/// cannot stall directory listings.
pub(super) const OWNER_LOOKUP_TIMEOUT_SECS: u64 = 10;

/// Whether an owner/group value still benefits from the `ls -l` name lookup:
/// absent, or a numeric id string (SFTPv3 servers report `0`, `1000`, ...).
pub(super) fn owner_needs_name(value: &Option<String>) -> bool {
    match value {
        None => true,
        Some(value) => !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
    }
}

/// Overwrites numeric/absent owner/group values on `entries` with names read
/// from one read-only `ls -l` round trip. Best effort by design: no shell,
/// no `ls`, a timeout or unparsable output all leave the listing untouched
/// (numeric ids or absent fields, which the UI renders as "-").
pub(super) async fn enrich_owner_names(
    runtime: &SshRuntime,
    session_id: &str,
    path: &str,
    entries: &mut [SftpEntry],
) {
    if entries.is_empty() || !entries.iter().any(|entry| owner_needs_name(&entry.owner)) {
        return;
    }
    let session = match runtime.session(session_id).await {
        Ok(session) => session,
        Err(_) => return,
    };
    // GNU renders epoch mtimes with `--time-style=+%s` (same trick sudo_fs
    // uses); BusyBox/BSD reject that option, so a plain `ls -l` retry covers
    // the classic layout. Owner/group sit in fields 3/4 on both.
    let gnu = format!("ls -l --time-style=+%s -- {}", exec::shell_quote(path));
    let output = match exec::exec_plain(
        &session.handle,
        &gnu,
        Duration::from_secs(OWNER_LOOKUP_TIMEOUT_SECS),
        &[],
    )
    .await
    {
        Ok(outcome) => outcome.output,
        Err(_) => {
            let plain = format!("ls -l -- {}", exec::shell_quote(path));
            match exec::exec_plain(
                &session.handle,
                &plain,
                Duration::from_secs(OWNER_LOOKUP_TIMEOUT_SECS),
                &[],
            )
            .await
            {
                Ok(outcome) => outcome.output,
                Err(_) => return,
            }
        }
    };
    let names = parse_ls_owner_map(&output);
    if names.is_empty() {
        return;
    }
    for entry in entries.iter_mut() {
        if let Some((owner, group)) = names.get(&entry.name) {
            if owner_needs_name(&entry.owner) {
                entry.owner = Some(owner.clone());
            }
            if owner_needs_name(&entry.group) {
                entry.group = Some(group.clone());
            }
        }
    }
}

/// 远程 `stat -c '%U %G' path` 查单个文件的 owner/group 名字。
/// SFTP 协议默认只返回 uid/gid 数字，russh-sftp 的 `Metadata.user/group`
/// 在服务器未开 `username@hostname` 扩展时为 None。此函数作为 stat 的
/// fallback，失败/超时返回 (None, None)，调用方用数字兜底。
pub(crate) async fn lookup_owner_group_names(
    runtime: &SshRuntime,
    session_id: &str,
    path: &str,
) -> (Option<String>, Option<String>) {
    let session = match runtime.session(session_id).await {
        Ok(s) => s,
        Err(_) => return (None, None),
    };
    let cmd = format!("stat -c '%U %G' -- {}", exec::shell_quote(path));
    let output = match exec::exec_plain(
        &session.handle,
        &cmd,
        Duration::from_secs(OWNER_LOOKUP_TIMEOUT_SECS),
        &[],
    )
    .await
    {
        Ok(outcome) => outcome.output,
        Err(_) => return (None, None),
    };
    let mut parts = output.split_whitespace();
    let owner = parts
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let group = parts
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    (owner, group)
}

/// Parses `ls -l` output into `name -> (owner, group)`. Owner/group sit in
/// fields 3/4 of every layout; the name start depends on whether the date
/// collapsed into one epoch field (GNU `--time-style=+%s`, name from field 6)
/// or spread over three classic fields (name from field 8). Returns an empty
/// map for headers (`total`, `ls:` diagnostics) and unparsable rows.
pub(super) fn parse_ls_owner_map(output: &str) -> HashMap<String, (String, String)> {
    let mut map = HashMap::new();
    for line in output.lines() {
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() || line.starts_with("total") || line.starts_with("ls:") {
            continue;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 7 || fields[0].len() < 10 {
            continue;
        }
        let name_start = if fields[5].parse::<u64>().is_ok() {
            6
        } else {
            8
        };
        if fields.len() <= name_start {
            continue;
        }
        let mut name = fields[name_start..].join(" ");
        // `ls -l` renders symlinks as "name -> target"; keep only the name.
        if let Some((head, _)) = name.split_once(" -> ") {
            name = head.to_string();
        }
        let name = unquote_ls_output_name(&name);
        if name.is_empty() {
            continue;
        }
        map.entry(name)
            .or_insert_with(|| (fields[2].to_string(), fields[3].to_string()));
    }
    map
}

/// Undoes the quoting `ls` applies to unusual names when its stdout is a
/// terminal (possible when `sudo_use_pty` is enabled): shell-escape style
/// renders `weird name's` as `'weird name'\''s'`.
pub(super) fn unquote_ls_output_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        return trimmed[1..trimmed.len() - 1].replace("'\\''", "'");
    }
    trimmed.to_string()
}
