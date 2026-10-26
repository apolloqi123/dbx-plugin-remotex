//! Container file browsing: path normalization, `ls`/`stat` parsing and
//! directory payload assembly. Transfer/upload submodules build on the same
//! path and staging rules.
//!
//! Paths here are *container* paths: they only ever travel through
//! `docker exec` argv or a shell-quoted `docker cp` source, never the
//! host filesystem. They are still gated (absolute, no control characters,
//! bounded length) so a hostile name cannot break the command or the
//! protocol.

use super::*;

mod list;
mod transfer;
mod uploads;

pub use list::*;
pub use transfer::*;
pub use uploads::*;

/// Wait cap for one `ls`/`stat` round-trip.
pub const FILES_TIMEOUT: Duration = Duration::from_secs(15);
/// Container path capacity, in characters.
pub const CONTAINER_PATH_MAX_CHARS: usize = 4096;

/// One directory row. `kind` is `"dir"` or `"file"`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerEntry {
    pub name: String,
    pub path: String,
    pub kind: String,
}

/// One `stat -c '%F|%s|%Y|%a'` row, projected to the fields the UI shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStat {
    pub kind: String,
    pub size: u64,
    pub mtime: u64,
    pub mode: String,
}

/// Normalizes an absolute container path: collapses empty and `.` segments,
/// applies `..` with root clamping, and drops the trailing slash (except for
/// `/`). Relative paths, control characters and over-long paths are
/// rejected before any command is rendered.
pub fn normalize_container_path(path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err("Container path must not be empty".to_string());
    }
    if !path.starts_with('/') {
        return Err(format!(
            "Container path '{path}' must be absolute (start with '/')"
        ));
    }
    if path.chars().count() > CONTAINER_PATH_MAX_CHARS {
        return Err(format!(
            "Container path is longer than {CONTAINER_PATH_MAX_CHARS} characters"
        ));
    }
    if path.chars().any(char::is_control) {
        return Err("Container path must not contain control characters".to_string());
    }
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        Ok("/".to_string())
    } else {
        Ok(format!("/{}", segments.join("/")))
    }
}

/// Parent of a normalized container path (`/` for top-level entries).
pub fn parent_container_path(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(index) => &path[..index],
    }
}

/// Joins a normalized directory path with one raw entry name.
pub fn join_container_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// Parses `ls -1ap` output into entries (name/kind; `path` is the bare name
/// until [`list_entries`] joins it with the directory). `-a` emits `.` and
/// `..`, which are dropped; a trailing `/` is the directory marker.
pub fn parse_ls_output(output: &str) -> Vec<ContainerEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line == "." || line == ".." {
            continue;
        }
        let (name, kind) = match line.strip_suffix('/') {
            Some(name) => (name, "dir"),
            None => (line, "file"),
        };
        if name.is_empty() {
            continue;
        }
        entries.push(ContainerEntry {
            name: name.to_string(),
            path: name.to_string(),
            kind: kind.to_string(),
        });
    }
    entries
}

/// Parses the directory listing into full entry rows, ordered directories
/// first then by name ascending (the UI's documented order).
pub fn list_entries(path: &str, output: &str) -> Vec<ContainerEntry> {
    let mut entries = parse_ls_output(output);
    for entry in &mut entries {
        entry.path = join_container_path(path, &entry.name);
    }
    sort_entries(entries)
}

/// Directory-first, then case-sensitive name ascending.
pub fn sort_entries(mut entries: Vec<ContainerEntry>) -> Vec<ContainerEntry> {
    entries.sort_by(|left, right| {
        (left.kind != "dir")
            .cmp(&(right.kind != "dir"))
            .then_with(|| left.name.cmp(&right.name))
    });
    entries
}

/// Assembles the `docker/files/list` payload.
pub fn files_list_payload(path: &str, entries: Vec<ContainerEntry>) -> Value {
    json!({
        "path": path,
        "parent": parent_container_path(path),
        "entries": entries,
    })
}

/// Parses one `stat -c '%F|%s|%Y|%a'` row. The file type is the GNU
/// human-readable `%F` text ("regular file", "directory", …).
pub fn parse_stat_output(output: &str) -> Option<ContainerStat> {
    let line = output.lines().map(str::trim).find(|line| !line.is_empty())?;
    let mut fields = line.split('|');
    let file_type = fields.next()?.trim();
    let size = fields.next()?.trim().parse().ok()?;
    let mtime = fields.next()?.trim().parse().ok()?;
    let mode = fields.next()?.trim().to_string();
    if mode.is_empty() {
        return None;
    }
    Some(ContainerStat {
        kind: if file_type.contains("directory") {
            "dir".to_string()
        } else {
            "file".to_string()
        },
        size,
        mtime,
        mode,
    })
}

/// `docker/files/stat` payload; an unparseable row degrades to the
/// `unavailable` shape the UI displays as "—".
pub fn stat_payload(stat: Option<ContainerStat>) -> Value {
    match stat {
        Some(stat) => json!({
            "kind": stat.kind,
            "size": stat.size,
            "mtime": stat.mtime,
            "mode": stat.mode,
        }),
        None => json!({ "unavailable": true }),
    }
}

/// True when a failed command means the tool itself is missing inside the
/// container (`ls`/`stat` absent from a distroless image), as opposed to a
/// genuine failure the caller should surface verbatim.
pub fn is_missing_tool(output: &str, tool: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("executable file not found")
        || lower.contains(&format!("{tool}: not found"))
        || lower.contains(&format!("{tool}: command not found"))
        || lower.contains(&format!("exec: \"{tool}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_container_paths() {
        assert_eq!(normalize_container_path("/").unwrap(), "/");
        assert_eq!(normalize_container_path("/var//log/../lib/").unwrap(), "/var/lib");
        assert!(normalize_container_path("relative").is_err());
        assert!(normalize_container_path("/tmp/\0x").is_err());
        assert!(normalize_container_path(&format!("/{}", "a".repeat(4097))).is_err());
    }

    #[test]
    fn derives_parent_paths_without_trailing_slashes() {
        assert_eq!(parent_container_path("/"), "/");
        assert_eq!(parent_container_path("/var"), "/");
        assert_eq!(parent_container_path("/var/lib"), "/var");
    }

    #[test]
    fn parses_stat_rows() {
        let stat = parse_stat_output("regular file|4096|1712345678|644\n").unwrap();
        assert_eq!(stat.kind, "file");
        assert_eq!(stat.size, 4096);
        assert_eq!(stat.mtime, 1712345678);
        assert_eq!(stat.mode, "644");
        assert_eq!(parse_stat_output("directory|4096|1|755").unwrap().kind, "dir");
        assert!(parse_stat_output("garbage").is_none());
    }

    #[test]
    fn sorts_entries_directories_first_then_name() {
        let entries = vec![
            entry("zeta", "file"),
            entry("beta", "dir"),
            entry("alpha", "dir"),
            entry("eta", "file"),
        ];
        let sorted = sort_entries(entries);
        let names: Vec<&str> = sorted.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "eta", "zeta"]);
    }

    fn entry(name: &str, kind: &str) -> ContainerEntry {
        ContainerEntry {
            name: name.to_string(),
            path: name.to_string(),
            kind: kind.to_string(),
        }
    }
}
