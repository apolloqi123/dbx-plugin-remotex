use super::*;

fn config_json(extra: Value) -> Value {
    let mut config = json!({
        "command": "echo hello",
        "working_directory": "/tmp",
        "environment": "FOO=bar\nBAZ=qux",
        "timeout_seconds": 60,
    });
    if let (Some(base), Some(extra)) = (config.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    config
}

/// JSON string array → Vec<String> for the validate error/warning lists.
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

// -- validate -----------------------------------------------------------

#[test]
fn validate_accepts_a_well_formed_execute_task() {
    let response = TaskProvider::validate_request(&json!({
        "task": {
            "providerId": "io.dbx.sshx.tasks",
            "triggerId": "execute",
            "connectionId": "conn-1",
            "configVersion": 1,
            "config": config_json(json!({})),
        }
    }))
    .expect("validate");
    assert_eq!(response["valid"], true, "{response}");
    assert_eq!(response["errors"].as_array().unwrap().len(), 0);
}

#[test]
fn validate_collects_every_config_error_at_once() {
    let response = TaskProvider::validate_request(&json!({
        "task": {
            "providerId": "io.dbx.sshx.tasks",
            "triggerId": "execute",
            "config": {
                "command": "",
                "environment": "GOOD=1\nbad key=no\nMULTI=line\nvalue",                    "timeout_seconds": -5,
            },
        }
    }))
    .expect("validate");
    assert_eq!(response["valid"], false, "{response}");
    let errors = strings(&response["errors"]);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("either command or script_path is required")),
        "{errors:?}"
    );
    assert!(errors.iter().any(|e| e.contains("bad key")), "{errors:?}");
    // `MULTI=line` is a valid KEY=VALUE pair; the bare `value` line is not.
    assert!(
        errors.iter().any(|e| e.contains("environment entry")),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|e| e.contains("timeout_seconds")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("connectionId is required")),
        "missing connection must surface: {errors:?}"
    );
}

#[test]
fn validate_rejects_unknown_trigger_and_non_resident_restart_knobs() {
    let response = TaskProvider::validate_request(&json!({
        "task": {
            "providerId": "io.dbx.sshx.tasks",
            "triggerId": "daemonize",
            "connectionId": "conn-1",
            "config": { "command": "top", "max_restarts": 3 },
        }
    }))
    .expect("validate");
    assert_eq!(response["valid"], false, "{response}");
    let errors = strings(&response["errors"]);
    assert!(
        errors.iter().any(|e| e.contains("unknown trigger")),
        "{errors:?}"
    );
    let warnings = strings(&response["warnings"]);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("max_restarts only applies to the resident")),
        "{warnings:?}"
    );
}

#[test]
fn validate_warns_when_pty_merges_streams() {
    let response = TaskProvider::validate_request(&json!({
        "task": {
            "providerId": "io.dbx.sshx.tasks",
            "triggerId": "execute",
            "connectionId": "conn-1",
            "config": config_json(json!({ "pty": true })),
        }
    }))
    .expect("validate");
    assert_eq!(response["valid"], true, "{response}");
    let warnings = response["warnings"].as_array().unwrap();
    assert!(warnings
        .iter()
        .any(|warning| warning.as_str().unwrap().contains("merged")));
}

// -- script_path vs command ------------------------------------------------

#[test]
fn validate_accepts_a_script_only_task() {
    for trigger in ["execute", "resident"] {
        let response = TaskProvider::validate_request(&json!({
            "task": {
                "providerId": "io.dbx.sshx.tasks",
                "triggerId": trigger,
                "connectionId": "conn-1",
                "config": { "script_path": "/opt/scripts/deploy.sh" },
            }
        }))
        .expect("validate");
        assert_eq!(response["valid"], true, "{trigger}: {response}");
    }
}

#[test]
fn validate_rejects_command_and_script_path_together() {
    let response = TaskProvider::validate_request(&json!({
        "task": {
            "providerId": "io.dbx.sshx.tasks",
            "triggerId": "execute",
            "connectionId": "conn-1",
            "config": {
                "command": "echo hi",
                "script_path": "/opt/scripts/deploy.sh",
            },
        }
    }))
    .expect("validate");
    assert_eq!(response["valid"], false, "{response}");
    let errors = strings(&response["errors"]);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("mutually exclusive; set exactly one")),
        "{errors:?}"
    );
    // Strict parse surfaces the same rule as a non-retryable config error.
    let error = TaskConfig::parse(
        "execute",
        &json!({ "command": "echo hi", "script_path": "/x.sh" }),
    )
    .expect_err("both set must fail");
    assert!(error.starts_with("invalid_config:"), "{error}");
}

#[test]
fn validate_rejects_non_string_script_path() {
    let response = TaskProvider::validate_request(&json!({
        "task": {
            "providerId": "io.dbx.sshx.tasks",
            "triggerId": "execute",
            "connectionId": "conn-1",
            "config": { "script_path": 42 },
        }
    }))
    .expect("validate");
    assert_eq!(response["valid"], false, "{response}");
    assert!(
        strings(&response["errors"])
            .iter()
            .any(|e| e.contains("script_path must be a string")),
        "{:?}",
        response["errors"]
    );
}

// -- config parsing & script building ------------------------------------

#[test]
fn parse_resident_config_applies_restart_defaults_and_bounds() {
    let config =
        TaskConfig::parse("resident", &json!({ "command": "tail -F x" })).expect("resident config");
    assert_eq!(config.restart, RestartPolicy::default());
    assert!(!config.pty);

    let bounded = TaskConfig::parse(
        "resident",
        &json!({
            "command": "tail -F x",
            "max_restarts": 999,
            "restart_backoff_seconds": 0,
        }),
    )
    .expect_err("out-of-bounds restart knobs must fail validation");
    assert!(bounded.starts_with("invalid_config:"), "{bounded}");
}

#[test]
fn build_script_exports_env_then_guards_cd_then_runs_command() {
    let script = build_script(&TaskConfig {
        command: "make deploy\nmake notify".to_string(),
        script_path: String::new(),
        working_directory: "/opt/app my dir".to_string(),
        environment: vec![
            ("FOO".to_string(), "it's".to_string()),
            ("BAR".to_string(), "x".to_string()),
        ],
        timeout_seconds: None,
        pty: false,
        restart: RestartPolicy::default(),
    });
    // Env first (single-quoted, apostrophe escaped), guarded cd, then
    // the raw command as the trailing statement so the exit status is
    // the command's own.
    assert!(
        script.starts_with("export FOO='it'\\''s'\nexport BAR='x'\n"),
        "{script}"
    );
    assert!(
        script.contains("cd '/opt/app my dir' || exit 125\n"),
        "{script}"
    );
    assert!(script.ends_with("make deploy\nmake notify"), "{script}");
}

#[test]
fn build_script_without_optional_parts_is_just_the_command() {
    let script = build_script(&TaskConfig {
        command: "uptime".to_string(),
        script_path: String::new(),
        working_directory: String::new(),
        environment: Vec::new(),
        timeout_seconds: None,
        pty: false,
        restart: RestartPolicy::default(),
    });
    assert_eq!(script, "uptime");
}

#[test]
fn build_script_with_swaps_the_trailing_statement_for_script_runs() {
    let config = TaskConfig {
        command: String::new(),
        script_path: "/tmp/uploaded.sh".to_string(),
        working_directory: String::new(),
        environment: vec![("FOO".to_string(), "1".to_string())],
        timeout_seconds: None,
        pty: false,
        restart: RestartPolicy::default(),
    };
    let script = build_script_with(&config, &script_command_line(true, "/tmp/uploaded.sh"));
    assert_eq!(script, "export FOO='1'\nbash '/tmp/uploaded.sh'");
}

#[test]
fn script_command_line_picks_bash_or_sh_and_quotes_the_path() {
    assert_eq!(
        script_command_line(true, "/tmp/a b.sh"),
        "bash '/tmp/a b.sh'"
    );
    assert_eq!(script_command_line(false, "/tmp/x.sh"), "sh '/tmp/x.sh'");
}

// -- local shell targets ---------------------------------------------------

#[test]
fn local_command_flag_only_matches_cmd_exe() {
    assert_eq!(local_command_flag("/bin/bash"), "-c");
    assert_eq!(local_command_flag("/usr/bin/zsh"), "-c");
    assert_eq!(local_command_flag("pwsh"), "-c");
    assert_eq!(local_command_flag("cmd"), "/C");
    assert_eq!(local_command_flag("cmd.exe"), "/C");
    assert_eq!(local_command_flag("C:\\WINDOWS\\system32\\cmd.EXE"), "/C");
}

fn local_config_fixture() -> TaskConfig {
    TaskConfig {
        command: "echo hi".to_string(),
        script_path: String::new(),
        working_directory: String::new(),
        environment: Vec::new(),
        timeout_seconds: None,
        pty: false,
        restart: RestartPolicy::default(),
    }
}

#[test]
fn local_spawn_args_run_the_command_via_the_shell_flag() {
    let config = local_config_fixture();
    assert_eq!(
        local_spawn_args("/bin/bash", &config),
        vec!["-c".to_string(), "echo hi".to_string()]
    );
    // cmd.exe spells its flag differently but still gets the raw text.
    assert_eq!(
        local_spawn_args("cmd.exe", &config),
        vec!["/C".to_string(), "echo hi".to_string()]
    );
}

#[test]
fn local_spawn_args_script_mode_hands_the_file_to_the_shell_directly() {
    let mut config = local_config_fixture();
    config.script_path = "/opt/scripts/deploy.sh".to_string();
    // The path becomes the shell's single argument — no content round-trip
    // through argv, and no command text alongside it (validation keeps
    // command and script_path mutually exclusive).
    assert_eq!(
        local_spawn_args("/bin/bash", &config),
        vec!["/opt/scripts/deploy.sh".to_string()]
    );
}

// -- restart budget -------------------------------------------------------

#[test]
fn restart_budget_is_bounded_within_the_window() {
    let policy = RestartPolicy {
        max_restarts: 3,
        backoff_seconds: 5,
        window_seconds: Some(60),
    };
    let now = Instant::now();
    // Two restarts inside the window: budget remains for one more.
    let within: Vec<Instant> = (0..2)
        .map(|i| now - Duration::from_secs(10 * (i + 1)))
        .collect();
    assert!(policy.restart_allowed(&within, now));
    // A restart older than the window does not count against the budget:
    // 2 stale + 2 recent stays under the cap of 3.
    let stale: Vec<Instant> = vec![
        now - Duration::from_secs(120),
        now - Duration::from_secs(130),
    ];
    let mixed: Vec<Instant> = stale
        .iter()
        .copied()
        .chain(within.iter().copied())
        .collect();
    assert!(policy.restart_allowed(&mixed, now));
    // At the cap the next crash must degrade, not restart.
    let at_cap: Vec<Instant> = (0..3)
        .map(|i| now - Duration::from_secs(i as u64 + 1))
        .collect();
    assert!(!policy.restart_allowed(&at_cap, now));
}

#[test]
fn restart_budget_without_window_counts_process_lifetime() {
    let policy = RestartPolicy {
        max_restarts: 2,
        backoff_seconds: 1,
        window_seconds: None,
    };
    let now = Instant::now();
    let old: Vec<Instant> = vec![now - Duration::from_secs(3_600)];
    assert!(policy.restart_allowed(&old, now));
    let at_cap: Vec<Instant> = vec![now - Duration::from_secs(3_600), now];
    assert!(!policy.restart_allowed(&at_cap, now));
}

// -- redaction -------------------------------------------------------------

#[test]
fn redactor_masks_connection_secrets_in_emitted_text() {
    // Same lifecycle shape the host pushes on connection/connect (see
    // model.rs tests); secrets are never carried in task config.
    let connection =
        crate::core::model::StoredConnection::from_lifecycle_params(&serde_json::json!({
            "connection": {
                "id": "conn-1",
                "host": "h",
                "port": 22,
                "username": "ops",
                "password": "sup3r-secret-pass",
                "external_config": { "authentication": "password" },
            }
        }))
        .expect("StoredConnection fixture");
    let redactor = Redactor::from_connection(&connection);
    let leaked = "auth failed for sup3r-secret-pass on host";
    assert_eq!(redactor.scrub(leaked), "auth failed for [redacted] on host");
    // Short secrets are left alone (redacting them mangles normal text).
    let mut tiny = Redactor::default();
    tiny.add("ab");
    assert_eq!(tiny.scrub("abcabd"), "abcabd");
}

// -- line buffering ----------------------------------------------------------

#[test]
fn line_buffer_emits_complete_lines_and_keeps_partials() {
    let mut buffer = LineBuffer::default();
    let mut emitted = Vec::new();
    buffer.push(b"hello ", |line| emitted.push(line));
    assert!(emitted.is_empty());
    buffer.push(b"world\nnext", |line| emitted.push(line));
    assert_eq!(emitted, vec!["hello world".to_string()]);
    buffer.push(b"\r\n", |line| emitted.push(line));
    // The CR belongs to the previous line's tail and is preserved
    // verbatim; lossy decoding never silently strips it.
    assert_eq!(emitted.last().unwrap(), "next\r");
    let before_flush = emitted.len();
    buffer.flush(|line| emitted.push(line));
    // Nothing pending → flush emits nothing extra.
    assert_eq!(emitted.len(), before_flush);
}

#[test]
fn line_buffer_flushes_oversized_partial_lines() {
    let mut buffer = LineBuffer::default();
    let mut emitted = Vec::new();
    let chunk = vec![b'x'; PARTIAL_LINE_LIMIT];
    buffer.push(&chunk, |line| emitted.push(line));
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].len(), PARTIAL_LINE_LIMIT);
}

// -- error codes & timestamps -------------------------------------------------

#[test]
fn error_codes_use_the_contract_vocabulary_prefix() {
    assert_eq!(
        task_error(CODE_TIMEOUT, "late"),
        "timeout: late".to_string()
    );
    assert_eq!(
        task_error(CODE_CONNECTION_MISSING, "gone"),
        "connection_missing: gone".to_string()
    );
}

#[test]
fn rfc3339_formats_known_timestamps() {
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339(1_000_000_000), "2001-09-09T01:46:40Z");
    assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
}

#[test]
fn resident_state_serializes_to_contract_vocabulary() {
    assert_eq!(ResidentState::Starting.as_str(), "starting");
    assert_eq!(ResidentState::Running.as_str(), "running");
    assert_eq!(ResidentState::Stopping.as_str(), "stopping");
    assert_eq!(ResidentState::Stopped.as_str(), "stopped");
    assert_eq!(ResidentState::Crashed.as_str(), "crashed");
    assert_eq!(ResidentState::Degraded.as_str(), "degraded");
}

// -- request envelope ------------------------------------------------------------

#[test]
fn task_request_parse_requires_connection_and_defaults_run_id() {
    let request = TaskRequest::parse(&json!({
        "task": {
            "providerId": "io.dbx.sshx.tasks",
            "triggerId": "execute",
            "connectionId": "conn-1",
            "config": { "command": "uptime" },
        }
    }))
    .expect("request");
    assert_eq!(request.connection_id, "conn-1");
    assert!(
        !request.run_id.is_empty(),
        "runId must be generated when absent"
    );

    let error = TaskRequest::parse(&json!({ "task": { "triggerId": "execute" } }))
        .expect_err("connectionId is mandatory");
    assert!(error.starts_with("invalid_config:"), "{error}");
}

#[test]
fn task_request_accepts_run_envelope_run_id() {
    let request = TaskRequest::parse(&json!({
        "task": { "triggerId": "resident", "connectionId": "c", "config": {} },
        "run": { "runId": "run-42", "attempt": 1 },
    }))
    .expect("request");
    assert_eq!(request.run_id, "run-42");
}

// -- local streaming runs (real /bin/sh spawns) ------------------------------

/// Collects one local run's streams. Unix-gated: the spawn tests drive a
/// real `/bin/sh`, which Windows runners do not have.
#[cfg(unix)]
async fn local_run(
    command: &str,
    working_directory: &str,
    environment: &[(String, String)],
    timeout: Option<Duration>,
    cancel: watch::Receiver<bool>,
) -> (RunEnd, Vec<String>, Vec<String>) {
    let config = TaskConfig {
        command: command.to_string(),
        script_path: String::new(),
        working_directory: working_directory.to_string(),
        environment: environment.to_vec(),
        timeout_seconds: None,
        pty: false,
        restart: RestartPolicy::default(),
    };
    let args = local_spawn_args("/bin/sh", &config);
    let mut stdout_lines = Vec::new();
    let mut stderr_lines = Vec::new();
    let mut cancel = cancel;
    let outcome = run_local_streaming(
        "/bin/sh",
        &args,
        working_directory,
        environment,
        timeout,
        &mut cancel,
        |stream, line| match stream {
            LogStream::Stdout => stdout_lines.push(line),
            LogStream::Stderr => stderr_lines.push(line),
            LogStream::System => {}
        },
        None,
    )
    .await;
    (outcome.expect("local run"), stdout_lines, stderr_lines)
}

#[cfg(unix)]
#[tokio::test]
async fn local_streaming_reports_output_streams_and_exit_code() {
    let (_cancel, cancel_rx) = watch::channel(false);
    let (end, stdout, stderr) = local_run(
        "printf 'a\\nb\\n'; printf 'oops\\n' >&2",
        "",
        &[],
        None,
        cancel_rx,
    )
    .await;
    assert_eq!(end, RunEnd::Completed(Some(0)));
    assert_eq!(stdout, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(stderr, vec!["oops".to_string()]);
}

#[cfg(unix)]
#[tokio::test]
async fn local_streaming_applies_process_env_and_working_directory() {
    let dir = std::env::temp_dir();
    let (_cancel, cancel_rx) = watch::channel(false);
    let (end, stdout, _stderr) = local_run(
        "printf \"$DBX_TASK_PROBE|%s\" \"$(pwd)\"",
        &dir.to_string_lossy(),
        &[("DBX_TASK_PROBE".to_string(), "ok".to_string())],
        None,
        cancel_rx,
    )
    .await;
    assert_eq!(end, RunEnd::Completed(Some(0)), "stdout: {stdout:?}");
    let (probe, cwd) = stdout[0].split_once('|').expect("probe|cwd");
    assert_eq!(probe, "ok");
    // macOS /tmp is a symlink to /private/tmp — compare canonical forms.
    assert_eq!(
        std::path::Path::new(cwd).canonicalize().ok(),
        dir.canonicalize().ok()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn local_streaming_reports_nonzero_exit_and_timeout() {
    let (_cancel, cancel_rx) = watch::channel(false);
    let (end, _stdout, _stderr) = local_run("exit 7", "", &[], None, cancel_rx).await;
    assert_eq!(end, RunEnd::Completed(Some(7)));

    // A silent process past its deadline ends as TimedOut (the deadline
    // fires on the log channel, not the process exit).
    let (_cancel, cancel_rx) = watch::channel(false);
    let (end, _stdout, _stderr) = local_run(
        "sleep 5",
        "",
        &[],
        Some(Duration::from_millis(150)),
        cancel_rx,
    )
    .await;
    assert_eq!(end, RunEnd::TimedOut);
}

#[cfg(unix)]
#[tokio::test]
async fn local_streaming_cancel_kills_the_run_promptly() {
    let (cancel, cancel_rx) = watch::channel(false);
    let config = TaskConfig {
        command: "sleep 30".to_string(),
        script_path: String::new(),
        working_directory: String::new(),
        environment: Vec::new(),
        timeout_seconds: None,
        pty: false,
        restart: RestartPolicy::default(),
    };
    let args = local_spawn_args("/bin/sh", &config);
    let cancel = cancel;
    let mut cancel_rx = cancel_rx;
    let run = tokio::spawn(async move {
        run_local_streaming(
            "/bin/sh",
            &args,
            "",
            &[],
            None,
            &mut cancel_rx,
            |stream, line| {
                let _ = (stream, line);
            },
            None,
        )
        .await
    });
    // Land the cancel mid-run; the teardown (group TERM + grace +
    // escalation) must resolve the run well inside the sleep's lifetime.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = cancel.send(true);
    let outcome = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("cancel must not hang on a sleeping process")
        .expect("join")
        .expect("local run");
    assert_eq!(outcome, RunEnd::Cancelled);
}

#[cfg(unix)]
#[tokio::test]
async fn local_run_honors_a_cancel_signal_before_spawn() {
    let (cancel, cancel_rx) = watch::channel(true);
    let (end, _stdout, _stderr) = local_run("echo hi", "", &[], None, cancel_rx).await;
    assert_eq!(end, RunEnd::Cancelled);
    drop(cancel);
}

// -- script_path file reading -------------------------------------------------

#[test]
fn read_script_file_rejects_missing_paths_and_directories() {
    let error =
        read_script_file("/definitely/not/here/task.sh").expect_err("missing script must fail");
    assert!(error.contains("is not readable"), "{error}");
    let error = read_script_file("/tmp").expect_err("directory must fail");
    assert!(error.contains("not a regular file"), "{error}");
}

#[test]
fn read_script_file_enforces_the_upload_cap() {
    let path = std::env::temp_dir().join(format!("dbx-task-script-{}", uuid::Uuid::new_v4()));
    let big = vec![b'#'; SCRIPT_UPLOAD_LIMIT + 1];
    std::fs::write(&path, &big).expect("write oversized fixture");
    let error =
        read_script_file(path.to_string_lossy().as_ref()).expect_err("oversized script must fail");
    std::fs::remove_file(&path).ok();
    assert!(error.contains("script limit"), "{error}");
}

#[test]
fn read_script_file_returns_exact_bytes() {
    let path = std::env::temp_dir().join(format!("dbx-task-script-{}", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"#!/bin/sh\necho hi\n").expect("write fixture");
    let bytes = read_script_file(path.to_string_lossy().as_ref()).expect("read");
    std::fs::remove_file(&path).ok();
    assert_eq!(bytes, b"#!/bin/sh\necho hi\n");
}

// -- resident registry lifecycle --------------------------------------------

/// Builds a provider with one live resident entry, mirroring `resident_start`
/// without needing an SSH runtime or emitter.
fn provider_with_resident(session_id: &str, task_id: &str) -> TaskProvider {
    let provider = TaskProvider::new();
    let (cancel, _cancel_rx) = watch::channel(false);
    provider.residents_lock().insert(
        session_id.to_string(),
        ResidentEntry {
            task_id: task_id.to_string(),
            state: ResidentState::Running,
            heartbeat_at: Some(SystemTime::now()),
            restart_count: 2,
            cancel,
        },
    );
    provider
}

#[test]
fn live_residents_are_visible_to_status_by_session_and_task() {
    let provider = provider_with_resident("session-live", "task-42");
    let by_session = provider
        .status(&json!({ "sessionId": "session-live" }))
        .expect("status by session");
    assert_eq!(by_session["state"], "running");
    assert_eq!(by_session["restartCount"], 2);
    let by_task = provider
        .status(&json!({ "taskId": "task-42" }))
        .expect("status by task");
    assert_eq!(by_task["state"], "running");
}

#[test]
fn a_stopped_resident_leaves_the_live_registry_but_keeps_its_final_state() {
    let provider = provider_with_resident("session-stop", "task-42");
    provider.resident_set_state("session-stop", ResidentState::Stopped);
    assert!(
        provider.residents_lock().is_empty(),
        "terminal sessions must not accumulate in the live registry"
    );
    let status = provider
        .status(&json!({ "sessionId": "session-stop" }))
        .expect("status");
    assert_eq!(status["state"], "stopped");
    assert_eq!(status["restartCount"], 2);
    assert!(status["heartbeatAt"].is_string());
}

#[test]
fn a_degraded_resident_is_reported_as_degraded_not_masked_as_stopped() {
    let provider = provider_with_resident("session-degraded", "task-42");
    provider.resident_set_state("session-degraded", ResidentState::Degraded);
    let status = provider
        .status(&json!({ "sessionId": "session-degraded" }))
        .expect("status");
    assert_eq!(status["state"], "degraded");
    assert_eq!(status["restartCount"], 2);
    // The host reconciles by taskId too.
    let by_task = provider
        .status(&json!({ "taskId": "task-42" }))
        .expect("status by task");
    assert_eq!(by_task["state"], "degraded");
}

#[test]
fn crashed_residents_stay_live_so_bounded_restarts_can_reuse_the_session_id() {
    let provider = provider_with_resident("session-crash", "task-42");
    provider.resident_set_state("session-crash", ResidentState::Crashed);
    assert!(
        provider.residents_lock().contains_key("session-crash"),
        "Crashed is transient; the supervisor restarts under the same id"
    );
    provider.resident_set_state("session-crash", ResidentState::Running);
    let status = provider
        .status(&json!({ "sessionId": "session-crash" }))
        .expect("status");
    assert_eq!(status["state"], "running");
}

#[test]
fn retired_history_is_bounded_and_evicts_the_oldest_entries() {
    let provider = TaskProvider::new();
    for index in 0..(RESIDENT_HISTORY_CAP as u32 + 10) {
        let session_id = format!("session-{index}");
        let (cancel, _cancel_rx) = watch::channel(false);
        provider.residents_lock().insert(
            session_id.clone(),
            ResidentEntry {
                task_id: "task-42".to_string(),
                state: ResidentState::Running,
                heartbeat_at: None,
                restart_count: index,
                cancel,
            },
        );
        provider.resident_set_state(&session_id, ResidentState::Stopped);
    }
    let history = provider
        .resident_history
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert_eq!(history.len(), RESIDENT_HISTORY_CAP);
    assert!(
        !history
            .iter()
            .any(|summary| summary.session_id == "session-0"),
        "oldest summaries must be evicted once the cap is reached"
    );
    let newest = format!("session-{}", RESIDENT_HISTORY_CAP as u32 + 9);
    assert!(
        history
            .iter()
            .any(|summary| summary.session_id == newest),
        "the newest summary must still be retained"
    );
    drop(history);
    // The live registry never grew with any of those sessions.
    assert!(provider.residents_lock().is_empty());
    let status = provider
        .status(&json!({ "sessionId": newest }))
        .expect("status");
    assert_eq!(status["restartCount"], RESIDENT_HISTORY_CAP as u32 + 9);
}
