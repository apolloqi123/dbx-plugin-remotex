//! Host-side directory listing for the remote-desktop file transfer window.
//!
//! Only metadata is ever returned here; file contents are read by the upload
//! task after the user explicitly picks a source. Paths stay on the user's
//! machine and are never logged.

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

const MAX_ENTRIES: usize = 2000;

pub(crate) fn home_dir() -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

fn default_dir() -> Result<PathBuf, String> {
    home_dir()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "Unable to resolve a local home directory".to_string())
}

fn modified_secs(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
}

/// Lists one absolute local directory (directories and files, never
/// contents). Symlinks are reported with `symlink: true` so the UI can show
/// them; uploads skip them.
pub(crate) fn list(params: &Value) -> Result<Value, String> {
    let requested = params
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let path = match requested {
        Some(path) => PathBuf::from(path),
        None => default_dir()?,
    };
    if !path.is_absolute() {
        return Err("Local path must be absolute".to_string());
    }
    let metadata = fs::metadata(&path)
        .or_else(|_| fs::symlink_metadata(&path))
        .map_err(|error| format!("Unable to read local path '{}': {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "Local path '{}' is not a directory",
            path.display()
        ));
    }
    let mut entries = Vec::new();
    let read_dir = fs::read_dir(&path)
        .map_err(|error| format!("Unable to list local path '{}': {error}", path.display()))?;
    for entry in read_dir {
        if entries.len() >= MAX_ENTRIES {
            break;
        }
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if name.is_empty() {
            continue;
        }
        let full = entry.path();
        let Ok(link_metadata) = fs::symlink_metadata(&full) else {
            continue;
        };
        let symlink = link_metadata.file_type().is_symlink();
        let metadata = if symlink {
            fs::metadata(&full).unwrap_or(link_metadata)
        } else {
            link_metadata
        };
        let kind = if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "file"
        } else {
            "other"
        };
        entries.push(json!({
            "name": name,
            "path": full.to_string_lossy(),
            "kind": kind,
            "size": if kind == "file" { metadata.len() } else { 0 },
            "modifiedAt": modified_secs(&metadata),
            "hidden": name.starts_with('.'),
            "symlink": symlink,
        }));
    }
    entries.sort_by(|left, right| {
        let left_dir = left["kind"].as_str() == Some("directory");
        let right_dir = right["kind"].as_str() == Some("directory");
        right_dir.cmp(&left_dir).then_with(|| {
            left["name"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
                .cmp(&right["name"].as_str().unwrap_or_default().to_lowercase())
        })
    });
    Ok(json!({
        "path": path.to_string_lossy(),
        "parent": path.parent().map(|parent| parent.to_string_lossy()),
        "home": home_dir().map(|home| home.to_string_lossy().to_string()),
        "drives": crate::filesystem::local::fs::list_local_drives(),
        "entries": entries,
    }))
}

/// Destination for files copied out of a VNC desktop. Keeping them under the
/// user's Downloads folder means a later Finder paste references real files
/// instead of a transient temp directory.
pub(crate) fn clipboard_dir() -> Result<PathBuf, String> {
    let home = home_dir().ok_or_else(|| "Unable to resolve a local home directory".to_string())?;
    let downloads = home.join("Downloads");
    let base = if downloads.is_dir() { downloads } else { home };
    let dir = base.join("DBX Remote Clipboard");
    fs::create_dir_all(&dir)
        .map_err(|error| format!("Unable to create '{}': {error}", dir.display()))?;
    Ok(dir)
}

/// Validates a local directory chosen as a download destination.
pub(crate) fn validate_destination(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path.trim());
    if !path.is_absolute() {
        return Err("Download directory must be absolute".to_string());
    }
    if !path.is_dir() {
        return Err(format!(
            "Download directory '{}' does not exist",
            path.display()
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_directories_first_then_case_insensitive_names() {
        let root = tempfile::tempdir().expect("root");
        fs::write(root.path().join("b.txt"), b"bb").unwrap();
        fs::write(root.path().join("A.txt"), b"a").unwrap();
        fs::create_dir(root.path().join("zdir")).unwrap();
        fs::create_dir(root.path().join("adir")).unwrap();
        fs::write(root.path().join(".hidden"), b"h").unwrap();
        let result = list(&json!({ "path": root.path().to_str().unwrap() })).expect("list");
        let rows = result["entries"].as_array().unwrap();
        let names: Vec<&str> = rows
            .iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["adir", "zdir", ".hidden", "A.txt", "b.txt"]);
        assert_eq!(rows[0]["kind"], "directory");
        assert_eq!(rows[2]["hidden"], true);
        assert_eq!(rows[3]["size"], 1);
    }

    #[test]
    fn rejects_relative_missing_and_file_paths() {
        assert!(list(&json!({ "path": "relative/dir" })).is_err());
        assert!(list(&json!({ "path": "/no/such/path/for/remote-files" })).is_err());
        let root = tempfile::tempdir().expect("root");
        let file = root.path().join("f.txt");
        fs::write(&file, b"x").unwrap();
        assert!(list(&json!({ "path": file.to_str().unwrap() })).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn marks_symlinks_without_following_the_listing() {
        let root = tempfile::tempdir().expect("root");
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, root.path().join("link")).unwrap();
        let result = list(&json!({ "path": root.path().to_str().unwrap() })).expect("list");
        let link = result["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == "link")
            .expect("link row");
        assert_eq!(link["symlink"], true);
        assert_eq!(link["kind"], "directory");
    }

    #[test]
    fn destination_validation_requires_an_existing_absolute_directory() {
        let root = tempfile::tempdir().expect("root");
        assert!(validate_destination(root.path().to_str().unwrap()).is_ok());
        assert!(validate_destination("relative/dir").is_err());
        assert!(validate_destination("/no/such/download-dir").is_err());
        let file = root.path().join("f.txt");
        fs::write(&file, b"x").unwrap();
        assert!(validate_destination(file.to_str().unwrap()).is_err());
    }
}
