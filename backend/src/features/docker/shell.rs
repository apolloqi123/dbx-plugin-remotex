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
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::sessions::ssh::{exec, SshRuntime};

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
/// State-event error for a terminal that hit [`TERMINAL_TTL`].
const TIMEOUT_ERROR: &str = "Container terminal timed out";

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
}

/// The receivers handed to a runner once its slot is reserved.
struct Reserved {
    cancel: watch::Sender<bool>,
    cancel_rx: watch::Receiver<bool>,
    input_rx: mpsc::Receiver<Vec<u8>>,
    resize_rx: mpsc::Receiver<(u32, u32)>,
    closed: Arc<AtomicBool>,
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
}

impl DockerTerminals {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, TerminalEntry>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Admission check + entry insertion in one critical section: two racing
    /// opens cannot both observe the last free slot.
    fn reserve(&self, terminal_id: &str) -> Result<Reserved, String> {
        let mut inner = self.lock();
        if inner.len() >= MAX_ACTIVE_TERMINALS {
            return Err(format!(
                "Too many active container terminals ({MAX_ACTIVE_TERMINALS}). Close one before opening another."
            ));
        }
        if inner.contains_key(terminal_id) {
            return Err(format!("terminalId '{terminal_id}' is already in use"));
        }
        let (cancel, cancel_rx) = watch::channel(false);
        let (input_tx, input_rx) = mpsc::channel(INPUT_QUEUE);
        let (resize_tx, resize_rx) = mpsc::channel(RESIZE_QUEUE);
        let closed = Arc::new(AtomicBool::new(false));
        inner.insert(
            terminal_id.to_string(),
            TerminalEntry {
                cancel: cancel.clone(),
                input: input_tx,
                resize: resize_tx,
                closed: Arc::clone(&closed),
                started: Instant::now(),
            },
        );
        Ok(Reserved {
            cancel,
            cancel_rx,
            input_rx,
            resize_rx,
            closed,
        })
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
        container_id: &str,
        engine: EngineConfig,
        terminal_id: Option<&str>,
        cols: u32,
        rows: u32,
    ) -> Result<Value, String> {
        let command = shell_ssh_command(&engine, container_id)?;
        let terminal_id = requested_terminal_id(terminal_id)?;
        let Reserved {
            cancel,
            cancel_rx,
            input_rx,
            resize_rx,
            closed,
        } = self.reserve(&terminal_id)?;
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
        terminal_id: Option<&str>,
        cols: u32,
        rows: u32,
    ) -> Result<Value, String> {
        let terminal_id = requested_terminal_id(terminal_id)?;
        validate_container_id(container_id)?;
        let Reserved {
            cancel,
            cancel_rx,
            input_rx,
            resize_rx,
            closed,
        } = self.reserve(&terminal_id)?;
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
                    cancel,
                    handle,
                    sink_rx,
                    TerminalLimits::default(),
                )
                .await;
        });
        Ok(json!({ "terminalId": terminal_id }))
    }

    /// Drains raw PTY chunks into binary events, enforces the TTL, then emits
    /// the terminal state (unless the close was explicit) and releases the
    /// slot. A runner that ends immediately still lands here: the closed sink
    /// wakes the loop and its result becomes the state event.
    #[allow(clippy::too_many_arguments)]
    async fn forward(
        self: Arc<Self>,
        terminal_id: String,
        emitter: PluginEmitter,
        closed: Arc<AtomicBool>,
        cancel: watch::Sender<bool>,
        mut runner: JoinHandle<Result<Option<i32>, String>>,
        mut sink: mpsc::Receiver<Vec<u8>>,
        limits: TerminalLimits,
    ) {
        let started = self.started(&terminal_id);
        let mut ticker = tokio::time::interval(limits.tick());
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut expired = false;
        let result = loop {
            tokio::select! {
                biased;
                _ = ticker.tick() => {
                    if started.is_some_and(|started| started.elapsed() >= limits.ttl) {
                        expired = true;
                        let _ = cancel.send(true);
                        break stop_runner(&mut runner, limits.stop_grace).await;
                    }
                }
                chunk = sink.recv() => match chunk {
                    Some(bytes) if !bytes.is_empty() => {
                        let _ = emitter.binary(&format!("docker/terminal/out/{terminal_id}"), &bytes);
                    }
                    Some(_) => {}
                    None => break stop_runner(&mut runner, limits.stop_grace).await,
                },
                status = &mut runner => break status,
            }
        };
        while let Ok(bytes) = sink.try_recv() {
            if !bytes.is_empty() {
                let _ = emitter.binary(&format!("docker/terminal/out/{terminal_id}"), &bytes);
            }
        }
        self.lock().remove(&terminal_id);
        // An explicit close is not a state transition the UI has to handle.
        if closed.load(Ordering::Relaxed) {
            return;
        }
        let payload = if expired {
            json!({ "terminalId": terminal_id, "state": "error", "error": TIMEOUT_ERROR })
        } else {
            match result {
                Ok(Ok(code)) => json!({
                    "terminalId": terminal_id,
                    "state": "exited",
                    "exitCode": code.unwrap_or(-1),
                }),
                Ok(Err(error)) => {
                    json!({ "terminalId": terminal_id, "state": "error", "error": error })
                }
                Err(error) => json!({
                    "terminalId": terminal_id,
                    "state": "error",
                    "error": format!("Container terminal task failed: {error}"),
                }),
            }
        };
        let _ = emitter.event("docker/terminal/state", payload);
    }

    fn started(&self, terminal_id: &str) -> Option<Instant> {
        self.lock()
            .get(terminal_id)
            .map(|entry| entry.started)
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
    /// still answers `true` (the next fit will resend the size).
    pub fn resize(&self, terminal_id: &str, cols: u32, rows: u32) -> bool {
        let inner = self.lock();
        match inner.get(terminal_id) {
            Some(entry) => {
                let _ = entry.resize.try_send((cols.max(1), rows.max(1)));
                true
            }
            None => false,
        }
    }

    /// Cancels one terminal. Unknown/expired ids answer `false`, never an
    /// error, so the RPC stays idempotent.
    pub fn close(&self, terminal_id: &str) -> bool {
        let inner = self.lock();
        match inner.get(terminal_id) {
            Some(entry) => {
                entry.closed.store(true, Ordering::Relaxed);
                let _ = entry.cancel.send(true);
                true
            }
            None => false,
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

/// Waits out a runner after the pump ends, bounded by `grace`. A runner that
/// ignores cancellation is aborted instead of awaited forever: the SSH
/// channel task is dropped and the local child is killed by its own pump.
async fn stop_runner(
    runner: &mut JoinHandle<Result<Option<i32>, String>>,
    grace: Duration,
) -> Result<Result<Option<i32>, String>, tokio::task::JoinError> {
    match tokio::time::timeout(grace, &mut *runner).await {
        Ok(result) => result,
        Err(_) => {
            runner.abort();
            runner.await
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

#[cfg(test)]
impl DockerTerminals {
    /// Inserts a placeholder entry (no runner) so cap tests stay cheap.
    pub(crate) async fn insert_for_test(&self, terminal_id: String) -> String {
        match self.reserve(&terminal_id) {
            Ok(reserved) => {
                drop(reserved);
                String::new()
            }
            Err(error) => error,
        }
    }

    pub(crate) fn remove_for_test(&self, terminal_id: &str) {
        self.lock().remove(terminal_id);
    }

    pub(crate) fn active_for_test(&self) -> usize {
        self.lock().len()
    }

    /// Starts a runner that emits one chunk and then ignores cancellation, so
    /// only the TTL can reap it (the same shape as a wedged child).
    pub(crate) fn start_fake_for_test(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        ttl: Duration,
    ) -> Result<String, String> {
        let terminal_id = requested_terminal_id(None)?;
        let Reserved {
            cancel,
            cancel_rx,
            input_rx,
            resize_rx,
            closed,
        } = self.reserve(&terminal_id)?;
        drop((cancel_rx, input_rx, resize_rx));
        let (sink, sink_rx) = mpsc::channel(OUTPUT_QUEUE);
        let handle = tokio::spawn(async move {
            let _ = sink.send(b"ready".to_vec()).await;
            std::future::pending::<()>().await;
            unreachable!("the fake terminal is only aborted by the TTL")
        });
        let registry = Arc::clone(self);
        let forward_id = terminal_id.clone();
        let limits = TerminalLimits {
            ttl,
            stop_grace: Duration::from_millis(20),
        };
        tokio::spawn(async move {
            registry
                .forward(
                    forward_id, emitter, closed, cancel, handle, sink_rx, limits,
                )
                .await;
        });
        Ok(terminal_id)
    }
}
