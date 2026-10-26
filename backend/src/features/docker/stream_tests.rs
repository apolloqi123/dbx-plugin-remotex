//! Registry and forwarder tests for [`super`], kept in their own file to stay
//! inside the source-size budget (same layout as the SSH exec tests).

use super::*;

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
        self.start_tracked(
            emitter,
            is_local,
            StreamLimits::default(),
            move |sink, _cancel| async move {
                for chunk in chunks {
                    let _ = sink.send(chunk.as_bytes().to_vec()).await;
                }
                result
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
        self.start_tracked(emitter, true, limits, move |sink, _cancel| async move {
            let _ = sink.send(b"stuck\n".to_vec()).await;
            std::future::pending::<()>().await;
            unreachable!("the stuck runner is always aborted")
        })
    }
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

    assert!(registry.stop(&stream_id));
    // The slot is held until the forwarder has reaped the runner.
    let events = wait_for_event(&sink, "docker/logs/end").await;
    assert_eq!(chunk_text(&events, &stream_id), "before stop");
    let end = end_event(&events, &stream_id);
    assert_eq!(end["params"]["error"], "cancelled");
    assert!(end["params"].get("exitCode").is_none(), "{end}");
    wait_for_empty(&registry).await;
    assert!(!registry.stop(&stream_id), "finished ids stop to false");
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

    assert!(registry.stop(&stream_id));
    let events = wait_for_event(&sink, "docker/logs/end").await;
    assert_eq!(chunk_text(&events, &stream_id), "before stop");
    let end = end_event(&events, &stream_id);
    assert_eq!(end["params"]["error"], "cancelled");
    assert!(end["params"].get("exitCode").is_none(), "{end}");
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
