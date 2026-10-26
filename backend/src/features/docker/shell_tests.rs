//! Command rendering, id validators and registry limits for the container
//! terminals. The registry tests use placeholder entries, so no real PTY or
//! SSH channel is needed; the TTL test runs a fake runner that never ends.

use super::*;

use dbx_plugin_sdk::{PluginEmitter, PluginTransport};
use std::io::Write;
use std::sync::Mutex;
use std::time::Duration;

#[test]
fn ssh_command_is_fixed_and_quotes_only_the_shell_body() {
    let engine = test_engine("docker", Some("unix:///tmp/d.sock"));
    let command = shell_ssh_command(&engine, "d4a7c9f1e2b3").expect("command");
    assert_eq!(
        command,
        "export DOCKER_HOST='unix:///tmp/d.sock' CONTAINER_HOST='unix:///tmp/d.sock'; \
         docker exec -i -t -e TERM=xterm-256color d4a7c9f1e2b3 sh -c \
         'if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi'"
    );
    assert_eq!(
        shell_audit_text(&engine, "d4a7c9f1e2b3").expect("audit"),
        "docker exec -it d4a7c9f1e2b3 sh"
    );
}

#[test]
fn container_id_gate_rejects_names_and_uppercase() {
    assert!(shell_ssh_command(&EngineConfig::default(), "web-nginx").is_err());
    assert!(shell_ssh_command(&EngineConfig::default(), "D4A7C9F1E2B3").is_err());
}

#[test]
fn requested_terminal_id_is_validated_or_generated() {
    assert_eq!(
        requested_terminal_id(Some(" 123e4567-e89b-12d3-a456-426614174000 ")).expect("id"),
        "123e4567-e89b-12d3-a456-426614174000"
    );
    assert!(requested_terminal_id(Some("../bad")).is_err());
    let generated = requested_terminal_id(None).expect("generated");
    assert!(generated.len() >= 8 && generated.is_ascii());
}

#[tokio::test]
async fn registry_enforces_the_concurrency_cap() {
    let registry = DockerTerminals::new();
    for index in 0..MAX_ACTIVE_TERMINALS {
        let id = format!("terminal-{index}-0000");
        let error = registry.insert_for_test(id).await;
        assert!(error.is_empty(), "{error}");
    }
    let error = registry.insert_for_test("terminal-9-0000".to_string()).await;
    assert!(error.contains("Too many active container terminals"), "{error}");
}

#[tokio::test]
async fn close_and_resize_are_idempotent() {
    let (emitter, _sink) = emitter_for_test();
    let registry = std::sync::Arc::new(DockerTerminals::new());
    assert!(!registry.close_and_wait("missing").await);
    assert!(!registry.resize("missing", 80, 24));
    let terminal_id = registry
        .start_cancellable_fake_for_test(emitter)
        .expect("start fake terminal");
    assert!(registry.resize(&terminal_id, 80, 24));
    assert!(registry.close_and_wait(&terminal_id).await);
    assert_eq!(registry.active_for_test(), 0);
    assert!(!registry.close_and_wait(&terminal_id).await);
}

/// Review regression: a failed local spawn must release the slot it reserved
/// (and keep the client-supplied id reusable), never leak a PTY.
#[tokio::test]
async fn local_spawn_failure_releases_the_reserved_slot() {
    let (emitter, _sink) = emitter_for_test();
    let registry = std::sync::Arc::new(DockerTerminals::new());
    let engine = test_engine("dbx-missing-container-cli", None);
    let error = registry
        .start_local(
            emitter,
            "d4a7c9f1e2b3",
            engine,
            Some("term-00000000"),
            80,
            24,
        )
        .expect_err("missing CLI");
    assert!(error.contains("CLI not found on this machine"), "{error}");
    assert_eq!(registry.active_for_test(), 0);
    assert!(!registry.close_and_wait("term-00000000").await);
}

#[tokio::test]
async fn ttl_reaps_an_idle_terminal() {
    let (emitter, sink) = emitter_for_test();
    let registry = std::sync::Arc::new(DockerTerminals::new());
    registry
        .start_fake_for_test(emitter, Duration::from_millis(20))
        .expect("start fake terminal");
    for _ in 0..200 {
        if registry.active_for_test() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(registry.active_for_test(), 0, "TTL must release the slot");
    let events = state_events(&sink);
    let end = events
        .iter()
        .find(|event| event["params"]["state"] == "error")
        .expect("timeout state event");
    assert!(
        end["params"]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("timed out"),
        "{end}"
    );
}

#[tokio::test]
async fn explicit_close_emits_no_state_event() {
    let (emitter, sink) = emitter_for_test();
    let registry = std::sync::Arc::new(DockerTerminals::new());
    let terminal_id = registry
        .start_cancellable_fake_for_test(emitter)
        .expect("start fake terminal");
    assert!(registry.close_and_wait(&terminal_id).await);
    assert_eq!(registry.active_for_test(), 0, "close must release the slot");
    assert!(
        state_events(&sink).is_empty(),
        "an explicit close must not emit docker/terminal/state"
    );
}

#[test]
fn audit_terminal_intent_records_only_the_fixed_command() {
    let dir = tempfile::tempdir().unwrap();
    audit_terminal_intent(dir.path(), "docker exec -it d4a7c9f1e2b3 sh");
    let entries = crate::features::security::audit::tail(dir.path(), 10, None).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].tool, "docker/terminal");
    assert_eq!(entries[0].outcome, crate::features::security::audit::EntryOutcome::Ok);
    assert_eq!(
        entries[0].command.as_deref(),
        Some("docker exec -it d4a7c9f1e2b3 sh")
    );
    assert!(entries[0].output.is_none());
}

// —— Tiny emitter harness (same shape as the log-stream tests) ——————————

struct SharedSink(std::sync::Arc<Mutex<Vec<u8>>>);

impl Write for SharedSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn emitter_for_test() -> (PluginEmitter, std::sync::Arc<Mutex<Vec<u8>>>) {
    let sink = std::sync::Arc::new(Mutex::new(Vec::new()));
    (
        PluginEmitter::for_tests(
            std::sync::Arc::new(Mutex::new(Box::new(SharedSink(sink.clone())))),
            PluginTransport::JsonLines,
        ),
        sink,
    )
}

fn state_events(sink: &std::sync::Arc<Mutex<Vec<u8>>>) -> Vec<Value> {
    String::from_utf8(sink.lock().unwrap().clone())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["method"] == "docker/terminal/state")
        .collect()
}
