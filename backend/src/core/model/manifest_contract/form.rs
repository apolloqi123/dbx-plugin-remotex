use super::*;

/// 表单“保存”判定 ⇔ 解析层接受度。宿主动画框只拦「可见 ∧ 必填 ∧ 为空」，
/// 真正的凭据规则在解析层，两者必须同向：
///   * 表单拦下、解析层却接受 → 用户被卡死（无法保存一个插件支持的配置）；
///   * 表单放行、解析层拒绝 → 只允许出现在 OR 语义（密码∨密码命令、
///     私钥路径∨私钥内容）无法在契约里表达处，且报错必须点名表单字段。
#[test]
fn form_save_state_matches_parser_acceptance() {
    let fields = connection_fields();
    let defaults: BTreeMap<String, Value> = fields
        .iter()
        .filter_map(|entry| {
            entry
                .get("default")
                .filter(|value| !value.is_null())
                .map(|value| {
                    (
                        entry["key"].as_str().unwrap_or_default().to_string(),
                        value.clone(),
                    )
                })
        })
        .collect();

    let mut cases = 0usize;
    let mut blocked_without_rejection = 0usize;
    let mut blocked_samples: Vec<String> = Vec::new();
    let mut unexplained_rejections: Vec<String> = Vec::new();
    for advanced_options in [false, true] {
        for authentication in [
            "password",
            "private-key",
            "private-key-password",
            "agent",
            "none",
        ] {
            for sudo_source in ["custom", "global", "off"] {
                for auth_flow_mode in [
                    "off",
                    "password_then_otp",
                    "password_plus_otp",
                    "password_only",
                ] {
                    for read_only in [false, true] {
                        for triggers_enabled in [false, true] {
                            for password in ["", "secret"] {
                                for password_command in ["", "echo pw"] {
                                    for password_source in ["direct", "command"] {
                                        for private_key_path in ["", "~/.ssh/id_ed25519"] {
                                            for private_key in
                                                ["", "-----BEGIN OPENSSH PRIVATE KEY-----\nkey\n"]
                                            {
                                                for private_key_passphrase in ["", "phrase"] {
                                                    let mut values = defaults.clone();
                                                    for (key, value) in [
                                                        (
                                                            "advanced_options",
                                                            json!(advanced_options),
                                                        ),
                                                        ("authentication", json!(authentication)),
                                                        ("sudo_source", json!(sudo_source)),
                                                        ("auth_flow_mode", json!(auth_flow_mode)),
                                                        ("read_only", json!(read_only)),
                                                        (
                                                            "triggers_enabled",
                                                            json!(triggers_enabled),
                                                        ),
                                                        ("password", json!(password)),
                                                        (
                                                            "password_command",
                                                            json!(password_command),
                                                        ),
                                                        ("password_source", json!(password_source)),
                                                        (
                                                            "private_key_path",
                                                            json!(private_key_path),
                                                        ),
                                                        ("private_key", json!(private_key)),
                                                        (
                                                            "private_key_passphrase",
                                                            json!(private_key_passphrase),
                                                        ),
                                                    ] {
                                                        values.insert(key.to_string(), value);
                                                    }
                                                    cases += 1;
                                                    let blocked =
                                                        form_blocking_field(&fields, &values);
                                                    let payload =
                                                        connection_payload(&fields, &values);
                                                    let parsed =
                                                        StoredConnection::from_lifecycle_params(
                                                            &json!({ "connection": payload }),
                                                        );
                                                    match (&blocked, &parsed) {
                                                        (Some(key), Ok(_)) => {
                                                            blocked_without_rejection += 1;
                                                            if blocked_samples.len() < 4 {
                                                                blocked_samples.push(format!(
                                                                "{key}: authentication={authentication} \
                                                                 password={password:?} \
                                                                 password_command={password_command:?} \
                                                                 private_key_path={private_key_path:?} \
                                                                 private_key={} \
                                                                 private_key_passphrase={private_key_passphrase:?}",
                                                                if private_key.is_empty() {
                                                                    "\"\""
                                                                } else {
                                                                    "<key>"
                                                                }
                                                            ));
                                                            }
                                                        }
                                                        (None, Err(error)) => {
                                                            let lower = error.to_lowercase();
                                                            // The message has to name the form
                                                            // fields that satisfy the rule, so a
                                                            // rejection is never a dead end.
                                                            let actionable = (lower
                                                                .contains("\"password\"")
                                                                && lower.contains(
                                                                    "\"password command\"",
                                                                ))
                                                                || (lower.contains(
                                                                    "\"private key path\"",
                                                                ) && lower.contains(
                                                                    "\"private key content\"",
                                                                ))
                                                                || lower.contains("ssh port");
                                                            if !actionable
                                                                && unexplained_rejections.len() < 4
                                                            {
                                                                unexplained_rejections
                                                                    .push(error.clone());
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    assert_eq!(
        blocked_without_rejection, 0,
        "the dialog blocks {blocked_without_rejection} configuration(s) the parser accepts \
         (dead-locked footer, no way to save). Samples: {blocked_samples:#?}"
    );
    assert!(
        unexplained_rejections.is_empty(),
        "parser rejects without naming the form field that fixes it: {unexplained_rejections:?}"
    );
    assert!(
        cases >= 15_000,
        "form/parser matrix shrank unexpectedly ({cases} cases)"
    );
}

/// 发布前的表单组合矩阵：认证方式、sudo 来源、只读开关、2FA 模式等
/// 下拉/开关的每一种交叉，在解析层要么成功且语义正确，要么干净报错
/// ——绝不 panic、绝不静默错位。表单切换认证方式会在不可见字段里留下
/// 残留凭据，这类组合必须被容忍（多余凭据被忽略而非报错），否则用户改
/// 一个下拉框就再也连不上。
#[test]
fn form_option_combinations_parse_without_conflicts() {
    let connection_with = |authentication: &str,
                           password: Option<&str>,
                           mut external_config: Value,
                           secrets: Value|
     -> Result<StoredConnection, String> {
        external_config["authentication"] = Value::String(authentication.to_string());
        let mut connection = serde_json::json!({
            "id": "combo-matrix",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": external_config
        });
        if let Some(password) = password {
            connection["password"] = Value::String(password.to_string());
        }
        if !secrets.is_null() {
            connection["connection_secrets"] = secrets;
        }
        StoredConnection::from_lifecycle_params(&serde_json::json!({ "connection": connection }))
    };

    // ① 每个认证方式的最小必需凭据组合都必须通过（含表单里显式提供
    //    的 none / agent 这两个"零凭据"选项）。
    let minimal: &[(&str, Option<&str>, Value, Value, AuthenticationMethod)] = &[
        (
            "password",
            Some("pw"),
            serde_json::json!({}),
            serde_json::json!({}),
            AuthenticationMethod::Password,
        ),
        (
            "private-key",
            None,
            serde_json::json!({ "private_key_path": "~/.ssh/id_ed25519" }),
            serde_json::json!({}),
            AuthenticationMethod::PrivateKey,
        ),
        (
            "private-key-password",
            Some("pw"),
            serde_json::json!({ "private_key_path": "/keys/id_ed25519" }),
            serde_json::json!({ "private_key_passphrase": "pp" }),
            AuthenticationMethod::PrivateKeyPassword,
        ),
        (
            "agent",
            None,
            serde_json::json!({}),
            serde_json::json!({}),
            AuthenticationMethod::Agent,
        ),
        // Auto 按序回退不要求任何前置凭据：密码/私钥缺失只是对应阶段
        // 被跳过（回退链里的既有记录），解析层必须接受零凭据组合。
        (
            "auto",
            None,
            serde_json::json!({}),
            serde_json::json!({}),
            AuthenticationMethod::Auto,
        ),
        (
            "none",
            None,
            serde_json::json!({}),
            serde_json::json!({}),
            AuthenticationMethod::None,
        ),
    ];
    for (authentication, password, external, secrets, expected) in minimal {
        let parsed = connection_with(authentication, *password, external.clone(), secrets.clone())
            .unwrap_or_else(|error| panic!("{authentication} minimal combo rejected: {error}"));
        assert_eq!(
            parsed.authentication, *expected,
            "minimal combo for '{authentication}' misparses"
        );
    }

    // ② 残留凭据容忍：表单里所有凭据字段都有值时（用户来回切换过
    //    认证方式），每种认证方式都必须照常解析且方法正确。
    let everything_external = serde_json::json!({ "private_key_path": "/keys/k" });
    let everything_secrets =
        serde_json::json!({ "private_key_passphrase": "pp", "totp_secret": "JBSWY3DPEHPK3PXP" });
    for (authentication, _, _, _, expected) in minimal {
        let parsed = connection_with(
            authentication,
            Some("pw"),
            everything_external.clone(),
            everything_secrets.clone(),
        )
        .unwrap_or_else(|error| {
            panic!("{authentication} leftover-credential combo rejected: {error}")
        });
        assert_eq!(
            parsed.authentication, *expected,
            "leftover-credential combo for '{authentication}' misparses"
        );
    }

    // ③ 必需凭据缺失 → 干净报错并指出缺什么。
    let missing_password = connection_with(
        "password",
        None,
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        missing_password.contains("requires a password"),
        "{missing_password}"
    );
    let missing_key = connection_with(
        "private-key",
        None,
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        missing_key.contains("requires a private key path"),
        "{missing_key}"
    );
    let half_of_private_key_password = connection_with(
        "private-key-password",
        None,
        serde_json::json!({ "private_key_path": "/keys/k" }),
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        half_of_private_key_password.contains("requires a password"),
        "{half_of_private_key_password}"
    );
    let other_half = connection_with(
        "private-key-password",
        Some("pw"),
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        other_half.contains("requires a private key path"),
        "{other_half}"
    );

    // ④ 无法识别的认证值 → 明确报错（而非静默回落 password，那会把
    //    拼错的组合变成一次注定失败的密码登录）。
    let typo = connection_with(
        "publickey",
        Some("pw"),
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .unwrap_err();
    assert!(
        typo.contains("Unsupported SSH authentication method 'publickey'"),
        "{typo}"
    );

    // ⑤ read_only × sudo_source 全矩阵：六个组合都必须可解析，
    //    sudo_enabled 只反映凭据来源——只读是正交的运行时门禁
    //    （ssh.rs 的 exec/sudo 通道负责拦截），解析层不得混淆二者。
    for read_only in [false, true] {
        for source in ["custom", "global", "off"] {
            let parsed = connection_with(
                "password",
                Some("pw"),
                serde_json::json!({ "sudo_source": source, "read_only": read_only }),
                serde_json::json!({}),
            )
            .unwrap_or_else(|error| {
                panic!("read_only={read_only} sudo_source={source} rejected: {error}")
            });
            assert_eq!(parsed.read_only, read_only, "sudo_source={source}");
            assert_eq!(
                parsed.sudo_enabled(),
                source != "off",
                "read_only={read_only} sudo_source={source}"
            );
        }
    }

    // ⑥ auth_flow_mode 交叉：sudo_source=global 时连接自身的 2FA 模式
    //    仍可解析（表单隐藏它，但登录期 keyboard-interactive 还要用）；
    //    password_only 携带 totp_secret 也合法——运行时静默忽略 OTP，
    //    不算配置冲突。
    for flow in [
        "password_only",
        "password_plus_otp",
        "password_then_otp",
        "ssh-agent-garbage",
    ] {
        for source in ["custom", "global", "off"] {
            let parsed = connection_with(
                "password",
                Some("pw"),
                serde_json::json!({
                    "sudo_source": source,
                    "auth_flow_mode": flow,
                    "totp_prompt_hint": "duo passcode"
                }),
                serde_json::json!({ "totp_secret": "JBSWY3DPEHPK3PXP" }),
            )
            .unwrap_or_else(|error| {
                panic!("auth_flow_mode={flow} sudo_source={source} rejected: {error}")
            });
            assert_eq!(parsed.auth_flow_mode, flow, "sudo_source={source}");
        }
    }

    // ⑦ 端口边界：0 与 65536 报错，1 与 65535 合法。
    for (port, ok) in [(0u64, false), (1, true), (65535, true), (65536, false)] {
        let connection = serde_json::json!({
            "id": "port-edge",
            "host": "example.com",
            "port": port,
            "username": "user",
            "password": "pw",
            "external_config": { "authentication": "password" }
        });
        let result = StoredConnection::from_lifecycle_params(
            &serde_json::json!({ "connection": connection }),
        );
        assert_eq!(
            result.is_ok(),
            ok,
            "port {port} should {}",
            if ok { "parse" } else { "be rejected" }
        );
    }

    // ⑧ connect_timeout_secs=0 钳到下限 1，而非让握手等待 0 秒后立即
    //    超时（表单允许输入 0，解析层负责收敛）。
    let zero_timeout = connection_with(
        "password",
        Some("pw"),
        serde_json::json!({ "connect_timeout_secs": 0 }),
        serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(zero_timeout.connect_timeout_secs, 1);
    assert!(zero_timeout.connect_timeout_explicit);

    // ⑧b 表单未填时跟随 manifest 默认（30s）：慢速 Windows/macOS 主机的
    //    握手+认证预算不再按旧默认 15s 一连接就打满。缺省同时标记为
    //    非显式——connection/test 的拨号预算据此对齐宿主 10s 回退截止。
    let default_timeout = connection_with(
        "password",
        Some("pw"),
        serde_json::json!({}),
        serde_json::json!({}),
    )
    .unwrap();
    assert_eq!(default_timeout.connect_timeout_secs, 30);
    assert!(!default_timeout.connect_timeout_explicit);

    // ⑨ 跳板机不接受 none：链上每一跳都必须认证，"No authentication"
    //    只对最终会话合法。
    let none_hop = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "hop-none",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "pw",
            "external_config": {
                "authentication": "password",
                "jump_hosts": [
                    { "host": "bastion", "port": 22, "username": "ops", "authentication": "none" }
                ]
            }
        }
    }))
    .unwrap_err();
    assert!(
        none_hop.contains("Unsupported jump host authentication method 'none'"),
        "{none_hop}"
    );
}
