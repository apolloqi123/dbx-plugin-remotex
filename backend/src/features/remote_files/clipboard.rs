//! Host clipboard bridge for the file transfer window.
//!
//! The DBX host clipboard API only carries text and images (verified against
//! the 0.6.62 host bridge), so Finder file lists are read/written directly
//! through the sidecar's macOS pasteboard helper. Non-macOS platforms report
//! `supported: false` and the UI hides the clipboard actions.

use serde_json::{json, Value};
use std::path::PathBuf;

use crate::platform::pasteboard;

const MAX_CLIPBOARD_FILES: usize = 500;

fn validated_paths(params: &Value) -> Result<Vec<PathBuf>, String> {
    let rows = params
        .get("paths")
        .and_then(Value::as_array)
        .ok_or("Missing clipboard file paths")?;
    if rows.len() > MAX_CLIPBOARD_FILES {
        return Err(format!(
            "At most {MAX_CLIPBOARD_FILES} files can be written to the clipboard at once"
        ));
    }
    let mut paths = Vec::with_capacity(rows.len());
    for row in rows {
        let text = row
            .as_str()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or("Clipboard file path is empty")?;
        let path = PathBuf::from(text);
        if !path.is_absolute() {
            return Err("Clipboard file paths must be absolute".to_string());
        }
        if std::fs::symlink_metadata(&path).is_err() {
            return Err(format!(
                "Clipboard file '{}' does not exist",
                path.display()
            ));
        }
        paths.push(path);
    }
    Ok(paths)
}

pub(crate) fn files() -> Value {
    let supported = pasteboard::supported();
    let files = if supported {
        pasteboard::file_urls()
            .into_iter()
            .filter(|path| path.is_absolute())
            .take(MAX_CLIPBOARD_FILES)
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    json!({ "supported": supported, "files": files })
}

pub(crate) fn write(params: &Value) -> Result<Value, String> {
    if !pasteboard::supported() {
        return Err("The native file clipboard is only available on macOS".to_string());
    }
    let paths = validated_paths(params)?;
    pasteboard::write_file_urls(&paths)?;
    Ok(json!({ "success": true, "count": paths.len() }))
}

/// Writes already-downloaded files to the host clipboard (the VNC remote-file
/// copy path). Returns the number of paths placed on the pasteboard.
pub(crate) fn write_paths(paths: &[String]) -> Result<usize, String> {
    if !pasteboard::supported() {
        return Err("The native file clipboard is only available on macOS".to_string());
    }
    let rows = paths.iter().map(|path| json!(path)).collect::<Vec<_>>();
    let paths = validated_paths(&json!({ "paths": rows }))?;
    pasteboard::write_file_urls(&paths)?;
    Ok(paths.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_paths_require_absolute_existing_entries() {
        let root = tempfile::tempdir().expect("root");
        let file = root.path().join("a.txt");
        std::fs::write(&file, b"a").unwrap();
        let ok = validated_paths(&json!({ "paths": [file.to_str().unwrap()] })).expect("paths");
        assert_eq!(ok, vec![file]);
        assert!(validated_paths(&json!({ "paths": ["relative.txt"] })).is_err());
        assert!(validated_paths(&json!({ "paths": ["/no/such/clipboard-file"] })).is_err());
        assert!(validated_paths(&json!({ "paths": [""] })).is_err());
    }

    #[test]
    fn clipboard_path_count_is_bounded() {
        let paths = (0..=MAX_CLIPBOARD_FILES)
            .map(|index| json!(format!("/tmp/file-{index}")))
            .collect::<Vec<_>>();
        assert!(validated_paths(&json!({ "paths": paths })).is_err());
    }
}
