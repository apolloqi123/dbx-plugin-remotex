use super::*;

// ---------------------------------------------------------------------------
// stat
// ---------------------------------------------------------------------------

/// GNU coreutils stat: type|inode|size|mtime|perm|user|uid|group|gid
const GNU_STAT_FORMAT: &str = "%F|%i|%s|%Y|%a|%U|%u|%G|%g";
/// BSD/macOS stat:  type|inode|size|mtime|perm|user|uid|group|gid
const BSD_STAT_FORMAT: &str = "%HT|%i|%z|%m|%Lp|%Su|%u|%Sg|%g";

/// Fields shared by both stat dialects, already normalized for JSON output.
pub(super) struct StatInfo {
    pub(super) kind: &'static str,
    pub(super) size: u64,
    pub(super) modified_at: u64,
    pub(super) mode: String,
    pub(super) owner: String,
    pub(super) group: String,
    pub(super) owner_uid: Option<u64>,
    pub(super) group_gid: Option<u64>,
}

/// Maps a `stat` file-type description (GNU `%F` or BSD `%HT`) to the kind
/// vocabulary used by `sftp/list` entries. GNU prints "regular empty file"
/// for zero-length regular files, hence the prefix match.
fn classify_file_type(file_type: &str) -> &'static str {
    let lowered = file_type.trim().to_ascii_lowercase();
    if lowered == "directory" {
        "directory"
    } else if lowered.starts_with("regular") || lowered == "file" {
        "file"
    } else if lowered.contains("symbolic link") {
        "symlink"
    } else {
        "other"
    }
}

/// Formats numeric permission bits as a 3-4 digit octal string ("0755").
fn format_mode_bits(bits: u32) -> String {
    format!("{:04o}", bits & 0o7777)
}

/// Parses `stat -c '%F|%i|%s|%Y|%a|%U|%u|%G|%g'` output (GNU coreutils).
pub(super) fn parse_gnu_stat_line(line: &str) -> Option<StatInfo> {
    let fields = line.trim().split('|').collect::<Vec<_>>();
    if fields.len() < 9 {
        return None;
    }
    let mode = u32::from_str_radix(fields[4].trim(), 8).ok()?;
    Some(StatInfo {
        kind: classify_file_type(fields[0]),
        size: fields[2].trim().parse().unwrap_or(0),
        modified_at: fields[3].trim().parse().unwrap_or(0),
        mode: format_mode_bits(mode),
        owner: fields[5].trim().to_string(),
        owner_uid: fields[6].trim().parse().ok(),
        group: fields[7].trim().to_string(),
        group_gid: fields[8].trim().parse().ok(),
    })
}

/// Parses BSD stat output — `stat -f '%HT|%i|%z|%m|%Lp|%Su|%u|%Sg|%g'`.
/// BSD %Lp is decimal st_mode including file-type bits; mask to 0o7777.
pub(super) fn parse_bsd_stat_line(line: &str) -> Option<StatInfo> {
    let fields = line.trim().split('|').collect::<Vec<_>>();
    if fields.len() < 9 {
        return None;
    }
    let mode = fields[4].trim().parse::<u32>().ok()?;
    Some(StatInfo {
        kind: classify_file_type(fields[0]),
        size: fields[2].trim().parse().unwrap_or(0),
        modified_at: fields[3].trim().parse().unwrap_or(0),
        mode: format_mode_bits(mode),
        owner: fields[5].trim().to_string(),
        owner_uid: fields[6].trim().parse().ok(),
        group: fields[7].trim().to_string(),
        group_gid: fields[8].trim().parse().ok(),
    })
}

/// Returns file metadata via sudo `stat`, trying GNU `stat -c` first and
/// falling back to `stat -f` on BSD-flavoured systems (tiny-rdm StatSudo).
pub async fn stat(runtime: &SshRuntime, session_id: &str, path: &str) -> Result<Value, String> {
    let path = normalize_remote_path(path)?;
    let gnu = format!("stat -c '{}' -- {}", GNU_STAT_FORMAT, shell_quote(&path));
    let parsed = match sudo_exec(runtime, session_id, &gnu, TIMEOUT_QUICK_SECS).await {
        Ok(output) => parse_gnu_stat_line(&output),
        Err(gnu_error) => {
            let bsd = format!("stat -f '{}' -- {}", BSD_STAT_FORMAT, shell_quote(&path));
            match sudo_exec(runtime, session_id, &bsd, TIMEOUT_QUICK_SECS).await {
                Ok(output) => Some(parse_bsd_stat_line(&output).ok_or(gnu_error)?),
                Err(_) => return Err(gnu_error),
            }
        }
    };
    let info = parsed.ok_or_else(|| format!("Could not parse stat output for {path}"))?;
    Ok(json!({
        "path": path,
        "kind": info.kind,
        "size": info.size,
        "modifiedAt": info.modified_at,
        "mode": info.mode,
        "owner": info.owner,
        "group": info.group,
        // GNU/BSD stat 总是会输出 %u / %g，所以数字字段必然有值。
        // owner / group 这里就是用户名/组名字符串，name 和 display 相同。
        "ownerName": info.owner,
        "ownerUid": info.owner_uid,
        "groupName": info.group,
        "groupGid": info.group_gid,
    }))
}

// ---------------------------------------------------------------------------
// exists / touch
// ---------------------------------------------------------------------------

/// Checks path existence with `test -e` (tiny-rdm Exists, sudo-elevated).
/// The `echo` fallback keeps the shell exit code at zero so a missing path is
/// distinguishable from a command failure.
pub async fn exists(runtime: &SshRuntime, session_id: &str, path: &str) -> Result<bool, String> {
    let path = normalize_remote_path(path)?;
    let command = format!("test -e {} && echo 1 || echo 0", shell_quote(&path));
    let output = sudo_exec(runtime, session_id, &command, TIMEOUT_QUICK_SECS).await?;
    Ok(output.trim() == "1")
}

/// Creates an empty file or refreshes its mtime (tiny-rdm Touch).
pub async fn touch(runtime: &SshRuntime, session_id: &str, path: &str) -> Result<(), String> {
    let path = normalize_remote_path(path)?;
    let command = format!("touch -- {}", shell_quote(&path));
    sudo_exec(runtime, session_id, &command, TIMEOUT_QUICK_SECS)
        .await
        .map(|_| ())
}

// ---------------------------------------------------------------------------
// list_dir
// ---------------------------------------------------------------------------

/// One parsed `ls -l` row before it is joined with the directory path.
struct LsEntry {
    name: String,
    kind: &'static str,
    size: u64,
    modified_at: Option<u64>,
    permissions: Option<String>,
    owner: Option<String>,
    group: Option<String>,
}

/// Maps a symbolic mode column to the `sftp/list` kind vocabulary.
fn kind_from_mode_string(mode: &str) -> &'static str {
    match mode.as_bytes().first() {
        Some(b'd') => "directory",
        Some(b'l') => "symlink",
        Some(b'-') => "file",
        _ => "other",
    }
}

/// Converts a symbolic mode column ("drwxr-xr-t") into numeric permission
/// bits including setuid/setgid/sticky (0755, 4755, 5755, ...).
pub(super) fn mode_string_to_octal(mode: &str) -> Option<u32> {
    let bytes = mode.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let mut bits = 0u32;
    for (shift, triad) in [
        (6_usize, &bytes[1..4]),
        (3, &bytes[4..7]),
        (0, &bytes[7..10]),
    ] {
        let mut value = 0u32;
        if triad[0] != b'-' {
            value |= 0o4;
        }
        if triad[1] != b'-' {
            value |= 0o2;
        }
        if matches!(triad[2], b'x' | b's' | b't') {
            value |= 0o1;
        }
        bits |= value << shift;
    }
    match bytes[3] {
        b's' | b'S' => bits |= 0o4000,
        _ => {}
    }
    match bytes[6] {
        b's' | b'S' => bits |= 0o2000,
        _ => {}
    }
    match bytes[9] {
        b't' | b'T' => bits |= 0o1000,
        _ => {}
    }
    Some(bits & 0o7777)
}

/// Undoes the quoting `ls` applies to unusual names when its stdout is a
/// terminal (possible here when `sudo_use_pty` is enabled): the shell-escape
/// style renders `weird name's` as `'weird name'\''s'`.
fn unquote_ls_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        return trimmed[1..trimmed.len() - 1].replace("'\\''", "'");
    }
    trimmed.to_string()
}

/// Converts one `ls -la` line into an entry, accepting both the GNU
/// `--time-style=+%s` layout (epoch in field 5, name from field 6) and the
/// classic BusyBox layout (date spread over fields 5-7, name from field 8,
/// mtime unavailable). Returns `None` for headers, `.`, `..`, and unparsable
/// rows such as device files whose size column contains a comma.
fn parse_ls_line(line: &str) -> Option<LsEntry> {
    let line = line.trim_end_matches('\r').trim();
    if line.is_empty() || line.starts_with("total") || line.starts_with("ls:") {
        return None;
    }
    let fields = line.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 7 || fields[0].len() < 10 {
        return None;
    }
    let mode = fields[0];
    let size = fields[4].parse::<u64>().ok()?;
    let (modified_at, name_start) = match fields[5].parse::<u64>() {
        Ok(epoch) => (Some(epoch), 6_usize),
        Err(_) => (None, 8_usize),
    };
    if fields.len() < name_start + 1 {
        return None;
    }
    let mut name = fields[name_start..].join(" ");
    // `ls -l` renders symlinks as "name -> target"; keep only the link name.
    if let Some((head, _)) = name.clone().split_once(" -> ") {
        name = head.to_string();
    }
    let name = unquote_ls_name(&name);
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    // Owner/group sit in fields 3/4 on both layouts and `ls -la` always
    // prints them, so sudo listings get names without an extra round trip.
    let (owner, group) = if fields.len() >= 4 {
        (Some(fields[2].to_string()), Some(fields[3].to_string()))
    } else {
        (None, None)
    };
    Some(LsEntry {
        name,
        kind: kind_from_mode_string(mode),
        size,
        modified_at,
        permissions: mode_string_to_octal(mode).map(format_mode_bits),
        owner,
        group,
    })
}

/// Mirrors the private `content_type_for_path` in `ssh.rs` so sudo listings
/// keep the exact same entry shape as `sftp/list` (it cannot be imported).
fn content_type_for_path(path: &str) -> Option<String> {
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

/// Parses `ls -la` output into `sftp/list`-shaped entries, sorted exactly
/// like `sftp_list_path`: directories first, then case-insensitive names.
pub(super) fn parse_ls_output(directory: &str, ls_output: &str) -> Vec<SftpEntry> {
    let base = directory.trim_end_matches('/');
    let mut entries: Vec<SftpEntry> = ls_output
        .lines()
        .filter_map(parse_ls_line)
        .map(|entry| {
            let path = format!("{}/{}", base, entry.name);
            SftpEntry {
                // sudo 提权的 ls 输出已是文本层；名字仍可能已被上游
                // lossy 解码，含 U+FFFD 即标记不可忠实还原。
                lossy: entry.name.contains('\u{FFFD}'),
                name: entry.name,
                uri: sftp_uri(&path),
                kind: entry.kind,
                size: Some(entry.size),
                modified_at: entry.modified_at,
                permissions: entry.permissions,
                content_type: content_type_for_path(&path),
                owner: entry.owner,
                group: entry.group,
            }
        })
        .collect();
    entries.sort_by(|left, right| {
        let left_dir = left.kind == "directory";
        let right_dir = right.kind == "directory";
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    entries
}

/// Lists a directory via sudo `ls -la` (tiny-rdm ListDirSudo). GNU coreutils
/// render epoch mtimes with `--time-style=+%s`; BusyBox rejects that option,
/// so a plain `ls -la` retry covers classic-date output (mtime omitted).
pub async fn list_dir(runtime: &SshRuntime, session_id: &str, path: &str) -> Result<Value, String> {
    let path = normalize_remote_path(path)?;
    let gnu = format!("ls -la --time-style=+%s -- {}", shell_quote(&path));
    let output = match sudo_exec(runtime, session_id, &gnu, TIMEOUT_LIST_SECS).await {
        Ok(output) => output,
        Err(gnu_error) => {
            let fallback = format!("ls -la -- {}", shell_quote(&path));
            sudo_exec(runtime, session_id, &fallback, TIMEOUT_LIST_SECS)
                .await
                .map_err(|_| gnu_error)?
        }
    };
    let entries = parse_ls_output(&path, &output);
    Ok(json!({ "entries": entries }))
}
