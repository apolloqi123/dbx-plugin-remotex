//! Drive and directory enumeration for the local picker.

use super::*;

/// Cap per listing so a huge directory cannot stall the picker.
pub(super) const MAX_ENTRIES: usize = 500;

/// Windows drive letters for the picker's "This PC" page (A/B are floppy-era
/// and probing them can hang on legacy hardware). Empty on other platforms —
/// the picker then starts at the default directory instead.
pub fn list_local_drives() -> Vec<String> {
    let mut out = Vec::new();
    if cfg!(windows) {
        for letter in b'C'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            if Path::new(&drive).exists() {
                out.push(drive);
            }
        }
    }
    out
}

/// Lists the subdirectories of `path` (default download dir when absent or
/// empty), name-sorted case-insensitively. Returns `{ path, parent, entries }`
/// with directories only; symlinks to directories are followed, entries that
/// cannot be stat'ed are skipped. Errors when the target is not an existing,
/// readable absolute directory.
pub fn browse_local_dir(path: Option<&str>, data_dir: &Path) -> Result<Value, String> {
    let dir = path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            crate::filesystem::local::downloads::downloads_base_dir(
                |key| std::env::var_os(key),
                data_dir,
            )
        });
    if !dir.is_absolute() {
        return Err("Path must be absolute".to_string());
    }
    let metadata = std::fs::metadata(&dir)
        .map_err(|error| format!("Cannot open '{}': {error}", dir.display()))?;
    if !metadata.is_dir() {
        return Err(format!("'{}' is not a directory", dir.display()));
    }
    let read = std::fs::read_dir(&dir)
        .map_err(|error| format!("Cannot read '{}': {error}", dir.display()))?;
    let mut entries: Vec<(String, String)> = Vec::new();
    for entry in read.flatten() {
        // metadata() follows symlinks so linked folders stay navigable.
        let is_dir = entry.metadata().map(|meta| meta.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        entries.push((name, entry.path().to_string_lossy().into_owned()));
    }
    // 先排序再截断：截断在排序前会按 readdir 顺序（ext4 为 hash 序，不确定）
    // 任取 500 条，字母序靠后的目录在用户眼里"消失"。
    entries.sort_by_key(|entry| entry.0.to_lowercase());
    entries.truncate(MAX_ENTRIES);
    Ok(json!({
        "path": dir.to_string_lossy(),
        "parent": dir.parent().map(|parent| parent.to_string_lossy()),
        "entries": entries
            .iter()
            .map(|(name, path)| json!({ "name": name, "path": path, "is_dir": true }))
            .collect::<Vec<_>>(),
    }))
}
