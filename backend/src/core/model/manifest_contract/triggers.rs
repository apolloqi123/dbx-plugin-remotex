use super::*;

/// 端口在表单里只受 `type: number` 约束，宿主 Rust 层与解析层各自校验
/// 1..65535；契约无 min/max 属性，所以范围提示必须留在描述里（前端契约
/// 脚本 `scripts/verify/connection-forms/verify.mjs` 断言七语都写了）。
#[test]
fn password_source_is_honored_by_the_parser() {
    let connection = |external: Value, password: Option<&str>| {
        let mut payload = json!({
            "id": "pw-source",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": external,
        });
        if let Some(password) = password {
            payload["password"] = json!(password);
        }
        StoredConnection::from_lifecycle_params(&json!({ "connection": payload }))
    };

    // 老配置（0.4.x 只有 password/password_command）与 MCP 内联参数没有
    // 来源字段：保持历史语义，两者二选一，显式密码优先、命令保留。
    let legacy = connection(
        json!({ "authentication": "password", "password_command": "echo pw" }),
        Some("stored"),
    )
    .unwrap();
    assert_eq!(legacy.password, "stored");
    assert_eq!(legacy.password_command, "echo pw");
    let legacy_command = connection(
        json!({ "authentication": "password", "password_command": "echo pw" }),
        None,
    )
    .unwrap();
    assert!(legacy_command.password.is_empty());
    assert_eq!(legacy_command.password_command, "echo pw");

    // direct：密码必填，缺失时点名两个表单字段（并给出改来源的出口）。
    let direct = connection(
        json!({ "authentication": "password", "password_source": "direct" }),
        Some("typed"),
    )
    .unwrap();
    assert_eq!(direct.password, "typed");
    let direct_missing = connection(
        json!({
            "authentication": "password",
            "password_source": "direct",
            "password_command": "echo pw"
        }),
        None,
    )
    .unwrap_err();
    assert!(
        direct_missing.contains("Password source"),
        "{direct_missing}"
    );
    assert!(direct_missing.contains("\"Password\""), "{direct_missing}");

    // command：命令必填，且存量密码让位——否则「显式密码优先」会让用户
    // 显式选择的本地命令永不执行。
    let command = connection(
        json!({
            "authentication": "password",
            "password_source": "command",
            "password_command": "echo pw"
        }),
        Some("stale"),
    )
    .unwrap();
    assert!(
        command.password.is_empty(),
        "command source must drop the stored password so the command runs"
    );
    assert_eq!(command.password_command, "echo pw");
    let command_missing = connection(
        json!({ "authentication": "password", "password_source": "command" }),
        Some("stale"),
    )
    .unwrap_err();
    assert!(
        command_missing.contains("Password command"),
        "{command_missing}"
    );

    // 非密码类认证不受来源约束（字段隐藏，值可能是残留）。
    let agent = connection(
        json!({ "authentication": "agent", "password_source": "command" }),
        None,
    )
    .unwrap();
    assert!(agent.password.is_empty());

    // 未知来源：明确报错，不静默回退成 direct。
    let unknown = connection(
        json!({ "authentication": "password", "password_source": "keychain" }),
        Some("typed"),
    )
    .unwrap_err();
    assert!(
        unknown.contains("Unsupported password source 'keychain'"),
        "{unknown}"
    );
}

/// 触发器（Expect）与外部密码管理器配置的解析面：external_config 的
/// triggers（对象/字符串两形态）、密文槽位解析、非法配置整连接失败
/// （D7）、password_command 让密码类认证允许留空（显式密码优先）。
#[test]
fn triggers_and_credential_commands_parse_strictly() {
    let lifecycle = |external_config: Value, secrets: Value| {
        let mut connection = serde_json::json!({
            "id": "triggers",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "pw",
            "external_config": external_config,
        });
        if !secrets.is_null() {
            connection["connection_secrets"] = secrets;
        }
        StoredConnection::from_lifecycle_params(&serde_json::json!({
            "connection": connection
        }))
    };

    // ① 对象形态（宿主 lifecycle / MCP 桥转发）：密文槽位解析成功。
    let parsed = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers_enabled": true,
            "triggers": {
                "timeoutSecs": 45,
                "sleepMs": 200,
                "passSleep": "enter",
                "stages": [
                    { "pattern": "code:", "sendSecretKey": "trigger_answer_1" },
                    { "pattern": "proceed", "sendText": "yes\\r",
                      "casePattern": "\\(y/n\\)", "caseSendText": "y" }
                ]
            },
            "password_command": "op read vault",
            "passphrase_command": "security find-password -w"
        }),
        serde_json::json!({ "trigger_answer_1": "one" }),
    )
    .unwrap();
    let triggers = parsed.triggers.expect("triggers must parse");
    assert_eq!(triggers.timeout_secs, 45);
    assert_eq!(triggers.sleep_ms, 200);
    assert_eq!(triggers.stages.len(), 2);
    assert_eq!(triggers.stages[0].answer.kind(), "secret");
    assert!(triggers.stages[1].case.is_some());
    assert_eq!(parsed.password_command, "op read vault");
    assert_eq!(parsed.passphrase_command, "security find-password -w");

    // ② 字符串形态（连接表单 textarea）同样解析。
    let parsed = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers_enabled": true,
            "triggers": r#"{"stages":[{"pattern":"code","sendText":"1\r"}]}"#
        }),
        Value::Null,
    )
    .unwrap();
    assert_eq!(parsed.triggers.expect("string form parses").stages.len(), 1);

    // ③ 独立开关默认关闭：旧的非空文本也必须不解析、不启用。
    let disabled_with_bad_text = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers": "{not json"
        }),
        Value::Null,
    )
    .unwrap();
    assert!(!disabled_with_bad_text.triggers_enabled);
    assert!(disabled_with_bad_text.triggers.is_none());

    // ④ 空配置仍表示关闭（即使显式打开开关也没有 engine）。
    for raw in [
        Value::Null,
        Value::from(""),
        serde_json::json!({ "stages": [] }),
    ] {
        let mut external = serde_json::json!({
            "authentication": "password"
        });
        if !raw.is_null() {
            external["triggers"] = raw;
        }
        let parsed = lifecycle(external, Value::Null).unwrap();
        assert!(parsed.triggers.is_none(), "expected disabled");
    }

    // ⑤ 非法 JSON / 未知密文槽位 → 连接失败并给出可定位的错误（D7）。
    let error = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers_enabled": true,
            "triggers": "{not json"
        }),
        Value::Null,
    )
    .unwrap_err();
    assert!(error.contains("triggers"), "{error}");
    let error = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers_enabled": true,
            "triggers": { "stages": [{ "pattern": "p", "sendSecretKey": "nope" }] }
        }),
        Value::Null,
    )
    .unwrap_err();
    assert!(error.contains("unknown secret slot"), "{error}");
    // 引用的槽位未填充同样失败（静默发空行比失败更难排查）。
    let error = lifecycle(
        serde_json::json!({
            "authentication": "password",
            "triggers_enabled": true,
            "triggers": { "stages": [{ "pattern": "p", "sendSecretKey": "trigger_answer_1" }] }
        }),
        Value::Null,
    )
    .unwrap_err();
    assert!(error.contains("is empty"), "{error}");

    // ⑥ password_command 允许密码类认证留空密码（命令在连接期取回）。
    // 顶层 connection 无 password 字段、仅配置命令时解析成功。
    let parsed = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "triggers",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": {
                "authentication": "password",
                "password_command": "op read vault"
            }
        }
    }))
    .unwrap();
    assert!(parsed.password.is_empty());
    assert_eq!(parsed.password_command, "op read vault");
    // 两者都没有才拒绝。
    let error = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "triggers",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": { "authentication": "password" }
        }
    }))
    .unwrap_err();
    assert!(error.contains("password_command"), "{error}");
}

/// Every manifest field must land in the store the model reads it from:
/// `secret` bindings come from `connection_secrets`, `config` bindings
/// from `external_config` and are consumed by the parser.
#[test]
fn bindings_match_parse_surfaces() {
    // private_key 现在被解析面消费（表单粘贴的私钥内容 / 外部工具写入的
    // 存量 secret），从 connection_secrets 读取。
    // trigger_answer_1/2 是触发器密文槽位，经 triggers 解析面消费（sendSecretKey 解析）。
    let secret_keys = [
        "private_key",
        "private_key_passphrase",
        "sudo_password",
        "totp_secret",
        "trigger_answer_1",
        "trigger_answer_2",
    ];
    let config_keys = [
        "protocol",
        "authentication",
        "private_key_path",
        "agent_socket",
        "advanced_options",
        "connect_timeout_secs",
        "keepalive_interval_secs",
        "terminal_keepalive_secs",
        "set_env",
        "triggers_enabled",
        "triggers",
        "password_command",
        "password_source",
        "passphrase_command",
        "remote_command",
        "sudo_source",
        "sudo_profile",
        "sudo_use_pty",
        "sudo_whitelist",
        "read_only",
        "auth_flow_mode",
        "password_prompt_hint",
        "totp_prompt_hint",
        // M32-B：serial/rdp 连接配置由前端 openSession 路由读取（不进
        // StoredConnection 凭据解析），但同为 external_config 键面。
        "serial_port",
        "serial_baud",
        "serial_data_bits",
        "serial_parity",
        "serial_stop_bits",
        "serial_backspace",
        "rdp_domain",
        "rdp_resolution",
        "rdp_certificate_policy",
        "rdp_clipboard",
    ];
    let mut seen_secret: Vec<String> = Vec::new();
    let mut seen_config: Vec<String> = Vec::new();
    for entry in connection_fields() {
        match entry["binding"].as_str().unwrap_or("") {
            "secret" => seen_secret.push(entry["key"].as_str().unwrap().to_string()),
            "config" => seen_config.push(entry["key"].as_str().unwrap().to_string()),
            "name" | "host" | "port" | "username" | "password" => {
                let key = entry["key"].as_str().unwrap();
                assert!(
                    matches!(
                        key,
                        "display_name" | "host" | "port" | "username" | "password"
                    ),
                    "unexpected binding for field '{key}'"
                );
            }
            other => panic!("unexpected binding '{other}' in manifest"),
        }
    }
    seen_secret.sort_unstable();
    seen_config.sort_unstable();
    let mut expected_secret: Vec<String> =
        secret_keys.iter().map(|value| value.to_string()).collect();
    expected_secret.sort_unstable();
    let mut expected_config: Vec<String> =
        config_keys.iter().map(|value| value.to_string()).collect();
    expected_config.sort_unstable();
    assert_eq!(
        seen_secret, expected_secret,
        "secret bindings must match connection_secrets keys"
    );
    assert_eq!(
        seen_config, expected_config,
        "config bindings must match external_config keys consumed by the parser"
    );
}
