//! Registry and forwarder tests for [`super`], kept in their own file to stay
//! inside the source-size budget (same layout as the SSH exec tests).

use super::*;
use super::stream_registry::StreamEntry;

use dbx_plugin_sdk::PluginTransport;
use std::io::Write;

impl DockerStreams {
    /// Inserts a placeholder entry so cap tests do not need live runners.
    pub(crate) fn insert_test(&self, stream_id: String) {
        let (cancel, _receiver) = watch::channel(false);
        self.lock().insert(
            stream_id,
            StreamEntry {
                cancel,
                started: Instant::now(),
                done: Arc::new(Notify::new()),
                owner: String::new(),
                session: String::new(),
            },
        );
    }

    pub(crate) fn insert_test_owned(
        &self,
        stream_id: String,
        owner: &str,
    ) -> watch::Receiver<bool> {
        let (cancel, receiver) = watch::channel(false);
        self.lock().insert(
            stream_id,
            StreamEntry {
                cancel,
                started: Instant::now(),
                done: Arc::new(Notify::new()),
                owner: owner.to_string(),
                session: String::new(),
            },
        );
        receiver
    }

    pub(crate) fn insert_test_session(
        &self,
        stream_id: String,
        session: &str,
    ) -> watch::Receiver<bool> {
        let (cancel, receiver) = watch::channel(false);
        self.lock().insert(
            stream_id,
            StreamEntry {
                cancel,
                started: Instant::now(),
                done: Arc::new(Notify::new()),
                owner: "pane".to_string(),
                session: session.to_string(),
            },
        );
        receiver
    }

    pub(crate) fn active_count(&self) -> usize {
        self.lock().len()
    }

    /// Registration attempt without spawning a runner (cap/admission tests).
    pub(crate) fn try_register_test(&self, owner: &str) -> Result<String, String> {
        let (cancel, _receiver) = watch::channel(false);
        self.register(cancel, owner.to_string(), String::new())
            .map(|(stream_id, _done)| stream_id)
    }

    /// Test runner: emits the given fragments, then reports `result`.
    pub(crate) fn start_fake(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        is_local: bool,
        chunks: Vec<&'static str>,
        result: Result<i32, String>,
    ) -> Result<String, String> {
        self.start_tracked(
            emitter,
            is_local,
            StreamLimits::default(),
            String::new(),
            String::new(),
            move |sink, _cancel| async move {
                for chunk in chunks {
                    let _ = sink.send(chunk.as_bytes().to_vec()).await;
                }
                result
            },
        )
    }

    /// Start attempt under an explicit owner (a log-pane scope).
    pub(crate) fn start_fake_owned(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        owner: &str,
    ) -> Result<String, String> {
        self.start_tracked(
            emitter,
            true,
            StreamLimits::default(),
            owner.to_string(),
            String::new(),
            move |_sink, mut cancel| async move {
                let _ = cancel.changed().await;
                Ok(0)
            },
        )
    }

    /// Test runner that follows until the registry cancels it, like a real
    /// `logs -f` process whose command is killed by `stop`.
    pub(crate) fn start_fake_until_cancelled(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        is_local: bool,
        result: Result<i32, String>,
    ) -> Result<String, String> {
        self.start_tracked(
            emitter,
            is_local,
            StreamLimits::default(),
            String::new(),
            String::new(),
            move |sink, mut cancel| async move {
                let _ = sink.send(b"before stop".to_vec()).await;
                let _ = cancel.changed().await;
                result
            },
        )
    }

    /// Test runner that ignores cancellation entirely, like a wedged child:
    /// nothing but the TTL's stop grace can end the stream.
    fn start_stuck(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        limits: StreamLimits,
    ) -> Result<String, String> {
        self.start_tracked(emitter, true, limits, String::new(), String::new(), move |sink, _cancel| async move {
            let _ = sink.send(b"stuck\n".to_vec()).await;
            std::future::pending::<()>().await;
            unreachable!("the stuck runner is always aborted")
        })
    }
}

    #[test]
    fn cancelling_an_owner_leaves_other_sessions_alone() {
        let registry = DockerStreams::new();
        let first = registry.insert_test_owned("first".to_string(), "session-1");
        let second = registry.insert_test_owned("second".to_string(), "session-1");
        let other = registry.insert_test_owned("other".to_string(), "session-2");
        registry.cancel_owner("session-1");
        assert!(*first.borrow(), "first stream of the owner must be cancelled");
        assert!(*second.borrow(), "every stream of the owner must be cancelled");
        assert!(!*other.borrow(), "another session keeps its stream");
    }

    #[test]
    fn closing_a_scope_cancels_its_streams_and_rejects_late_starts() {
        let registry = DockerStreams::new();
        let stream = registry.insert_test_owned("pane-stream".to_string(), "pane-1");
        let keep = registry.insert_test_owned("other-stream".to_string(), "pane-2");
        registry.close_owner("pane-1");
        assert!(*stream.borrow(), "closing a scope cancels its stream");
        assert!(!*keep.borrow(), "another scope keeps its stream");
        assert!(registry.is_owner_closed("pane-1"));
        assert!(!registry.is_owner_closed("pane-2"));
        assert_eq!(registry.active_count(), 1, "the slot is released at once");
    }

    #[tokio::test]
    async fn a_new_start_supersedes_a_same_owner_stream_atomically() {
        let registry = Arc::new(DockerStreams::new());
        let old = registry.insert_test_owned("old".to_string(), "pane-1");
        let keep = registry.insert_test_owned("keep".to_string(), "pane-2");
        let (emitter, _sink) = emitter_for_test();

        let new_id = registry.start_fake_owned(emitter, "pane-1").expect("start");

        assert_ne!(new_id, "old");
        assert!(*old.borrow(), "the previous stream of the same scope is cancelled");
        assert!(!*keep.borrow(), "another scope keeps its stream");
        assert_eq!(registry.active_count(), 2, "the pane is replaced, not doubled");
        registry.stop_all();
    }

    #[test]
    fn a_closed_scope_rejects_a_late_start_atomically() {
        let registry = Arc::new(DockerStreams::new());
        registry.close_owner("pane-1");
        let (emitter, _sink) = emitter_for_test();
        let error = registry
            .start_fake_owned(emitter, "pane-1")
            .expect_err("a start after close_owner must be rejected");
        assert!(error.contains("scope is closed"), "{error}");
        assert_eq!(registry.active_count(), 0);
    }

    #[test]
    fn closing_a_session_only_cancels_that_sessions_streams() {
        let registry = DockerStreams::new();
        let first = registry.insert_test_session("first".to_string(), "sess-1");
        let other = registry.insert_test_session("other".to_string(), "sess-2");
        registry.cancel_session("sess-1");
        assert!(*first.borrow());
        assert!(!*other.borrow(), "another session keeps its stream");
        assert_eq!(registry.active_count(), 1);
    }

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

fn end_event<'a>(events: &'a [Value], stream_id: &str) -> &'a Value {
    events
        .iter()
        .find(|event| event["method"] == "docker/logs/end")
        .inspect(|event| {
            assert_eq!(event["params"]["streamId"], stream_id, "{event}");
        })
        .expect("end event")
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
    let end = end_event(&events, &stream_id);
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
    let end = end_event(&events, &stream_id);
    assert_eq!(end["params"]["exitCode"], 1);
    assert_eq!(end["params"]["needsSudo"], true);
    wait_for_empty(&registry).await;
}

#[tokio::test]
async fn local_permission_failures_point_at_the_docker_group() {
    let (emitter, sink) = emitter_for_test();
    let registry = Arc::new(DockerStreams::new());
    let stream_id = registry
        .start_fake(
            emitter,
            true,
            vec!["permission denied while trying to connect to the Docker daemon socket\n"],
            Ok(1),
        )
        .expect("start stream");

    let events = wait_for_event(&sink, "docker/logs/end").await;
    let end = end_event(&events, &stream_id);
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
        .start_fake_until_cancelled(emitter, true, Ok(130))
        .expect("start stream");
    assert_eq!(registry.active_count(), 1);

    assert!(registry.stop_and_wait(&stream_id).await);
    // stop_and_wait must not return before the slot is free: the next start
    // (a container switch) is admitted only after this channel is released.
    assert_eq!(
        registry.active_count(),
        0,
        "slot released before stop returns"
    );
    let events = wait_for_event(&sink, "docker/logs/end").await;
    assert_eq!(chunk_text(&events, &stream_id), "before stop");
    let end = end_event(&events, &stream_id);
    assert_eq!(end["params"]["error"], "cancelled");
    assert!(end["params"].get("exitCode").is_none(), "{end}");
    wait_for_empty(&registry).await;
    assert!(
        !registry.stop_and_wait(&stream_id).await,
        "finished ids stop to false"
    );
}

#[tokio::test]
async fn ssh_cancellation_reports_the_same_cancelled_error() {
    let (emitter, sink) = emitter_for_test();
    let registry = Arc::new(DockerStreams::new());
    // The SSH runner reports cancellation as an error, not exit 130.
    let stream_id = registry
        .start_fake_until_cancelled(
            emitter,
            false,
            Err(crate::sessions::ssh::exec::EXEC_CANCELLED_MESSAGE.to_string()),
        )
        .expect("start stream");

    assert!(registry.stop_and_wait(&stream_id).await);
    let events = wait_for_event(&sink, "docker/logs/end").await;
    assert_eq!(chunk_text(&events, &stream_id), "before stop");
    let end = end_event(&events, &stream_id);
    assert_eq!(end["params"]["error"], "cancelled");
    assert!(end["params"].get("exitCode").is_none(), "{end}");
    wait_for_empty(&registry).await;
}

/// One sub-8KiB chunk followed by total silence must reach the host through
/// the time-driven flush, not the end-of-stream flush: an idle container
/// would otherwise keep its last burst (including `--tail` history) hidden
/// indefinitely. The runner streams the chunk as soon as the pump starts, so
/// it lands well inside the first batch window and `push` cannot flush it.
#[tokio::test]
async fn idle_stream_flushes_a_small_chunk_before_stream_end() {
    let (emitter, sink) = emitter_for_test();
    let registry = Arc::new(DockerStreams::new());
    let stream_id = registry
        .start_fake_until_cancelled(emitter, true, Ok(130))
        .expect("start stream");

    let events = wait_for_event(&sink, "docker/logs/chunk").await;
    assert_eq!(chunk_text(&events, &stream_id), "before stop");
    assert!(
        !events
            .iter()
            .any(|event| event["method"] == "docker/logs/end"),
        "the chunk must be flushed by the batch interval, not by stream end: {events:?}"
    );
    assert_eq!(registry.active_count(), 1, "the stream is still live");

    assert!(registry.stop_and_wait(&stream_id).await);
    wait_for_event(&sink, "docker/logs/end").await;
    wait_for_empty(&registry).await;
}

#[tokio::test]
async fn ttl_expiry_ends_the_stream_and_frees_the_slot_for_a_stuck_runner() {
    let (emitter, sink) = emitter_for_test();
    let registry = Arc::new(DockerStreams::new());
    let limits = StreamLimits {
        ttl: Duration::from_millis(60),
        stop_grace: Duration::from_millis(40),
    };
    let stream_id = registry.start_stuck(emitter, limits).expect("start stream");
    assert_eq!(registry.active_count(), 1);

    let events = wait_for_event(&sink, "docker/logs/end").await;
    assert_eq!(chunk_text(&events, &stream_id), "stuck\n");
    let end = end_event(&events, &stream_id);
    assert_eq!(end["params"]["error"], "stream timed out");
    assert!(end["params"].get("exitCode").is_none(), "{end}");
    assert!(end["params"].get("needsSudo").is_none(), "{end}");

    // The runner never returned; the slot must be free anyway.
    wait_for_empty(&registry).await;
    for index in 0..MAX_ACTIVE_STREAMS {
        registry.insert_test(format!("slot{index}"));
    }
    assert_eq!(registry.active_count(), MAX_ACTIVE_STREAMS);
}
