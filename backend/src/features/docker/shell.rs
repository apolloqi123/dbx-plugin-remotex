//! Live container terminals: one bounded registry per sidecar, command
//! rendering, event emission and TTL reaping. SSH and local shells share the
//! same registry and event surface.
//!
//! Each terminal owns a cancellation switch plus bounded input/resize queues.
//! A runner task (SSH PTY channel or local `portable-pty`) feeds an output
//! sink; the forwarder turns every chunk into one binary event on
//! `docker/terminal/out/{id}` and one `docker/terminal/state` event when the
//! shell exits or the transport fails.

use super::shell_local::{drive_local_pty, spawn_local_pty, SHELL_INNER};
use super::*;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use dbx_plugin_sdk::PluginEmitter;
use tokio::sync::{mpsc, watch, Notify};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::sessions::ssh::{exec, SshRuntime};

#[path = "shell_forward.rs"]
mod shell_forward;

/// Per-sidecar ceiling on concurrent container terminals. Each terminal holds
/// an SSH channel or a child PTY plus a forwarder task, so the cap keeps a
/// stuck UI (or a runaway script) from exhausting either.
pub const MAX_ACTIVE_TERMINALS: usize = 4;
/// Hard lifetime of one terminal. A forgotten pane would otherwise hold its
/// PTY (and the container's `exec` session) forever; the forwarder cancels it
/// and releases the slot.
pub const TERMINAL_TTL: Duration = Duration::from_secs(2 * 60 * 60);

/// How often the forwarder checks [`TERMINAL_TTL`]. The check rides the same
/// task as the output pump, so it cannot outlive the registry entry.
const TTL_TICK: Duration = Duration::from_secs(30);
/// Output queue bound: a slow host bridge back-pressures the PTY reader
/// instead of buffering an unbounded terminal backlog.
const OUTPUT_QUEUE: usize = 64;
const INPUT_QUEUE: usize = 256;
const RESIZE_QUEUE: usize = 32;
/// How long a cancelled runner gets to wind down before its task is aborted.
const STOP_GRACE: Duration = Duration::from_secs(5);
/// Upper bound an explicit close waits for the runner to release its channel.
const CLOSE_WAIT: Duration = Duration::from_secs(2);
/// State-event error for a terminal that hit [`TERMINAL_TTL`].
const TIMEOUT_ERROR: &str = "Container terminal timed out";
/// How long a closed terminal id stays tombstoned. An open that raced the
/// close (webview torn down mid-open) must not register a PTY that nobody
/// can stop; ids are client-generated and never reused.
const CLOSED_TERMINAL_TTL: Duration = Duration::from_secs(120);
const CLOSED_TERMINAL_ERROR: &str = "Container terminal was closed";

/// Tunables of one terminal. Production uses [`TerminalLimits::default`];
/// tests shorten the TTL so expiry is observable in milliseconds.
#[derive(Clone, Copy)]
struct TerminalLimits {
    ttl: Duration,
    stop_grace: Duration,
}

impl Default for TerminalLimits {
    fn default() -> Self {
        Self {
            ttl: TERMINAL_TTL,
            stop_grace: STOP_GRACE,
        }
    }
}

impl TerminalLimits {
    fn tick(&self) -> Duration {
        self.ttl
            .min(TTL_TICK)
            .max(Duration::from_millis(1))
    }
}

/// One registered terminal: the cancellation switch, the input/resize queues,
/// the explicit-close flag (close must not emit a state event) and the start
/// instant the TTL is measured from.
struct TerminalEntry {
    cancel: watch::Sender<bool>,
    input: mpsc::Sender<Vec<u8>>,
    resize: mpsc::Sender<(u32, u32)>,
    closed: Arc<AtomicBool>,
    started: Instant,
    /// Log-pane scope that owns this terminal (frontend instance id). A new
    /// open on the same scope replaces the old PTY even if its close RPC was
    /// lost.
    owner: String,
    /// SSH session the PTY runs on (empty for local PTYs); `ssh/session/close`
    /// cancels by this key.
    session: String,
    /// Signalled once the forwarder has removed this entry.
    done: Arc<Notify>,
}

/// The receivers handed to a runner once its slot is reserved.
struct Reserved {
    cancel: watch::Sender<bool>,
    cancel_rx: watch::Receiver<bool>,
    input_rx: mpsc::Receiver<Vec<u8>>,
    resize_rx: mpsc::Receiver<(u32, u32)>,
    closed: Arc<AtomicBool>,
    done: Arc<Notify>,
}

/// The fixed remote command for one container shell: engine endpoint export
/// (when configured) + `exec -i -t -e TERM=xterm-256color <id> sh -c
/// '<bash, else sh>'`. User input never reaches this text.
pub fn shell_ssh_command(engine: &EngineConfig, container_id: &str) -> Result<String, String> {
    validate_container_id(container_id)?;
    Ok(format!(
        "{}{} exec -i -t -e TERM=xterm-256color {container_id} sh -c {}",
        engine.env_prefix(),
        engine.cli,
        exec::shell_quote(SHELL_INNER),
    ))
}

/// Audit/approval text for one terminal: the short form without the endpoint
/// export and without the PTY flags or shell body.
pub fn shell_audit_text(engine: &EngineConfig, container_id: &str) -> Result<String, String> {
    validate_container_id(container_id)?;
    Ok(format!("{} exec -it {container_id} sh", engine.cli))
}

/// Validates a client-supplied terminal id or generates one. A supplied id is
/// honored verbatim (trimmed) so the frontend can subscribe before `open`
/// returns and never loses early output.
pub fn requested_terminal_id(requested: Option<&str>) -> Result<String, String> {
    let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Uuid::new_v4().to_string());
    };
    let valid = (8..=64).contains(&requested.len())
        && requested
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !valid {
        return Err(format!(
            "Invalid terminalId '{requested}': expected 8-64 letters, digits or '-'"
        ));
    }
    Ok(requested.to_string())
}

/// Boundary marker between the shared reservation helpers and the start paths.
pub struct DockerTerminals {
    inner: Mutex<HashMap<String, TerminalEntry>>,
    /// Terminal ids whose close already arrived. See [`CLOSED_TERMINAL_TTL`].
    closed_ids: Mutex<HashMap<String, Instant>>,
}

impl DockerTerminals {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            closed_ids: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, TerminalEntry>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn closed_lock(&self) -> MutexGuard<'_, HashMap<String, Instant>> {
        self.closed_ids
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Prunes expired tombstones and reports whether `terminal_id` is one.
    fn is_closed(&self, terminal_id: &str) -> bool {
        let mut closed = self.closed_lock();
        let now = Instant::now();
        closed.retain(|_, at| now.duration_since(*at) < CLOSED_TERMINAL_TTL);
        closed.contains_key(terminal_id)
    }

    /// Remembers a closed id so an open that raced the close is rejected.
    /// Called while holding the streams lock (same order as `reserve`).
    fn tombstone(&self, terminal_id: &str) {
        let mut closed = self.closed_lock();
        let now = Instant::now();
        closed.retain(|_, at| now.duration_since(*at) < CLOSED_TERMINAL_TTL);
        closed.insert(terminal_id.to_string(), now);
    }

    /// Admission check + entry insertion in one critical section: two racing
    /// opens cannot both observe the last free slot. A new open on the same
    /// owner (pane) or SSH session supersedes the old entry, so a lost close
    /// or a rapid remount cannot leave a second PTY alive.
    fn reserve(
        &self,
        terminal_id: &str,
        owner: &str,
        session: &str,
    ) -> Result<Reserved, String> {
        let mut superseded = Vec::new();
        let reserved = {
            let mut inner = self.lock();
            if self.is_closed(terminal_id) {
                return Err(CLOSED_TERMINAL_ERROR.to_string());
            }
            if !owner.is_empty() || !session.is_empty() {
                inner.retain(|_, entry| {
                    let matches = (!owner.is_empty() && entry.owner == owner)
                        || (!session.is_empty() && entry.session == session);
                    if matches {
                        superseded.push((entry.cancel.clone(), Arc::clone(&entry.closed)));
                        false
                    } else {
                        true
                    }
                });
            }
            if inner.len() >= MAX_ACTIVE_TERMINALS {
                Err(format!(
                    "Too many active container terminals ({MAX_ACTIVE_TERMINALS}). Close one before opening another."
                ))
            } else if inner.contains_key(terminal_id) {
                Err(format!("terminalId '{terminal_id}' is already in use"))
            } else {
                let (cancel, cancel_rx) = watch::channel(false);
                let (input_tx, input_rx) = mpsc::channel(INPUT_QUEUE);
                let (resize_tx, resize_rx) = mpsc::channel(RESIZE_QUEUE);
                let closed = Arc::new(AtomicBool::new(false));
                let done = Arc::new(Notify::new());
                inner.insert(
                    terminal_id.to_string(),
                    TerminalEntry {
                        cancel: cancel.clone(),
                        input: input_tx,
                        resize: resize_tx,
                        closed: Arc::clone(&closed),
                        started: Instant::now(),
                        owner: owner.to_string(),
                        session: session.to_string(),
                        done: Arc::clone(&done),
                    },
                );
                Ok(Reserved {
                    cancel,
                    cancel_rx,
                    input_rx,
                    resize_rx,
                    closed,
                    done,
                })
            }
        };
        // Superseded PTYs are explicit replacements: no state event.
        for (cancel, closed) in superseded {
            closed.store(true, Ordering::Relaxed);
            let _ = cancel.send(true);
        }
        reserved
    }

    /// Starts a container shell over an SSH session. `ensure_writable` is the
    /// RPC gate (it must reject before the audit row), so it is deliberately
    /// not repeated here.
    ///
    /// Must be called from inside the plugin's Tokio runtime: the runner and
    /// forwarder are spawned onto it and outlive this call.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_ssh(
        self: &Arc<Self>,
        ssh: Arc<SshRuntime>,
        emitter: PluginEmitter,
        session_id: String,
        owner: String,
        container_id: &str,
        engine: EngineConfig,
        terminal_id: Option<&str>,
        cols: u32,
        rows: u32,
    ) -> Result<Value, String> {
        let command = shell_ssh_command(&engine, container_id)?;
        let terminal_id = requested_terminal_id(terminal_id)?;
        let owner = if owner.is_empty() {
            session_id.clone()
        } else {
            owner
        };
        // Resolve the session before admission so a vanished session is a
        // clear RPC error instead of an open that immediately emits `state`.
        ssh.ensure_container_session(&session_id).await?;
        let Reserved {
            cancel,
            cancel_rx,
            input_rx,
            resize_rx,
            closed,
            done,
        } = self.reserve(&terminal_id, &owner, &session_id)?;
        let (sink, sink_rx) = mpsc::channel(OUTPUT_QUEUE);
        let runner_id = terminal_id.clone();
        let handle = tokio::spawn(async move {
            ssh.run_container_terminal(
                &session_id, &command, cols, rows, sink, input_rx, resize_rx, cancel_rx,
            )
            .await
        });
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            registry
                .forward(
                    runner_id,
                    emitter,
                    closed,
                    done,
                    cancel,
                    handle,
                    sink_rx,
                    TerminalLimits::default(),
                )
                .await;
        });
        Ok(json!({ "terminalId": terminal_id }))
    }

    /// Starts a container shell on the sidecar's own daemon. The id and the
    /// registry slot are settled before the PTY exists; a spawn failure
    /// releases the reservation so neither a slot nor a PTY can leak.
    pub fn start_local(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        container_id: &str,
        engine: EngineConfig,
        owner: String,
        terminal_id: Option<&str>,
        cols: u32,
        rows: u32,
    ) -> Result<Value, String> {
        let terminal_id = requested_terminal_id(terminal_id)?;
        validate_container_id(container_id)?;
        let owner = if owner.is_empty() {
            "local".to_string()
        } else {
            owner
        };
        let Reserved {
            cancel,
            cancel_rx,
            input_rx,
            resize_rx,
            closed,
            done,
        } = self.reserve(&terminal_id, &owner, "")?;
        let pty = match spawn_local_pty(&engine, container_id, cols, rows) {
            Ok(pty) => pty,
            Err(error) => {
                self.lock().remove(&terminal_id);
                return Err(error);
            }
        };
        let (sink, sink_rx) = mpsc::channel(OUTPUT_QUEUE);
        let runner_id = terminal_id.clone();
        let handle = tokio::spawn(async move {
            drive_local_pty(pty, sink, input_rx, resize_rx, cancel_rx).await
        });
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            registry
                .forward(
                    runner_id,
                    emitter,
                    closed,
                    done,
                    cancel,
                    handle,
                    sink_rx,
                    TerminalLimits::default(),
                )
                .await;
        });
        Ok(json!({ "terminalId": terminal_id }))
    }

    /// Enqueues keystrokes for one terminal. Only a closed queue (the runner
    /// is gone) is an error; a transiently full queue drops one chunk.
    pub fn write_input(&self, terminal_id: &str, data: Vec<u8>) -> Result<(), String> {
        let sender = {
            let inner = self.lock();
            inner
                .get(terminal_id)
                .map(|entry| entry.input.clone())
                .ok_or("Container terminal was not found or expired")?
        };
        crate::sessions::input_queue::enqueue_bounded(&sender, data, "Container terminal")
    }

    /// Applies a window size. Unknown ids answer `false`; a full resize queue
    /// also answers `false` so the caller can retry the newest geometry.
    pub fn resize(&self, terminal_id: &str, cols: u32, rows: u32) -> bool {
        let inner = self.lock();
        match inner.get(terminal_id) {
            Some(entry) => entry
                .resize
                .try_send((cols.max(1), rows.max(1)))
                .is_ok(),
            None => false,
        }
    }

    /// Cancels one terminal. Unknown/expired ids answer `false`, never an
    /// error, so the RPC stays idempotent.
    /// Cancels a terminal and waits (briefly) for its runner to release the
    /// SSH channel. Serialising close→open avoids tripping the server's
    /// `MaxSessions` guard when the user switches containers quickly.
    pub async fn close_and_wait(&self, terminal_id: &str) -> bool {
        let found = {
            let inner = self.lock();
            // Tombstone even an unknown id: an open that was still on the
            // wire when the pane closed must not register a PTY afterwards.
            self.tombstone(terminal_id);
            inner.get(terminal_id).map(|entry| {
                (
                    entry.cancel.clone(),
                    Arc::clone(&entry.closed),
                    Arc::clone(&entry.done),
                )
            })
        };
        let Some((cancel, closed, done)) = found else {
            return false;
        };
        closed.store(true, Ordering::Relaxed);
        let notified = done.notified();
        let _ = cancel.send(true);
        let _ = tokio::time::timeout(CLOSE_WAIT, notified).await;
        true
    }

    /// Cancels every terminal running on one SSH session (session teardown).
    /// The PTY channel would die with the transport anyway; doing it here
    /// releases the registry slot and reaps the runner deterministically.
    pub(crate) fn cancel_session(&self, session: &str) {
        if session.is_empty() {
            return;
        }
        let retired = {
            let mut inner = self.lock();
            let mut retired = Vec::new();
            inner.retain(|_, entry| {
                if entry.session == session {
                    retired.push((entry.cancel.clone(), Arc::clone(&entry.closed)));
                    false
                } else {
                    true
                }
            });
            retired
        };
        for (cancel, closed) in retired {
            closed.store(true, Ordering::Relaxed);
            let _ = cancel.send(true);
        }
    }

    /// Cancels every registered terminal (registry teardown). The runners wind
    /// down through their own cancel paths, so remote channels and local
    /// children are reaped instead of abandoned.
    pub fn stop_all(&self) {
        let entries: Vec<TerminalEntry> = {
            let mut inner = self.lock();
            inner.drain().map(|(_, entry)| entry).collect()
        };
        for entry in entries {
            entry.closed.store(true, Ordering::Relaxed);
            let _ = entry.cancel.send(true);
        }
    }
}

impl Default for DockerTerminals {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DockerTerminals {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// Fixed-command audit intent for one container terminal
/// (`docker/terminal`). Keystrokes and PTY output are never audited.
pub fn audit_terminal_intent(data_dir: &Path, command: &str) {
    super::local::audit_intent(data_dir, "docker/terminal", command);
}

