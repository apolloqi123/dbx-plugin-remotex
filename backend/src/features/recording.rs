//! Session recording: captures a terminal session's output stream into an
//! asciicast v2 file (`.cast`, JSONL) under `<plugin_data_dir>/recordings/`.
//! One recording per session at a time; the recorder is fed by the session's
//! read loop at the same point as the agent-terminal recorder, so everything
//! the terminal shows (stdout + stderr, after the directory-tracking filter)
//! lands in the file.
//!
//! Format: line 1 is the asciicast v2 header (`{"version":2,...}` with a
//! plugin `meta` block carrying sessionId/connectionId/host); every further
//! line is `{"time": <secs since start>, "eventtype": "o", "eventdata":
//! "<utf-8 output>"}`. Bounded custom `x` events carry normalized pointer /
//! caret-cell metadata for replay overlays; they are never terminal output.
//! Replay consumers (asciinema-compatible) ignore the extra `meta` field;
//! the plugin's own replay reads it back for the list view without touching
//! output event lines. Legacy lines without `eventtype` remain output.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::core::time::unix_now_secs;

/// Directory holding the `.cast` files.
pub const RECORDINGS_DIR: &str = "recordings";
/// Cap on one event's stored size — a single PTY burst above this is
/// truncated on write (the terminal itself renders lossily anyway).
const MAX_EVENT_BYTES: usize = 256 * 1024;
/// How much of the session's recent output is written at t=0 when a
/// recording starts on a live terminal, so replay opens on the screen that
/// was already visible instead of a blank canvas.
pub const RECORDING_SEED_BYTES: usize = 256 * 1024;
/// Max events served by one `ssh/recording/get` page.
pub const PAGE_LIMIT: usize = 500;
/// Max recordings a single `ssh/recording/search` scan walks — the RPC is an
/// instant on-demand scan over existing files, not a persistent index, so the
/// budget is bounded (task spec: ≤200 recordings).
pub const SEARCH_MAX_RECORDINGS: usize = 200;
/// Max text hits returned per recording (excerpts are small, but the payload
/// must stay bounded).
pub const SEARCH_MAX_HITS: usize = 5;
/// Cap on one structured metadata (`x`) event's serialized size. Pointer and
/// caret samples are tiny; this keeps the custom event lane bounded even if a
/// caller sends a padded object.
pub const MAX_METADATA_EVENT_BYTES: usize = 512;

/// Fast-path `auto_record` flag (mirrors the `x11_forwarding` pattern: an
/// in-process atomic kept in lockstep with the allowlisted preferences.json
/// key; read per `open_session` without re-parsing the file).
static AUTO_RECORD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_auto_record(enabled: bool) {
    AUTO_RECORD.store(enabled, std::sync::atomic::Ordering::Release);
}

pub fn auto_record_enabled() -> bool {
    AUTO_RECORD.load(std::sync::atomic::Ordering::Acquire)
}

pub fn recordings_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(RECORDINGS_DIR)
}

/// Builds the header for a fresh recording. `width`/`height` mirror the
/// session's PTY size when known; the plugin's replay scales to its own
/// xterm size regardless.
pub fn header(connection_id: &str, host: &str, session_id: &str, width: u32, height: u32) -> Value {
    json!({
        "version": 2,
        "width": width.max(1),
        "height": height.max(1),
        "timestamp": unix_now_secs(),
        "env": { "TERM": "xterm-256color" },
        "meta": {
            "recorderId": session_id,
            "sessionId": session_id,
            "connectionId": connection_id,
            "host": host,
        },
    })
}

fn metadata_keys_exact(
    object: &serde_json::Map<String, Value>,
    expected: &[&str],
) -> Result<(), String> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err("Recording metadata event has unexpected keys".to_string());
    }
    Ok(())
}

fn metadata_geometry(value: Option<&Value>) -> Result<u64, String> {
    let geometry = value
        .and_then(Value::as_u64)
        .ok_or("Recording metadata geometry must be a positive integer")?;
    if !(1..=1000).contains(&geometry) {
        return Err("Recording metadata geometry must be within 1..=1000".to_string());
    }
    Ok(geometry)
}

fn metadata_unit(value: Option<&Value>) -> Result<f64, String> {
    let coordinate = value
        .and_then(Value::as_f64)
        .ok_or("Recording metadata coordinate must be a finite number")?;
    if !coordinate.is_finite() || !(0.0..=1.0).contains(&coordinate) {
        return Err("Recording metadata coordinate must be within 0..=1".to_string());
    }
    Ok(coordinate)
}

fn metadata_cell(value: Option<&Value>, bound: u64) -> Result<u64, String> {
    let cell = value
        .and_then(Value::as_u64)
        .ok_or("Recording metadata caret cell must be an integer")?;
    if cell >= bound {
        return Err("Recording metadata caret cell is outside the terminal geometry".to_string());
    }
    Ok(cell)
}

/// Validates and canonicalizes one bounded pointer / caret sample. Only the
/// two exact shapes are accepted; no text, path, keystroke or OS-global
/// coordinate is ever part of the payload.
pub fn validate_metadata_event(data: &Value) -> Result<Value, String> {
    let encoded = serde_json::to_string(data)
        .map_err(|error| format!("Failed to encode recording metadata: {error}"))?;
    if encoded.len() > MAX_METADATA_EVENT_BYTES {
        return Err(format!(
            "Recording metadata event exceeds {MAX_METADATA_EVENT_BYTES} bytes"
        ));
    }
    let Some(object) = data.as_object() else {
        return Err("Recording metadata event must be an object".to_string());
    };
    let Some(kind) = object.get("kind").and_then(Value::as_str) else {
        return Err("Recording metadata event is missing its kind".to_string());
    };
    let cols = metadata_geometry(object.get("cols"))?;
    let rows = metadata_geometry(object.get("rows"))?;
    match kind {
        "pointer" => {
            metadata_keys_exact(
                object,
                &["kind", "x", "y", "pressed", "cols", "rows"],
            )?;
            let x = metadata_unit(object.get("x"))?;
            let y = metadata_unit(object.get("y"))?;
            let pressed = object
                .get("pressed")
                .and_then(Value::as_bool)
                .ok_or("Recording pointer pressed state must be a boolean")?;
            Ok(json!({
                "kind": "pointer",
                "x": x,
                "y": y,
                "pressed": pressed,
                "cols": cols,
                "rows": rows,
            }))
        }
        "cursor" => {
            metadata_keys_exact(object, &["kind", "col", "row", "cols", "rows"])?;
            let col = metadata_cell(object.get("col"), cols)?;
            let row = metadata_cell(object.get("row"), rows)?;
            Ok(json!({
                "kind": "cursor",
                "col": col,
                "row": row,
                "cols": cols,
                "rows": rows,
            }))
        }
        _ => Err("Unknown recording metadata event kind".to_string()),
    }
}

mod catalog;
mod recorder;
mod search;

#[cfg(test)]
mod tests;

pub use catalog::*;
pub use recorder::*;
pub use search::*;
