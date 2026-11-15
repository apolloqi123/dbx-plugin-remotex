use super::*;

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
            .unwrap_or_else(|| panic!("{name} missing from tools/list"))["inputSchema"]
            ["properties"]
            .as_object()
            .cloned()
            .unwrap()
    };

    for name in ["ssh_exec", "ssh_exec_sudo", "sftp_upload"] {
        assert!(
            properties(&full, name).contains_key("connectionId"),
            "{name} standalone should keep connectionId"
        );
        assert!(
            !properties(&host, name).contains_key("connectionId"),
            "{name} host listing should hide connectionId"
        );
    }
    for name in ["ssh_exec", "ssh_exec_sudo", "ssh_multi_exec"] {
        assert!(
            properties(&full, name).contains_key("timeoutSecs"),
            "{name} standalone should keep timeoutSecs"
        );
        assert!(
            !properties(&host, name).contains_key("timeoutSecs"),
            "{name} host listing should hide timeoutSecs"
        );
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
    assert_eq!(init["result"]["serverInfo"]["name"], "io.dbx.remotex");

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
