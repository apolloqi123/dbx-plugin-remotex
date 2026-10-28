use super::*;

use std::io::BufRead;

/// Resolves the `.cast` path of a recording id, rejecting path traversal —
/// recording ids are sidecar-minted UUIDs, but the RPC boundary validates.
pub fn cast_path(data_dir: &Path, recording_id: &str) -> Result<PathBuf, String> {
    if recording_id.is_empty()
        || recording_id.contains('/')
        || recording_id.contains('\\')
        || recording_id.contains("..")
    {
        return Err("Invalid recording id".to_string());
    }
    Ok(recordings_dir(data_dir).join(format!("{recording_id}.cast")))
}

/// Lists recordings (newest first by file mtime): header line per file +
/// duration read from the last event. Corrupt files are skipped silently.
pub fn list_recordings(data_dir: &Path) -> Vec<Value> {
    let dir = recordings_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut items: Vec<(std::time::SystemTime, Value)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("cast") {
            continue;
        }
        let Some(recording_id) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Some(item) = describe_recording(&path, &recording_id) else {
            continue;
        };
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        items.push((modified, item));
    }
    items.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    items.into_iter().map(|(_, item)| item).collect()
}

/// Deletes every `.cast` recording (one-click "clear all" from the workbench).
/// Only `.cast` files inside the recordings dir are touched; a file that
/// cannot be removed (e.g. an in-progress recording still open on Windows)
/// is skipped rather than failing the whole batch. Returns the deleted count.
pub fn clear_recordings(data_dir: &Path) -> usize {
    let dir = recordings_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut deleted = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("cast") {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            deleted += 1;
        }
    }
    deleted
}

/// Reads one recording's header and last event into a list row.
///
/// Duration is the stop marker's `time` written by `SessionRecorder::finish`
/// (falling back to the last output event for files finalized before the
/// marker existed). A recording that captured no output at all — old files or
/// a crash before the marker — is estimated from the file mtime so a silent
/// recording no longer reports 0 seconds forever.
pub(super) fn describe_recording(path: &Path, recording_id: &str) -> Option<Value> {
    let mut lines = std::io::BufReader::new(std::fs::File::open(path).ok()?).lines();
    let first = lines.next()?.ok()?;
    let head: Value = serde_json::from_str(first.trim()).ok()?;
    let meta = head.get("meta").cloned().unwrap_or(json!({}));
    let mut last_time = 0.0_f64;
    let mut last_line = String::new();
    for line in lines {
        match line {
            Ok(text) if !text.trim().is_empty() => {
                last_line = text;
            }
            _ => continue,
        }
    }
    if let Ok(event) = serde_json::from_str::<Value>(last_line.trim()) {
        last_time = event.get("time").and_then(Value::as_f64).unwrap_or(0.0);
    }
    let started_at = head.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0);
    let duration = if last_time > 0.0 {
        last_time
    } else {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_secs_f64())
            .filter(|modified| *modified > started_at)
            .map(|modified| modified - started_at)
            .unwrap_or(0.0)
    };
    Some(json!({
        "recordingId": recording_id,
        "sessionId": meta.get("sessionId").cloned().unwrap_or(json!("")),
        "connectionId": meta.get("connectionId").cloned().unwrap_or(json!("")),
        "host": meta.get("host").cloned().unwrap_or(json!("")),
        "startedAt": head.get("timestamp").cloned().unwrap_or(json!(0)),
        "durationSecs": duration,
        "bytes": std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
    }))
}
