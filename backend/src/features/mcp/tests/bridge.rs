use super::*;

#[test]
fn parses_bg_start_markers_with_default_log_fallback() {
    let (pid, log) = parse_bg_start_output(
        "PID=4242\nLOG=/tmp/.dbx-ssh-tasks/bg-1-2.log\n",
        "/tmp/.dbx-ssh-tasks/default.log".to_string(),
    );
    assert_eq!(pid, "4242");
    assert_eq!(log, "/tmp/.dbx-ssh-tasks/bg-1-2.log");

    // A remote without echo output (busybox edge) still yields the
    // derived default path.
    let (pid, log) = parse_bg_start_output("", "/tmp/.dbx-ssh-tasks/x.log".to_string());
    assert_eq!(pid, "");
    assert_eq!(log, "/tmp/.dbx-ssh-tasks/x.log");
}

#[test]
fn parses_task_status_running_done_and_missing() {
    let (state, code, alive, tail) = parse_task_status_output(
        "STATE=RUNNING\nPID_ALIVE=yes\n===TAIL===\nstep 1 done\nstep 2 running\n",
    );
    assert_eq!(state, "running");
    assert_eq!(code, None);
    assert_eq!(alive, Some(true));
    assert_eq!(tail, "step 1 done\nstep 2 running\n");

    let (state, code, alive, _tail) =
        parse_task_status_output("STATE=DONE\nCODE=EXIT_3\nPID_ALIVE=no\n===TAIL===\nEXIT_3\n");
    assert_eq!(state, "done");
    assert_eq!(code, Some(3));
    assert_eq!(alive, Some(false));

    let (state, code, alive, tail) = parse_task_status_output("STATE=MISSING\n===TAIL===\n");
    assert_eq!(state, "missing");
    assert_eq!(code, None);
    assert_eq!(alive, None);
    assert_eq!(tail, "");
}

#[tokio::test]
async fn join_all_runs_its_futures_concurrently() {
    // 确定性并发探针：f1 轮询门闩直到 f2 置位。串行执行下 f1 的 100 次
    // 1ms 轮询全空转、返回 0；真并发下 f2 先完成置位、f1 返回 1。
    let gate = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // 两个 async block 类型不同，统一装箱成 trait object 才能进同一个 Vec。
    let first: std::pin::Pin<Box<dyn Future<Output = i32> + Send>> = Box::pin({
        let gate = gate.clone();
        async move {
            for _ in 0..100 {
                if gate.load(std::sync::atomic::Ordering::SeqCst) {
                    return 1;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            0
        }
    });
    let second: std::pin::Pin<Box<dyn Future<Output = i32> + Send>> = Box::pin(async move {
        gate.store(true, std::sync::atomic::Ordering::SeqCst);
        2
    });
    assert_eq!(join_all(vec![first, second]).await, vec![1, 2]);
}

#[tokio::test]
async fn join_all_preserves_input_order_across_recursion() {
    // 二分递归路径（>2 元）：输出顺序必须仍与输入下标一致。
    let futures: Vec<_> = (0..7).map(|value| async move { value * 10 }).collect();
    assert_eq!(join_all(futures).await, vec![0, 10, 20, 30, 40, 50, 60]);
}

/// Reliability round 5: malformed request envelopes get structured
/// -32600 errors (never a panic, never a silently tolerated shape), and
/// a valid request still works right after every hostile one.
#[tokio::test]
async fn malformed_envelopes_get_structured_invalid_request_errors() {
    let state = state();
    let hostile: Vec<(Value, &str)> = vec![
        // Wrong / missing protocol version.
        (
            json!({ "jsonrpc": "1.0", "id": 1, "method": "ping" }),
            "jsonrpc",
        ),
        (
            json!({ "jsonrpc": 2.0, "id": 2, "method": "ping" }),
            "jsonrpc",
        ),
        (json!({ "id": 3, "method": "ping" }), "jsonrpc"),
        // Malformed id shapes (JSON-RPC allows string/number only).
        (
            json!({ "jsonrpc": "2.0", "id": { "n": 1 }, "method": "ping" }),
            "id",
        ),
        (
            json!({ "jsonrpc": "2.0", "id": true, "method": "ping" }),
            "id",
        ),
        (
            json!({ "jsonrpc": "2.0", "id": [7], "method": "ping" }),
            "id",
        ),
        // Missing / non-string method.
        (json!({ "jsonrpc": "2.0", "id": 4 }), "method"),
        (json!({ "jsonrpc": "2.0", "id": 5, "method": 42 }), "method"),
        (json!({ "jsonrpc": "2.0", "id": 6, "method": "" }), "method"),
    ];
    for (request, fragment) in hostile {
        let response = state.dispatch(request).await.unwrap();
        assert_eq!(
            response["error"]["code"], -32600,
            "expected -32600 for {response}"
        );
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains(fragment));
        // The echoed id is protocol-valid (string/number) or null.
        assert!(
            response["id"].is_null() || response["id"].is_string() || response["id"].is_number(),
            "invalid id echo: {response}"
        );
        // The connection stays healthy: ping still answers.
        let pong = state
            .dispatch(json!({ "jsonrpc": "2.0", "id": 99, "method": "ping" }))
            .await
            .unwrap();
        assert_eq!(pong["id"], 99);
        assert!(pong.get("error").is_none(), "ping failed: {pong}");
    }

    // An explicit `id: null` is invalid per JSON-RPC and answers with a
    // null-id -32600 (documented tiering: 无 id/坏 id → -32600).
    let null_id = state
        .dispatch(json!({ "jsonrpc": "2.0", "id": null, "method": "ping" }))
        .await
        .unwrap();
    assert_eq!(null_id["error"]["code"], -32600);
    assert!(null_id["id"].is_null());

    // Well-formed string and number ids still echo verbatim.
    let string_id = state
        .dispatch(json!({ "jsonrpc": "2.0", "id": "abc-1", "method": "ping" }))
        .await
        .unwrap();
    assert_eq!(string_id["id"], "abc-1");
    assert!(string_id.get("error").is_none());
}

/// Reliability round 5: the stdio line classifier must survive hostile
/// lines — invalid UTF-8 (previously killed the whole session), garbage
/// JSON, blank/CRLF lines — replying -32700 where a reply is owed and
/// surfacing real I/O errors.
#[test]
fn stdio_line_classification_survives_hostile_lines() {
    // Blank and whitespace/CRLF-only lines are silent (no death loop).
    assert!(matches!(
        classify_stdio_line(Ok(String::new())),
        Ok(StdioLine::Silent)
    ));
    assert!(matches!(
        classify_stdio_line(Ok("   \t ".to_string())),
        Ok(StdioLine::Silent)
    ));
    assert!(matches!(
        classify_stdio_line(Ok("\r".to_string())),
        Ok(StdioLine::Silent)
    ));

    // Invalid UTF-8 becomes a -32700 parse error, not a process exit.
    let invalid_utf8 = Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "stream did not contain valid UTF-8",
    ));
    match classify_stdio_line(invalid_utf8) {
        Ok(StdioLine::Reply(response)) => {
            assert_eq!(response["error"]["code"], -32700);
            assert!(response["id"].is_null());
        }
        other => panic!("expected a parse-error reply, got {other:?}"),
    }

    // Real I/O errors still abort the loop.
    let io_error = Err(io::Error::new(io::ErrorKind::BrokenPipe, "gone"));
    assert!(classify_stdio_line(io_error).is_err());

    // Garbage JSON answers -32700; valid JSON reaches the dispatcher.
    match classify_stdio_line(Ok("{not json".to_string())) {
        Ok(StdioLine::Reply(response)) => {
            assert_eq!(response["error"]["code"], -32700);
        }
        other => panic!("expected a parse-error reply, got {other:?}"),
    }
    match classify_stdio_line(Ok(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_string())) {
        Ok(StdioLine::Request(request)) => assert_eq!(request["method"], "ping"),
        other => panic!("expected a request, got {other:?}"),
    }
    // CRLF line endings parse fine (serde_json tolerates the trailing \r).
    match classify_stdio_line(Ok(
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\r".to_string()
    )) {
        Ok(StdioLine::Request(request)) => assert_eq!(request["id"], 2),
        other => panic!("expected a request, got {other:?}"),
    }
}

/// Round 6 family contract: the single-line ceiling parses its env knob
/// with a safe fallback (junk / zero / unset → the 16 MiB default), and
/// an over-limit line answers a structured -32700 naming the limit and
/// the env var (the loop consumed the line through its newline, so the
/// session continues — pinned e2e by the smoke line-limit section).
#[test]
fn stdio_max_line_env_is_parsed_with_safe_fallback() {
    std::env::set_var("DBX_SSH_MCP_STDIO_MAX_LINE", "1024");
    assert_eq!(stdio_max_line_bytes(), 1024);
    std::env::set_var("DBX_SSH_MCP_STDIO_MAX_LINE", " 8388608 ");
    assert_eq!(stdio_max_line_bytes(), 8_388_608);
    for junk in ["", "abc", "0", "-5", "16 MiB"] {
        std::env::set_var("DBX_SSH_MCP_STDIO_MAX_LINE", junk);
        assert_eq!(
            stdio_max_line_bytes(),
            DEFAULT_STDIO_MAX_LINE_BYTES,
            "junk value {junk:?} must fall back to the default"
        );
    }
    std::env::remove_var("DBX_SSH_MCP_STDIO_MAX_LINE");
    assert_eq!(stdio_max_line_bytes(), DEFAULT_STDIO_MAX_LINE_BYTES);
}

#[test]
fn over_limit_line_gets_structured_parse_error() {
    let response = over_limit_line_response(20 * 1024 * 1024, DEFAULT_STDIO_MAX_LINE_BYTES);
    assert_eq!(response["error"]["code"], -32700);
    assert!(response["id"].is_null());
    let message = response["error"]["message"].as_str().unwrap();
    assert!(message.contains("exceeds the"), "{message}");
    assert!(message.contains("16777216-byte limit"), "{message}");
    assert!(message.contains("DBX_SSH_MCP_STDIO_MAX_LINE"), "{message}");
}

/// Round 6: missing-required errors enumerate EVERY absent parameter in
/// one message (`Missing required parameters: a, b`) so an LLM fixes all
/// gaps in one turn; present-but-wrong-typed values keep the precise
/// per-parameter error instead of being misreported as missing.
#[tokio::test]
async fn missing_required_errors_enumerate_every_gap() {
    let state = state();
    // ssh_multi_exec: both gaps named in one error, schema required order.
    let message = multi_exec_error(&state, json!({})).await;
    assert!(message.contains("Missing required parameters"), "{message}");
    assert!(
        message.contains("targets") && message.contains("command"),
        "{message}"
    );

    // Only one gap → only that one named (the other was supplied).
    let message = multi_exec_error(&state, json!({ "targets": ["conn-1"] })).await;
    assert_eq!(message, "Missing required parameters: command", "{message}");
    let message = multi_exec_error(&state, json!({ "command": "uptime" })).await;
    assert_eq!(message, "Missing required parameters: targets", "{message}");

    // sftp_upload / sftp_download: both paths enumerated together.
    for name in ["sftp_upload", "sftp_download"] {
        let message = tool_error(&state, name, json!({})).await;
        assert!(
            message.contains("Missing required parameters"),
            "{name}: {message}"
        );
        assert!(
            message.contains("localPath") && message.contains("remotePath"),
            "{name}: {message}"
        );
    }

    // Present-but-wrong-typed stays a per-parameter error (smoke pins the
    // array case too): enumeration must not mislabel it as missing. Note
    // `{"targets": "conn-1"}` alone still enumerates command — command is
    // genuinely absent there.
    let message =
        multi_exec_error(&state, json!({ "targets": "conn-1", "command": "uptime" })).await;
    assert!(message.contains("targets must be an array"), "{message}");
    // Null counts as absent (JSON's explicit "no value").
    let message = multi_exec_error(&state, json!({ "targets": ["conn-1"], "command": null })).await;
    assert_eq!(message, "Missing required parameters: command", "{message}");
}

/// Reliability round 5 (churn): 500 register→resolve→drop cycles on the
/// lifecycle registry must keep every lookup shape correct (exact id,
/// display name, endpoint identity, ambiguity, stale-id miss) and the
/// map bounded to the live entries.
#[tokio::test]
async fn registry_churn_keeps_lookup_correct() {
    let mut state = state();
    state.bridge_fallback = false;
    const CYCLES: usize = 500;
    for index in 0..CYCLES {
        let id = format!("churn-conn-{index}");
        let name = format!("Web {index}");
        let connection = stored_connection(&id, "192.0.2.10", 22, json!({ "name": name }));
        // Register.
        state
            .dbx_connections
            .write()
            .await
            .insert(id.clone(), connection);
        assert!(
            state.dbx_connections.read().await.len() <= 1,
            "registry leaked before cycle {index}"
        );
        // Exact id lookup.
        let resolved = state
            .registered_connection_by_ref(&json!({ "connectionId": id }))
            .await
            .expect("id lookup");
        assert!(resolved.is_some(), "registered id not found at {index}");
        // Display-name lookup.
        let resolved = state
            .registered_connection_by_ref(&json!({ "connectionName": name }))
            .await
            .expect("name lookup");
        assert_eq!(resolved.unwrap().id, id);
        // Endpoint lookup (host + username, default port).
        let resolved = state
            .registered_connection_by_ref(&json!({ "host": "192.0.2.10", "username": "deploy" }))
            .await
            .expect("endpoint lookup");
        assert_eq!(resolved.unwrap().id, id);
        // Mismatched selectors are an error, not a silent miss.
        assert!(state
            .registered_connection_by_ref(&json!({
                "connectionId": id, "connectionName": "other"
            }))
            .await
            .is_err());
        // Drop and confirm the miss (stale ids keep the self-heal path).
        state.dbx_connections.write().await.remove(&id);
        let resolved = state
            .registered_connection_by_ref(&json!({ "connectionId": id }))
            .await
            .expect("post-drop lookup");
        assert!(resolved.is_none(), "dropped id still resolves at {index}");
        assert!(state.dbx_connections.read().await.is_empty());
    }
    assert!(state.dbx_connections.read().await.is_empty());
}

/// Reliability round 5 (churn/idempotency): 100 repeated calls of the
/// read-only tools (alert triage, known_hosts, settings read) must
/// return byte-identical results — no state accumulation, no drift.
#[tokio::test]
async fn read_only_tools_are_stable_under_repeated_calls() {
    let state = state();
    let first_connections = state
        .call_tool("ssh_list_connections", &json!({}), None)
        .await
        .expect("connections");
    let first_hosts = state
        .call_tool("ssh_list_known_hosts", &json!({}), None)
        .await
        .expect("known_hosts");
    let first_settings = state.settings_get();
    for index in 1..100 {
        let connections = state
            .call_tool("ssh_list_connections", &json!({}), None)
            .await
            .expect("connections");
        assert_eq!(
            connections, first_connections,
            "connections drifted at call {index}"
        );
        let hosts = state
            .call_tool("ssh_list_known_hosts", &json!({}), None)
            .await
            .expect("known_hosts");
        assert_eq!(hosts, first_hosts, "known_hosts drifted at call {index}");
        assert_eq!(
            state.settings_get(),
            first_settings,
            "settings read drifted at call {index}"
        );
    }
}

/// Reliability round 5 (churn): the `ssh_run_bg` / `ssh_task_status`
/// offline surface is the two output parsers (the task table itself
/// lives on the remote host under ~/.dbx-ssh-tasks — there is no
/// in-process registry to leak). 300 start→running→done→missing cycles
/// must parse identically every time.
#[test]
fn bg_task_parsers_stay_stable_under_churn() {
    let start = "PID=4242\nLOG=/tmp/.dbx-ssh-tasks/bg-1.log";
    let running = "STATE=RUNNING\nPID_ALIVE=yes\n===TAIL===\nworking\n";
    let done = "STATE=DONE\nCODE=EXIT_0\nPID_ALIVE=no\n===TAIL===\nEXIT_0\n";
    let missing = "STATE=MISSING\n===TAIL===\n";
    for cycle in 0..300 {
        assert_eq!(
            parse_bg_start_output(start, "/tmp/.dbx-ssh-tasks/default.log".to_string()),
            (
                "4242".to_string(),
                "/tmp/.dbx-ssh-tasks/bg-1.log".to_string()
            ),
            "bg start drifted at cycle {cycle}"
        );
        assert_eq!(
            parse_task_status_output(running),
            (
                "running".to_string(),
                None,
                Some(true),
                "working\n".to_string()
            ),
            "running parse drifted at cycle {cycle}"
        );
        assert_eq!(
            parse_task_status_output(done),
            (
                "done".to_string(),
                Some(0),
                Some(false),
                "EXIT_0\n".to_string()
            ),
            "done parse drifted at cycle {cycle}"
        );
        assert_eq!(
            parse_task_status_output(missing),
            ("missing".to_string(), None, None, String::new()),
            "missing parse drifted at cycle {cycle}"
        );
    }
}

/// In stdio mode (no emitter), `runInTerminal: true` needs a saved DBX
/// connectionId to forward through the app bridge; without one it is
/// refused with guidance instead of silently running hidden, and the
/// read-only/destructive gates keep firing before the routing branch.
#[tokio::test]
async fn run_in_terminal_stdio_requires_a_connection_id() {
    let state = state();

    let refused = state
        .run_tool(
            "ssh_exec",
            &json!({ "host": "h", "username": "u", "command": "echo hi", "runInTerminal": true }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        refused.contains("runInTerminal needs a saved DBX connection"),
        "unexpected: {refused}"
    );
    // L0 self-heal hint appended to the guidance error.
    assert!(
        refused.contains("ssh_list_connections"),
        "expected the discovery-tool hint, got: {refused}"
    );

    // The same refusal covers the sudo tool; runInTerminal absent or
    // false keeps the existing hidden-channel behavior (which then fails
    // on the missing password, proving the route was not taken).
    let sudo = state
        .run_tool(
            "ssh_exec_sudo",
            &json!({ "host": "h", "username": "u", "command": "uptime", "runInTerminal": true }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        sudo.contains("runInTerminal needs a saved DBX connection"),
        "unexpected: {sudo}"
    );
    let hidden = state
        .run_tool(
            "ssh_exec",
            &json!({ "host": "h", "username": "u", "command": "echo hi", "runInTerminal": false }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        !hidden.contains("runInTerminal"),
        "false must keep the hidden channel: {hidden}"
    );
}

/// L0: with the bridge fallback disabled (hermetic stand-in for "the
/// bridge is also unavailable"), an unregistered connectionId reports
/// the full self-heal path instead of the old bare message.
#[tokio::test]
async fn unregistered_connection_id_error_carries_selfheal_hints() {
    let mut state = state();
    state.bridge_fallback = false;
    let error = state
        .call_tool(
            "sftp_stat",
            &json!({ "connectionId": "ghost", "path": "/tmp" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains(
            "not registered with this plugin session and the DBX app bridge is unavailable"
        ),
        "unexpected: {error}"
    );
    assert!(
        error.contains("ssh_list_connections") && error.contains("inline credentials"),
        "expected the self-heal hints, got: {error}"
    );
}

/// L1: the forward decision reads only the registry — a registered id
/// stays local, an unregistered id forwards with the original
/// arguments, local-only tools and runInTerminal routing never forward,
/// and the kill switch disables the fallback entirely.
#[tokio::test]
async fn bridge_forward_plan_decides_by_registry() {
    let mut state = state();
    state.bridge_fallback = true;
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-fwd",
            "name": "Prod",
            "host": "192.0.2.10",
            "port": 22,
            "username": "ops",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
        }
    }))
    .unwrap();
    state
        .dbx_connections
        .write()
        .await
        .insert("conn-fwd".to_string(), stored);

    // Registered id: local path owns the call.
    let plan = state
        .bridge_forward_plan(
            "ssh_exec",
            &json!({ "connectionId": "conn-fwd", "command": "uptime" }),
        )
        .await
        .unwrap();
    assert!(plan.is_none(), "registered id must stay local");

    // Unregistered id: forward with the arguments untouched.
    let (id, args) = state
        .bridge_forward_plan(
            "ssh_exec",
            &json!({ "connectionId": "ghost", "command": "uptime" }),
        )
        .await
        .unwrap()
        .expect("unregistered id must forward");
    assert_eq!(id, "ghost");
    assert_eq!(args["command"], "uptime");

    // Local-only tools never forward, even with an unregistered id.
    let plan = state
        .bridge_forward_plan("ssh_close", &json!({ "connectionId": "ghost" }))
        .await
        .unwrap();
    assert!(plan.is_none(), "ssh_close keeps local semantics");

    // Registry name hit stays local (connection() resolves it).
    let plan = state
        .bridge_forward_plan(
            "sftp_list_dir",
            &json!({ "connectionName": "Prod", "path": "/tmp" }),
        )
        .await
        .unwrap();
    assert!(plan.is_none(), "registry name hit stays local");

    // No connection reference: local path.
    let plan = state
        .bridge_forward_plan("sftp_list_dir", &json!({}))
        .await
        .unwrap();
    assert!(plan.is_none());

    // runInTerminal keeps its exclusive forward path.
    let plan = state
        .bridge_forward_plan(
            "ssh_exec",
            &json!({ "connectionId": "ghost", "command": "uptime", "runInTerminal": true }),
        )
        .await
        .unwrap();
    assert!(plan.is_none(), "runInTerminal owns its bridge forward");

    // Fallback disabled: never forward.
    state.bridge_fallback = false;
    let plan = state
        .bridge_forward_plan(
            "ssh_exec",
            &json!({ "connectionId": "ghost", "command": "uptime" }),
        )
        .await
        .unwrap();
    assert!(plan.is_none(), "disabled fallback must not forward");
}

/// 降级矩阵 ①：每一个连接级工具在 stdio 会话里带未注册 connectionId 时
/// 都会进入 L1 桥转发计划（即桥不可用时全部走同一 fail-closed 回落），
/// 而本地工具从不转发。只读进程级开关不改变转发决定（转发语义由
/// 应用侧门禁负责）。
#[tokio::test]
async fn every_connection_bound_tool_plans_a_bridge_forward_for_a_ghost_id() {
    let mut state = state();
    state.bridge_fallback = true;
    let forwarders: Vec<&str> = TOOL_NAMES
        .iter()
        .copied()
        .filter(|name| is_connection_bound_tool(name))
        .collect();
    assert_eq!(forwarders.len(), 10, "connection-bound tool set drifted");
    for name in forwarders {
        let arguments = match name {
            "ssh_exec" | "ssh_exec_sudo" | "ssh_run_bg" => {
                json!({ "connectionId": "ghost", "command": "uptime" })
            }
            "ssh_task_status" => {
                json!({ "connectionId": "ghost", "logPath": "/tmp/.dbx-ssh-tasks/x.log" })
            }
            "sftp_upload" => json!({ "connectionId": "ghost",
                "localPath": "/tmp/in", "remotePath": "/tmp/out" }),
            "sftp_download" => json!({ "connectionId": "ghost",
                "remotePath": "/tmp/in", "localPath": "/tmp/out" }),
            _ => json!({ "connectionId": "ghost", "path": "/tmp" }),
        };
        let plan = state
            .bridge_forward_plan(name, &arguments)
            .await
            .unwrap_or_else(|error| panic!("{name} plan errored: {error}"));
        assert!(
            plan.is_some(),
            "{name} with an unregistered connectionId must plan a bridge forward"
        );
        let (id, forwarded) = plan.unwrap();
        assert_eq!(id, "ghost", "{name} must forward under the requested id");
        assert_eq!(forwarded["connectionId"], "ghost");
    }
    // 本地工具与本地无连接工具不进转发计划。
    for name in ["ssh_close", "ssh_list_connections", "ssh_list_known_hosts"] {
        let plan = state
            .bridge_forward_plan(name, &json!({ "connectionId": "ghost" }))
            .await
            .unwrap();
        assert!(plan.is_none(), "{name} must stay local");
    }
}

#[test]
fn mcp_remote_path_normalization_contract() {
    // M25 契约：所有接受远端路径的 MCP SFTP 工具分发臂（list_dir /
    // stat / exists / read / write / mkdir / remove / rename /
    // chmod / disk_usage / upload / download）在拿到路径后立即
    // `normalize_remote_path`，再进 latin-1 编码还原或 auto 直用。
    // 归一 = 绝对化 + 去 ./../ 空段，不做 ~ 展开（工作台同口径）。
    assert_eq!(normalize_remote_path("/a//b/../c").unwrap(), "/a/c");
    assert_eq!(normalize_remote_path("a/b/").unwrap(), "/a/b");
    assert_eq!(normalize_remote_path("/x/./y").unwrap(), "/x/y");
    assert_eq!(normalize_remote_path("../etc").unwrap(), "/etc");
    assert_eq!(normalize_remote_path("/").unwrap(), "/");
}

#[test]
fn mcp_remote_path_normalization_rejects_empty_and_nul() {
    // 空路径与含 NUL 的路径在进入任何 SFTP 操作前被拒绝，
    // 与工作台车道的拒绝语义一致。
    assert!(normalize_remote_path("").is_err());
    assert!(normalize_remote_path("/a/\0b").is_err());
}

#[test]
fn mcp_latin1_encoding_unchanged_by_normalization_for_clean_names() {
    // 归一是纯字符级清洗：latin-1 显示域内的 clean 名经过
    // normalize_remote_path 后字节还原结果不变（AI 回传列表路径的
    // 往返闭环不被 M25 破坏）。
    let display = "/data/caf\u{e9}.txt";
    assert_eq!(
        sftp_name::latin1_encode_display(&normalize_remote_path(display).unwrap()),
        sftp_name::latin1_encode_display(display)
    );
}
