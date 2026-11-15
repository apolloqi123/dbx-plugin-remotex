//! Local (client-machine) terminal sessions.
//!
//! Spawns an interactive login shell in a pseudo terminal on the machine the
//! sidecar runs on and pumps it over the same binary frame protocol the SSH
//! terminal uses — `local/terminal/in/{sessionId}` carries the 8-byte BE
//! sequence + keystrokes, `local/terminal/out/{sessionId}` answers with the
//! 9-byte `TerminalFrame` prefix (stream + u64 sequence) backed by a shared
//! [`ReplayBuffer`]. Output streams merge inside the PTY, so every data frame
//! is `Stdout`; `State` frames only carry the terminal lifecycle.
//!
//! Deliberately out of scope here: everything SSH-specific (triggers,
//! auto-sudo, directory handshake, recording, SFTP). The runtime owns its own
//! session table — the SSH `SessionEntry` binds a russh handle and cannot
//! carry a local child process.
//!
//! Shell choice follows the platform terminals' hard-won lesson (Ghostty,
//! Warp, and the ttyd/Tabby breakage reports): on macOS a GUI process has no
//! usable `PATH`, so the shell MUST start as a login shell, resolved from
//! Directory Services' `UserShell` first. Injection only ever wraps the
//! user's own startup chain — a broken rc must never keep the shell from
//! reaching a prompt.

pub(crate) mod target;

mod conpty;
mod env;
mod integration;
mod runtime;
mod shells;

#[cfg(test)]
mod tests;

use crate::features::recording as session_recording;

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dbx_plugin_sdk::PluginEmitter;
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::{mpsc, RwLock};

use crate::core::model::TerminalStream;
use crate::sessions::ssh::ReplayBuffer;

use conpty::*;
pub(crate) use env::sanitize_env_pair;
/// Grace for a EOF'd/HUP'd child to exit on its own (zsh/bash flush command
/// history only on a clean exit) before the killer escalates to SIGKILL.
use env::*;
use integration::*;
#[allow(unused_imports)]
use runtime::*;
use shells::*;
pub(crate) use shells::{resolve_configured_shell, resolve_default_shell};

const LOCAL_CLOSE_GRACE: Duration = Duration::from_secs(5);
/// Second, shorter window after SIGKILL for the waiter thread to deliver the
/// status; the child is being reaped either way.
const LOCAL_KILL_GRACE: Duration = Duration::from_secs(2);
/// Per-chunk budget for draining the PTY tail after the child is gone; the
/// reader thread gets this long between chunks before the exited State frame
/// is published without it.
const LOCAL_EXIT_DRAIN: Duration = Duration::from_millis(500);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalTerminalStartRequest {
    pub workbench_id: String,
    pub cols: u32,
    pub rows: u32,
    /// Explicit shell override (absolute path or PATH-resolvable name).
    pub shell: Option<String>,
    /// Opt-out of shell integration injection; defaults to on.
    pub shell_integration: Option<bool>,
    /// Working directory to start in (workbench restart inherits the last
    /// tracked cwd, VS Code new-terminal-in-workspace style). Must be an
    /// existing directory; invalid values fall back to the home directory.
    pub cwd: Option<String>,
    /// The terminal's cursor position (1-based, on-screen) as the workbench
    /// sees it at spawn time. ConPTY is created with
    /// `PSEUDOCONSOLE_INHERIT_CURSOR`, so its opening `CSI 6n` asks the
    /// terminal where the cursor is and starts the shell there; the reply must
    /// carry this value, never the viewport size. Defaults to the top-left for
    /// older workbenches that do not send it.
    pub cursor_row: Option<u16>,
    pub cursor_col: Option<u16>,
    /// User-supplied environment variables layered over the inherited system
    /// environment. Renderer input is sanitized defensively
    /// ([`sanitize_user_env`]): integration env and the managed TERM family
    /// always win, so a bad value can never break the rendering contract —
    /// and a bad entry must never keep the shell from reaching a prompt.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

enum LocalTerminalCommand {
    Input(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    Close,
}

struct LocalSession {
    workbench_id: String,
    shell: String,
    /// Unix seconds when the session was opened (for `local/session/list`).
    created_at_secs: u64,
    terminal_tx: mpsc::Sender<LocalTerminalCommand>,
    replay: Arc<AsyncMutex<ReplayBuffer>>,
    /// Active asciicast recorder (`local/recording/start`); `None` while not
    /// recording. Fed from the pump's output path, finished by the stop RPC —
    /// a shell exit drops the slot, and the buffered writer flushes on drop.
    recorder: Arc<Mutex<Option<session_recording::SessionRecorder>>>,
    /// Spawn-time geometry, reused as the recording header's dimensions.
    cols: u16,
    rows: u16,
}

pub struct LocalTerminalRuntime {
    sessions: Arc<RwLock<HashMap<String, Arc<LocalSession>>>>,
}

impl ConptyHandshakeFilter {
    fn new(needed: bool) -> Self {
        Self {
            done: !needed,
            held: Vec::new(),
        }
    }

    /// Filters one PTY chunk. Returns the bytes to forward to the terminal and
    /// whether this chunk carries the handshake query — which the caller must
    /// answer through the PTY input instead of the terminal. Takes the chunk by
    /// value: after the handshake is done (always on macOS/Linux) or once no
    /// query prefix is held back, the buffer is forwarded without a copy.
    fn filter(&mut self, chunk: Vec<u8>) -> (Vec<u8>, bool) {
        if self.done {
            return (chunk, false);
        }
        let mut scan = std::mem::take(&mut self.held);
        scan.extend_from_slice(&chunk);
        if let Some(at) = scan
            .windows(CONPTY_CPR_QUERY.len())
            .position(|window| window == CONPTY_CPR_QUERY)
        {
            self.done = true;
            let mut forward = Vec::with_capacity(scan.len() - CONPTY_CPR_QUERY.len());
            forward.extend_from_slice(&scan[..at]);
            forward.extend_from_slice(&scan[at + CONPTY_CPR_QUERY.len()..]);
            return (forward, true);
        }
        // No query yet: hold back a trailing proper prefix of it so a query
        // split across chunks is still caught, and forward everything else.
        let keep = (1..CONPTY_CPR_QUERY.len())
            .rev()
            .find(|length| scan.ends_with(&CONPTY_CPR_QUERY[..*length]))
            .unwrap_or(0);
        if keep == 0 {
            return (scan, false);
        }
        let split = scan.len() - keep;
        self.held = scan[split..].to_vec();
        scan.truncate(split);
        (scan, false)
    }

    /// Releases bytes still held when the stream ends.
    fn flush(&mut self) -> Vec<u8> {
        self.done = true;
        std::mem::take(&mut self.held)
    }
}

impl StartupResizeGate {
    fn new(platform: Platform) -> Self {
        Self {
            platform,
            last_output_at: None,
            pending: None,
        }
    }

    fn on_output(&mut self, elapsed: Duration) {
        self.last_output_at = Some(elapsed);
    }

    fn resize(&mut self, size: PtySize, elapsed: Duration) -> StartupResizeDecision {
        if self.platform != Platform::Windows || elapsed >= STARTUP_RESIZE_WINDOW {
            self.pending = None;
            return StartupResizeDecision::Apply;
        }
        let output_in_flight = self
            .last_output_at
            .is_some_and(|at| elapsed.saturating_sub(at) < STARTUP_RESIZE_QUIET);
        if output_in_flight {
            self.pending = Some(size);
            StartupResizeDecision::Defer
        } else {
            // No output yet, or it already went quiet: a repaint now is safe.
            self.pending = None;
            StartupResizeDecision::Apply
        }
    }

    /// The elapsed-since-spawn instant when a pending resize can flush:
    /// last output + QUIET, capped at the startup window. `None` when nothing
    /// is pending (drives a never-firing timer on non-Windows).
    fn next_wake(&self) -> Option<Duration> {
        let _pending = self.pending.as_ref()?;
        let quiet_at = self.last_output_at? + STARTUP_RESIZE_QUIET;
        Some(quiet_at.min(STARTUP_RESIZE_WINDOW))
    }

    /// Releases the pending resize once quiet (or the window) has been
    /// reached; consumes it so it applies exactly once.
    fn poll(&mut self, elapsed: Duration) -> Option<PtySize> {
        let size = self.pending?;
        let flush_at = self.last_output_at.map_or(STARTUP_RESIZE_WINDOW, |at| {
            (at + STARTUP_RESIZE_QUIET).min(STARTUP_RESIZE_WINDOW)
        });
        if elapsed >= flush_at {
            self.pending = None;
            Some(size)
        } else {
            None
        }
    }

    /// Test-only view of the deferred geometry.
    #[cfg(test)]
    fn pending(&self) -> Option<PtySize> {
        self.pending
    }
}
