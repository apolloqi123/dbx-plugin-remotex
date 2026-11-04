use super::*;

#[test]
fn normalizes_remote_paths_without_escaping_root() {
    assert_eq!(
        normalize_remote_path("/home/user/../file").unwrap(),
        "/home/file"
    );
    assert_eq!(normalize_remote_path("../../etc").unwrap(), "/etc");
}

#[test]
fn terminal_frame_carries_stream_and_sequence() {
    let encoded = TerminalFrame {
        sequence: 42,
        stream: TerminalStream::Stderr,
        data: b"x".to_vec(),
    }
    .encode();
    assert_eq!(encoded[0], 1);
    assert_eq!(u64::from_be_bytes(encoded[1..9].try_into().unwrap()), 42);
    assert_eq!(&encoded[9..], b"x");
}

#[test]
fn stdin_stream_tag_is_three_and_avoids_state() {
    // B1 契约：Stdin = 3，避开 local 终端已占用带内状态帧的 State = 2。
    assert_eq!(TerminalStream::Stdin as u8, 3);
    let encoded = TerminalFrame {
        sequence: 7,
        stream: TerminalStream::Stdin,
        data: b"hi".to_vec(),
    }
    .encode();
    assert_eq!(encoded[0], 3);
    assert_eq!(u64::from_be_bytes(encoded[1..9].try_into().unwrap()), 7);
    assert_eq!(&encoded[9..], b"hi");
}

#[test]
fn session_open_request_accepts_legacy_payload() {
    let request: SessionOpenRequest = serde_json::from_value(serde_json::json!({
        "connectionId": "conn",
        "workbenchId": "wb"
    }))
    .unwrap();
    assert_eq!(request.connection_id, "conn");
    assert_eq!(request.workbench_id, "wb");
    assert!(!request.reuse_authenticated_transport);
    assert!(request.reuse_authenticated_session_id.is_none());
    assert!(request.requested_session_id.is_none());
    assert_eq!(request.cols, 120);
    assert_eq!(request.rows, 32);
    assert!(request.spawn_command.is_none());
}

#[test]
fn session_open_request_accepts_requested_id_and_unknown_fields() {
    let request: SessionOpenRequest = serde_json::from_value(serde_json::json!({
        "connectionId": "conn",
        "requestedSessionId": "session-1",
        "futureField": true,
        "cols": 80,
        "rows": 24
    }))
    .unwrap();
    assert_eq!(request.requested_session_id.as_deref(), Some("session-1"));
    assert_eq!(request.cols, 80);
    assert_eq!(request.rows, 24);
}

#[test]
fn session_open_request_accepts_spawn_command() {
    let request: SessionOpenRequest = serde_json::from_value(serde_json::json!({
        "connectionId": "conn",
        "reuseAuthenticatedTransport": true,
        "reuseAuthenticatedSessionId": "source",
        "spawnCommand": "htop"
    }))
    .unwrap();
    assert_eq!(request.spawn_command.as_deref(), Some("htop"));
    assert!(request.reuse_authenticated_transport);
}

#[test]
fn legacy_password_connections_remain_compatible() {
    let connection = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "legacy",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "secret"
        }
    }))
    .unwrap();

    assert_eq!(connection.authentication, AuthenticationMethod::Password);
    assert_eq!(connection.password, "secret");
}

#[test]
fn read_only_flags_from_host_and_form_force_read_only() {
    // 插件特定配置项：连接表单 read_only（external_config）→ 只读门禁。
    let form_read_only = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "form-ro",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "secret",
            "external_config": { "read_only": true }
        }
    }))
    .unwrap();
    assert!(
        form_read_only.read_only,
        "form read_only must force the gate"
    );

    // 宿主标准 read_only（ConnectionConfig.read_only，通用连接设置）同样生效。
    let host_read_only = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "ro",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "secret",
            "read_only": true
        }
    }))
    .unwrap();
    assert!(
        host_read_only.read_only,
        "host read_only must force the gate"
    );

    let writable = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "rw",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "secret"
        }
    }))
    .unwrap();
    assert!(
        !writable.read_only,
        "writable connections must stay writable"
    );
}

#[test]
fn manifest_connection_fields_stay_in_sync_with_parsing() {
    // 契约：manifest.json 的 connection-provider 字段与 from_lifecycle_params
    // 的解析覆盖互为镜像——manifest 加字段而解析不消费（或反向）都会漂移，
    // 这里直读 manifest 逐项对账。
    let manifest: Value = serde_json::from_str(include_str!("../../../../manifest.json")).unwrap();
    let provider = manifest["contributions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["type"] == "connection-provider")
        .expect("connection-provider contribution");
    let fields = provider["fields"].as_array().unwrap();

    let keys: Vec<&str> = fields
        .iter()
        .filter_map(|field| field["key"].as_str())
        .collect();
    let expected = [
        "display_name",
        "protocol",
        "host",
        "port",
        "username",
        "authentication",
        "password_source",
        "password",
        "password_command",
        "private_key_path",
        "private_key_passphrase",
        "private_key",
        "agent_socket",
        "sudo_source",
        "sudo_profile",
        "sudo_password",
        "sudo_use_pty",
        "sudo_whitelist",
        "auth_flow_mode",
        "totp_secret",
        "totp_prompt_hint",
        "password_prompt_hint",
        "advanced_options",
        "read_only",
        "connect_timeout_secs",
        "keepalive_interval_secs",
        "terminal_keepalive_secs",
        "set_env",
        "triggers_enabled",
        "triggers",
        "trigger_answer_1",
        "trigger_answer_2",
        "passphrase_command",
        "remote_command",
        // M32-B：serial/rdp 连接类型字段（协议门控见各字段 visible_when；
        // 解析面只消费 external_config.protocol，其余字段由前端路由读取）。
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
        // 统一 RemoteX 连接把本地 Shell 折叠为 protocol=local 的一组字段。
        "shell",
        "cwd",
        "env",
    ];
    assert_eq!(keys, expected, "manifest field list drifted from parsing");

    // secret binding 只允许落在凭据字段；config binding 不得承载凭据语义。
    // private_key 承载表单粘贴的私钥内容（外部工具写入的存量 secret 也
    // 从这里生效），必须被 provider 声明，否则宿主校验拒绝整个连接。
    // trigger_answer_1/2 是触发器密文槽位（sendSecretKey 引用）。
    let secret_keys = [
        "password",
        "private_key_passphrase",
        "private_key",
        "sudo_password",
        "totp_secret",
        "trigger_answer_1",
        "trigger_answer_2",
    ];
    for field in fields {
        let key = field["key"].as_str().unwrap();
        match field["binding"].as_str() {
            Some("secret") => assert!(
                secret_keys.contains(&key),
                "unexpected secret binding: {key}"
            ),
            Some("config") => assert!(
                !key.contains("password")
                    || key == "password_prompt_hint"
                    || key == "password_command"
                    // A selector ("direct"/"command"), never credential
                    // material: both halves stay secret/config bound.
                    || key == "password_source",
                "config binding must not carry credential material: {key}"
            ),
            _ => {}
        }
    }

    // 登录密码走显式「密码来源」二选一（password_source）：
    //   direct  → password 必填；command → password_command 必填。
    // 两个分支合起来必须覆盖 password_source 的全部取值，否则某个选项
    // 会变成「两边都不需要」或「两边都要」，前者保存后被解析层拒绝、后者
    // 死锁（宿主契约只能表达单字段 required_when，做不到「除非另一字段有
    // 值」，所以来源必须由用户显式选）。私钥那组仍是「路径∨内容」，
    // manifest 表达不了 OR，保持两边都非必填（见
    // credential_requirements_stay_parse_time_only）。
    let password_source = fields
        .iter()
        .find(|field| field["key"] == "password_source")
        .expect("password_source field");
    let source_options: Vec<String> = password_source["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|option| option["value"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(password_source["default"], "direct");
    assert_eq!(source_options, ["direct", "command"]);
    // required_when 支持两种形态:单字段门控 { field, one_of } 与协议门控
    // (M9)后的 all_of 叠加。归一成 (field, one_of) 列表统一断言。
    let required_gates = |key: &str| -> Vec<(String, Vec<String>)> {
        let gate = &fields.iter().find(|field| field["key"] == key).unwrap()["required_when"];
        let clauses: Vec<serde_json::Value> = match gate.get("all_of") {
            Some(all) => all.as_array().cloned().unwrap_or_default(),
            None if gate.is_object() => vec![gate.clone()],
            None => vec![],
        };
        clauses
            .iter()
            .map(|clause| {
                (
                    clause["field"].as_str().unwrap_or_default().to_string(),
                    clause["one_of"]
                        .as_array()
                        .map(|values| {
                            values
                                .iter()
                                .map(|v| v.as_str().unwrap().to_string())
                                .collect()
                        })
                        .unwrap_or_default(),
                )
            })
            .collect()
    };
    // password:仅 SSH 且「表单直填」时必填——telnet/vnc/rdp 直接使用该
    // 密码但不强制(serial 不显示),故协议门控叠加在来源门控之外。
    assert_eq!(
        required_gates("password"),
        vec![
            ("protocol".to_string(), vec!["ssh".to_string()]),
            ("password_source".to_string(), vec!["direct".to_string()]),
        ],
        "password must be required only for SSH with the direct password source"
    );
    assert_eq!(
        required_gates("password_command"),
        vec![("password_source".to_string(), vec!["command".to_string()])],
    );
    // 分支覆盖不变式:两个来源分支的 one_of 合计必须恰好覆盖
    // password_source 的全部取值,否则某选项「两边都不需要」或「两边都要」。
    // 覆盖不变式只看来源门控本身:password 分支的协议门控(protocol∈ssh)
    // 是叠加条件,不参与 password_source 的取值覆盖。
    let mut covered: Vec<String> = required_gates("password")
        .into_iter()
        .chain(required_gates("password_command"))
        .filter(|(field, _)| field == "password_source")
        .flat_map(|(_, values)| values)
        .collect();
    covered.sort();
    let mut expected = source_options.clone();
    expected.sort();
    assert_eq!(
        covered, expected,
        "password_source branches must cover every option exactly once"
    );
    // private_key_path 是「路径或私钥内容」二选一：无 required_when，
    // 凭据齐全性由解析层校验（见 private_key_accepts_path_or_content）。
    assert!(
        fields
            .iter()
            .find(|field| field["key"] == "private_key_path")
            .unwrap()["required_when"]
            .is_null(),
        "private_key_path must not force the path when key content may be pasted instead"
    );

    // sudo 覆盖簇跟随表单选定的凭据来源（sudo_source）：自定义模式下才
    // 出现本连接密码/PTY；global 模式出现全局配置引用；2FA 编排字段
    // （totp_secret/auth_flow_mode/hints）服务登录期 keyboard-interactive，
    // global 模式下整体由全局配置接管故隐藏，off/custom 模式仍常显。
    // 协议门控（M9）后 SSH 字段的 visible_when 为 all_of（protocol=ssh
    // 叠加原条件）；改用完整结构比对，含 protocol 子句。
    let visible_when = |key: &str| -> serde_json::Value {
        fields
            .iter()
            .find(|field| field["key"] == key)
            .unwrap()
            .get("visible_when")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };
    let protocol_gate = serde_json::json!({ "field": "protocol", "one_of": ["ssh"] });
    assert_eq!(
        visible_when("sudo_password"),
        serde_json::json!({
            "all_of": [
                protocol_gate,
                { "field": "sudo_source", "one_of": ["custom"] },
            ]
        }),
        "sudo_password must stay gated on protocol=ssh + sudo_source=custom"
    );
    assert_eq!(
        visible_when("sudo_use_pty"),
        serde_json::json!({
            "all_of": [
                protocol_gate,
                { "field": "sudo_source", "one_of": ["custom"] },
            ]
        }),
        "sudo_use_pty must stay gated on protocol=ssh + sudo_source=custom"
    );
    assert_eq!(
        visible_when("sudo_profile"),
        serde_json::json!({
            "all_of": [
                protocol_gate,
                { "field": "sudo_source", "one_of": ["global"] },
            ]
        }),
        "sudo_profile must show only for sudo_source=global"
    );
    let key = "auth_flow_mode";
    assert_eq!(
        visible_when(key),
        serde_json::json!({
            "all_of": [
                protocol_gate,
                { "field": "sudo_source", "one_of": ["custom", "off"] },
            ]
        }),
        "{key} must hide under sudo_source=global (the bound profile owns the whole credential source) and stay visible otherwise"
    );
    // password_prompt_hint 属于 2FA 三件套（TOTP 密钥 / OTP 提示词 / 密码
    // 提示词）：跟随 sudo 来源，且只在自动回码的两种 OTP 模式下出现——
    // 2FA 关闭时整组折叠，不再残留一行孤零零的密码提示词。
    assert_eq!(
        fields
            .iter()
            .find(|field| field["key"] == "password_prompt_hint")
            .unwrap()["visible_when"],
        serde_json::json!({
            "all_of": [
                { "field": "protocol", "one_of": ["ssh"] },
                { "field": "sudo_source", "one_of": ["custom", "off"] },
                { "field": "auth_flow_mode", "one_of": ["password_then_otp", "password_plus_otp"] },
            ]
        }),
        "password_prompt_hint must follow the sudo source and fold with the 2FA trio when OTP auto-answer is off"
    );
    // passphrase_command 只服务密钥解密：密码 / agent 认证下是死 UI，
    // 需要同时满足高级区与密钥类认证（Auto 按序回退同样解密密钥，纳入
    // 密钥类门控）。
    assert_eq!(
        fields
            .iter()
            .find(|field| field["key"] == "passphrase_command")
            .unwrap()["visible_when"],
        serde_json::json!({
            "all_of": [
                protocol_gate,
                { "field": "advanced_options", "one_of": ["true"] },
                { "field": "authentication", "one_of": ["private-key", "private-key-password", "auto"] },
            ]
        }),
        "passphrase_command must combine the advanced switch with key-based auth"
    );
    for key in ["totp_secret", "totp_prompt_hint"] {
        assert_eq!(
            visible_when(key),
            serde_json::json!({
                "all_of": [
                    protocol_gate,
                    { "field": "auth_flow_mode", "one_of": ["password_then_otp", "password_plus_otp"] },
                ]
            }),
            "{key} must be gated on protocol=ssh + auth_flow_mode OTP auto-answer"
        );
    }

    // 2FA 关闭选项（0.4.77）：表单默认 off（新连接不自动回 OTP）；
    // 存量连接带显式 auth_flow_mode 值不受新默认影响，缺省字段在
    // 解析层仍落到 PasswordThenOtp（见 from_lifecycle_params）。
    let auth_flow = fields
        .iter()
        .find(|field| field["key"] == "auth_flow_mode")
        .unwrap();
    assert_eq!(
        auth_flow["default"], "off",
        "new connections default to 2FA off"
    );
    let flow_options: Vec<&str> = auth_flow["options"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|option| option["value"].as_str())
        .collect();
    assert_eq!(
        flow_options,
        [
            "off",
            "password_then_otp",
            "password_plus_otp",
            "password_only"
        ],
        "off must be the first (default) 2FA flow option"
    );
}

#[test]
fn auth_method_names_round_trip_for_display() {
    for name in [
        "password",
        "private-key",
        "private-key-password",
        "agent",
        "auto",
        "none",
    ] {
        // Name round-trip is a pure enum mapping; credential validation is
        // exercised separately by the connection parsing tests.
        let method = AuthenticationMethod::from_method_name(name);
        assert_eq!(method.method_name(), name);
    }
}

#[test]
fn sudo_password_is_a_credential_and_keeps_edge_whitespace() {
    // 第 3 轮对抗审查遗留项：跳板/私钥口令已原样读取，sudo_password 同样
    // 不在解析层 trim（首尾空格是合法密码字符）；纯空白语义由
    // SudoAuth::new 的「空白即回退」兜底，不受影响。
    let connection = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "sudo-ws",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "login",
            "connection_secrets": { "sudo_password": " padded " }
        }
    }))
    .unwrap();
    assert_eq!(connection.sudo_password, " padded ");

    let blank = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "sudo-blank",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "login",
            "connection_secrets": { "sudo_password": "   " }
        }
    }))
    .unwrap();
    assert_eq!(blank.sudo_password, "   ");
}

#[test]
fn sudo_source_maps_three_modes_with_legacy_fallback() {
    let parse = |external_config: serde_json::Value| {
        StoredConnection::from_lifecycle_params(&serde_json::json!({
            "connection": {
                "id": "sudo-source",
                "host": "example.com",
                "port": 22,
                "username": "user",
                "password": "login",
                "external_config": external_config
            }
        }))
        .unwrap()
    };

    // 表单三选一：off / custom / global（global 附带全局配置引用，trim 后生效）。
    assert_eq!(
        parse(serde_json::json!({ "sudo_source": "off" })).sudo_source,
        SudoSource::Off
    );
    assert_eq!(
        parse(serde_json::json!({ "sudo_source": "custom" })).sudo_source,
        SudoSource::Custom
    );
    let global = parse(serde_json::json!({
        "sudo_source": "global",
        "sudo_profile": " ops "
    }));
    assert_eq!(global.sudo_source, SudoSource::Global);
    assert_eq!(global.sudo_profile_ref, "ops");
    assert!(global.sudo_enabled());
    assert!(!parse(serde_json::json!({ "sudo_source": "off" })).sudo_enabled());

    // 旧连接无 sudo_source：由 quick_sudo 布尔映射，保持原行为。
    assert_eq!(
        parse(serde_json::json!({ "quick_sudo": true })).sudo_source,
        SudoSource::Custom
    );
    assert_eq!(
        parse(serde_json::json!({ "quick_sudo": false })).sudo_source,
        SudoSource::Off
    );
    assert_eq!(parse(serde_json::json!({})).sudo_source, SudoSource::Custom);

    // 显式 sudo_source 优先于遗留 quick_sudo（新表单保存后旧键残留）。
    assert_eq!(
        parse(serde_json::json!({ "sudo_source": "off", "quick_sudo": true })).sudo_source,
        SudoSource::Off
    );
    assert_eq!(
        parse(serde_json::json!({ "sudo_source": "custom", "quick_sudo": false })).sudo_source,
        SudoSource::Custom
    );

    // 空值/未知值回落 legacy。
    assert_eq!(
        parse(serde_json::json!({ "sudo_source": "", "quick_sudo": false })).sudo_source,
        SudoSource::Off
    );
    assert_eq!(
        parse(serde_json::json!({ "sudo_source": "bogus", "quick_sudo": true })).sudo_source,
        SudoSource::Custom
    );
}

#[test]
fn parses_private_key_password_credentials_from_separate_stores() {
    let connection = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "key-password",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "fallback",
            "external_config": {
                "authentication": "private-key-password",
                "private_key_path": "C:/keys/id_ed25519"
            },
            "connection_secrets": {
                "private_key_passphrase": "key-secret"
            }
        }
    }))
    .unwrap();

    assert_eq!(
        connection.authentication,
        AuthenticationMethod::PrivateKeyPassword
    );
    assert_eq!(connection.private_key_path, "C:/keys/id_ed25519");
    assert_eq!(connection.private_key_passphrase, "key-secret");
}

#[test]
fn runtime_endpoint_parses_non_ssh_lifecycle_without_ssh_credentials() {
    let endpoint = RuntimeEndpoint::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "telnet-tunnel",
            "host": "logical.telnet.internal",
            "port": 23,
            "external_config": { "protocol": "telnet" }
        },
        "runtime": { "host": "127.0.0.1", "port": 39123 }
    }))
    .expect("Telnet lifecycle endpoint must not require SSH credentials");

    assert_eq!(endpoint.connection_id, "telnet-tunnel");
    assert_eq!(endpoint.protocol, "telnet");
    assert_eq!(endpoint.host, "logical.telnet.internal");
    assert_eq!(endpoint.port, 23);
    assert_eq!(endpoint.runtime_host, "127.0.0.1");
    assert_eq!(endpoint.runtime_port, 39123);
}

#[test]
fn uses_the_host_runtime_endpoint_without_changing_host_key_identity() {
    let connection = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "tunneled",
            "host": "target.internal",
            "port": 22,
            "username": "user",
            "password": "secret"
        },
        "runtime": {
            "host": "127.0.0.1",
            "port": 39122
        }
    }))
    .unwrap();

    assert_eq!(connection.host, "target.internal");
    assert_eq!(connection.port, 22);
    assert_eq!(connection.runtime_host, "127.0.0.1");
    assert_eq!(connection.runtime_port, 39122);
}

#[test]
fn lifecycle_parser_preserves_utf8_connection_text_and_windows_paths() {
    let payload = serde_json::json!({
        "connection": {
            "id": "windows-unicode",
            "name": "生产机-北京",
            "host": "2001:db8::42",
            "port": 2222,
            "username": "运维",
            "password": "密码🔐",
            "external_config": {
                "authentication": "private-key",
                "private_key_path": "C:\\Users\\测试\\.ssh\\id_ed25519",
                "setEnv": "LANG=zh_CN.UTF-8\nDBX_LABEL=应用服务器",
                "remoteCommand": "printf '已连接\\n'"
            },
            "connection_secrets": {
                "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\r\n测试\r\n-----END OPENSSH PRIVATE KEY-----\r\n"
            }
        },
        "runtime": { "host": "127.0.0.1", "port": 39222 }
    });
    // Exercise the same UTF-8 JSON boundary used by the host bridge, not
    // only the in-memory `serde_json::Value` representation.
    let encoded = serde_json::to_vec(&payload).unwrap();
    let decoded: Value = serde_json::from_slice(&encoded).unwrap();
    let connection = StoredConnection::from_lifecycle_params(&decoded).unwrap();

    // The logical endpoint is used for host-key identity; the runtime
    // endpoint is the actual dial target supplied by the host tunnel.
    assert_eq!(connection.name.as_deref(), Some("生产机-北京"));
    assert_eq!(connection.host, "2001:db8::42");
    assert_eq!(connection.port, 2222);
    assert_eq!(connection.runtime_host, "127.0.0.1");
    assert_eq!(connection.runtime_port, 39222);
    assert_eq!(connection.username, "运维");
    assert_eq!(connection.password, "密码🔐");
    assert_eq!(
        connection.private_key_path,
        "C:\\Users\\测试\\.ssh\\id_ed25519"
    );
    assert_eq!(
        connection.private_key,
        "-----BEGIN OPENSSH PRIVATE KEY-----\r\n测试\r\n-----END OPENSSH PRIVATE KEY-----\r\n"
    );
    assert_eq!(
        connection.set_env,
        vec![
            ("LANG".to_string(), "zh_CN.UTF-8".to_string()),
            ("DBX_LABEL".to_string(), "应用服务器".to_string()),
        ]
    );
    assert_eq!(connection.remote_command, "printf '已连接\\n'");
}

#[test]
fn lifecycle_parser_accepts_network_address_forms_and_rejects_ambiguous_hosts() {
    for host in [
        "127.0.0.1",
        "ssh.example.test",
        "2001:db8::1",
        "[2001:db8::1]",
    ] {
        let connection = StoredConnection::from_lifecycle_params(&serde_json::json!({
            "connection": {
                "id": "address",
                "host": host,
                "port": 22,
                "username": "user",
                "password": "secret"
            }
        }))
        .unwrap();
        assert_eq!(connection.host, host);
    }

    for host in ["my server", "ssh://server.example.test:22", "server\nname"] {
        let result = StoredConnection::from_lifecycle_params(&serde_json::json!({
            "connection": {
                "id": "address",
                "host": host,
                "port": 22,
                "username": "user",
                "password": "secret"
            }
        }));
        assert!(result.is_err(), "ambiguous host must fail fast: {host:?}");
    }

    for port in [0_u64, 65_536] {
        let result = StoredConnection::from_lifecycle_params(&serde_json::json!({
            "connection": {
                "id": "port",
                "host": "server.example.test",
                "port": port,
                "username": "user",
                "password": "secret"
            }
        }));
        assert!(result.is_err(), "invalid port must fail fast: {port}");
    }
}

#[test]
fn sftp_uri_roundtrip_keeps_utf8_paths_and_confines_parent_segments() {
    let path = "/home/运维/../应用/日志.txt";
    assert_eq!(
        path_from_sftp_uri(&sftp_uri(path)).unwrap(),
        "/home/应用/日志.txt"
    );
    assert_eq!(
        path_from_sftp_uri("sftp:relative/目录/./文件.txt").unwrap(),
        "/relative/目录/文件.txt"
    );
}

#[test]
fn parses_jump_host_chains() {
    let connection = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "jumped",
            "host": "target.internal",
            "port": 22,
            "username": "user",
            "password": "secret",
            "external_config": {
                "jump_hosts": [
                    { "host": "bastion.example.com", "port": 2202, "username": "ops", "password": "jump-pw" },
                    { "host": "inner.example.com", "authentication": "private-key", "username": "relay", "private_key_path": "~/.ssh/id_ed25519" }
                ]
            }
        }
    }))
    .unwrap();

    assert_eq!(connection.jump_hosts.len(), 2);
    assert_eq!(connection.jump_hosts[0].port, 2202);
    assert_eq!(connection.jump_hosts[0].password, "jump-pw");
    assert_eq!(connection.jump_hosts[1].authentication, "private-key");
    let synthesized = connection.jump_hosts[1].to_connection("jump-2", 20, 30);
    assert_eq!(synthesized.host, "inner.example.com");
    assert_eq!(synthesized.runtime_port, 22);
    assert_eq!(synthesized.authentication, AuthenticationMethod::PrivateKey);
}

#[test]
fn rejects_invalid_jump_host_chains() {
    let with_missing_password = serde_json::json!({
        "connection": {
            "id": "x", "host": "t", "port": 22, "username": "u", "password": "p",
            "external_config": { "jump_hosts": [ { "host": "bastion", "port": 22, "username": "ops" } ] }
        }
    });
    assert!(StoredConnection::from_lifecycle_params(&with_missing_password).is_err());

    let too_many = serde_json::json!({
        "connection": {
            "id": "x", "host": "t", "port": 22, "username": "u", "password": "p",
            "external_config": {
                "jump_hosts": [
                    { "host": "a", "port": 22, "username": "u", "password": "p" },
                    { "host": "b", "port": 22, "username": "u", "password": "p" },
                    { "host": "c", "port": 22, "username": "u", "password": "p" },
                    { "host": "d", "port": 22, "username": "u", "password": "p" }
                ]
            }
        }
    });
    assert!(StoredConnection::from_lifecycle_params(&too_many).is_err());
}

#[test]
fn rejects_adversarial_jump_host_fields() {
    let build = |jump: Value| {
        StoredConnection::from_lifecycle_params(&serde_json::json!({
            "connection": {
                "id": "x", "host": "t", "port": 22, "username": "u", "password": "p",
                "external_config": { "jump_hosts": [jump] }
            }
        }))
    };
    // Port boundaries.
    for port in [0_u64, 65536, 70000] {
        assert!(
            build(serde_json::json!({ "host": "bastion", "port": port, "username": "u", "password": "p" }))
                .is_err(),
            "port {port} must be rejected"
        );
    }
    // Empty/whitespace username.
    assert!(build(serde_json::json!({ "host": "bastion", "username": "  " })).is_err());
    // Host with internal whitespace or URI syntax must fail fast instead
    // of becoming a bogus TCP target.
    assert!(
        build(serde_json::json!({ "host": "my bastion", "username": "u", "password": "p" }))
            .is_err()
    );
    assert!(build(
        serde_json::json!({ "host": "ssh://bastion:22", "username": "u", "password": "p" })
    )
    .is_err());
    // The target host field is held to the same rules.
    let bad_target = serde_json::json!({
        "connection": {
            "id": "x", "host": "tar get", "port": 22, "username": "u", "password": "p"
        }
    });
    assert!(StoredConnection::from_lifecycle_params(&bad_target).is_err());
}

#[test]
fn rejects_repeated_jump_hops() {
    let repeated = serde_json::json!({
        "connection": {
            "id": "x", "host": "t", "port": 22, "username": "u", "password": "p",
            "external_config": {
                "jump_hosts": [
                    { "host": "bastion", "port": 2202, "username": "u", "password": "p" },
                    { "host": "bastion", "port": 2202, "username": "u", "password": "p" }
                ]
            }
        }
    });
    let error = StoredConnection::from_lifecycle_params(&repeated).unwrap_err();
    assert!(error.contains("repeats an earlier hop"), "{error}");
    // Same host on a different port is still a valid chain.
    let distinct = serde_json::json!({
        "connection": {
            "id": "x", "host": "t", "port": 22, "username": "u", "password": "p",
            "external_config": {
                "jump_hosts": [
                    { "host": "bastion", "port": 2202, "username": "u", "password": "p" },
                    { "host": "bastion", "port": 2203, "username": "u", "password": "p" }
                ]
            }
        }
    });
    assert!(StoredConnection::from_lifecycle_params(&distinct).is_ok());
}

#[test]
fn jump_credentials_survive_special_characters_verbatim() {
    let connection = StoredConnection::from_lifecycle_params(&serde_json::json!({
        "connection": {
            "id": "x", "host": "t", "port": 22, "username": "u", "password": "p",
            "external_config": {
                "jump_hosts": [
                    { "host": "bastion", "username": "u",
                      "password": "  p@$$w0rd 'with' \"quotes\" $backslash\\ ",
                      "authentication": "private-key",
                      "private_key_path": "~/.ssh/id_ed25519",
                      "private_key_passphrase": "  phrase with spaces  " }
                ]
            }
        }
    }))
    .unwrap();
    assert_eq!(
        connection.jump_hosts[0].password,
        "  p@$$w0rd 'with' \"quotes\" $backslash\\ "
    );
    assert_eq!(
        connection.jump_hosts[0].private_key_passphrase,
        "  phrase with spaces  "
    );
}
