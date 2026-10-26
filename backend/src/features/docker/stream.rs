//! Live container-log streams: one registry per sidecar, a bounded active
//! set, batching into sidecar events, and the TTL that reaps forgotten
//! `logs -f` sessions. Commands are rendered here so the SSH and local
//! planes share one tail clamp and one engine prefix.

use super::*;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use dbx_plugin_sdk::PluginEmitter;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::sessions::ssh::SshRuntime;

/// Per-sidecar ceiling on concurrent `logs -f` streams. Each stream holds an
/// SSH channel or a child process plus a forwarder task, so the cap keeps a
/// stuck UI (or a bug) from exhausting either.
pub const MAX_ACTIVE_STREAMS: usize = 4;
/// Hard lifetime of one stream. `docker logs -f` never exits on its own, so
/// a forgotten pane would otherwise follow forever; the forwarder expires
/// the stream and emits its end event.
pub const STREAM_TTL: Duration = Duration::from_secs(30 * 60);

/// How often the forwarder checks [`STREAM_TTL`]. The check rides the same
/// task as the chunk pump, so it cannot outlive the registry entry.
const TTL_CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// Bound on the runner→forwarder channel; a slow host bridge back-pressures
/// the reader instead of buffering an unbounded log backlog in memory.
const STREAM_CHANNEL_CAPACITY: usize = 64;
/// Retained output tail used for the daemon-permission signature check on
/// the end event. Only the last few chunks matter for the hint; the log text
/// itself is never retained.
const PERMISSION_TAIL_BYTES: usize = 4096;

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

/// One registered stream: the cancellation switch the runner watches and the
/// start instant the TTL is measured from.
pub(crate) struct StreamEntry {
    cancel: watch::Sender<bool>,
    started: Instant,
}

/// Registry of live log streams keyed by the `streamId` handed to the host.
/// Entries are inserted before their tasks start (so a stop that races the
/// spawn still cancels) and removed by the forwarder when the stream ends.
pub struct DockerStreams {
    inner: Mutex<HashMap<String, StreamEntry>>,
}

impl DockerStreams {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, StreamEntry>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Admission check shared by both starts: an empty slot, returned locked
    /// so the caller can register atomically (two racing starts cannot both
    /// observe the last slot).
    pub(crate) fn start_guard(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, StreamEntry>>, String> {
        let inner = self.lock();
        if inner.len() >= MAX_ACTIVE_STREAMS {
            return Err(format!(
                "Too many active log streams ({MAX_ACTIVE_STREAMS}). Stop one before starting another."
            ));
        }
        Ok(inner)
    }

    /// Reserves one slot and inserts the entry. The caller owns the returned
    /// id for both the response and the chunk/end events.
    fn register(&self, cancel: watch::Sender<bool>) -> Result<String, String> {
        let mut inner = self.start_guard()?;
        let stream_id = Uuid::new_v4().to_string();
        inner.insert(
            stream_id.clone(),
            StreamEntry {
                cancel,
                started: Instant::now(),
            },
        );
        Ok(stream_id)
    }

    fn started_at(&self, stream_id: &str) -> Instant {
        self.lock()
            .get(stream_id)
            .map(|entry| entry.started)
            .unwrap_or_else(Instant::now)
    }

    /// Starts following a container's logs over an SSH session. `command` is
    /// [`logs_follow_command`]'s rendering for the same engine/container;
    /// `tail` only documents (and debug-asserts) the baked-in clamp.
    ///
    /// Must be called from inside the plugin's Tokio runtime (the RPC layer
    /// wraps it in `Runtime::block_on`): the runner and forwarder are spawned
    /// onto that runtime and outlive this call.
    pub fn start_ssh(
        self: &Arc<Self>,
        runtime: Arc<SshRuntime>,
        emitter: PluginEmitter,
        session_id: String,
        command: String,
        sudo: bool,
        tail: u64,
    ) -> Result<String, String> {
        debug_assert!(
            command.contains(&format!("--tail {}", clamp_tail(tail))),
            "logs follow command must carry the clamped tail: {command}"
        );
        self.start_tracked(emitter, false, move |sink, cancel| async move {
            runtime
                .exec_stream(&session_id, &command, sudo, cancel, sink)
                .await
        })
    }

    /// Starts following a container's logs on the sidecar's own daemon.
    pub fn start_local(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        engine: EngineConfig,
        container_id: String,
        tail: u64,
    ) -> Result<String, String> {
        validate_container_id(&container_id)?;
        self.start_tracked(emitter, true, move |sink, cancel| async move {
            spawn_logs_stream(&engine, &container_id, tail, sink, cancel).await
        })
    }

    /// Cancels one stream and drops its slot. Returns false for unknown ids
    /// (already finished or never started), so the RPC can answer
    /// `stopped: false` without treating it as an error.
    pub fn stop(&self, stream_id: &str) -> bool {
        let entry = self.lock().remove(stream_id);
        match entry {
            Some(entry) => {
                let _ = entry.cancel.send(true);
                true
            }
            None => false,
        }
    }

    /// Cancels every live stream. The runners wind down through their own
    /// cancel paths, so remote processes are reaped instead of abandoned.
    pub fn stop_all(&self) {
        let entries: Vec<StreamEntry> = {
            let mut inner = self.lock();
            inner.drain().map(|(_, entry)| entry).collect()
        };
        for entry in entries {
            let _ = entry.cancel.send(true);
        }
    }

    /// Shared start path: reserve a slot, spawn the runner, then the
    /// forwarder that batches chunks and emits the end event.
    fn start_tracked<F, Fut>(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        is_local: bool,
        runner: F,
    ) -> Result<String, String>
    where
        F: FnOnce(mpsc::Sender<Vec<u8>>, watch::Receiver<bool>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<i32, String>> + Send + 'static,
    {
        let (cancel, cancel_rx) = watch::channel(false);
        let (sink, chunks) = mpsc::channel(STREAM_CHANNEL_CAPACITY);
        let stream_id = self.register(cancel.clone())?;
        let registry = Arc::clone(self);
        let forward_id = stream_id.clone();
        tokio::spawn(async move {
            let runner: JoinHandle<Result<i32, String>> = tokio::spawn(runner(sink, cancel_rx));
            registry
                .forward(forward_id, emitter, cancel, chunks, runner, is_local)
                .await;
        });
        Ok(stream_id)
    }

    /// Drains runner chunks into batched `docker/logs/chunk` events, enforces
    /// the TTL, then emits `docker/logs/end` and unregisters the stream.
    async fn forward(
        self: Arc<Self>,
        stream_id: String,
        emitter: PluginEmitter,
        cancel: watch::Sender<bool>,
        mut chunks: mpsc::Receiver<Vec<u8>>,
        runner: JoinHandle<Result<i32, String>>,
        is_local: bool,
    ) {
        let started = self.started_at(&stream_id);
        let mut batcher = ChunkBatcher::new();
        let mut ticker = tokio::time::interval(TTL_CHECK_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut recent: Vec<u8> = Vec::new();
        let mut expired = false;
        loop {
            tokio::select! {
                biased;
                _ = ticker.tick() => {
                    if !expired && started.elapsed() >= STREAM_TTL {
                        expired = true;
                        let _ = cancel.send(true);
                    }
                }
                received = chunks.recv() => match received {
                    Some(bytes) => {
                        remember_tail(&mut recent, &bytes);
                        if let Some(batch) = batcher.push(&bytes, Instant::now()) {
                            emit_chunk(&emitter, &stream_id, &batch);
                        }
                    }
                    // Every sink holder lives inside the runner, so a closed
                    // channel means the runner is finishing; await its outcome.
                    None => break,
                },
            }
        }
        if let Some(batch) = batcher.flush() {
            emit_chunk(&emitter, &stream_id, &batch);
        }
        let result = match runner.await {
            Ok(result) => result,
            Err(error) => Err(format!("Log stream task failed: {error}")),
        };
        let payload = end_payload(&stream_id, result, &recent, is_local, expired);
        let _ = emitter.event("docker/logs/end", payload);
        self.lock().remove(&stream_id);
    }
}

impl Default for DockerStreams {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DockerStreams {
    /// Cancels leftovers when the registry itself is dropped, so a torn-down
    /// plugin never leaves `docker logs -f` children or remote channels
    /// running behind a dead host bridge.
    fn drop(&mut self) {
        self.stop_all();
    }
}

fn emit_chunk(emitter: &PluginEmitter, stream_id: &str, batch: &[u8]) {
    let _ = emitter.event(
        "docker/logs/chunk",
        json!({
            "streamId": stream_id,
            "text": String::from_utf8_lossy(batch),
        }),
    );
}

fn remember_tail(recent: &mut Vec<u8>, bytes: &[u8]) {
    recent.extend_from_slice(bytes);
    if recent.len() > PERMISSION_TAIL_BYTES {
        let excess = recent.len() - PERMISSION_TAIL_BYTES;
        recent.drain(..excess);
    }
}

/// End-event payload: the runner outcome plus the daemon-permission hint.
/// Remote streams advertise `needsSudo` for the explicit sudo retry; local
/// streams have no Quick Sudo pipeline, so they carry the docker-group
/// guidance as the error instead.
fn end_payload(
    stream_id: &str,
    result: Result<i32, String>,
    recent: &[u8],
    is_local: bool,
    expired: bool,
) -> Value {
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
    if expired {
        payload["error"] = json!("Log stream ended after the 30 minute limit");
    }
    payload
}

#[cfg(test)]
impl DockerStreams {
    /// Inserts a placeholder entry so cap tests do not need live runners.
    pub(crate) fn insert_test(&self, stream_id: String) {
        let (cancel, _receiver) = watch::channel(false);
        self.lock().insert(
            stream_id,
            StreamEntry {
                cancel,
                started: Instant::now(),
            },
        );
    }

    pub(crate) fn active_count(&self) -> usize {
        self.lock().len()
    }

    /// Test runner: emits the given fragments, then reports `result`.
    pub(crate) fn start_fake(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        is_local: bool,
        chunks: Vec<&'static str>,
        result: Result<i32, String>,
    ) -> Result<String, String> {
        self.start_tracked(emitter, is_local, move |sink, _cancel| async move {
            for chunk in chunks {
                let _ = sink.send(chunk.as_bytes().to_vec()).await;
            }
            result
        })
    }

    /// Test runner that follows until the registry cancels it, like a real
    /// `logs -f` process whose command is killed by `stop`.
    pub(crate) fn start_fake_until_cancelled(
        self: &Arc<Self>,
        emitter: PluginEmitter,
    ) -> Result<String, String> {
        self.start_tracked(emitter, false, move |sink, mut cancel| async move {
            let _ = sink.send(b"before stop".to_vec()).await;
            let _ = cancel.changed().await;
            Ok(130)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_plugin_sdk::PluginTransport;
    use std::io::Write;

    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn emitter_for_test() -> (PluginEmitter, Arc<Mutex<Vec<u8>>>) {
        let sink = Arc::new(Mutex::new(Vec::new()));
        (
            PluginEmitter::for_tests(
                Arc::new(Mutex::new(Box::new(SharedSink(sink.clone())))),
                PluginTransport::JsonLines,
            ),
            sink,
        )
    }

    fn events(sink: &Arc<Mutex<Vec<u8>>>) -> Vec<Value> {
        String::from_utf8(sink.lock().unwrap().clone())
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    async fn wait_for_event(sink: &Arc<Mutex<Vec<u8>>>, method: &str) -> Vec<Value> {
        for _ in 0..200 {
            let events = events(sink);
            if events.iter().any(|event| event["method"] == method) {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("event {method} was not emitted: {:?}", events(sink));
    }

    async fn wait_for_empty(registry: &Arc<DockerStreams>) {
        for _ in 0..200 {
            if registry.active_count() == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("stream registry did not drain: {}", registry.active_count());
    }

    fn chunk_text(events: &[Value], stream_id: &str) -> String {
        events
            .iter()
            .filter(|event| event["method"] == "docker/logs/chunk")
            .inspect(|event| {
                assert_eq!(
                    event["params"]["streamId"], stream_id,
                    "chunk carries its stream id: {event}"
                );
            })
            .map(|event| {
                event["params"]["text"]
                    .as_str()
                    .expect("chunk text is a string")
                    .to_string()
            })
            .collect()
    }

    #[tokio::test]
    async fn forwards_batched_chunks_then_stream_end() {
        let (emitter, sink) = emitter_for_test();
        let registry = Arc::new(DockerStreams::new());
        let stream_id = registry
            .start_fake(emitter, false, vec!["hello ", "world\n"], Ok(0))
            .expect("start stream");

        let events = wait_for_event(&sink, "docker/logs/end").await;
        assert_eq!(chunk_text(&events, &stream_id), "hello world\n");
        let end = events
            .iter()
            .find(|event| event["method"] == "docker/logs/end")
            .expect("end event");
        assert_eq!(end["params"]["streamId"], stream_id);
        assert_eq!(end["params"]["exitCode"], 0);
        assert!(end["params"].get("error").is_none(), "{end}");
        wait_for_empty(&registry).await;
    }

    #[tokio::test]
    async fn end_event_marks_remote_permission_failures_as_needs_sudo() {
        let (emitter, sink) = emitter_for_test();
        let registry = Arc::new(DockerStreams::new());
        let stream_id = registry
            .start_fake(
                emitter,
                false,
                vec!["permission denied while trying to connect to the Docker daemon socket\n"],
                Ok(1),
            )
            .expect("start stream");

        let events = wait_for_event(&sink, "docker/logs/end").await;
        assert!(chunk_text(&events, &stream_id).contains("permission denied"));
        let end = events
            .iter()
            .find(|event| event["method"] == "docker/logs/end")
            .expect("end event");
        assert_eq!(end["params"]["exitCode"], 1);
        assert_eq!(end["params"]["needsSudo"], true);
        wait_for_empty(&registry).await;
    }

    #[tokio::test]
    async fn local_permission_failures_point_at_the_docker_group() {
        let (emitter, sink) = emitter_for_test();
        let registry = Arc::new(DockerStreams::new());
        registry
            .start_fake(
                emitter,
                true,
                vec!["permission denied while trying to connect to the Docker daemon socket\n"],
                Ok(1),
            )
            .expect("start stream");

        let events = wait_for_event(&sink, "docker/logs/end").await;
        let end = events
            .iter()
            .find(|event| event["method"] == "docker/logs/end")
            .expect("end event");
        assert!(end["params"].get("needsSudo").is_none(), "{end}");
        let error = end["params"]["error"].as_str().expect("error text");
        assert!(error.contains("docker group"), "{error}");
        wait_for_empty(&registry).await;
    }

    #[tokio::test]
    async fn caps_active_streams_at_the_limit() {
        let (emitter, _sink) = emitter_for_test();
        let registry = Arc::new(DockerStreams::new());
        for index in 0..MAX_ACTIVE_STREAMS {
            registry.insert_test(format!("s{index}"));
        }
        let error = registry
            .start_fake(emitter, false, vec![], Ok(0))
            .expect_err("fifth stream is refused");
        assert!(error.contains("Too many active log streams"), "{error}");
        assert_eq!(registry.active_count(), MAX_ACTIVE_STREAMS);
        registry.stop_all();
        assert_eq!(registry.active_count(), 0);
    }

    #[tokio::test]
    async fn stop_cancels_the_runner_and_unregisters_the_stream() {
        let (emitter, sink) = emitter_for_test();
        let registry = Arc::new(DockerStreams::new());
        let stream_id = registry
            .start_fake_until_cancelled(emitter)
            .expect("start stream");
        assert_eq!(registry.active_count(), 1);

        assert!(registry.stop(&stream_id));
        assert_eq!(registry.active_count(), 0);
        assert!(!registry.stop(&stream_id), "stop is idempotent");

        let events = wait_for_event(&sink, "docker/logs/end").await;
        assert_eq!(chunk_text(&events, &stream_id), "before stop");
        let end = events
            .iter()
            .find(|event| event["method"] == "docker/logs/end")
            .expect("end event");
        assert_eq!(end["params"]["streamId"], stream_id);
        assert_eq!(end["params"]["exitCode"], 130);
    }
}
