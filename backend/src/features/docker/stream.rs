//! Live container-log streams: one registry per sidecar, a bounded active
//! set, batching into sidecar events, and the TTL that reaps forgotten
//! `logs -f` sessions. Commands are rendered here so the SSH and local
//! planes share one tail clamp and one engine prefix.

use super::*;

use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use dbx_plugin_sdk::PluginEmitter;
use tokio::sync::{mpsc, watch, Notify};
use tokio::task::JoinHandle;

use crate::sessions::ssh::SshRuntime;

/// Per-sidecar ceiling on concurrent `logs -f` streams. Each stream holds an
/// SSH channel or a child process plus a forwarder task, so the cap keeps a
/// stuck UI (or a bug) from exhausting either.
pub const MAX_ACTIVE_STREAMS: usize = 4;
/// Hard lifetime of one stream. `docker logs -f` never exits on its own, so
/// a forgotten pane would otherwise follow forever; the forwarder expires
/// the stream, cancels the runner and emits its end event.
pub const STREAM_TTL: Duration = Duration::from_secs(30 * 60);

/// How often the forwarder checks [`STREAM_TTL`]. The check rides the same
/// task as the chunk pump, so it cannot outlive the registry entry.
const TTL_CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// How long a stopped runner gets to wind down before its task is aborted.
/// A runner that ignores cancellation (a wedged child or channel) must never
/// keep the registry slot or the end event hostage.
const STOP_GRACE: Duration = Duration::from_secs(5);
/// Upper bound an explicit stop waits for the forwarder to release its slot
/// (the runner's own grace plus the channel close round-trip).
const STOP_WAIT: Duration = Duration::from_secs(4);
/// How long a closed owner scope (one log pane instance) rejects new starts.
/// A start RPC that was already on the wire when the pane closed would
/// otherwise register a follower nobody can stop; the tombstone is long
/// enough to cover that round-trip and short enough to never affect a pane
/// that reuses the same scope id (which the frontend does not do).
const CLOSED_OWNER_TTL: Duration = Duration::from_secs(120);
/// End-event error for a stream that hit [`STREAM_TTL`].
const TIMEOUT_ERROR: &str = "stream timed out";
/// Start error for a scope whose pane already closed (see [`CLOSED_OWNER_TTL`]).
const CLOSED_OWNER_ERROR: &str = "Log stream scope is closed";
/// End-event error for both transports' cancellation (SSH errors, the local
/// runner exits 130 — the payload is normalized so the host sees one shape).
const CANCELLED_ERROR: &str = "cancelled";
/// Bound on the runner→forwarder channel; a slow host bridge back-pressures
/// the reader instead of buffering an unbounded log backlog in memory.
const STREAM_CHANNEL_CAPACITY: usize = 64;
/// Retained output tail used for the daemon-permission signature check on
/// the end event. Only the last few chunks matter for the hint; the log text
/// itself is never retained.
const PERMISSION_TAIL_BYTES: usize = 4096;

/// Tunables of one stream. Production uses [`StreamLimits::default`]; tests
/// shorten both durations so expiry is observable in milliseconds.
#[derive(Clone, Copy)]
struct StreamLimits {
    ttl: Duration,
    stop_grace: Duration,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            ttl: STREAM_TTL,
            stop_grace: STOP_GRACE,
        }
    }
}

impl StreamLimits {
    /// TTL probe cadence: the production interval, shortened for a short TTL
    /// so even a test-sized lifetime is checked more than once.
    fn tick(&self) -> Duration {
        self.ttl
            .min(TTL_CHECK_INTERVAL)
            .max(Duration::from_millis(1))
    }
}

/// Why a stream ended; decides the end-event payload shape.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EndReason {
    Natural,
    Cancelled,
    Expired,
}

const LOCAL_PERMISSION_HINT: &str = "Local container access needs daemon permission: start \
     Docker Desktop/OrbStack/Podman Desktop, or add your user to the docker group.";

/// The exact remote command for following one container's logs: engine
/// endpoint export (when configured), CLI, `-f` plus the clamped tail. The
/// container id must already pass [`validate_container_id`]; every caller
/// does, and the value is inserted verbatim.
pub fn logs_follow_command(engine: &EngineConfig, container_id: &str, tail: u64) -> String {
    format!(
        "{}{} logs -f --tail {} {}",
        follow_env_prefix(engine),
        engine.cli,
        clamp_tail(tail),
        container_id
    )
}

#[path = "stream_command.rs"]
mod stream_command;
pub use stream_command::supervised_logs_follow_command;

/// Session ids whose Docker CLI runs under Windows `cmd.exe`, detected by the
/// docker/list fallback. The log-follow supervisor is POSIX-only; Windows
/// remotes keep the flat command (cmd has no `exec`/`sh`).
fn windows_sessions() -> &'static Mutex<HashSet<String>> {
    static WINDOWS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(crate) fn mark_remote_platform(session_id: &str, windows: bool) {
    let Ok(mut sessions) = windows_sessions().lock() else {
        return;
    };
    if sessions.len() > 64 {
        sessions.clear();
    }
    if windows {
        sessions.insert(session_id.to_string());
    } else {
        sessions.remove(session_id);
    }
}

pub(crate) fn remote_is_windows(session_id: &str) -> bool {
    windows_sessions()
        .lock()
        .map(|sessions| sessions.contains(session_id))
        .unwrap_or(false)
}

/// Endpoint export for the follow command. Unlike the heredoc collectors'
/// [`EngineConfig::env_prefix`], the endpoint here is always charset-gated
/// (`is_engine_charset`), so the shell-inert value rides unquoted and the
/// command stays one flat line in logs and approval text.
fn follow_env_prefix(engine: &EngineConfig) -> String {
    match engine.endpoint.as_deref() {
        None => String::new(),
        Some(endpoint) => format!("export DOCKER_HOST={endpoint} CONTAINER_HOST={endpoint}; "),
    }
}

#[path = "stream_registry.rs"]
mod stream_registry;
pub use stream_registry::DockerStreams;

impl DockerStreams {
    /// Starts following a container's logs over an SSH session. `command` is
    /// [`logs_follow_command`]'s rendering for the same engine/container;
    /// `tail` only documents (and debug-asserts) the baked-in clamp. Stop and
    /// timeout go through [`crate::sessions::ssh::exec::abort_stream_channel`],
    /// which signals the remote supervisor (with a settle window for its trap)
    /// before closing the channel — closing alone leaves `docker logs -f`
    /// running and its sshd session allocated against `MaxSessions`.
    ///
    /// Must be called from inside the plugin's Tokio runtime (the RPC layer
    /// wraps it in `Runtime::block_on`): the runner and forwarder are spawned
    /// onto that runtime and outlive this call.
    pub fn start_ssh(
        self: &Arc<Self>,
        runtime: Arc<SshRuntime>,
        emitter: PluginEmitter,
        session_id: String,
        owner: String,
        command: String,
        sudo: bool,
        tail: u64,
    ) -> Result<String, String> {
        debug_assert!(
            command.contains(&format!("--tail {}", clamp_tail(tail))),
            "logs follow command must carry the clamped tail: {command}"
        );
        // One log follower per SSH session: a new start (container switch)
        // cancels every prior stream on the session, including legacy streams
        // keyed on the session id. `register` repeats the same-owner
        // supersede atomically, so even a racing start cannot leave two.
        let owner = if owner.is_empty() {
            session_id.clone()
        } else {
            owner
        };
        if self.is_owner_closed(&owner) {
            return Err(CLOSED_OWNER_ERROR.to_string());
        }
        self.cancel_owner(&owner);
        if owner != session_id {
            self.cancel_session(&session_id);
        }
        let exec_session = session_id.clone();
        self.start_tracked(
            emitter,
            false,
            StreamLimits::default(),
            owner,
            session_id,
            move |sink, cancel| async move {
                runtime
                    .exec_stream(&exec_session, &command, sudo, cancel, sink)
                    .await
            },
        )
    }

    /// Starts following a container's logs on the sidecar's own daemon.
    /// `owner` is the frontend log-pane scope; a new start on the same scope
    /// cancels the previous stream (container switch) and a closed scope
    /// rejects late starts (pane already gone).
    pub fn start_local(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        engine: EngineConfig,
        container_id: String,
        tail: u64,
        owner: String,
    ) -> Result<String, String> {
        validate_container_id(&container_id)?;
        let owner = if owner.is_empty() {
            "local".to_string()
        } else {
            owner
        };
        if self.is_owner_closed(&owner) {
            return Err(CLOSED_OWNER_ERROR.to_string());
        }
        self.cancel_owner(&owner);
        self.start_tracked(
            emitter,
            true,
            StreamLimits::default(),
            owner,
            String::new(),
            move |sink, cancel| async move {
                spawn_logs_stream(&engine, &container_id, tail, sink, cancel).await
            },
        )
    }

    /// Shared start path: reserve a slot, spawn the runner, then the
    /// forwarder that batches chunks and emits the end event.
    fn start_tracked<F, Fut>(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        is_local: bool,
        limits: StreamLimits,
        owner: String,
        session: String,
        runner: F,
    ) -> Result<String, String>
    where
        F: FnOnce(mpsc::Sender<Vec<u8>>, watch::Receiver<bool>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<i32, String>> + Send + 'static,
    {
        let (cancel, cancel_rx) = watch::channel(false);
        let (sink, chunks) = mpsc::channel(STREAM_CHANNEL_CAPACITY);
        let (stream_id, done) = self.register(cancel.clone(), owner, session)?;
        let registry = Arc::clone(self);
        let forward_id = stream_id.clone();
        tokio::spawn(async move {
            let runner: JoinHandle<Result<i32, String>> = tokio::spawn(runner(sink, cancel_rx));
            registry
                .forward(
                    forward_id, emitter, cancel, chunks, runner, is_local, limits, done,
                )
                .await;
        });
        Ok(stream_id)
    }

    /// Drains runner chunks into batched `docker/logs/chunk` events, enforces
    /// the TTL, then emits `docker/logs/end` and unregisters the stream.
    /// A size-triggered [`ChunkBatcher::push`] and a time-driven flush arm
    /// both drain the batch, so a lone burst followed by silence is still
    /// visible within [`BATCH_INTERVAL`] instead of waiting for the next
    /// chunk or the stream end.
    /// Expiry terminates the stream by itself: it stops the pump, cancels the
    /// runner and releases the slot even when the runner never returns.
    #[allow(clippy::too_many_arguments)]
    async fn forward(
        self: Arc<Self>,
        stream_id: String,
        emitter: PluginEmitter,
        cancel: watch::Sender<bool>,
        mut chunks: mpsc::Receiver<Vec<u8>>,
        runner: JoinHandle<Result<i32, String>>,
        is_local: bool,
        limits: StreamLimits,
        done: Arc<Notify>,
    ) {
        let started = self.started_at(&stream_id);
        let mut batcher = ChunkBatcher::new();
        let mut text = StreamText::new();
        let mut ticker = tokio::time::interval(limits.tick());
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Independent of arrivals: an idle container's last burst must not
        // stay buffered until the next output or the stream end.
        let batch_ticker = tokio::time::interval(BATCH_INTERVAL);
        tokio::pin!(batch_ticker);
        batch_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut recent: Vec<u8> = Vec::new();
        let mut expired = false;
        loop {
            tokio::select! {
                biased;
                _ = ticker.tick() => {
                    if started.elapsed() >= limits.ttl {
                        expired = true;
                        let _ = cancel.send(true);
                        break;
                    }
                }
                _ = batch_ticker.tick() => {
                    if let Some(batch) = batcher.flush() {
                        emit_text(&emitter, &stream_id, &mut text, &batch);
                    }
                }
                received = chunks.recv() => match received {
                    Some(bytes) => {
                        remember_tail(&mut recent, &bytes);
                        if let Some(batch) = batcher.push(&bytes, Instant::now()) {
                            emit_text(&emitter, &stream_id, &mut text, &batch);
                        }
                    }
                    // Every sink holder lives inside the runner, so a closed
                    // channel means the runner is finishing; await its outcome.
                    None => break,
                },
            }
        }
        if let Some(batch) = batcher.flush() {
            emit_text(&emitter, &stream_id, &mut text, &batch);
        }
        if let Some(tail) = text.finish() {
            emit_chunk(&emitter, &stream_id, &tail);
        }
        let result = stop_runner(runner, limits.stop_grace).await;
        let reason = if expired {
            EndReason::Expired
        } else if *cancel.borrow() {
            EndReason::Cancelled
        } else {
            EndReason::Natural
        };
        let payload = end_payload(&stream_id, result, &recent, is_local, reason);
        let _ = emitter.event("docker/logs/end", payload);
        self.remove(&stream_id);
        done.notify_one();
    }
}

fn emit_chunk(emitter: &PluginEmitter, stream_id: &str, text: &str) {
    if text.is_empty() {
        return;
    }
    let _ = emitter.event(
        "docker/logs/chunk",
        json!({
            "streamId": stream_id,
            "text": text,
        }),
    );
}

/// Batch bytes through the stream's UTF-8 carry-over, then forward whatever
/// is complete.
fn emit_text(emitter: &PluginEmitter, stream_id: &str, text: &mut StreamText, batch: &[u8]) {
    emit_chunk(emitter, stream_id, &text.push(batch));
}

/// Waits out a runner after the pump ends, bounded by `grace`. A runner that
/// ignores cancellation (`docker logs -f` stuck in an uninterruptible read, a
/// wedged SSH channel) is aborted instead of awaited forever: the local child
/// is `kill_on_drop`, so aborting the task still reaps the process.
async fn stop_runner(
    mut runner: JoinHandle<Result<i32, String>>,
    grace: Duration,
) -> Result<i32, String> {
    match tokio::time::timeout(grace, &mut runner).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!("Log stream task failed: {error}")),
        Err(_) => {
            runner.abort();
            Err("Log stream did not stop in time".to_string())
        }
    }
}

fn remember_tail(recent: &mut Vec<u8>, bytes: &[u8]) {
    recent.extend_from_slice(bytes);
    if recent.len() > PERMISSION_TAIL_BYTES {
        let excess = recent.len() - PERMISSION_TAIL_BYTES;
        recent.drain(..excess);
    }
}

/// End-event payload, normalized so the host keys on one shape per outcome:
/// expiry is `{streamId, error:"stream timed out"}`, cancellation is
/// `{streamId, error:"cancelled"}` for both transports (the SSH runner
/// errors, the local runner exits 130), and a natural end keeps the exit
/// code, the error text, and the daemon-permission hint.
fn end_payload(
    stream_id: &str,
    result: Result<i32, String>,
    recent: &[u8],
    is_local: bool,
    reason: EndReason,
) -> Value {
    match reason {
        EndReason::Expired => {
            return json!({ "streamId": stream_id, "error": TIMEOUT_ERROR });
        }
        EndReason::Cancelled => {
            return json!({ "streamId": stream_id, "error": CANCELLED_ERROR });
        }
        EndReason::Natural => {}
    }
    let mut payload = json!({ "streamId": stream_id });
    let scan_text = match &result {
        Ok(code) => {
            payload["exitCode"] = json!(code);
            String::from_utf8_lossy(recent).to_string()
        }
        Err(error) => {
            payload["error"] = json!(error);
            format!("{}\n{error}", String::from_utf8_lossy(recent))
        }
    };
    let exit_code = payload
        .get("exitCode")
        .and_then(Value::as_i64)
        .map(|code| code as i32)
        .unwrap_or(1);
    if is_daemon_permission_failure(exit_code, &scan_text) {
        if is_local {
            payload["error"] = json!(LOCAL_PERMISSION_HINT);
        } else {
            payload["needsSudo"] = json!(true);
        }
    }
    payload
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod stream_tests;
