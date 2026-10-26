use super::*;

#[test]
fn replay_buffer_is_sequence_addressable() {
    let mut replay = ReplayBuffer::default();
    replay.push(TerminalStream::Stdout, b"one".to_vec());
    replay.push(TerminalStream::Stderr, b"two".to_vec());
    let frames = replay.after(1);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].sequence, 2);
}

#[test]
fn replay_buffer_honours_a_custom_byte_budget() {
    // 串口会话的 128 KiB 预算（设计稿 §3）：绕回后 first_sequence 前移，
    // after() 只回可得的尾部，complete 语义随 first_available 变化。
    let mut replay = ReplayBuffer::with_byte_limit(8);
    replay.push(TerminalStream::Stdout, b"12345".to_vec());
    replay.push(TerminalStream::Stdout, b"67890".to_vec());
    assert_eq!(replay.first_sequence(), 2, "frame 1 was evicted");
    assert_eq!(replay.tail_sequence(), 2);
    assert!(replay.after(0).iter().all(|frame| frame.sequence >= 2));
    // afterSequence+1 >= first_available → complete。
    assert!(
        replay.after(1).len() == 1,
        "replaying from 1 yields the surviving frame"
    );
    // 默认预算仍是共享的 2 MiB 上限。
    let replay = ReplayBuffer::default();
    let serialized = format!("{:?}", replay.byte_limit);
    assert_eq!(serialized, format!("{TERMINAL_REPLAY_LIMIT}"));
}

#[test]
fn terminal_input_enqueue_retries_then_drops_instead_of_blocking_the_worker() {
    let (sender, mut receiver) = mpsc::channel(1);
    sender
        .try_send(TerminalCommand::Input(b"first".to_vec()))
        .expect("seed the bounded queue");

    // The SDK binary handler runs on a shared worker thread. A full queue is
    // retried for a short bounded budget and then the chunk is dropped: it
    // must never park the worker, and it must never surface as an error (the
    // workbench treats `ssh/terminal/error` as session death).
    let started = std::time::Instant::now();
    enqueue_terminal_input(&sender, b"second".to_vec())
        .expect("a transient full queue is a drop, not an error");
    assert!(
        started.elapsed() < std::time::Duration::from_millis(350),
        "enqueue must stay within its bounded retry budget: {:?}",
        started.elapsed()
    );
    assert!(matches!(
        receiver.try_recv(),
        Ok(TerminalCommand::Input(data)) if data == b"first"
    ));
    assert!(
        receiver.try_recv().is_err(),
        "the dropped input must not be queued later"
    );

    drop(receiver);
    let error = enqueue_terminal_input(&sender, b"third".to_vec())
        .expect_err("a closed queue must be reported");
    assert!(error.contains("closed"), "unexpected error: {error}");
}

/// Stress test: 5 MiB of continuous terminal output through the 2 MiB
/// ring buffer. The buffer must stay pinned at the cap, keep sequence
/// addressing contiguous (so replay `complete` stays exact), and hand
/// back only the newest 2 MiB.
#[test]
fn replay_buffer_stress_keeps_five_megabytes_within_the_cap() {
    const CHUNK: usize = 64 * 1024;
    const TOTAL: usize = 5 * 1024 * 1024;
    let mut replay = ReplayBuffer::default();
    let pushes = TOTAL / CHUNK;
    for index in 0..pushes {
        let mut chunk = vec![b'x'; CHUNK];
        chunk[0] = b'a' + (index % 26) as u8;
        replay.push(TerminalStream::Stdout, chunk);
        assert!(
            replay.bytes <= TERMINAL_REPLAY_LIMIT,
            "buffer exceeded the cap at push {index}"
        );
    }
    // The buffer holds exactly the newest 2 MiB, not the full 5 MiB.
    assert_eq!(replay.bytes, TERMINAL_REPLAY_LIMIT);
    let first = replay.first_sequence();
    assert_eq!(first as usize, pushes - TERMINAL_REPLAY_LIMIT / CHUNK + 1);

    let frames = replay.after(0);
    let bytes: usize = frames.iter().map(|frame| frame.data.len()).sum();
    assert_eq!(bytes, TERMINAL_REPLAY_LIMIT);
    assert_eq!(frames.first().unwrap().sequence, first);
    // Sequence addressing stays contiguous after the eviction ramp.
    for pair in frames.windows(2) {
        assert_eq!(pair[1].sequence, pair[0].sequence + 1);
    }
    assert_eq!(frames.last().unwrap().sequence, replay.sequence);
    // after(first-1) returns the whole retained tail; older queries stay
    // empty, which is what drives replay `complete == false`.
    assert_eq!(replay.after(first - 1).len(), frames.len());
    assert!(replay.after(replay.sequence).is_empty());
}

/// `sudo/profiles/options` backs the connection form's dynamic dropdown:
/// value = profile id (what `sudo_profile` stores), label = display name,
/// sorted like the listings, and never any secret field.
#[test]
fn profile_options_expose_id_name_pairs_without_secrets() {
    let dir = std::env::temp_dir().join(format!("dbx-profile-options-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let runtime = SshRuntime::new(dir.clone());
    let mut store = sudo_profiles::SudoProfileStore::default();
    let (_, _) = sudo_profiles::save_profile(
        &mut store,
        &json!({ "name": "zeta", "sudoPassword": "secret-z" }),
    )
    .unwrap();
    let (alpha, _) = sudo_profiles::save_profile(
        &mut store,
        &json!({ "name": "alpha", "sudoPassword": "secret-a" }),
    )
    .unwrap();
    // Keyfile injection: the production save_store would probe the real
    // macOS keychain from a test; the envelope then resolves reads to the
    // same keyfile, so the runtime path below stays keychain-free too.
    sudo_profiles::save_store_with(
        &dir,
        &store,
        &crate::core::vault::KeyfileProvider::new(crate::core::vault::keyfile_path(&dir)),
    )
    .unwrap();

    let payload = runtime.profiles_options();
    let options = payload["options"].as_array().unwrap();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0]["label"], json!("alpha"));
    assert_eq!(options[0]["value"], json!(alpha.id));
    assert_eq!(options[1]["label"], json!("zeta"));
    let rendered = serde_json::to_string(&payload).unwrap();
    assert!(!rendered.contains("secret-z") && !rendered.contains("secret-a"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cached_metrics_payload_marks_age_without_mutating_the_snapshot() {
    let snapshot = json!({ "loadAverage": 0.4, "network": [] });
    let payload = cached_metrics_payload(&snapshot, 1_700_000_123);
    assert_eq!(payload["loadAverage"], json!(0.4));
    assert_eq!(payload["cachedAt"], json!(1_700_000_123));
    // The stored snapshot stays clean of the marker.
    assert!(snapshot.get("cachedAt").is_none());
}

#[test]
fn metrics_snapshot_cache_serves_and_cleans_per_session() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let snapshot = json!({ "loadAverage": 1.25 });

    assert!(runtime.cached_metrics_snapshot("sess-1").is_none());

    runtime.store_metrics_snapshot("sess-1", &snapshot);
    runtime.store_metrics_snapshot("sess-2", &json!({ "loadAverage": 2.5 }));

    let served = runtime.cached_metrics_snapshot("sess-1").expect("cached");
    assert_eq!(served["loadAverage"], json!(1.25));
    assert!(served["cachedAt"].is_u64(), "cachedAt must be unix seconds");
    // Sessions are independent: the second session serves its own data.
    let other = runtime.cached_metrics_snapshot("sess-2").expect("cached");
    assert_eq!(other["loadAverage"], json!(2.5));

    // Session close semantics drop only that session's entry.
    if let Ok(mut cache) = runtime.metrics_cache.lock() {
        cache.remove("sess-1");
    }
    assert!(runtime.cached_metrics_snapshot("sess-1").is_none());
    assert!(runtime.cached_metrics_snapshot("sess-2").is_some());
}

#[test]
fn session_info_payload_carries_identity_and_liveness() {
    let endpoint = ConnectionEndpoint {
        host: "prod-01".to_string(),
        port: 2222,
        username: "ops".to_string(),
    };
    let row = session_info_payload(
        "sess-1",
        "conn-1",
        "wb-1",
        true,
        true,
        true,
        90,
        1_700_000_123,
        "private-key",
        &endpoint,
        false,
    );
    assert_eq!(row["recording"], false);
    assert_eq!(row["sessionId"], json!("sess-1"));
    assert_eq!(row["connectionId"], json!("conn-1"));
    assert_eq!(row["workbenchId"], json!("wb-1"));
    assert_eq!(row["readOnly"], json!(true));
    assert_eq!(row["connected"], json!(true));
    assert_eq!(row["sudoKeepalive"], json!(true));
    assert_eq!(row["terminalKeepaliveSecs"], json!(90));
    assert_eq!(row["createdAt"], json!(1_700_000_123));
    assert_eq!(row["authMethod"], json!("private-key"));
    assert_eq!(row["host"], json!("prod-01"));
    assert_eq!(row["port"], json!(2222));
    assert_eq!(row["username"], json!("ops"));
    // No secrets leak through the inventory payload.
    let serialized = row.to_string();
    assert!(!serialized.contains("password"));
    assert!(!serialized.contains("passphrase"));
    assert!(!serialized.contains("privateKeyMaterial"));
}

#[test]
fn batch_input_payload_normalizes_newlines_and_bounds_size() {
    assert_eq!(
        SshRuntime::batch_input_payload("df -h", true),
        b"df -h\r".to_vec()
    );
    assert_eq!(
        SshRuntime::batch_input_payload("df -h", false),
        b"df -h".to_vec()
    );
    // Every newline flavour becomes one Enter.
    assert_eq!(
        SshRuntime::batch_input_payload("a\nb\r\nc\rd", true),
        b"a\rb\rc\rd\r".to_vec()
    );
    // Oversized commands are truncated, never rejected: a batch send is
    // keyboard-level input, the shell copes with long lines. The cut
    // backs off to a UTF-8 boundary instead of splitting a multibyte
    // character into invalid bytes.
    let huge = "x".repeat(300 * 1024);
    let payload = SshRuntime::batch_input_payload(&huge, true);
    assert_eq!(payload.len(), 256 * 1024 + 1);
    assert_eq!(*payload.last().unwrap(), b'\r');
    let mut wide = String::new();
    // '中' is 3 bytes; the cut lands on the 2nd byte of the final char.
    while wide.len() <= 256 * 1024 {
        wide.push('中');
    }
    let payload = SshRuntime::batch_input_payload(&wide, false);
    assert_eq!(payload.len(), 256 * 1024 - 1);
    assert!(std::str::from_utf8(&payload).is_ok());
}

#[test]
fn batch_input_helpers_dedupe_and_shape_rows() {
    let targets = SshRuntime::dedupe_session_ids(&[
        "b".to_string(),
        "a".to_string(),
        "b".to_string(),
        String::new(),
        "a".to_string(),
    ]);
    assert_eq!(targets, vec!["b".to_string(), "a".to_string()]);

    let ok = SshRuntime::batch_input_row("s1", None);
    assert_eq!(ok["sessionId"], json!("s1"));
    assert_eq!(ok["success"], json!(true));
    assert!(ok.get("error").is_none());
    let bad = SshRuntime::batch_input_row("s2", Some("SSH session was not found"));
    assert_eq!(bad["success"], json!(false));
    assert_eq!(bad["error"], json!("SSH session was not found"));
}

#[tokio::test]
async fn batch_terminal_input_reports_unknown_sessions_as_target_failures() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let response = runtime
        .batch_terminal_input(
            &["ghost-1".to_string(), "ghost-2".to_string()],
            "echo hi",
            true,
        )
        .await;
    assert_eq!(response["sent"], json!(0));
    assert_eq!(response["failed"], json!(2));
    let results = response["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["sessionId"], json!("ghost-1"));
    assert_eq!(results[0]["error"], json!("SSH session was not found"));
    assert_eq!(results[1]["sessionId"], json!("ghost-2"));
}

#[test]
fn sessions_list_reports_an_empty_inventory_without_sessions() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let payload = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(runtime.list_sessions());
    assert_eq!(payload["sessions"].as_array().expect("array").len(), 0);
}

#[test]
fn directory_tracking_scripts_are_session_local() {
    let bash = directory_tracking_script("session-1", RemoteShell::Bash);
    let zsh = directory_tracking_script("session-1", RemoteShell::Zsh);
    assert!(bash.contains("PROMPT_COMMAND"));
    assert!(bash.contains("dbx-directory-ready-session-1"));
    assert!(zsh.contains("precmd_functions"));
    // Unknown shell: the script is deferred until the first toggle probes
    // the remote shell; before that it must degrade to a no-op script.
    assert_eq!(
        directory_tracking_script("session-1", RemoteShell::Unknown),
        directory_tracking_script("session-1", RemoteShell::Other)
    );
    // #10750: arming is unconditional for the session lifetime — there is
    // no teardown variant anymore, so a bash script never restores
    // PROMPT_COMMAND and a zsh script never unsets the hook function.
    assert!(!bash.contains("${__DBX_OLD_PROMPT_COMMAND-}"));
    assert!(!zsh.contains("unfunction"));
    // Both installs are guarded and idempotent: a second arm is a no-op
    // remotely instead of stacking a second emit function.
    assert!(bash.contains("[ -z \"${__DBX_CWD_ACTIVE+x}\" ]"));
    assert!(zsh.contains("[ -z \"${__DBX_CWD_ACTIVE+x}\" ]"));
    // zsh needs one extra Enter to trigger the first precmd run inside the
    // handshake window; bash's PROMPT_COMMAND fires on the next prompt.
    assert!(zsh.ends_with("\r \r"));
    assert!(bash.ends_with("\r"));
}

#[test]
fn prompt_hook_installs_osc7_report_before_every_prompt() {
    // The whole point of #10750: the frontend keys the prompt-newline fix
    // off OSC 7 frames, so both shell flavors must emit exactly that
    // sequence (host + pwd, BEL-terminated) from their per-prompt hook.
    for script in [
        directory_tracking_script("s", RemoteShell::Bash),
        directory_tracking_script("s", RemoteShell::Zsh),
    ] {
        assert!(script.contains("printf '\\033]7;file://%s%s\\007'"));
    }
}

#[test]
fn interactive_shell_sessions_gate_shell_syntax_lines() {
    // Plain shell session: prompt hook and startup commands both apply.
    assert!(interactive_shell_session(None, ""));
    // A spawn-exec command session or a connection-level remote_command
    // replaces the shell — typing the hook line there would feed the
    // command's stdin instead of a shell.
    assert!(!interactive_shell_session(Some("tail -f log"), ""));
    assert!(!interactive_shell_session(None, "exec zsh"));
    // Gating mirrors the startup-commands injector so both lines share the
    // command FIFO with a consistent notion of "interactive".
    assert_eq!(
        interactive_shell_session(None, "anything"),
        startup_commands::executes_for("anything")
    );
}

#[test]
fn stream_text_decode_carries_incomplete_utf8_across_frames() {
    let mut carry: Vec<u8> = Vec::new();
    // '中' (3 字节) 被劈成 1+2 两帧：第一帧尾部残留，第二帧拼出完整字符。
    let first = decode_stream_text(&mut carry, "密码: ".as_bytes());
    assert_eq!(first, "密码: ");
    assert!(carry.is_empty());
    let bytes = "中".as_bytes();
    let part1 = decode_stream_text(&mut carry, &bytes[..1]);
    assert_eq!(part1, "");
    assert_eq!(carry, &bytes[..1]);
    let part2 = decode_stream_text(&mut carry, &bytes[1..]);
    assert_eq!(part2, "中");
    assert!(carry.is_empty());
    // 大帧（>8 字节，现实 SSH 数据帧形态）尾部残缺字符同样跨帧拼装：
    // 旧实现对 >8 的 carry 整体 lossy 清仓，会把这里劈成双 U+FFFD。
    let mut carry: Vec<u8> = Vec::new();
    let filler = "a".repeat(16);
    let mut frame = filler.clone().into_bytes();
    let han = "中".as_bytes();
    frame.extend_from_slice(&han[..1]);
    let part1 = decode_stream_text(&mut carry, &frame);
    assert_eq!(part1, filler);
    assert_eq!(carry, &han[..1]);
    let part2 = decode_stream_text(&mut carry, &han[1..]);
    assert_eq!(part2, "中");
    assert!(carry.is_empty());
    // 非法字节流不能永久滞留：按最大非法子序列立即 lossy 冲刷。
    let mut carry: Vec<u8> = Vec::new();
    let flushed = decode_stream_text(
        &mut carry,
        &[0xFF, 0xFE, 0xFD, 0xFC, 0xFB, 0xFA, 0xF9, 0xF8, 0x41],
    );
    assert!(flushed.contains('\u{FFFD}'));
    assert!(flushed.ends_with('A'));
    assert!(carry.is_empty());
    // 非法字节大帧同样不滞留：逐子序列冲刷后 carry 为空。
    let mut carry: Vec<u8> = Vec::new();
    let flushed = decode_stream_text(&mut carry, &[0xFF; 16]);
    assert_eq!(flushed, "\u{FFFD}".repeat(16));
    assert!(carry.is_empty());
}

#[test]
fn requested_session_ids_are_validated_before_registry_use() {
    // 合法：非空、白名单字符（[A-Za-z0-9._-]）、不超过 128 字节。
    assert!(valid_requested_session_id("session-1"));
    assert!(valid_requested_session_id(&"a".repeat(128)));
    // 非法：空串、白名单外字符、超长。白名单外的代表行既防路径分隔符，
    // 也防 id 进 directory_tracking_script 的单引号 shell 字面量后被闭合
    // 注入（`'`、`` ` ``、`$`）。
    assert!(!valid_requested_session_id(""));
    assert!(!valid_requested_session_id("a/b"));
    assert!(!valid_requested_session_id("it's"));
    assert!(!valid_requested_session_id("a$(reboot)b"));
    assert!(!valid_requested_session_id(&"a".repeat(129)));
}

#[test]
fn primary_session_pick_is_deterministic_not_iteration_order() {
    // HashMap iteration order is randomized per process; whichever entry
    // "comes first" must not decide where terminal-routed commands run.
    let older_first = select_primary_session(vec![
        ("1111".to_string(), 0, true),
        ("2222".to_string(), 1, true),
    ]);
    let newer_first = select_primary_session(vec![
        ("2222".to_string(), 1, true),
        ("1111".to_string(), 0, true),
    ]);
    assert_eq!(older_first.as_deref(), Some("1111"));
    assert_eq!(newer_first.as_deref(), Some("1111"));
}

#[test]
fn primary_session_pick_skips_disconnected_and_tiebreaks_by_id() {
    assert_eq!(
        select_primary_session(vec![
            ("3333".to_string(), 0, false),
            ("2222".to_string(), 1, true),
        ])
        .as_deref(),
        Some("2222")
    );
    assert_eq!(select_primary_session(Vec::new()), None);
}

#[test]
fn directory_handshake_filters_split_marker() {
    let marker = directory_tracking_marker("session-1");
    let mut filter = DirectoryHandshakeFilter::default();
    filter.begin(marker.clone());
    assert_eq!(filter.filter(b"echoed script"), None);
    let split = marker.len() / 2;
    assert_eq!(filter.filter(&marker[..split]), None);
    let mut final_frame = marker[split..].to_vec();
    final_frame.extend_from_slice(b"prompt$ ");
    // The suppressed echo took the submitted line's CRLF with it: reset the
    // line so the post-arming prompt is not glued to the previous one.
    assert_eq!(
        filter.filter(&final_frame),
        Some(b"\r\x1b[2Kprompt$ ".to_vec())
    );
}

#[test]
fn directory_handshake_clears_the_line_even_when_no_tail_follows_the_marker() {
    let marker = directory_tracking_marker("session-2");
    let mut filter = DirectoryHandshakeFilter::default();
    filter.begin(marker.clone());
    let mut frame = b"echoed script".to_vec();
    frame.extend_from_slice(&marker);
    assert_eq!(
        filter.filter(&frame),
        Some(b"\r\x1b[2K".to_vec()),
        "the prompt arrives in a later frame: still reset the line now"
    );
}

#[test]
fn write_terminal_unknown_session_fails_structured() {
    // write_terminal 在宿主 dispatch 线程上同步执行（blocking_read），
    // 这里同样在 runtime 之外调用以锁死同一语义。
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let error = runtime
        .write_terminal("s-missing", b"ls\r".to_vec())
        .expect_err("write to an unknown session must fail");
    assert_eq!(error, "SSH session was not found or expired");
}

#[test]
fn full_terminal_input_queue_is_not_session_death() {
    // Regression: a full queue used to return Err, which the binary handler
    // mirrored as `ssh/terminal/error` and the workbench treated as session
    // death (reconnect/close). A transient drop must stay an Ok write; only a
    // closed queue (session actually gone) is an error.
    let (tx, _rx) = mpsc::channel::<TerminalCommand>(1);
    tx.try_send(TerminalCommand::Input(b"queued".to_vec()))
        .expect("fill queue");
    assert!(
        enqueue_terminal_input(&tx, b"dropped".to_vec()).is_ok(),
        "a transient full queue must not surface as terminal/error"
    );
    // The receiver is then dropped: session death still errors.
    drop(_rx);
    assert_eq!(
        enqueue_terminal_input(&tx, b"dead".to_vec()),
        Err("SSH input queue is closed".to_string())
    );
}

#[tokio::test]
async fn store_connection_is_idempotent_and_sweeps_stale_residue() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let lifecycle = |name: &str| {
        json!({
            "connection": {
                "id": "conn-1",
                "name": name,
                "host": "127.0.0.1",
                "port": 22,
                "username": "user",
                "authentication": "password",
                "password": "test-only-not-a-real-secret",
            }
        })
    };

    runtime
        .store_connection(
            crate::core::model::StoredConnection::from_lifecycle_params(&lifecycle("first"))
                .expect("parse connection"),
        )
        .expect("first connect registers");
    assert!(runtime.has_connection("conn-1"));

    // 残留：绑定到已死会话的 sudo 保活、孤儿会话缓存、无会话的连接级
    // remote-forward 表——普通路径由 close/disconnect 清理，重推 connect
    // 时由 sweep 兜底。
    runtime
        .sudo_keepalive
        .lock()
        .expect("keepalive registry")
        .insert(
            "conn-1".to_string(),
            SudoKeepalive {
                session_id: "dead-s".to_string(),
                task: tokio::spawn(async {}),
            },
        );
    runtime.metrics_cache.lock().expect("metrics cache").insert(
        "dead-s".to_string(),
        CachedMetrics {
            value: json!({}),
            collected_at: 0,
        },
    );
    runtime
        .compat_hinted
        .lock()
        .expect("compat hints")
        .insert("dead-s".to_string());
    runtime
        .compress_tool_probe
        .lock()
        .expect("probe cache")
        .insert("dead-s".to_string(), false);
    runtime.remote_tables.lock().expect("remote tables").insert(
        "conn-1".to_string(),
        Arc::new(RemoteForwardTable::default()),
    );

    // 同 connectionId 重复 connect：幂等替换（刷新后宿主重推凭据的路径）。
    runtime
        .store_connection(
            crate::core::model::StoredConnection::from_lifecycle_params(&lifecycle("second"))
                .expect("parse connection"),
        )
        .expect("replayed connect is idempotent");
    assert!(runtime.has_connection("conn-1"), "registry must stay live");

    assert!(
        !runtime
            .sudo_keepalive
            .lock()
            .expect("keepalive registry")
            .contains_key("conn-1"),
        "keepalive bound to a dead session must be swept"
    );
    assert!(
        !runtime
            .metrics_cache
            .lock()
            .expect("metrics cache")
            .contains_key("dead-s"),
        "orphan metrics snapshot must be swept"
    );
    assert!(
        !runtime
            .compat_hinted
            .lock()
            .expect("compat hints")
            .contains("dead-s"),
        "orphan compat hint must be swept"
    );
    assert!(
        !runtime
            .compress_tool_probe
            .lock()
            .expect("probe cache")
            .contains_key("dead-s"),
        "orphan probe result must be swept"
    );
    assert!(
        !runtime
            .remote_tables
            .lock()
            .expect("remote tables")
            .contains_key("conn-1"),
        "connection-scoped remote-forward table without a live session must be swept"
    );
}

// ===== issue #108：堡垒机 MFA（keyboard-interactive）行为锁定 =====
//
// 登录期 keyboard-interactive 由 `authenticate_keyboard_interactive` 驱动：
// 最多 KEYBOARD_INTERACTIVE_MAX_ROUNDS 轮，每轮逐 prompt 分类应答（密码 /
// OTP / 组合，见 exec.rs `keyboard_interactive_answers` 的既有测试）；分类
// 不出的 prompt 用 challenge 上下文兜底，仍为空的答案弹结构化一次性输入
// 对话框（`request_keyboard_interactive_answer`），绝不是在 PTY 里瞎敲。
// 以下三个纯助手是该链路的用户可见面——逐字锁定（堡垒机排障依赖这些文案
// 的稳定性，且 sanitize 保证服务器可控文本进不了 UI 注入面）。
