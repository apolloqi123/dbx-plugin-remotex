use super::*;

fn state() -> McpState {
    // Unique data dir per call: settings persistence (limits AND the
    // §1.3 permission keys) must not leak across parallel tests.
    McpState::new(std::env::temp_dir().join(format!("dbx-mcp-test-{}", uuid::Uuid::new_v4())))
}

/// tools/list 尺寸预算（#172 / #203）。计量口径必须写清，避免误读：
///
/// **量的是什么**：本测试量的是 sidecar `tools/list` 的 raw JSON 序列化
/// （serde `Value::to_string`），不是 MCP 客户端实际收到的那份文本。桥模式
/// 下宿主 `dbx_plugin_tools`（dbx-mcp，宿主仓 crates/dbx-mcp/src/server.rs）
/// 把工具目录重新拼装成自己的文本再返回，两次序列化不是同一结果（实测
/// 桥文本 ≈ raw × 0.9：#203 前单轮实测 99,909 → 90,114，桥比率 0.902）。
///
/// **实测值与断言口径**：#203 描述瘦身二轮后实测 raw JSON = 43,842 字符
/// （43,866 字节），估算桥文本 ≈ 39.5k。断言 50_000 = raw 上限，对应
/// 桥文本 ≤ ≈45k，仍低于 **Claude Code ≈50,000 字符落盘线**
/// （persistenceThresholdCeiling，实测客户端中最严；Qoder CLI ≈75,000）——
/// 即断言内即可保证 Claude Code 不落盘。#172 时代的 100k 预算只是防回归，
/// 不是客户端无落盘判据——当时 33 个工具实测 90k 桥文本在两个实测客户端
/// 都落盘。
///
/// **工具面怎么到 16 个的**：#203 两轮瘦身，移除冗余/可推导/命令可实现
/// 的工具（sftp_exists、sftp_pwd、ssh_test_connection、ssh_alert_triage、
/// Quick Sudo 三件套、ssh_metrics、sftp_disk_usage、docker 两件套、SFTP
/// 变更族 mkdir/remove/rename/chmod/copy/move——最后这族与 shell
/// mkdir/rm/mv/chmod/cp 语义重复，走 ssh_exec，只读白名单与
/// sensitive-path 门在 exec 路径上等价生效）。前端工作台的对应 sidecar
/// RPC 方法全部保留，MCP 面收缩不影响 UI。新增工具前先看这个预算：
/// 能并进既有工具或用 ssh_exec 实现的不再单列。
///
/// 超预算时先砍最长 description，而不是删属性名。
#[test]
fn tool_definitions_stay_under_budget() {
    let serialized = tool_definitions().to_string();
    let chars = serialized.chars().count();
    assert!(
        chars <= 50_000,
        "tools/list raw JSON is {chars} chars (budget 50_000; measured 43_842 after \
         the #203 description slim-down); at the ~0.9 bridge ratio that keeps the \
         host dbx_plugin_tools text under ~45k, inside Claude Code's ~50k \
         persistence line. Trim the fattest descriptions in \
         connection_properties/tool_definitions or reconsider whether the new \
         tool belongs in the MCP face (#203)"
    );
}

// —— §1.3 confirm 档 + connectionScope ——

#[test]
fn scope_allows_matches_id_name_and_host() {
    let scope = |entries: &[&str]| -> Vec<String> {
        entries.iter().map(|entry| entry.to_string()).collect()
    };
    // 空 entry 列表本身不授予权限；持久化空数组会在 ConnectionScope
    // 层映射为 Unrestricted，显式空环境变量则映射为 DenyAll。
    assert!(!scope_allows(&[], "c1", Some("prod"), "db.local"));
    let entries = scope(&["conn-9", "ops@LEGACY", "Web-01"]);
    assert!(scope_allows(&entries, "conn-9", None, "other.local"));
    assert!(scope_allows(&entries, "other-id", Some("ops@LEGACY"), "x")); // name 精确（大小写敏感）
    assert!(scope_allows(&entries, "other-id", None, "web-01")); // host 大小写不敏感
    assert!(!scope_allows(
        &entries,
        "other-id",
        Some("prod"),
        "db.local"
    ));
    assert!(!scope_allows(&entries, "other-id", None, "db.local"));
}

#[test]
fn env_permission_parsers_distinguish_unset_allowlist_and_deny_all() {
    assert_eq!(
        permission_mode_from_env(|_| Some("confirm".to_string())).as_deref(),
        Some("confirm")
    );
    // 未设置/未知值不覆盖。
    assert!(permission_mode_from_env(|_| None).is_none());
    assert!(permission_mode_from_env(|_| Some("yolo".to_string())).is_none());
    assert_eq!(
        scope_from_env(|_| Some(" a , b,,c ".to_string())),
        Some(ConnectionScope::AllowList(vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
        ]))
    );
    // 显式空列表是有效的 fail-closed 覆盖，绝不能与持久化空数组
    // （未限制）复用同一种状态。
    assert_eq!(
        scope_from_env(|_| Some(" , ".to_string())),
        Some(ConnectionScope::DenyAll)
    );
    assert!(scope_from_env(|_| None).is_none());
}

#[test]
fn connection_scope_policy_preserves_persisted_empty_as_unrestricted() {
    assert!(ConnectionScope::from_persisted(Vec::new()).allows("id", Some("name"), "host"));
    assert!(!ConnectionScope::DenyAll.allows("id", Some("name"), "host"));
    assert!(ConnectionScope::AllowList(vec!["host".to_string()]).allows("id", None, "HOST"));
}

#[test]
fn confirm_gate_set_covers_exec_family_and_spares_read_tools() {
    for gated in [
        "ssh_exec",
        "ssh_exec_sudo",
        "ssh_run_bg",
        "ssh_multi_exec",
        "ssh_terminal_input",
        "sftp_write_file",
        "sftp_upload",
    ] {
        assert!(is_confirm_gated_tool(gated), "{gated} should be gated");
    }
    for spared in [
        "ssh_close",
        "sftp_list_dir",
        "sftp_stat",
        "sftp_read_file",
        "sftp_download",
    ] {
        assert!(!is_confirm_gated_tool(spared), "{spared} must not be gated");
    }
}

#[test]
fn scoped_tool_set_excludes_local_tools() {
    for scoped in ["ssh_exec", "sftp_upload", "ssh_close", "ssh_task_status"] {
        assert!(is_connection_scoped_tool(scoped), "{scoped}");
    }
    for local in [
        "ssh_list_connections",
        "ssh_list_known_hosts",
        "ssh_remove_known_host",
    ] {
        assert!(!is_connection_scoped_tool(local), "{local}");
    }
}

#[test]
fn every_tool_carries_complete_annotations() {
    let definitions = tool_definitions();
    let tools = definitions.as_array().unwrap();
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let annotations = &tool["annotations"];
        let obj = annotations
            .as_object()
            .unwrap_or_else(|| panic!("{name} lacks annotations"));
        for hint in [
            "readOnlyHint",
            "destructiveHint",
            "idempotentHint",
            "openWorldHint",
        ] {
            assert!(
                obj.get(hint).and_then(Value::as_bool).is_some(),
                "{name} annotations lack boolean {hint}"
            );
        }
        assert!(
            obj.get("title").and_then(Value::as_str).is_some(),
            "{name} annotations lack a title"
        );
    }
}

/// Embedded-AI listings hide arguments that only add noise to the host's
/// approval card: the host injects connectionId from lifecycle and the sidecar
/// applies timeout defaults. Standalone listings keep them for external MCP
/// clients that address connections/caps explicitly.
#[test]
fn host_tool_listing_hides_display_noise_arguments() {
    let full = tool_definitions();
    let host = tool_definitions_for_host();
    let properties = |definitions: &Value, name: &str| {
        definitions
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from tools/list"))["inputSchema"]["properties"]
            .as_object()
            .cloned()
            .unwrap()
    };

    for name in ["ssh_exec", "ssh_exec_sudo", "sftp_upload"] {
        assert!(properties(&full, name).contains_key("connectionId"), "{name} standalone should keep connectionId");
        assert!(!properties(&host, name).contains_key("connectionId"), "{name} host listing should hide connectionId");
    }
    for name in ["ssh_exec", "ssh_exec_sudo", "ssh_multi_exec"] {
        assert!(properties(&full, name).contains_key("timeoutSecs"), "{name} standalone should keep timeoutSecs");
        assert!(!properties(&host, name).contains_key("timeoutSecs"), "{name} host listing should hide timeoutSecs");
    }
    // The command remains the reviewable payload.
    let exec = properties(&host, "ssh_exec");
    assert!(exec.contains_key("command"), "ssh_exec must keep command");
    let required = host
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "ssh_exec")
        .unwrap()["inputSchema"]["required"]
        .clone();
    assert_eq!(required, json!(["command"]));
}

#[test]
fn annotation_hints_align_with_gate_classification() {
    let definitions = tool_definitions();
    let annotations = |name: &str| {
        definitions
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from tools/list"))["annotations"]
            .clone()
    };
    // Provably read-only: the whole set the confirm gate spares for
    // being read-only (plus ssh_close, which is also spared).
    for name in [
        "ssh_list_connections",
        "ssh_list_known_hosts",
        "ssh_task_status",
        "sftp_list_dir",
        "sftp_stat",
        "sftp_read_file",
    ] {
        let a = annotations(name);
        assert_eq!(a["readOnlyHint"], true, "{name} should be readOnly");
        assert_eq!(
            a["destructiveHint"], false,
            "{name} should not be destructive"
        );
    }
    // Exec family and remove/move/overwrite/delete tools advertise
    // destructive so clients gate them like the confirm gate does.
    for name in [
        "ssh_exec",
        "ssh_exec_sudo",
        "ssh_multi_exec",
        "ssh_terminal_input",
        "ssh_run_bg",
        "ssh_remove_known_host",
        "sftp_upload",
        "sftp_download",
        "sftp_write_file",
    ] {
        let a = annotations(name);
        assert_eq!(a["readOnlyHint"], false, "{name} should not be readOnly");
        assert_eq!(a["destructiveHint"], true, "{name} should be destructive");
    }
    // Mutating but non-destructive (idempotent) write surface.
    let a = annotations("ssh_close");
    assert_eq!(a["readOnlyHint"], false, "ssh_close");
    assert_eq!(a["destructiveHint"], false, "ssh_close");
    // Closed-world: every catalogued tool talks to the outside world.
    assert_eq!(annotations("ssh_exec")["openWorldHint"], true);
    // Every declared tool is covered by the explicit sets — no tool may
    // slip through with unreviewed hints.
    let tools = definitions.as_array().unwrap();
    assert_eq!(
        tools.len(),
        16,
        "tool count changed; revisit annotation sets"
    );
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let a = &tool["annotations"];
        assert_eq!(
            a["readOnlyHint"].as_bool().unwrap(),
            matches!(
                name,
                "ssh_list_connections"
                    | "ssh_list_known_hosts"
                    | "ssh_task_status"
                    | "sftp_list_dir"
                    | "sftp_stat"
                    | "sftp_read_file"
            ),
            "{name} readOnlyHint drift"
        );
    }
}

#[test]
fn settings_roundtrip_permission_fields_and_env_override() {
    let state = state();
    // 默认 autonomous + 空作用域。
    let view = state.settings_get();
    assert_eq!(view["execPermissionMode"], "autonomous");
    assert_eq!(view["connectionScope"], json!([]));
    // 合法更新。
    let updated = state
        .settings_set(&json!({
            "execPermissionMode": "confirm",
            "connectionScope": ["conn-1", "Prod-DB"],
        }))
        .unwrap();
    assert_eq!(updated["execPermissionMode"], "confirm");
    assert_eq!(updated["connectionScope"], json!(["conn-1", "Prod-DB"]));
    assert_eq!(updated["persistedExecPermissionMode"], "confirm");
    // 非法 mode / 非 scope 数组被拒绝。
    assert!(state
        .settings_set(&json!({ "execPermissionMode": "yolo" }))
        .is_err());
    assert!(state
        .settings_set(&json!({ "connectionScope": "conn-1" }))
        .is_err());
    // 重读（含从盘加载路径）不丢。
    let reloaded = McpPermission::load(&state.limits_path);
    assert_eq!(reloaded.exec_permission_mode, "confirm");
    assert_eq!(
        reloaded.connection_scope,
        vec!["conn-1".to_string(), "Prod-DB".to_string()]
    );
    // size-limit 键与 permission 键同文件共存（merge 写入）。
    state
        .settings_set(&json!({ "maxReadBytes": 4096 }))
        .unwrap();
    let document: Value =
        serde_json::from_str(&std::fs::read_to_string(&state.limits_path).unwrap()).unwrap();
    assert_eq!(document["maxReadBytes"], 4096);
    assert_eq!(document["execPermissionMode"], "confirm");
}

#[test]
fn connection_list_result_filters_by_scope() {
    let stored = || {
        StoredConnection::from_lifecycle_params(&json!({
            "connection": { "id": "conn-1", "name": "prod", "host": "db.local",
                            "port": 22, "username": "ops", "password": "pw" }
        }))
        .unwrap()
    };
    // 持久化空作用域映射为未限制：两来源都全量。
    let all = connection_list_result(
        Ok(vec![
            json!({"id":"conn-9","name":"staging","host":"stg.local"}),
        ]),
        &[stored()],
        &ConnectionScope::Unrestricted,
    );
    assert_eq!(all["connections"].as_array().unwrap().len(), 2);
    // host 作用域：bridge 条目按 host 过滤，registry 条目按 id/name/host。
    let filtered = connection_list_result(
        Ok(vec![
            json!({"id":"conn-9","name":"staging","host":"stg.local"}),
            json!({"id":"conn-1","name":"prod","host":"db.local"}),
        ]),
        &[stored()],
        &ConnectionScope::AllowList(vec!["db.local".to_string()]),
    );
    let ids: Vec<&str> = filtered["connections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["conn-1"]);
    // registry 降级路径同样过滤。
    let degraded = connection_list_result(
        Err("bridge down".to_string()),
        &[stored()],
        &ConnectionScope::AllowList(vec!["conn-9".to_string()]),
    );
    assert_eq!(degraded["connections"].as_array().unwrap().len(), 0);
}

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

#[tokio::test]
async fn command_text_gates_refuse_destructive_and_unknown_on_read_only() {
    // 提取自 call_tool_inner 的命令文本闸（确认改写后二次过闸复用同一
    // 入口）：read-only 下破坏性命令无条件拒绝、白名单外命令拒绝、白名单
    // 内只读命令放行。
    let state = state();
    let error = state
        .enforce_command_text_gates("ssh_exec", "rm -rf /tmp/x", true, &json!({}))
        .await
        .unwrap_err();
    assert!(error.contains("read-only"), "got: {error}");
    let error = state
        .enforce_command_text_gates("ssh_exec", "curl example.test", true, &json!({}))
        .await
        .unwrap_err();
    assert!(
        error.contains("not recognized as read-only"),
        "got: {error}"
    );
    state
        .enforce_command_text_gates("ssh_exec", "ls /tmp", true, &json!({}))
        .await
        .expect("inspection command passes the text gates");
}

#[test]
fn read_bounded_line_stays_within_the_ceiling() {
    let max_line = 16;
    // 正常行：含换行原样返回。
    let mut reader = io::Cursor::new(b"ping\nnext\n".to_vec());
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::Line
    ));
    assert_eq!(raw, b"ping\n");
    // EOF 尾行（无换行、且在上限内）：不算超限，正常返回。
    let mut reader = io::Cursor::new(b"tail-no-nl".to_vec());
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::Line
    ));
    // 恰好 max_line 字节内容 + 换行：合法满行。
    let mut full_line = vec![b'a'; max_line];
    full_line.push(b'\n');
    let mut reader = io::Cursor::new(full_line);
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::Line
    ));
    assert_eq!(raw.len(), max_line + 1);
    // 超限：读满上限仍无换行 → OverLimit，残余被吞弃，下一条是后续行。
    let mut input = vec![b'x'; 100];
    input.extend_from_slice(b"\nok\n");
    let mut reader = io::Cursor::new(input);
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::OverLimit
    ));
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::Line
    ));
    assert_eq!(raw, b"ok\n");
    // EOF：空读。
    let mut reader = io::Cursor::new(Vec::<u8>::new());
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::Eof
    ));
}

#[tokio::test]
async fn run_bg_respects_read_only_gate() {
    let mut state = state();
    state.global_read_only = true;
    let error = state
        .call_tool(
            "ssh_run_bg",
            &json!({
                "host": "example.test",
                "username": "op",
                "command": "sleep 30"
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("read-only"),
        "expected read-only refusal, got: {error}"
    );
}

#[tokio::test]
async fn inline_dial_inherits_registered_read_only_gate() {
    let state = state();
    // All credential values in tests are assembled at runtime — never
    // real credentials, never literals in source.
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-ro",
            "host": "prod.example.test",
            "port": 2222,
            "username": "deploy",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
            "read_only": true,
        }
    }))
    .unwrap();
    assert!(stored.read_only, "lifecycle payload must carry read_only");
    state
        .dbx_connections
        .write()
        .await
        .insert("conn-ro".to_string(), stored);
    // Same endpoint re-dialed inline (no connectionId, host case-insensitive):
    // the read-only gate still applies to write tools — the error fires
    // before run_tool, so no localPath is needed to distinguish it from
    // a plain validation error.
    let error = state
        .call_tool(
            "sftp_upload",
            &json!({
                "host": "PROD.example.test",
                "port": 2222,
                "username": "deploy",
                "remotePath": "/tmp/x",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("read-only"),
        "expected read-only refusal for inline dial of a registered host, got: {error}"
    );
    // A different username or host is not covered by that registration.
    for identity in [
        json!({"host": "prod.example.test", "port": 2222, "username": "other"}),
        json!({"host": "other.example.test", "username": "deploy"}),
    ] {
        let error = state
            .call_tool("sftp_upload", &identity, None)
            .await
            .unwrap_err();
        assert!(
            !error.contains("read-only"),
            "expected non-matching identity to pass the gate, got: {error}"
        );
    }
}

#[tokio::test]
async fn sudo_allowlist_gates_privileged_tools() {
    let state = state();
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-sudo-list",
            "host": "web.example.test",
            "port": 22,
            "username": "deploy",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
            "external_config": {
                "sudo_whitelist": "systemctl restart nginx\ndocker restart *"
            },
        }
    }))
    .unwrap();
    assert_eq!(stored.sudo_whitelist.len(), 2, "external_config must parse");
    state
        .dbx_connections
        .write()
        .await
        .insert("conn-sudo-list".to_string(), stored);

    // ssh_exec_sudo with an unlisted command: refused, patterns listed.
    let error = state
        .call_tool(
            "ssh_exec_sudo",
            &json!({
                "connectionId": "conn-sudo-list",
                "command": "systemctl restart mysql",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("not allowed by this connection's whitelist")
            && error.contains("docker restart *"),
        "expected allowlist refusal, got: {error}"
    );

    // A matching command passes the gate and fails later (no dial here).
    let error = state
        .call_tool(
            "ssh_exec_sudo",
            &json!({
                "connectionId": "conn-sudo-list",
                "command": "docker restart api",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("whitelist"),
        "expected matching command to pass the gate, got: {error}"
    );

    // Inline `sudo …` inside plain ssh_exec is gated too.
    let error = state
        .call_tool(
            "ssh_exec",
            &json!({
                "connectionId": "conn-sudo-list",
                "command": "sudo systemctl restart mysql",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("not allowed by this connection's whitelist"),
        "expected inline-sudo refusal, got: {error}"
    );

    // Inline identity fallback: re-dialing the same host without a
    // connectionId inherits the whitelist; a different host does not.
    let error = state
        .call_tool(
            "ssh_exec_sudo",
            &json!({
                "host": "web.example.test",
                "username": "deploy",
                "command": "reboot",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("not allowed by this connection's whitelist"),
        "expected identity-fallback refusal, got: {error}"
    );
    let error = state
        .call_tool(
            "ssh_exec_sudo",
            &json!({
                "host": "other.example.test",
                "username": "deploy",
                "command": "reboot",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("whitelist"),
        "expected other host to skip the allowlist, got: {error}"
    );
}

#[tokio::test]
async fn read_only_tools_respect_sensitive_path_denylist() {
    let mut read_only_state = state();
    read_only_state.global_read_only = true;
    for (name, arguments) in [
        (
            "sftp_read_file",
            json!({"host": "h.test", "username": "op", "path": "/root/.ssh/id_rsa"}),
        ),
        (
            "sftp_list_dir",
            json!({"host": "h.test", "username": "op", "path": "/root/.ssh"}),
        ),
        (
            "sftp_download",
            json!({"host": "h.test", "username": "op", "remotePath": "/etc/shadow",
                   "localPath": "/tmp/dbx-denylist-test"}),
        ),
        (
            "ssh_task_status",
            json!({"host": "h.test", "username": "op", "logPath": "/root/.bash_history"}),
        ),
    ] {
        let error = read_only_state
            .call_tool(name, &arguments, None)
            .await
            .unwrap_err();
        assert!(
            error.contains("sensitive"),
            "expected sensitive-path refusal for {name}, got: {error}"
        );
    }
    // Ordinary inspection paths still pass the gate (they fail later, at
    // the dial).
    let error = read_only_state
        .call_tool(
            "sftp_read_file",
            &json!({"host": "h.test", "username": "op", "path": "/var/log/app.log"}),
            None,
        )
        .await
        .unwrap_err();
    assert!(!error.contains("sensitive"), "got: {error}");
    // On normal connections the denylist does not apply — the operator
    // already grants full access.
    let normal_state = state();
    let error = normal_state
        .call_tool(
            "sftp_read_file",
            &json!({"host": "h.test", "username": "op", "path": "/root/.ssh/id_rsa"}),
            None,
        )
        .await
        .unwrap_err();
    assert!(!error.contains("sensitive"), "got: {error}");
}

#[tokio::test]
async fn exec_whitelist_refuses_sensitive_paths_on_read_only() {
    let mut state = state();
    state.global_read_only = true;
    let error = state
        .call_tool(
            "ssh_exec",
            &json!({
                "host": "h.test",
                "username": "op",
                "command": "cat /root/.ssh/id_rsa",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("not recognized"),
        "expected whitelist refusal for a sensitive path, got: {error}"
    );
}

#[tokio::test]
async fn sftp_download_refuses_sensitive_local_targets() {
    let state = state();
    for local in [
        "/home/u/.bashrc",
        "/home/u/.ssh/authorized_keys",
        "/etc/cron.d/payload",
        "/Library/LaunchDaemons/com.example.payload.plist",
    ] {
        let error = state
            .call_tool(
                "sftp_download",
                &json!({
                    "host": "h.test",
                    "username": "op",
                    "remotePath": "/tmp/payload.sh",
                    "localPath": local,
                }),
                None,
            )
            .await
            .unwrap_err();
        assert!(
            error.contains("Refusing to write the local sensitive path"),
            "expected local denylist refusal for {local}, got: {error}"
        );
    }
    // Ordinary destinations pass the denylist (they fail later, at the
    // dial).
    let error = state
        .call_tool(
            "sftp_download",
            &json!({
                "host": "h.test",
                "username": "op",
                "remotePath": "/tmp/payload.sh",
                "localPath": "/tmp/dbx-download-ok/payload.sh",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("Refusing to write"),
        "expected plain target to pass the denylist, got: {error}"
    );
}

#[test]
fn connection_tools_declare_connection_id() {
    // Strict MCP hosts drop arguments the input schema does not declare,
    // so connection selectors must be advertised for both stdio routing
    // and saved-connection reuse. The selector anyOf is checked below so
    // strict clients can call with id, name, or endpoint fields.
    let tools = tool_definitions();
    let array = tools.as_array().unwrap();
    for name in [
        "ssh_exec",
        "ssh_exec_sudo",
        "ssh_run_bg",
        "ssh_terminal_input",
        "ssh_task_status",
        "sftp_list_dir",
        "sftp_upload",
    ] {
        let properties = array
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("tool {name} missing from definitions"))["inputSchema"]
            ["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("tool {name} schema has no properties object"));
        for key in ["connectionId", "connectionName"] {
            let schema = properties
                .get(key)
                .unwrap_or_else(|| panic!("tool {name} schema does not declare {key}"));
            assert_eq!(schema["type"], "string", "tool {name} {key} type");
        }
        assert!(
            array.iter().find(|tool| tool["name"] == name).unwrap()["inputSchema"]["anyOf"]
                .as_array()
                .is_some(),
            "tool {name} must accept a saved id/name or endpoint"
        );
    }
}

// —— ssh_terminal_input 门矩阵（IMPL_PLAN_NETCATTY A2-T5）———

#[test]
fn terminal_input_gate_read_only_only_allows_control_sequences() {
    // 纯控制序列（回车/Ctrl+C）在只读连接放行。
    assert!(terminal_input_gate("\r", true, false, &[]).is_ok());
    assert!(terminal_input_gate("\u{3}", true, false, &[]).is_ok());
    assert!(terminal_input_gate("", true, false, &[]).is_ok());
    // 任何可见文本（哪怕只读命令）都拒绝：终端注入不在只读白名单模型内。
    let error = terminal_input_gate("echo hi\r", true, false, &[]).unwrap_err();
    assert!(error.contains("read-only"), "{error}");
}

#[test]
fn terminal_input_gate_destructive_lines_need_confirmation() {
    // 灾难行（递归删除系统根）：无确认拒绝，确认放行（可写连接）。
    // 非灾难目标的 rm -rf /tmp/x 只是 Unknown 写操作，不触发本门。
    let error = terminal_input_gate("rm -rf /\r", false, false, &[]).unwrap_err();
    assert!(error.contains("destructive"), "{error}");
    assert!(terminal_input_gate("rm -rf /\r", false, true, &[]).is_ok());
    // 只读连接上灾难行即使带确认也拒绝（与 ssh_exec 一致）。
    let error = terminal_input_gate("rm -rf /\r", true, true, &[]).unwrap_err();
    assert!(error.contains("read-only"), "{error}");
    // 多行取最坏：干净行 + 灾难行整体拒绝。
    let error = terminal_input_gate("df -h\rrm -rf /\r", false, false, &[]).unwrap_err();
    assert!(error.contains("destructive"), "{error}");
}

#[test]
fn terminal_input_gate_sudo_lines_follow_the_allowlist() {
    // 连接声明了白名单：sudo 行必须命中条目。
    let allowlist = vec![vec![
        "systemctl".to_string(),
        "restart".to_string(),
        "*".to_string(),
    ]];
    let error = terminal_input_gate("sudo yum update -y\r", false, false, &allowlist).unwrap_err();
    assert!(error.contains("whitelist"), "{error}");
    assert!(
        terminal_input_gate("sudo systemctl restart nginx\r", false, false, &allowlist).is_ok()
    );
    // 未声明白名单的连接不做该门（行为与 ssh_exec 一致）。
    assert!(terminal_input_gate("sudo yum update -y\r", false, false, &[]).is_ok());
}

#[tokio::test]
async fn run_bg_requires_destructive_confirmation() {
    let state = state();
    let error = state
        .call_tool(
            "ssh_run_bg",
            &json!({
                "host": "example.test",
                "username": "op",
                "command": "rm -rf /"
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("destructive"),
        "expected destructive refusal, got: {error}"
    );
}

#[test]
fn pre_exec_transport_errors_are_retryable_post_exec_are_not() {
    assert!(exec::is_pre_exec_transport_error(
        "Failed to open exec channel: Disconnected"
    ));
    assert!(exec::is_pre_exec_transport_error(
        "Failed to start command: broken pipe"
    ));
    // Post-exec failures must NOT look retryable: the command may have
    // already run server-side.
    assert!(!exec::is_pre_exec_transport_error(
        "Timed out waiting for the remote command to finish."
    ));
    assert!(!exec::is_pre_exec_transport_error(
        "sudo exited 1: wrong password"
    ));
}

#[tokio::test]
async fn initialize_and_list_tools_follow_mcp_shape() {
    let state = state();
    let init = state
        .dispatch(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }))
        .await
        .unwrap();
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert!(init["result"]["capabilities"]["tools"].is_object());
    // 同族基线：serverInfo.name 用完整插件 id（files/ldap/kafka 同款）。
    assert_eq!(init["result"]["serverInfo"]["name"], "io.dbx.sshx");

    let list = state
        .dispatch(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .await
        .unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    for expected in [
        "ssh_exec",
        "ssh_exec_sudo",
        "ssh_run_bg",
        "ssh_terminal_input",
        "ssh_multi_exec",
        "ssh_task_status",
        "ssh_close",
        "ssh_list_known_hosts",
        "ssh_list_connections",
        "ssh_remove_known_host",
        "sftp_list_dir",
        "sftp_stat",
        "sftp_read_file",
        "sftp_write_file",
        "sftp_upload",
        "sftp_download",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
    assert!(tools
        .iter()
        .all(|tool| tool["inputSchema"]["type"] == "object"));

    // The discovery tool takes no arguments (same shape as
    // ssh_list_known_hosts) and points at the reference parameters.
    let discovery = tools
        .iter()
        .find(|t| t["name"] == "ssh_list_connections")
        .unwrap();
    assert!(
        discovery["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .is_empty(),
        "ssh_list_connections must take no arguments"
    );
    assert!(
        discovery["description"]
            .as_str()
            .unwrap()
            .contains("connectionId"),
        "description must explain how to use the entries"
    );

    // The read tool exposes the offset knob so MCP clients can page
    // through files; the metrics tool advertises the extended dimensions.
    let read = tools
        .iter()
        .find(|t| t["name"] == "sftp_read_file")
        .unwrap();
    let props = read["inputSchema"]["properties"].as_object().unwrap();
    assert!(props.contains_key("offset"), "sftp_read_file lacks offset");
    // Transfer tools expose the local/remote path pair and overwrite knob.
    for tool_name in ["sftp_upload", "sftp_download"] {
        let tool = tools.iter().find(|t| t["name"] == tool_name).unwrap();
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        assert!(
            props.contains_key("localPath"),
            "{tool_name} lacks localPath"
        );
        assert!(
            props.contains_key("remotePath"),
            "{tool_name} lacks remotePath"
        );
        assert!(
            props.contains_key("overwrite"),
            "{tool_name} lacks overwrite"
        );
    }
    let sudo = tools.iter().find(|t| t["name"] == "ssh_exec_sudo").unwrap();
    let sudo_props = sudo["inputSchema"]["properties"].as_object().unwrap();
    assert!(
        sudo_props.contains_key("quickSudoProfile"),
        "ssh_exec_sudo lacks quickSudoProfile"
    );
    // Both exec tools advertise the terminal routing knob.
    for tool_name in ["ssh_exec", "ssh_exec_sudo"] {
        let tool = tools.iter().find(|t| t["name"] == tool_name).unwrap();
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        assert!(
            props.contains_key("runInTerminal"),
            "{tool_name} lacks runInTerminal"
        );
    }
    // Notifications produce no response; unknown methods return the
    // standard JSON-RPC -32601 (Method not found), family-wide with
    // ldap/kafka/files.
    assert!(state
        .dispatch(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .await
        .is_none());
    let error = state
        .dispatch(json!({ "jsonrpc": "2.0", "id": 3, "method": "no/such" }))
        .await
        .unwrap();
    assert_eq!(error["error"]["code"], -32601);
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Method not found"));
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

async fn multi_exec_error(state: &McpState, arguments: Value) -> String {
    tool_error(state, "ssh_multi_exec", arguments).await
}

/// Round 7 (§3.3 live coverage input): the `mode` enum is case-sensitive
/// by design — invalid and wrong-case values must fail fast with the
/// full legal value list (matching the schema enum) before any dial or
/// saved-connection resolution, so the refusal has no side effects.
#[tokio::test]
async fn multi_exec_mode_enum_rejects_invalid_and_wrong_case() {
    let state = state();
    for mode in ["wrong", "PARALLEL", "Sequential"] {
        let message = multi_exec_error(
            &state,
            json!({ "targets": ["conn-1"], "command": "echo probe", "mode": mode }),
        )
        .await;
        assert!(
            message.contains(r#"mode must be "parallel" or "sequential""#)
                && message.contains(mode),
            "mode {mode}: {message}"
        );
    }
}

async fn tool_error(state: &McpState, name: &str, arguments: Value) -> String {
    String::from(
        state
            .dispatch(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": name, "arguments": arguments },
            }))
            .await
            .unwrap()["error"]["message"]
            .as_str()
            .unwrap_or_default(),
    )
}

#[tokio::test]
async fn tool_calls_validate_parameters_before_connecting() {
    let state = state();
    let missing = state
        .dispatch(json!({
            "jsonrpc": "2.0", "id": 9, "method": "tools/call",
            "params": { "name": "sftp_upload", "arguments": { "host": "example.com", "username": "u" } },
        }))
        .await
        .unwrap();
    let text = missing["error"]["message"].as_str().unwrap();
    assert!(text.contains("localPath"), "unexpected error: {text}");

    let no_password = state
        .dispatch(json!({
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": { "name": "ssh_exec", "arguments": { "host": "example.com", "username": "u", "command": "true" } },
        }))
        .await
        .unwrap();
    let message = no_password["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("password"), "unexpected error: {message}");
}

#[tokio::test]
async fn transfer_tools_validate_the_local_side_before_dialing() {
    let directory = tempfile::tempdir().unwrap();
    let state = McpState::new(directory.path().join("data"));
    let remote = json!({ "host": "203.0.113.1", "username": "u", "password": "p" });
    let merge = |mut base: Value, extra: Value| {
        for (key, value) in extra.as_object().unwrap() {
            base[key.as_str()] = value.clone();
        }
        base
    };

    // A missing local file fails before any connection attempt.
    let missing_file = state
        .run_tool(
            "sftp_upload",
            &merge(
                remote.clone(),
                json!({ "localPath": "/no/such/file.bin", "remotePath": "/tmp/x" }),
            ),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        missing_file.contains("Cannot read local file"),
        "unexpected error: {missing_file}"
    );

    // The upload cap follows the configured maxUploadBytes.
    let local_file = directory.path().join("payload.bin");
    std::fs::write(&local_file, vec![0u8; 64]).unwrap();
    state.settings_set(&json!({ "maxUploadBytes": 8 })).unwrap();
    let too_big = state
        .run_tool(
            "sftp_upload",
            &merge(
                remote.clone(),
                json!({ "localPath": local_file.display().to_string(), "remotePath": "/tmp/x" }),
            ),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        too_big.contains("upload limit"),
        "unexpected error: {too_big}"
    );

    // An existing local target refuses to be replaced without overwrite.
    let existing_target = directory.path().join("already-here.txt");
    std::fs::write(&existing_target, "keep").unwrap();
    let refused = state
        .run_tool(
            "sftp_download",
            &merge(
                remote,
                json!({
                    "remotePath": "/tmp/remote.txt",
                    "localPath": existing_target.display().to_string(),
                }),
            ),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        refused.contains("Local path already exists"),
        "unexpected error: {refused}"
    );
}

#[tokio::test]
async fn quick_sudo_profiles_roundtrip_without_echoing_secrets() {
    let dir = std::env::temp_dir().join(format!("dbx-mcp-profiles-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = McpState::new(dir.clone());
    // Test-only secret assembled at runtime (never a real credential).
    let secret = format!("test-{}", uuid::Uuid::new_v4());

    // MCP 工具面已收（#203 工具瘦身）；store 行为经 runtime 方法直接覆盖。
    let saved = state
        .runtime
        .profiles_save(&json!({
            "name": "ops",
            "sudoPassword": secret,
            "totpSecret": "JBSWY3DPEHPK3PXP",
            "authFlowMode": "password_plus_otp",
            "sudoUsePty": true,
        }))
        .await
        .unwrap();
    assert_eq!(saved["created"], true);
    assert_eq!(saved["profile"]["sudoPasswordSet"], true);
    assert_eq!(saved["profile"]["totpConfigured"], true);

    let listed = state.runtime.profiles_list();
    let rendered = listed.to_string();
    assert!(!rendered.contains(&secret), "secret leaked: {rendered}");
    assert!(rendered.contains("\"sudoPasswordSet\":true"));

    // Duplicate names are rejected, unknown references report clearly.
    let duplicate = state.runtime.profiles_save(&json!({ "name": "OPS" })).await;
    assert!(duplicate.unwrap_err().contains("already in use"));
    let missing = resolve_profile_reference(
        &sudo_profiles::load_store(&dir),
        &json!({ "quickSudoProfile": "ghost" }),
    );
    assert!(missing.unwrap_err().contains("not found"));

    let id = saved["profile"]["id"].as_str().unwrap().to_string();
    let removed = state.runtime.profiles_delete(&id).await.unwrap();
    assert_eq!(removed["removed"], true);
    let listed = state.runtime.profiles_list();
    assert_eq!(listed["profiles"].as_array().unwrap().len(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Builds a saved-connection payload with the given declared sudo source;
/// returns the connection plus its (runtime-composed) login password.
/// All credential values in tests are assembled at runtime — never real
/// credentials, never literals in source.
fn source_connection(source: &str, profile_ref: &str) -> (StoredConnection, String) {
    let login = format!("login-{}", uuid::Uuid::new_v4());
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-src",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": login,
            "external_config": {
                "sudo_source": source,
                "sudo_profile": profile_ref,
            },
        }
    }))
    .unwrap();
    (stored, login)
}

#[tokio::test]
async fn sudo_auth_resolution_follows_declared_source() {
    let dir = std::env::temp_dir().join(format!("dbx-mcp-sudo-src-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = McpState::new(dir.clone());
    let profile_secret = format!("profile-{}", uuid::Uuid::new_v4());
    let call_secret = format!("call-{}", uuid::Uuid::new_v4());

    let saved = state
        .runtime
        .profiles_save(&json!({
            "name": "src-ops",
            "sudoPassword": profile_secret,
            "totpSecret": "JBSWY3DPEHPK3PXP",
        }))
        .await
        .unwrap();
    assert_eq!(saved["created"], true);

    // Global source: the referenced global profile owns the credential
    // source (with TOTP), exactly like the workbench.
    let (stored, _) = source_connection("global", "src-ops");
    let auth = state
        .resolve_sudo_auth(&json!({}), None, Some(stored))
        .await
        .unwrap();
    assert_eq!(auth.password, profile_secret);
    assert!(!auth.totp_secrets.is_empty());

    // Explicit per-call arguments win over the declared source.
    let (stored, _) = source_connection("global", "src-ops");
    let auth = state
        .resolve_sudo_auth(&json!({ "sudoPassword": call_secret }), None, Some(stored))
        .await
        .unwrap();
    assert_eq!(auth.password, call_secret);

    // The per-call quickSudoProfile reference replaces the declared one
    // and is applied even without a saved connection (inline MCP calls).
    let explicit_profile = sudo_profiles::SudoProfile {
        id: "inline".to_string(),
        name: "inline".to_string(),
        sudo_password: call_secret.clone(),
        totp_secret: String::new(),
        auth_flow_mode: "password_only".to_string(),
        password_prompt_hint: String::new(),
        totp_prompt_hint: String::new(),
        sudo_use_pty: false,
        created_at: 0,
        updated_at: 0,
    };
    let (stored, _) = source_connection("global", "src-ops");
    let auth = state
        .resolve_sudo_auth(&json!({}), Some(explicit_profile.clone()), Some(stored))
        .await
        .unwrap();
    assert_eq!(auth.password, call_secret);
    assert!(auth.totp_secrets.is_empty());
    let auth = state
        .resolve_sudo_auth(&json!({}), Some(explicit_profile.clone()), None)
        .await
        .unwrap();
    assert_eq!(auth.password, call_secret);

    // Custom source without a binding: the connection's own sudo config
    // (empty here) degrades to the login password fallback.
    let (stored, login) = source_connection("custom", "");
    let auth = state
        .resolve_sudo_auth(&json!({}), None, Some(stored))
        .await
        .unwrap();
    assert_eq!(auth.password, login);

    // Off refuses like the workbench gate, unless the caller passes
    // explicit credentials.
    let (stored, _) = source_connection("off", "");
    let refused = state
        .resolve_sudo_auth(&json!({}), None, Some(stored))
        .await;
    assert!(refused.unwrap_err().contains("Quick Sudo is disabled"));
    let (stored, _) = source_connection("off", "");
    let auth = state
        .resolve_sudo_auth(&json!({ "sudoPassword": call_secret }), None, Some(stored))
        .await
        .unwrap();
    assert_eq!(auth.password, call_secret);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pool_ids_key_by_identity() {
    let id = connection_pool_id(&json!({ "host": "h", "port": 2222, "username": "u" }));
    assert_eq!(id, "mcp-u@h:2222");
    assert_eq!(
        connection_pool_id(&json!({ "host": "h", "username": "u" })),
        "mcp-u@h:22"
    );
}

#[test]
fn jump_hosts_parse_into_the_chain() {
    let connection = stored_connection_from_arguments(&json!({
        "host": "target", "username": "u", "password": "p",
        "jumpHosts": [
            { "host": "bastion", "port": 2222, "username": "ops", "password": "jp" },
            { "host": "inner", "username": "relay", "authentication": "private-key", "private_key_path": "/k" },
        ],
    }))
    .unwrap();
    assert_eq!(connection.jump_hosts.len(), 2);
    assert_eq!(connection.jump_hosts[0].port, 2222);
    assert_eq!(connection.jump_hosts[1].authentication, "private-key");

    let too_many = json!({
        "host": "t", "username": "u", "password": "p",
        "jumpHosts": [
            { "host": "a", "username": "u", "password": "p" },
            { "host": "b", "username": "u", "password": "p" },
            { "host": "c", "username": "u", "password": "p" },
            { "host": "d", "username": "u", "password": "p" },
        ],
    });
    assert!(stored_connection_from_arguments(&too_many)
        .err()
        .unwrap()
        .contains("At most 3"));

    let missing_password = json!({
        "host": "t", "username": "u", "password": "p",
        "jumpHosts": [{ "host": "bastion", "username": "ops" }],
    });
    assert!(stored_connection_from_arguments(&missing_password).is_err());
}

#[test]
fn connections_map_authentication_sensibly() {
    let by_key = stored_connection_from_arguments(&json!({
        "host": "h", "username": "u", "command": "x",
        "privateKeyPath": "/k", "totpSecret": "123456", "authFlowMode": "password_plus_otp"
    }))
    .unwrap();
    assert_eq!(by_key.authentication, AuthenticationMethod::PrivateKey);
    assert_eq!(by_key.totp_secret, "123456");
    assert!(stored_connection_from_arguments(&json!({ "host": "h", "username": "u" })).is_err());
}

/// MCP inline dials accept pasted key contents as the path alternative:
/// content-only selects private-key without touching the filesystem, and
/// a dial with neither path nor content is rejected up front.
#[test]
fn connections_accept_private_key_content() {
    let by_content = stored_connection_from_arguments(&json!({
        "host": "h", "username": "u", "command": "x",
        "privateKeyContent": "-----BEGIN OPENSSH PRIVATE KEY-----\nbody\n-----END OPENSSH PRIVATE KEY-----\n"
    }))
    .unwrap();
    assert_eq!(by_content.authentication, AuthenticationMethod::PrivateKey);
    assert!(by_content.private_key_path.is_empty());
    assert!(by_content.private_key.starts_with("-----BEGIN"));

    let explicit_password_mode = stored_connection_from_arguments(&json!({
        "host": "h", "username": "u", "command": "x",
        "authentication": "private-key-password",
        "privateKeyContent": "-----BEGIN OPENSSH PRIVATE KEY-----\nbody\n-----END OPENSSH PRIVATE KEY-----\n",
        "password": "login"
    }))
    .unwrap();
    assert_eq!(
        explicit_password_mode.authentication,
        AuthenticationMethod::PrivateKeyPassword
    );

    // 既有的推断语义保持不变：没有密钥材料时显式 private-key 仍按
    // 「有密码则回退 password」解析（内联拨号从不挂空路径的私钥）。
    let degraded = stored_connection_from_arguments(&json!({
        "host": "h", "username": "u", "command": "x",
        "authentication": "private-key", "password": "login"
    }))
    .unwrap();
    assert_eq!(degraded.authentication, AuthenticationMethod::Password);
    let neither = stored_connection_from_arguments(&json!({
        "host": "h", "username": "u", "command": "x",
        "authentication": "private-key"
    }))
    .unwrap_err();
    assert!(
        neither.contains("Password authentication requires a password"),
        "{neither}"
    );
}

#[test]
fn mcp_limits_default_and_clamp_into_ceilings() {
    let defaults = McpLimits::default();
    assert_eq!(defaults.max_read_bytes, 256 * 1024);
    assert_eq!(defaults.max_download_bytes, 1024 * 1024);
    assert_eq!(defaults.local_transfer_root, "");
    assert_eq!(
        McpLimits {
            max_read_bytes: 0,
            max_upload_bytes: u64::MAX,
            max_download_bytes: 0,
            local_transfer_root: String::new(),
        }
        .sanitized(),
        McpLimits {
            max_read_bytes: 1,
            max_upload_bytes: UPLOAD_LIMIT_CEILING,
            max_download_bytes: 1,
            local_transfer_root: String::new(),
        }
    );
    // Values inside the ceilings pass through untouched.
    let inside = McpLimits {
        max_read_bytes: 512 * 1024,
        max_upload_bytes: 64 * 1024 * 1024,
        max_download_bytes: 4 * 1024 * 1024,
        local_transfer_root: String::new(),
    };
    assert_eq!(inside.clone().sanitized(), inside);
}

#[cfg(windows)]
const ABS_TEST_ROOT: &str = "C:\\tmp\\transfers";
#[cfg(not(windows))]
const ABS_TEST_ROOT: &str = "/tmp/transfers";

#[test]
fn local_transfer_root_setting_accepts_absolute_and_empty_only() {
    assert_eq!(validated_transfer_root(&json!("")).unwrap(), "");
    assert_eq!(
        validated_transfer_root(&json!(ABS_TEST_ROOT)).unwrap(),
        ABS_TEST_ROOT
    );
    assert!(validated_transfer_root(&json!("relative/path")).is_err());
    assert!(validated_transfer_root(&json!(42)).is_err());
}

#[test]
fn local_transfer_settings_persist_and_roundtrip() {
    let dir = std::env::temp_dir().join(format!("mcp-limits-test-{}", std::process::id()));
    let path = dir.join("mcp-settings.json");
    let _ = std::fs::remove_file(&path);
    let defaults = McpLimits::load(&path);
    assert_eq!(defaults.local_transfer_root, "");
    let mut updated = defaults;
    updated.local_transfer_root = ABS_TEST_ROOT.to_string();
    write_settings_document(&path, &updated, &McpPermission::default()).unwrap();
    let reloaded = McpLimits::load(&path);
    assert_eq!(reloaded.local_transfer_root, ABS_TEST_ROOT);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn local_transfer_gate_confines_paths_to_allowed_roots() {
    let scratch = std::env::temp_dir().join(format!("mcp-gate-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let inside_dir = scratch.join("inside");
    std::fs::create_dir_all(&inside_dir).unwrap();
    std::fs::write(inside_dir.join("a.bin"), b"x").unwrap();
    let inside = std::fs::canonicalize(inside_dir.join("a.bin")).unwrap();
    let roots = vec![std::fs::canonicalize(&inside_dir).unwrap()];
    // Inside the only configured root passes…
    assert!(ensure_local_transfer_allowed_in(&roots, "/x", &inside).is_ok());
    // …anything else (even non-sensitive) is refused…
    let outside = std::fs::canonicalize("/usr").unwrap_or_else(|_| PathBuf::from("/usr"));
    let error = ensure_local_transfer_allowed_in(&roots, "/x", &outside).unwrap_err();
    assert!(error.contains("localTransferRoot"), "{error}");
    // …and the default-roots message names the setting to widen.
    let empty_roots: Vec<PathBuf> = vec![];
    let error = ensure_local_transfer_allowed_in(&empty_roots, "", &inside).unwrap_err();
    assert!(
        error.contains("outside the allowed transfer roots"),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn local_transfer_gate_blocklist_applies_inside_allowed_roots() {
    let scratch = std::env::temp_dir().join(format!("mcp-gate-bl-{}", std::process::id()));
    let secret_dir = scratch.join(".ssh");
    std::fs::create_dir_all(&secret_dir).unwrap();
    std::fs::write(secret_dir.join("id_rsa"), b"k").unwrap();
    let key = std::fs::canonicalize(secret_dir.join("id_rsa")).unwrap();
    let rc = std::fs::canonicalize(&scratch).unwrap().join(".bashrc");
    let roots = vec![std::fs::canonicalize(&scratch).unwrap()];
    assert!(ensure_local_transfer_allowed_in(&roots, "", &key).is_err());
    assert!(ensure_local_transfer_allowed_in(&roots, "", &rc).is_err());
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn local_transfer_roots_default_to_temp_and_data_dir() {
    let data_dir = std::env::temp_dir().join(format!("mcp-data-{}", std::process::id()));
    std::fs::create_dir_all(&data_dir).unwrap();
    let roots = local_transfer_roots_for("", &data_dir).unwrap();
    let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let data = std::fs::canonicalize(&data_dir).unwrap();
    assert!(roots.contains(&temp), "{roots:?}");
    assert!(roots.contains(&data), "{roots:?}");
    // A configured root narrows the policy to exactly that root…
    let roots = local_transfer_roots_for(data.to_str().unwrap(), &data_dir).unwrap();
    assert_eq!(roots, vec![data.clone()]);
    // …and an unresolvable configured root is an error, not a fallback.
    assert!(local_transfer_roots_for("/nonexistent/mcp-root", &data_dir).is_err());
    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn mcp_limits_load_falls_back_on_missing_or_corrupt_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mcp-settings.json");
    // Missing file and unparseable content both fall back to defaults.
    assert_eq!(McpLimits::load(&path), McpLimits::default());
    std::fs::write(&path, "not json at all {{{").unwrap();
    assert_eq!(McpLimits::load(&path), McpLimits::default());
    // Partial/garbage fields fall back per-field and out-of-range values
    // are clamped on load.
    std::fs::write(
        &path,
        serde_json::to_string(&json!({
            "maxReadBytes": "bogus",
            "maxUploadBytes": 32 * 1024 * 1024,
            "maxDownloadBytes": 999u64 * 1024 * 1024 * 1024,
        }))
        .unwrap(),
    )
    .unwrap();
    let loaded = McpLimits::load(&path);
    assert_eq!(loaded.max_read_bytes, McpLimits::default().max_read_bytes);
    assert_eq!(loaded.max_upload_bytes, 32 * 1024 * 1024);
    assert_eq!(loaded.max_download_bytes, DOWNLOAD_LIMIT_CEILING);
}

#[test]
fn mcp_limits_persist_roundtrip() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mcp-settings.json");
    let limits = McpLimits {
        max_read_bytes: 128 * 1024,
        max_upload_bytes: 8 * 1024 * 1024,
        max_download_bytes: 2 * 1024 * 1024,
        local_transfer_root: String::new(),
    };
    write_settings_document(&path, &limits, &McpPermission::default()).unwrap();
    assert_eq!(McpLimits::load(&path), limits);
    // A fresh state over the same data dir picks the persisted values up
    // (this is how the --mcp stdio process shares the settings).
    let state = McpState::new(directory.path().to_path_buf());
    // settings_get carries the §1.3 permission keys too; the limit
    // fields must round-trip unchanged.
    let view = state.settings_get();
    let expected = limits.to_json();
    assert_eq!(view["maxReadBytes"], expected["maxReadBytes"]);
    assert_eq!(view["maxUploadBytes"], expected["maxUploadBytes"]);
    assert_eq!(view["maxDownloadBytes"], expected["maxDownloadBytes"]);
    assert_eq!(view["localTransferRoot"], expected["localTransferRoot"]);
}

#[test]
fn mcp_settings_set_validates_and_persists_partial_updates() {
    let directory = tempfile::tempdir().unwrap();
    let state = McpState::new(directory.path().to_path_buf());

    // Partial update: only the named field changes.
    let updated = state
        .settings_set(&json!({ "maxReadBytes": 64 * 1024 }))
        .unwrap();
    assert_eq!(updated["maxReadBytes"], 64 * 1024);
    assert_eq!(
        updated["maxUploadBytes"],
        McpLimits::default().max_upload_bytes
    );
    assert_eq!(state.settings_get(), updated);

    // Invalid values are rejected without touching the stored settings.
    for bad in [
        json!({ "maxReadBytes": 0 }),
        json!({ "maxReadBytes": READ_LIMIT_CEILING + 1 }),
        json!({ "maxUploadBytes": "big" }),
        json!({ "maxDownloadBytes": -1 }),
        json!({ "localTransferRoot": "relative/root" }),
        json!({ "localTransferRoot": 7 }),
    ] {
        assert!(
            state.settings_set(&bad).is_err(),
            "expected error for {bad}"
        );
    }
    assert_eq!(
        state.settings_get()["maxReadBytes"],
        64 * 1024,
        "rejected updates must not change the settings"
    );

    // The transfer root accepts an absolute path and clears on empty.
    let with_root = state
        .settings_set(&json!({ "localTransferRoot": ABS_TEST_ROOT }))
        .unwrap();
    assert_eq!(with_root["localTransferRoot"], ABS_TEST_ROOT);
    let cleared = state
        .settings_set(&json!({ "localTransferRoot": "" }))
        .unwrap();
    assert_eq!(cleared["localTransferRoot"], "");

    // The soft ceilings themselves are accepted.
    let ceilings = state
        .settings_set(&json!({
            "maxReadBytes": READ_LIMIT_CEILING,
            "maxUploadBytes": UPLOAD_LIMIT_CEILING,
            "maxDownloadBytes": DOWNLOAD_LIMIT_CEILING,
        }))
        .unwrap();
    assert_eq!(ceilings["maxReadBytes"], READ_LIMIT_CEILING);

    // Persistence: a second state over the same directory reloads the
    // last accepted values.
    let reloaded = McpState::new(directory.path().to_path_buf());
    assert_eq!(reloaded.settings_get(), ceilings);
}

#[tokio::test]
async fn destructive_commands_require_explicit_confirmation() {
    let state = state();
    // Refused without the flag; the refusal happens before credential
    // validation (no "password" complaint), proving the gate ordering.
    let refused = state
        .dispatch(json!({
            "jsonrpc": "2.0", "id": 30, "method": "tools/call",
            "params": { "name": "ssh_exec", "arguments": {
                "host": "203.0.113.1", "username": "u",
                "command": "mkfs.ext4 /dev/sda1",
            }},
        }))
        .await
        .unwrap();
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("confirmDestructive"),
        "unexpected: {message}"
    );
    assert!(
        !message.contains("password"),
        "gate must fire first: {message}"
    );

    // With the flag the gate passes and the call proceeds to parameter
    // validation (missing password), proving it was not blocked.
    let gated_through = state
        .dispatch(json!({
            "jsonrpc": "2.0", "id": 31, "method": "tools/call",
            "params": { "name": "ssh_exec", "arguments": {
                "host": "203.0.113.1", "username": "u",
                "command": "mkfs.ext4 /dev/sda1",
                "confirmDestructive": true,
            }},
        }))
        .await
        .unwrap();
    let message = gated_through["error"]["message"].as_str().unwrap();
    assert!(message.contains("password"), "unexpected: {message}");

    // The same gate covers the sudo tool, and pipelines leak through
    // chain segments.
    for tool in ["ssh_exec_sudo"] {
        let chained = state
            .dispatch(json!({
                "jsonrpc": "2.0", "id": 32, "method": "tools/call",
                "params": { "name": tool, "arguments": {
                    "host": "203.0.113.1", "username": "u",
                    "command": "uptime && shutdown -h now",
                }},
            }))
            .await
            .unwrap();
        let message = chained["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("confirmDestructive"),
            "unexpected: {message}"
        );
    }
}

/// Reliability round 5: sudo-prefixed and substitution-hidden
/// destructive commands demand `confirmDestructive` at the tool gate
/// too (they used to slip through as plain Unknown and run unconfirmed
/// on writable connections), while ordinary commands still pass the
/// gate through to the credential check.
#[tokio::test]
async fn sudo_and_substitution_destructive_variants_demand_confirmation() {
    let state = state();
    for command in [
        "sudo rm -rf /",
        "sudo -u root rm -rf /etc",
        "sudo sh -c 'rm -rf /etc'",
        "sh -c 'mkfs.ext4 /dev/sda1'",
        "echo $(rm -rf /)",
        "echo `shutdown -h now`",
    ] {
        let refused = state
            .call_tool(
                "ssh_exec",
                &json!({ "host": "203.0.113.1", "username": "u", "command": command }),
                None,
            )
            .await
            .unwrap_err();
        assert!(
            refused.contains("confirmDestructive"),
            "{command}: {refused}"
        );
        assert!(!refused.contains("password"), "{command}: {refused}");
    }
    // The gate did not over-fire: a benign command proceeds past it.
    let through = state
        .call_tool(
            "ssh_exec",
            &json!({ "host": "203.0.113.1", "username": "u", "command": "echo hi" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(through.contains("password"), "{through}");
}

/// Reliability round 5: the local download/denylist gate holds across
/// double-slash and `./` path aliasing.
#[test]
fn sensitive_local_path_gate_survives_path_shapes() {
    for path in [
        "/etc//cron.d/payload",
        "/etc/./cron.d/payload",
        "/etc/./profile",
        "/home/u//.ssh/authorized_keys",
        "/var/spool//cron/x",
        "/Library/./LaunchDaemons/evil.plist",
    ] {
        assert!(
            is_sensitive_local_path(path),
            "expected '{path}' to be a sensitive local path"
        );
    }
    for path in [
        "/tmp/ok.bin",
        "/home/u/project/cron-doc.txt",
        "/home/u/ssh-config-backup", // prefix must match components, not substrings
    ] {
        assert!(
            !is_sensitive_local_path(path),
            "expected '{path}' NOT to be a sensitive local path"
        );
    }
}

// —— 第五轮：会话/存储 churn 可靠性 ————————————————

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

/// Reliability round 5 (churn): 500 settings write cycles must keep the
/// persisted document bounded (fixed shape, no accumulation files) and
/// every newly-set value immediately visible (no stale-read staleness);
/// clamp semantics keep working across the whole churn.
#[tokio::test]
async fn settings_storage_churn_stays_bounded() {
    let state = state();
    state
        .settings_set(&json!({ "execPermissionMode": "autonomous" }))
        .expect("initial settings write");
    let baseline_files = std::fs::read_dir(state.limits_path.parent().unwrap())
        .expect("data dir")
        .count();
    for index in 0..500 {
        let mode = if index % 2 == 0 {
            "confirm"
        } else {
            "autonomous"
        };
        let upload: u64 = if index % 3 == 0 { 1024 } else { 2048 };
        state
            .settings_set(&json!({
                "execPermissionMode": mode,
                "maxUploadBytes": upload,
            }))
            .unwrap_or_else(|error| panic!("settings_set failed at {index}: {error}"));
        let settings = state.settings_get();
        assert_eq!(
            settings["execPermissionMode"],
            json!(mode),
            "mode not visible immediately at {index}"
        );
        assert_eq!(settings["maxUploadBytes"], json!(upload));
    }
    // The store stays a fixed-shape document: file size bounded, no new
    // files appeared next to it (no per-write accumulation).
    let metadata = std::fs::metadata(&state.limits_path).expect("settings file");
    assert!(
        metadata.len() < 4 * 1024,
        "settings document grew to {} bytes",
        metadata.len()
    );
    let after_files = std::fs::read_dir(state.limits_path.parent().unwrap())
        .expect("data dir")
        .count();
    assert_eq!(after_files, baseline_files, "data dir accumulated files");
    // Clamp semantics survive churn: over-ceiling values never persist.
    state
        .settings_set(&json!({ "maxUploadBytes": u64::MAX }))
        .expect_err("over-ceiling upload cap must be rejected");
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

/// L2: list views expose metadata plus credential PRESENCE flags only —
/// the runtime-composed secret never appears in any branch, and the
/// degraded branch carries the start/upgrade note.
#[test]
fn connection_list_views_never_echo_credentials() {
    let secret = format!("pw-{}", uuid::Uuid::new_v4());
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-list",
            "name": "Web",
            "host": "web.example.test",
            "port": 2222,
            "username": "deploy",
            "password": secret,
            "read_only": true,
        }
    }))
    .unwrap();
    assert_eq!(stored.name.as_deref(), Some("Web"));

    // Degraded branch: registry alone + note.
    let degraded = connection_list_result(
        Err("bridge down".to_string()),
        std::slice::from_ref(&stored),
        &ConnectionScope::Unrestricted,
    );
    assert_eq!(degraded["source"], "session-registry");
    assert!(
        degraded["note"]
            .as_str()
            .unwrap()
            .contains("upgrade the DBX app"),
        "expected the upgrade note: {degraded}"
    );
    let entry = &degraded["connections"][0];
    assert_eq!(entry["id"], "conn-list");
    assert_eq!(entry["name"], "Web");
    assert_eq!(entry["host"], "web.example.test");
    assert_eq!(entry["port"], 2222);
    assert_eq!(entry["username"], "deploy");
    assert_eq!(entry["authentication"], "password");
    assert_eq!(entry["readOnly"], true);
    assert_eq!(entry["passwordSet"], true);
    let rendered = degraded.to_string();
    assert!(!rendered.contains(&secret), "secret leaked: {rendered}");
    assert!(
        !rendered.contains("\"password\":"),
        "raw credential field leaked: {rendered}"
    );

    // Bridge branch: bridge data wins by id, registry-only ids append,
    // and no note is attached.
    let bridge_entry = json!({
        "id": "conn-list", "name": "Web", "host": "web.example.test",
        "port": 2222, "username": "deploy", "authentication": "password",
        "readOnly": false,
    });
    let extra = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-extra", "host": "db.example.test", "port": 22,
            "username": "ops", "password": format!("pw-{}", uuid::Uuid::new_v4()),
        }
    }))
    .unwrap();
    let merged = connection_list_result(
        Ok(vec![bridge_entry]),
        &[stored, extra],
        &ConnectionScope::Unrestricted,
    );
    assert_eq!(merged["source"], "dbx-app-bridge");
    assert!(merged.get("note").is_none(), "bridge hit must not degrade");
    let entries = merged["connections"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "bridge entry + registry-only append");
    assert_eq!(entries[0]["id"], "conn-list");
    // Bridge entries carry no credential flags (contract: absent means
    // the app did not send one).
    assert!(entries[0].get("passwordSet").is_none());
    assert_eq!(entries[1]["id"], "conn-extra");
}

/// stdio selector→id resolution over the bridge list: unique hit,
/// ambiguous (both candidates listed with host), and missing. The
/// endpoint selector disambiguates a duplicate name.
#[test]
fn resolve_connection_in_bridge_list_matches_name_and_endpoint() {
    let entries = vec![
        json!({ "id": "a", "name": "Web", "host": "h1", "port": 22, "username": "ops" }),
        // Trailing space in the payload name still matches after trim.
        json!({ "id": "b", "name": "Web ", "host": "h2", "port": 2222, "username": "deploy" }),
        json!({ "id": "c", "name": "DB", "host": "h3", "port": 22, "username": "ops" }),
    ];
    assert_eq!(
        resolve_connection_in_bridge_list(&entries, &json!({ "connectionName": "DB" })).unwrap(),
        "c"
    );
    let error = resolve_connection_in_bridge_list(&entries, &json!({ "connectionName": "Web" }))
        .unwrap_err();
    assert!(
        error.contains("ambiguous")
            && error.contains("a")
            && error.contains("b")
            && error.contains("h1")
            && error.contains("h2"),
        "expected candidates in the error: {error}"
    );
    // Endpoint narrows the duplicate name to exactly one candidate.
    assert_eq!(
        resolve_connection_in_bridge_list(
            &entries,
            &json!({ "connectionName": "Web", "host": "h2", "port": 2222, "username": "deploy" })
        )
        .unwrap(),
        "b"
    );
    // A complete endpoint alone reuses the saved connection.
    assert_eq!(
        resolve_connection_in_bridge_list(
            &entries,
            &json!({ "host": "H2", "port": 2222, "username": "deploy" })
        )
        .unwrap(),
        "b"
    );
    assert!(
        resolve_connection_in_bridge_list(&entries, &json!({ "connectionName": "ghost" }))
            .unwrap_err()
            .contains("No saved connection matched"),
        "missing selector must be a clear error"
    );
}

/// L3: lifecycle payloads carry the display name through; absent names
/// stay None (older hosts, inline dials).
#[test]
fn lifecycle_name_roundtrip_and_optional_default() {
    let named = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "c1", "name": "  Bastion  ", "host": "h", "port": 22,
            "username": "u", "password": "p",
        }
    }))
    .unwrap();
    assert_eq!(named.name.as_deref(), Some("Bastion"));
    let unnamed = StoredConnection::from_lifecycle_params(&json!({
        "connection": { "id": "c2", "host": "h", "port": 22, "username": "u", "password": "p" }
    }))
    .unwrap();
    assert_eq!(unnamed.name, None);
}

/// L3: two same-named registry entries make the reference ambiguous —
/// the error lists every candidate id + host so the caller can
/// disambiguate by id.
#[tokio::test]
async fn ambiguous_connection_name_lists_candidates() {
    let state = state();
    for (id, host) in [("conn-x", "10.0.0.1"), ("conn-y", "10.0.0.2")] {
        let stored = StoredConnection::from_lifecycle_params(&json!({
            "connection": {
                "id": id, "name": "Twin", "host": host, "port": 22,
                "username": "ops",
                "password": format!("pw-{}", uuid::Uuid::new_v4()),
            }
        }))
        .unwrap();
        state
            .dbx_connections
            .write()
            .await
            .insert(id.to_string(), stored);
    }
    let error = state
        .call_tool(
            "sftp_stat",
            &json!({ "connectionName": "Twin", "path": "/tmp" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("ambiguous")
            && error.contains("conn-x")
            && error.contains("conn-y")
            && error.contains("10.0.0.1")
            && error.contains("10.0.0.2"),
        "expected every candidate in the error: {error}"
    );
}

#[tokio::test]
async fn endpoint_identity_resolves_a_unique_saved_connection_without_id() {
    let state = state();
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-endpoint",
            "name": "Prod bastion",
            "host": "prod.example.test",
            "port": 2222,
            "username": "deploy",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
        }
    }))
    .unwrap();
    state
        .dbx_connections
        .write()
        .await
        .insert(stored.id.clone(), stored);

    let resolved = state
        .registered_connection_by_ref(&json!({
            "host": "PROD.example.test",
            "port": 2222,
            "username": "deploy",
        }))
        .await
        .unwrap()
        .expect("endpoint should resolve the saved connection");
    assert_eq!(resolved.id, "conn-endpoint");
}

#[tokio::test]
async fn connection_name_uses_endpoint_to_disambiguate_candidates() {
    let state = state();
    for (id, host) in [("conn-x", "10.0.0.1"), ("conn-y", "10.0.0.2")] {
        let stored = StoredConnection::from_lifecycle_params(&json!({
            "connection": {
                "id": id,
                "name": "Twin",
                "host": host,
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
            .insert(id.to_string(), stored);
    }

    let resolved = state
        .registered_connection_by_ref(&json!({
            "connectionName": "Twin",
            "host": "10.0.0.2",
            "port": 22,
            "username": "ops",
        }))
        .await
        .unwrap()
        .expect("name plus endpoint should resolve one candidate");
    assert_eq!(resolved.id, "conn-y");
}

#[tokio::test]
async fn conflicting_connection_selectors_are_rejected() {
    let state = state();
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-prod",
            "name": "Prod",
            "host": "prod.example.test",
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
        .insert(stored.id.clone(), stored);

    let error = state
        .registered_connection_by_ref(&json!({
            "connectionId": "conn-prod",
            "host": "other.example.test",
            "username": "ops",
        }))
        .await
        .unwrap_err();
    assert!(
        error.contains("does not match"),
        "unexpected error: {error}"
    );
}

/// Saved-connection addressing must reach the dialers: an sftp/exec call
/// referencing a registered connection attempts the dial (a refused-port
/// error) instead of dying on the pre-fix pool lookup.
#[tokio::test]
async fn sftp_and_exec_resolve_saved_reference_before_dialing() {
    let state = state();
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-prod",
            "name": "Prod",
            "host": "127.0.0.1",
            "port": 1,
            "username": "ops",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
        }
    }))
    .unwrap();
    state
        .dbx_connections
        .write()
        .await
        .insert(stored.id.clone(), stored);

    for (tool, arguments) in [
        (
            "sftp_stat",
            json!({ "connectionName": "Prod", "path": "/tmp" }),
        ),
        (
            "ssh_exec",
            json!({ "connectionName": "Prod", "command": "true" }),
        ),
    ] {
        let error = state.call_tool(tool, &arguments, None).await.unwrap_err();
        assert!(
            !error.contains("Connection is not established")
                && !error.contains("Missing required parameter"),
            "{tool} bypassed saved-connection resolution: {error}"
        );
    }
}

/// The MCP tool surface must expose the alert triage playbook: the
/// stdio smoke asserts the tool by name, so a dispatch-only protocol
/// method (`ssh/alert/triage`) without an MCP tool would regress the
/// intent-recognition surface. Offline: no connection I/O involved.

#[tokio::test]
async fn read_only_connection_gates_writes_and_unknown_commands() {
    let state = McpState::shared(Arc::new(SshRuntime::new(
        std::env::temp_dir().join("dbx-mcp-readonly-test"),
    )));
    let lifecycle = json!({
        "connection": {
            "id": "conn-readonly",
            "name": "Prod bastion",
            "host": "192.0.2.10",
            "port": 22,
            "username": "ops",
            "password": "secret",
            "external_config": { "authentication": "password", "read_only": true },
        },
        "runtime": { "host": "192.0.2.10", "port": 22 },
        "operationId": "op-ro",
    });

    // Write-class tools are rejected on sight.
    for tool in ["sftp_write_file", "ssh_exec_sudo"] {
        let error = state
            .call_dbx_with(
                &json!({
                    "tool": tool,
                    "arguments": { "command": "uptime", "path": "/tmp/x", "content": "y" },
                    "lifecycle": lifecycle,
                }),
                None,
            )
            .await
            .expect_err("write tool must be refused on a read-only connection");
        assert!(error.contains("read-only"), "unexpected: {error}");
    }

    // ssh_exec survives only for provably read-only commands: an
    // unrecognized mutating command is refused by the whitelist (before
    // any dialing), while a destructive pattern is refused outright.
    let unknown = state
        .call_dbx_with(
            &json!({
                "tool": "ssh_exec",
                "arguments": { "command": "systemctl restart nginx" },
                "lifecycle": lifecycle,
            }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(unknown.contains("not recognized"), "unexpected: {unknown}");

    let destructive = state
        .call_dbx_with(
            &json!({
                "tool": "ssh_exec",
                "arguments": { "command": "rm -rf /etc" },
                "lifecycle": lifecycle,
            }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        destructive.contains("Refused on read-only"),
        "unexpected: {destructive}"
    );

    // confirmDestructive cannot override a read-only connection.
    let confirmed = state
        .call_dbx_with(
            &json!({
                "tool": "ssh_exec",
                "arguments": { "command": "rm -rf /etc", "confirmDestructive": true },
                "lifecycle": lifecycle,
            }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        confirmed.contains("Refused on read-only"),
        "unexpected: {confirmed}"
    );
}

// —— MCP 容错性专项（2026-09-13 测试覆盖审计）———

#[test]
fn argument_type_tolerant_parsers_accept_numeric_and_boolean_strings() {
    let arguments = json!({ "port": "2222", "timeoutSecs": 90, "tail": " 4000 " });
    assert_eq!(arg_u64(&arguments, "port").unwrap(), Some(2222));
    assert_eq!(arg_u64(&arguments, "timeoutSecs").unwrap(), Some(90));
    assert_eq!(arg_u64(&arguments, "tail").unwrap(), Some(4000));
    // 缺省与 null 都是 None；非法值给可行动错误而不是静默回默认。
    assert_eq!(arg_u64(&json!({}), "port").unwrap(), None);
    assert_eq!(arg_u64(&json!({ "port": null }), "port").unwrap(), None);
    let error = arg_u64(&json!({ "port": "abc" }), "port").unwrap_err();
    assert!(
        error.contains("port must be an integer") && error.contains("abc"),
        "{error}"
    );
    assert_eq!(arg_port(&json!({})).unwrap(), 22);
    assert_eq!(arg_port(&json!({ "port": "22" })).unwrap(), 22);
    assert!(arg_port(&json!({ "port": 0 }))
        .unwrap_err()
        .contains("between 1 and 65535"));
    assert!(arg_port(&json!({ "port": 99999 }))
        .unwrap_err()
        .contains("between 1 and 65535"));
    // 布尔字符串变体："true"/"yes"/"1" 为真，"off"/"0" 为假。
    for (value, expected) in [
        ("true", true),
        ("TRUE", true),
        (" yes ", true),
        ("1", true),
        ("false", false),
        ("Off", false),
        ("0", false),
    ] {
        assert_eq!(
            arg_bool(&json!({ "flag": value }), "flag").unwrap(),
            Some(expected),
            "{value}"
        );
    }
    assert_eq!(arg_bool(&json!({}), "flag").unwrap(), None);
    assert!(arg_bool(&json!({ "flag": "maybe" }), "flag")
        .unwrap_err()
        .contains("must be a boolean"));
}

#[test]
fn tool_names_constant_matches_tool_definitions() {
    let tools = tool_definitions();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), TOOL_NAMES.len(), "tool count drifted");
    for (declared, constant) in names.iter().zip(TOOL_NAMES.iter()) {
        assert_eq!(declared, constant, "tool order/name mismatch");
    }
}

#[tokio::test]
async fn unknown_tool_names_get_actionable_errors() {
    let state = state();
    // 分隔符/大小写变体给出 did-you-mean。
    let error = state
        .call_tool("sftp-listdir", &json!({}), None)
        .await
        .unwrap_err();
    assert!(
        error.contains("Unknown tool") && error.contains("Did you mean 'sftp_list_dir'?"),
        "{error}"
    );
    // 大小写变体精确命中。
    let error = state
        .call_tool("SSH_EXEC", &json!({}), None)
        .await
        .unwrap_err();
    assert!(error.contains("Did you mean 'ssh_exec'?"), "{error}");
    // 完全无关的名字指向 tools/list。
    let error = state
        .call_tool("database_query", &json!({}), None)
        .await
        .unwrap_err();
    assert!(
        error.contains("Unknown tool") && error.contains("tools/list"),
        "{error}"
    );
    // 未注册的 tool 名是 tools/call 之内的工具级错误，仍是 -32000
    // （传输层未知 method 才用标准 -32601，见 dispatch 注释）。
    let response = state
        .dispatch(json!({
            "jsonrpc": "2.0", "id": 41, "method": "tools/call",
            "params": { "name": "nope", "arguments": {} },
        }))
        .await
        .unwrap();
    assert_eq!(response["error"]["code"], -32000);
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Unknown tool"));
}

#[tokio::test]
async fn malformed_ports_fail_fast_and_numeric_strings_parse() {
    let state = state();
    // 非法端口在一切门/拨号之前报范围错误（此前会静默回退 22 端口）。
    for bad_port in [json!(0), json!(99999), json!("0"), json!("abc"), json!(1.5)] {
        let error = state
            .call_tool(
                "ssh_exec",
                &json!({ "host": "h.test", "username": "op", "command": "true", "port": bad_port }),
                None,
            )
            .await
            .unwrap_err();
        assert!(
            error.contains("port must be"),
            "expected port error for {bad_port}, got: {error}"
        );
    }
    // 合法端口的数字字符串继续走完校验链（在缺凭据处报错）。
    let error = state
        .call_tool(
            "ssh_exec",
            &json!({ "host": "h.test", "username": "op", "command": "true", "port": "2222" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("password"), "unexpected: {error}");
}

#[tokio::test]
async fn string_booleans_and_numbers_flow_through_the_gates() {
    let state = state();
    // confirmDestructive 写成字符串 "true" 同样过灾难门（此前恒读 false）。
    let error = state
        .call_tool(
            "ssh_exec",
            &json!({
                "host": "h.test", "username": "op",
                "command": "mkfs.ext4 /dev/sda1", "confirmDestructive": "true",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("password"),
        "string confirmDestructive must pass the gate, got: {error}"
    );
    // 非法布尔值给清晰报错（confirmDestructive 只在灾难分支被读取，
    // 所以必须配一条灾难命令才能触达解析）。
    let error = state
        .call_tool(
            "ssh_exec",
            &json!({
                "host": "h.test", "username": "op",
                "command": "mkfs.ext4 /dev/sda1", "confirmDestructive": "maybe",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("confirmDestructive must be a boolean"),
        "{error}"
    );
    // timeoutSecs 数字字符串被解析（非法值同样报错）。
    let error = state
        .call_tool(
            "ssh_exec",
            &json!({ "host": "h.test", "username": "op", "command": "true", "timeoutSecs": "30" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("password"), "unexpected: {error}");
    let error = state
        .call_tool(
            "ssh_exec",
            &json!({ "host": "h.test", "username": "op", "command": "true", "timeoutSecs": "soon" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("timeoutSecs must be an integer"), "{error}");
}

#[tokio::test]
async fn multi_exec_rejects_non_array_targets_with_guidance() {
    let state = state();
    let error = state
        .call_tool(
            "ssh_multi_exec",
            &json!({ "targets": "conn-1", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("targets must be an array") && error.contains("connectionId"),
        "{error}"
    );
}

#[tokio::test]
async fn ssh_close_demands_a_connection_reference() {
    let state = state();
    let error = state
        .call_tool("ssh_close", &json!({}), None)
        .await
        .unwrap_err();
    assert!(
        error.contains("ssh_close needs a connection reference"),
        "{error}"
    );
}

#[test]
fn required_str_reports_type_mismatch_with_guidance() {
    let arguments = json!({ "localPath": 42, "path": "" });
    let error = required_str(&arguments, "localPath").unwrap_err();
    assert!(
        error.contains("localPath") && error.contains("non-empty string"),
        "{error}"
    );
    // 空字符串与缺失走同一条消息（键名始终可见）。
    let error = required_str(&arguments, "path").unwrap_err();
    assert!(error.contains("path"), "{error}");
}

// —— MCP 第二轮：降级矩阵 + 安全门组合矩阵（2026-09-13）———

fn stored_connection(id: &str, host: &str, port: u16, extra: Value) -> StoredConnection {
    let mut connection = json!({
        "connection": {
            "id": id,
            "host": host,
            "port": port,
            "username": "deploy",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
        }
    });
    if let (Some(target), Some(source)) = (connection.get_mut("connection"), extra.as_object()) {
        for (key, value) in source {
            target[key.clone()] = value.clone();
        }
    }
    StoredConnection::from_lifecycle_params(&connection).unwrap()
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

/// 降级矩阵 ②：连接寻址双源（bridge registry vs session registry）
/// 同 id 冲突时桥数据胜出且不产生重复条目；不同 id 各自成列；作用域
/// 对两个来源都生效。
#[test]
fn connection_list_merge_resolves_dual_source_conflicts() {
    let bridge_entry = |id: &str, host: &str| {
        json!({
            "id": id, "name": id.to_uppercase(), "host": host, "port": 22,
            "username": "deploy", "authentication": "password", "readOnly": false,
        })
    };
    let registry = vec![
        // 同 id 不同 host：会话注册表声明更旧的元数据。
        stored_connection("conn-x", "old.example.test", 22, json!({})),
        // 不同 id：必须与桥条目并列出现。
        stored_connection("conn-y", "other.example.test", 2222, json!({})),
    ];
    let bridge = Ok(vec![
        bridge_entry("conn-x", "new.example.test"),
        bridge_entry("conn-y", "other.example.test"),
    ]);

    let merged = connection_list_result(bridge.clone(), &registry, &ConnectionScope::Unrestricted);
    assert_eq!(merged["source"], "dbx-app-bridge");
    let rows = merged["connections"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "id conflict must deduplicate, not append");
    let x = rows.iter().find(|row| row["id"] == "conn-x").unwrap();
    assert_eq!(
        x["host"], "new.example.test",
        "bridge metadata wins on an id conflict"
    );

    // 作用域对桥与注册表两来源都过滤：只留 conn-y。
    let scoped = connection_list_result(
        bridge,
        &registry,
        &ConnectionScope::AllowList(vec!["conn-y".to_string()]),
    );
    let rows = scoped["connections"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["id"], "conn-y");

    // 桥不可用时降级到注册表 + note（同 id 冲突无从发生）。
    let degraded = connection_list_result(
        Err("bridge down".into()),
        &registry,
        &ConnectionScope::Unrestricted,
    );
    assert_eq!(degraded["source"], "session-registry");
    assert_eq!(degraded["connections"].as_array().unwrap().len(), 2);
}

/// 安全门组合矩阵：只读门 × confirmDestructive × sudo 白名单 × 进程级
/// 只读开关 × connectionScope 的组合行为。核心不变量：
/// 白名单/确认位都不放大权限——灾难门永远在白名单之后仍然生效；
/// 只读门先于一切；scope 是 fail-closed 的第一道闸。
#[tokio::test]
async fn gate_combo_matrix_read_only_destructive_sudo_and_scope() {
    // 普通连接（sudo 白名单：精确 systemctl restart nginx + shutdown *）。
    let rw_state = state();
    rw_state.dbx_connections.write().await.insert(
        "conn-rw".to_string(),
        stored_connection(
            "conn-rw",
            "web.example.test",
            22,
            json!({ "external_config": {
                    "sudo_whitelist": "systemctl restart nginx\nshutdown *"
                } }),
        ),
    );
    // 只读连接（同白名单）：只读门必须先于白名单与确认位。
    let ro_state = state();
    ro_state.dbx_connections.write().await.insert(
        "conn-ro".to_string(),
        stored_connection(
            "conn-ro",
            "web.example.test",
            22,
            json!({ "read_only": true, "external_config": {
                    "sudo_whitelist": "systemctl restart nginx\nshutdown *"
                } }),
        ),
    );
    // 组合 1：非只读 + 灾难命令 + confirmDestructive:true → 过灾难门
    // （错误是拨号失败，不再是 confirm 提示）。
    let error = rw_state
        .call_tool(
            "ssh_exec",
            &json!({
                "connectionId": "conn-rw",
                "command": "shutdown -h now",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("confirmDestructive") && !error.contains("looks destructive"),
        "confirmed destructive command must proceed past the gate, got: {error}"
    );

    // 组合 2：非只读 + 灾难命令 + 无确认 → confirm 提示（对照组）。
    let error = rw_state
        .call_tool(
            "ssh_exec",
            &json!({ "connectionId": "conn-rw", "command": "shutdown -h now" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("confirmDestructive"), "{error}");

    // 组合 3：白名单命中的灾难命令（shutdown -h now ∈ shutdown *）→
    // 白名单放行不豁免灾难门，仍需 confirmDestructive。
    let error = rw_state
        .call_tool(
            "ssh_exec_sudo",
            &json!({ "connectionId": "conn-rw", "command": "shutdown -h now" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("confirmDestructive") && !error.contains("whitelist"),
        "whitelist must not bypass the destructive gate, got: {error}"
    );
    // 同命令带确认后过两道门（白名单 + 灾难），落到拨号失败。
    let error = rw_state
        .call_tool(
            "ssh_exec_sudo",
            &json!({
                "connectionId": "conn-rw",
                "command": "shutdown -h now",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("whitelist") && !error.contains("confirmDestructive"),
        "confirmed whitelisted command must pass both gates, got: {error}"
    );

    // 组合 4：只读连接 + ssh_exec_sudo（写类）+ 命令命中白名单 →
    // 只读门先行，白名单救不回写工具。
    let error = ro_state
        .call_tool(
            "ssh_exec_sudo",
            &json!({ "connectionId": "conn-ro", "command": "systemctl restart nginx" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("read-only"),
        "read-only gate must fire before the whitelist, got: {error}"
    );

    // 组合 5：只读连接 + 白名单巡检命令 + confirmDestructive:true →
    // 确认位不改变白名单判定，调用照常走到拨号。
    let error = ro_state
        .call_tool(
            "ssh_exec",
            &json!({
                "connectionId": "conn-ro",
                "command": "df -h",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("read-only") && !error.contains("not recognized"),
        "inspection command must stay available on read-only, got: {error}"
    );

    // 组合 6：只读连接 + 灾难命令 + confirmDestructive:true →
    // 确认位无法覆盖只读（同命令在非只读上只需要确认）。
    let error = ro_state
        .call_tool(
            "ssh_exec",
            &json!({
                "connectionId": "conn-ro",
                "command": "shutdown -h now",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("Refused on read-only"),
        "read-only must refuse destructive commands outright, got: {error}"
    );

    // 组合 7：进程级只读开关（DBX_SSH_MCP_READ_ONLY）× 非只读注册连接
    // × 灾难命令 + 确认 → 操作员开关压过连接自身的可写属性。
    let mut killed_state = state();
    killed_state.global_read_only = true;
    killed_state.dbx_connections.write().await.insert(
        "conn-rw".to_string(),
        stored_connection("conn-rw", "web.example.test", 22, json!({})),
    );
    let error = killed_state
        .call_tool(
            "ssh_exec",
            &json!({
                "connectionId": "conn-rw",
                "command": "shutdown -h now",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("Refused on read-only"),
        "process kill switch must dominate the connection flag, got: {error}"
    );
    // 同开关下白名单巡检命令仍可用（fail-closed 不等于全拒）：错误来自
    // 拨号层而不是任何一道门。
    let error = killed_state
        .call_tool(
            "ssh_exec",
            &json!({ "connectionId": "conn-rw", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("read-only")
            && !error.contains("not recognized")
            && !error.contains("confirmDestructive"),
        "uptime must pass the gates and fail at dialing, got: {error}"
    );

    // 组合 8：connectionScope 非空 × 内联凭据 × 灾难命令 → scope 是
    // fail-closed 的第一道闸，报 scope 而非 confirm 提示。
    let scoped_state = state();
    scoped_state
        .settings_set(&json!({ "connectionScope": ["conn-rw"] }))
        .unwrap();
    let error = scoped_state
        .call_tool(
            "ssh_exec",
            &json!({
                "host": "web.example.test", "username": "deploy",
                "command": "shutdown -h now", "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("connectionScope"),
        "scope must refuse inline dials before the destructive gate, got: {error}"
    );
}

/// 组合矩阵补充：terminal_input 的只读 × 灾难 × sudo 白名单三闸叠加，
/// 以及灾难 + 确认后仍受白名单约束（确认位只解锁灾难门一层）。
#[tokio::test]
async fn terminal_input_combo_gates_stack_in_order() {
    let state = state();
    state.dbx_connections.write().await.insert(
        "conn-ti".to_string(),
        stored_connection(
            "conn-ti",
            "web.example.test",
            22,
            json!({ "external_config": { "sudo_whitelist": "systemctl restart nginx" } }),
        ),
    );
    // 灾难行 + confirmDestructive → 过灾难门，但 sudo 行仍受白名单拒绝。
    let error = state
        .call_tool(
            "ssh_terminal_input",
            &json!({
                "connectionId": "conn-ti",
                "input": "sudo reboot\r",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("not allowed by this connection's whitelist"),
        "confirm must not lift the sudo allowlist, got: {error}"
    );
    // 白名单命中的 sudo 行 + 确认 → 两闸皆过，落到会话引导错误。
    let error = state
        .call_tool(
            "ssh_terminal_input",
            &json!({
                "connectionId": "conn-ti",
                "input": "sudo systemctl restart nginx\r",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("terminal"),
        "allowed sudo line must pass both gates, got: {error}"
    );
}

/// alert_triage 工具面（ssh_alert_triage）扩类：network / oom /
/// service / generic / 中英混合按 KEYWORDS 表实际行为断言；每条建议
/// settings 联动回归：mcp/settings/set 的每个可调项都能立即驱动受影响
/// 工具的行为（传输根收窄、上传限额、confirm 档 fail-closed、scope 闸），
/// settings_get 回显生效值与 persisted 值。
#[tokio::test]
async fn settings_set_changes_drive_downstream_tool_behavior() {
    // (a) localTransferRoot 收窄：默认根内的文件过根闸，收窄后被拒。
    let st = state();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("inside.bin"), b"data").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("outside.bin"), b"data").unwrap();
    let upload_args = |path: &Path| {
        json!({
            "host": "web.example.test", "username": "deploy",
            "localPath": path.to_str().unwrap(), "remotePath": "/tmp/x",
        })
    };
    // 默认允许根（系统临时目录）放行两个文件 → 错误是拨号失败而非根拒绝。
    let error = st
        .call_tool(
            "sftp_upload",
            &upload_args(&outside.path().join("outside.bin")),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("allowed transfer roots") && !error.contains("localTransferRoot"),
        "default roots must allow temp files, got: {error}"
    );
    // 收窄到 root 后：根外文件在拨号前被拒。
    st.settings_set(&json!({ "localTransferRoot": root.path().to_str().unwrap() }))
        .unwrap();
    let error = st
        .call_tool(
            "sftp_upload",
            &upload_args(&outside.path().join("outside.bin")),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("localTransferRoot"),
        "narrowed root must refuse outside paths, got: {error}"
    );
    // 根内文件过根闸（错误回到拨号层）。
    let error = st
        .call_tool(
            "sftp_upload",
            &upload_args(&root.path().join("inside.bin")),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("localTransferRoot") && !error.contains("allowed transfer roots"),
        "in-root file must pass the root gate, got: {error}"
    );

    // (b) maxUploadBytes：设置即时生效，超限文件在拨号前被拒。
    let st = state();
    let big = tempfile::tempdir().unwrap();
    std::fs::write(big.path().join("big.bin"), vec![0u8; 64]).unwrap();
    st.settings_set(&json!({ "maxUploadBytes": 32 })).unwrap();
    let error = st
        .call_tool(
            "sftp_upload",
            &json!({
                "host": "web.example.test", "username": "deploy",
                "localPath": big.path().join("big.bin").to_str().unwrap(),
                "remotePath": "/tmp/x",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("exceeds the MCP upload limit"),
        "upload limit must apply immediately, got: {error}"
    );

    // (c) execPermissionMode=confirm：stdio 无审批通道 → 立即 fail-closed；
    // 切回 autonomous 后调用恢复（落到拨号错误）。
    let st = state();
    st.settings_set(&json!({ "execPermissionMode": "confirm" }))
        .unwrap();
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({ "host": "web.example.test", "username": "deploy", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("execPermissionMode=confirm"),
        "confirm mode must fail closed without an approval channel, got: {error}"
    );
    st.settings_set(&json!({ "execPermissionMode": "autonomous" }))
        .unwrap();
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({ "host": "web.example.test", "username": "deploy", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("execPermissionMode"),
        "switching back to autonomous must lift the gate, got: {error}"
    );

    // (d) connectionScope：非空即拒绝内联拨打（fail-closed），list 工具
    // 仍可用；清空后恢复。settings_get 回显生效值与 persisted 值。
    let st = state();
    st.settings_set(&json!({ "connectionScope": ["only-this"] }))
        .unwrap();
    let view = st.settings_get();
    assert_eq!(view["connectionScope"], json!(["only-this"]));
    assert_eq!(view["persistedConnectionScope"], json!(["only-this"]));
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({ "host": "web.example.test", "username": "deploy", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("connectionScope"), "{error}");
    let listed = st.call_tool("ssh_list_connections", &json!({}), None).await;
    assert!(listed.is_ok(), "list stays answerable under a scope");
    st.settings_set(&json!({ "connectionScope": [] })).unwrap();
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({ "host": "web.example.test", "username": "deploy", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("connectionScope"),
        "clearing the scope must lift the gate, got: {error}"
    );
}

/// 生产路径端口早校验挡死 arg_port_lossy 的静默回退：任何工具带非法
/// 端口都在门/池键/known_hosts 触碰之前报错，绝不按 22 继续。
#[tokio::test]
async fn malformed_port_is_rejected_before_lossy_gate_reads() {
    let st = state();
    // sftp 工具：非法端口在 lossy 门查找/池键派生之前被早校验拦下。
    let error = st
        .call_tool(
            "sftp_stat",
            &json!({
                "host": "web.example.test", "username": "deploy",
                "path": "/tmp", "port": "abc",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("port must be an integer"),
        "expected the early port check, got: {error}"
    );
    // ssh_close：端口早校验先于工具自身的引用指引。
    let error = st
        .call_tool(
            "ssh_close",
            &json!({ "connectionName": "x", "port": 0 }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("port must be between 1 and 65535"),
        "{error}"
    );
    // known_hosts 工具同样先过早校验，不触碰文件。
    let error = st
        .call_tool(
            "ssh_remove_known_host",
            &json!({ "host": "h.example.test", "port": 99999 }),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("port must be between"), "{error}");
}

/// schema 外未知参数静默忽略（宿主职责）不得反噬：多余字段不改变既有
/// 参数的解析，大小写变体不被识别（也不解锁任何门）。
#[tokio::test]
async fn unknown_arguments_never_distort_known_parameter_parsing() {
    let st = state();
    // 未知参数 + 合法参数混合：仍在缺密码处报错（解析未被带偏）。
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({
                "host": "web.example.test", "username": "deploy", "command": "true",
                "totallyUnknown": { "nested": true }, "cmd": "evil",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("password"), "{error}");
    // 大小写不同的 "Port" 不被识别为 port：不触发范围错误，按缺省 22。
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({
                "host": "web.example.test", "username": "deploy",
                "command": "true", "Port": 99999,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("port must be"),
        "unknown-case key must be ignored, got: {error}"
    );
    assert!(error.contains("password"), "{error}");
    // 大写 ConfirmDestructive 不解锁灾难门（未知参数不能放大权限）。
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({
                "host": "web.example.test", "username": "deploy",
                "command": "mkfs.ext4 /dev/sda1", "ConfirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("confirmDestructive"), "{error}");
    // 本地工具携带未知参数照常成功。
    let result = st
        .call_tool("ssh_list_known_hosts", &json!({ "extra": 42 }), None)
        .await
        .unwrap();
    assert_eq!(result["isError"], false);
}

// —— M25：MCP 面远端路径组件归一（与工作台车道同口径）——

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
