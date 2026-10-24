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
//! "<utf-8 output>"}`. Replay consumers (asciinema-compatible) ignore the
//! extra `meta` field; the plugin's own replay reads it back for the list
//! view without touching event lines.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::core::time::unix_now_secs;

/// Directory holding the `.cast` files.
pub const RECORDINGS_DIR: &str = "recordings";
/// Cap on one event's stored size — a single PTY burst above this is
/// truncated on write (the terminal itself renders lossily anyway).
const MAX_EVENT_BYTES: usize = 256 * 1024;
/// Max events served by one `ssh/recording/get` page.
pub const PAGE_LIMIT: usize = 500;
/// Max recordings a single `ssh/recording/search` scan walks — the RPC is an
/// instant on-demand scan over existing files, not a persistent index, so the
/// budget is bounded (task spec: ≤200 recordings).
pub const SEARCH_MAX_RECORDINGS: usize = 200;
/// Max text hits returned per recording (excerpts are small, but the payload
/// must stay bounded).
pub const SEARCH_MAX_HITS: usize = 5;

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

mod catalog;
mod recorder;
mod search;

#[cfg(test)]
mod tests;

pub use catalog::*;
pub use recorder::*;
pub use search::*;
