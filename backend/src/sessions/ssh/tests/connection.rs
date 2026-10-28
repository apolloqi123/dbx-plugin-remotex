use super::*;

#[test]
fn method_offered_reads_only_the_server_advertised_set() {
    let offered = auth_failure(&[MethodKind::KeyboardInteractive], false);
    assert!(method_offered(&offered, MethodKind::KeyboardInteractive));
    // Tabby parity (authMethodSelection): a method the server did not
    // advertise must never be tried — blind retries trip MaxAuthTries.
    assert!(!method_offered(&offered, MethodKind::Password));
    assert!(!method_offered(&offered, MethodKind::PublicKey));
}

#[test]
fn method_offered_is_false_for_success_and_empty_advertisements() {
    assert!(!method_offered(&AuthResult::Success, MethodKind::Password));
    assert!(!method_offered(
        &auth_failure(&[], false),
        MethodKind::KeyboardInteractive
    ));
}

#[test]
fn auth_partial_success_only_fires_when_the_server_flags_it() {
    // koko / PAM 2FA shape: first factor accepted, second factor demanded.
    assert!(auth_partial_success(&auth_failure(
        &[MethodKind::KeyboardInteractive],
        true
    )));
    assert!(!auth_partial_success(&auth_failure(
        &[MethodKind::Password],
        false
    )));
    assert!(!auth_partial_success(&AuthResult::Success));
}

#[test]
fn reconcile_advertised_requires_both_advertisements_to_list_the_method() {
    // none 探测广告 password + KI；密码步之后服务器只放行 KI（koko
    // 第一因子已过的形状）：KI 双份广告集都在 → 可试；password 被逐步
    // 广告集收走 → 不可再试。
    let none = method_set(&[MethodKind::Password, MethodKind::KeyboardInteractive]);
    let step = auth_failure(&[MethodKind::KeyboardInteractive], true);
    assert!(reconcile_advertised(
        Some(&none),
        &step,
        MethodKind::KeyboardInteractive
    ));
    assert!(!reconcile_advertised(
        Some(&none),
        &step,
        MethodKind::Password
    ));
}

#[test]
fn reconcile_advertised_lets_the_none_probe_veto_later_methods() {
    // 严格服务器：none 探测只广告 publickey。即使后续逐步广告集放行
    // password/KI，也不得盲试 —— Tabby authMethodSelection 语义，防止
    // 把 MaxAuthTries 预算烧在服务器根本不开的方法上。
    let none = method_set(&[MethodKind::PublicKey]);
    let step = auth_failure(
        &[MethodKind::Password, MethodKind::KeyboardInteractive],
        false,
    );
    assert!(!reconcile_advertised(
        Some(&none),
        &step,
        MethodKind::Password
    ));
    assert!(!reconcile_advertised(
        Some(&none),
        &step,
        MethodKind::KeyboardInteractive
    ));
}

#[test]
fn reconcile_advertised_takes_the_stricter_side_on_conflicting_sets() {
    // 两份广告集互相冲突：none 只广告 password，逐步广告集只广告 KI。
    // 合并判定必须两边都放行才可试，因此两个方法都被否掉。
    let none = method_set(&[MethodKind::Password]);
    let step = auth_failure(&[MethodKind::KeyboardInteractive], true);
    assert!(!reconcile_advertised(
        Some(&none),
        &step,
        MethodKind::Password
    ));
    assert!(!reconcile_advertised(
        Some(&none),
        &step,
        MethodKind::KeyboardInteractive
    ));
}

#[test]
fn reconcile_advertised_fails_open_without_a_none_advertisement() {
    // 异常形状（拿不到 none 广告集）→ 回退旧行为：只看逐步广告集。
    let step = auth_failure(&[MethodKind::Password], false);
    assert!(reconcile_advertised(None, &step, MethodKind::Password));
    assert!(!reconcile_advertised(
        None,
        &step,
        MethodKind::KeyboardInteractive
    ));
    assert!(!reconcile_advertised(
        None,
        &AuthResult::Success,
        MethodKind::Password
    ));
}

#[test]
fn advertised_methods_maps_empty_and_non_failure_shapes_to_none() {
    // russh 在传输层断开时也回 Failure{空集}：空集信息量为零，必须判为
    // "无广告集"，否则幽灵限制会把登录 fail-closed 卡死。
    assert!(advertised_methods(&auth_failure(&[], false)).is_none());
    assert!(advertised_methods(&AuthResult::Success).is_none());
    let rejection = auth_failure(&[MethodKind::Password], true);
    let none =
        advertised_methods(&rejection).expect("a non-empty rejection carries the advertisement");
    assert!(none.contains(&MethodKind::Password));
}

#[test]
fn advertisement_rejection_message_keeps_the_base_text_and_appends_the_set() {
    let base = "SSH server did not advertise password or keyboard-interactive authentication; refusing to send the password";
    assert_eq!(advertisement_rejection_message(None), base);
    let none = method_set(&[MethodKind::PublicKey]);
    let message = advertisement_rejection_message(Some(&none));
    assert!(
        message.starts_with(base),
        "the historical message must stay a prefix: {message}"
    );
    assert!(
        message.contains("publickey"),
        "the server's advertisement must be named: {message}"
    );
}

#[test]
fn agent_planner_stops_at_the_identity_budget() {
    // OpenSSH MaxAuthTries defaults to 6 *total* failures including the
    // leading none probe, so the 5th identity attempt is the last one.
    let full_rejection = auth_failure(&[MethodKind::PublicKey], false);
    assert_eq!(
        plan_agent_identity_attempt(AGENT_IDENTITY_MAX_ATTEMPTS - 1, Some(&full_rejection), None),
        AgentPlan::TryNext,
        "budget not yet exhausted: the next identity may still be tried"
    );
    match plan_agent_identity_attempt(AGENT_IDENTITY_MAX_ATTEMPTS, Some(&full_rejection), None) {
        AgentPlan::StopRejection(message) => assert!(
            message.contains(
                "server may have disconnected after too many auth attempts (MaxAuthTries)"
            ) && message.contains("consider specifying a private key"),
            "the budget message must name MaxAuthTries and the private-key workaround: {message}"
        ),
        other => panic!("the exhausted budget must stop the loop, got {other:?}"),
    }
}

#[test]
fn agent_planner_stops_on_partial_success_advertising_keyboard_interactive() {
    // koko shape: identity accepted, MFA still owed. Advertising
    // keyboard-interactive while flagging partial success means the
    // identity landed — more identities could only burn the budget.
    let partial = auth_failure(&[MethodKind::KeyboardInteractive], true);
    assert_eq!(
        plan_agent_identity_attempt(0, Some(&partial), None),
        AgentPlan::StopPartialSuccess
    );
    // Without the KI advertisement (or the partial flag) a rejection is
    // full: the advertised set does not change between per-identity
    // attempts, so the planner must not over-read it.
    let full = auth_failure(&[MethodKind::Password], true);
    assert_eq!(
        plan_agent_identity_attempt(0, Some(&full), None),
        AgentPlan::TryNext
    );
    assert_eq!(
        plan_agent_identity_attempt(0, Some(&partial), None),
        AgentPlan::StopPartialSuccess
    );
    assert_eq!(
        plan_agent_identity_attempt(
            0,
            Some(&auth_failure(&[MethodKind::PublicKey], false)),
            None
        ),
        AgentPlan::TryNext
    );
}

#[test]
fn agent_planner_stops_on_first_transport_error_and_surfaces_it() {
    // All identities share the same agent handle: after the first
    // transport error no remaining attempt can recover, and the surfaced
    // message must carry the original error text.
    let plan = plan_agent_identity_attempt(
        1,
        Some(&auth_failure(&[MethodKind::PublicKey], false)),
        Some("agent signature request failed: connection reset"),
    );
    match plan {
        AgentPlan::StopRejection(message) => assert_eq!(
            message, "agent signature request failed: connection reset",
            "the first error must be surfaced verbatim"
        ),
        other => panic!("a transport error must stop the loop, got {other:?}"),
    }
    // Even a budget-exhausted plan defers to the concrete error.
    assert_eq!(
        plan_agent_identity_attempt(AGENT_IDENTITY_MAX_ATTEMPTS, None, Some("any error")),
        AgentPlan::StopRejection("any error".to_string())
    );
}

#[test]
fn agent_planner_tries_next_while_budget_remains() {
    let rejection = auth_failure(&[MethodKind::PublicKey], false);
    assert_eq!(
        plan_agent_identity_attempt(0, None, None),
        AgentPlan::TryNext,
        "before the first attempt the planner has nothing to stop on"
    );
    assert_eq!(
        plan_agent_identity_attempt(0, Some(&rejection), None),
        AgentPlan::TryNext
    );
    assert_eq!(
        plan_agent_identity_attempt(AGENT_IDENTITY_MAX_ATTEMPTS - 1, Some(&rejection), None),
        AgentPlan::TryNext
    );
}

#[test]
fn normalized_spawn_command_treats_absent_and_blank_as_no_command() {
    assert_eq!(normalized_spawn_command(None).unwrap(), None);
    assert_eq!(normalized_spawn_command(Some("")).unwrap(), None);
    assert_eq!(normalized_spawn_command(Some("   ")).unwrap(), None);
    assert_eq!(normalized_spawn_command(Some(" \t\r\n ")).unwrap(), None);
}

#[test]
fn normalized_spawn_command_trims_and_keeps_inner_whitespace() {
    assert_eq!(
        normalized_spawn_command(Some("  htop  "))
            .unwrap()
            .as_deref(),
        Some("htop")
    );
    assert_eq!(
        normalized_spawn_command(Some("tail -f /var/log/syslog"))
            .unwrap()
            .as_deref(),
        Some("tail -f /var/log/syslog")
    );
}

#[test]
fn normalized_spawn_command_fails_closed_on_nul_and_oversize() {
    let nul_error = normalized_spawn_command(Some("echo a\0b")).unwrap_err();
    assert_eq!(nul_error, "spawnCommand must not contain NUL bytes");

    let oversized = "x".repeat(SPAWN_COMMAND_MAX_BYTES + 1);
    let error = normalized_spawn_command(Some(&oversized)).unwrap_err();
    assert_eq!(
        error,
        format!("spawnCommand exceeds the {SPAWN_COMMAND_MAX_BYTES} byte limit")
    );
    // At-cap commands are accepted; the cap counts UTF-8 bytes.
    let at_cap = "x".repeat(SPAWN_COMMAND_MAX_BYTES);
    assert!(normalized_spawn_command(Some(&at_cap)).unwrap().is_some());
    let at_cap_multibyte = "中".repeat(SPAWN_COMMAND_MAX_BYTES / 3);
    assert!(normalized_spawn_command(Some(&at_cap_multibyte))
        .unwrap()
        .is_some());
}

#[test]
fn spawn_exec_payload_single_quote_escapes_and_preserves_shell_semantics() {
    // Repo hard rule: remote commands are single-quote escaped. The
    // payload is one `sh -c '<escaped>'` argument for the outer shell;
    // the inner sh keeps pipes and redirects intact.
    assert_eq!(spawn_exec_payload("htop"), "sh -c 'htop'");
    assert_eq!(
        spawn_exec_payload("it's"),
        "sh -c 'it'\\''s'",
        "embedded single quotes use the '\\'' escape (exec::shell_quote)"
    );
    assert_eq!(
        spawn_exec_payload("echo a && cat /etc/passwd | wc -l > /tmp/x; rm -rf '$(pwd)'"),
        "sh -c 'echo a && cat /etc/passwd | wc -l > /tmp/x; rm -rf '\\''$(pwd)'\\'''"
    );
    // The metacharacters stay inside the quoted payload: nothing outside
    // the single quotes except the fixed `sh -c ` prefix.
    let payload = spawn_exec_payload("a; b | c & d");
    assert!(payload.starts_with("sh -c '") && payload.ends_with("'"));
}

#[test]
fn test_connection_budget_aligns_with_host_deadline() {
    assert_eq!(test_dial_budget_secs(30, true), 29);
    assert_eq!(test_dial_budget_secs(30, false), 9);
    assert_eq!(test_dial_budget_secs(1, true), 1);
}

#[test]
fn next_test_budget_extends_once_for_dialog_path() {
    // Without a host dialog in play the probe keeps the current readable
    // timeout error (1.0 hosts, workbench fallback, exhausted extension).
    assert_eq!(next_test_budget(9, false), None);
    assert_eq!(next_test_budget(29, false), None);
    // Dialog in flight: one extension by the challenge wait plus the
    // connect timeout.
    assert_eq!(
        next_test_budget(9, true),
        Some(HOST_KEY_CHALLENGE_WAIT.as_secs() + 9)
    );
    // A zero stored connect timeout must not shrink the window.
    assert_eq!(
        next_test_budget(0, true),
        Some(HOST_KEY_CHALLENGE_WAIT.as_secs())
    );
}

#[test]
fn test_timeout_message_names_budget_source_and_remedy() {
    let explicit = test_timeout_message("dbx-ssh-test", 22, 29, false, false);
    assert_eq!(
        explicit,
        "SSH connection to dbx-ssh-test:22 timed out after 29 seconds. Increase 'SSH timeout' under Advanced options and retry."
    );
    let fallback = test_timeout_message("dbx-ssh-test", 22, 9, true, false);
    assert!(
        fallback.contains("timed out after 9 seconds (host default)"),
        "{fallback}"
    );
    assert!(
        fallback.contains("Increase 'SSH timeout' under Advanced options"),
        "{fallback}"
    );
}

#[test]
fn test_timeout_message_names_pending_host_key_confirmation() {
    let base = test_timeout_message("h", 22, 9, true, false);
    assert!(base.contains("timed out after 9 seconds (host default)"));
    assert!(base.contains("Increase 'SSH timeout'"));

    let with_challenge = test_timeout_message("h", 22, 9, false, true);
    assert!(with_challenge.contains("host-key confirmation"));
    assert!(!with_challenge.contains("(host default)"));
}

#[test]
fn negotiated_algorithms_cover_legacy_without_weakening_secure_order() {
    // 存量连接可能还带着已下线的 ssh_algorithm_profile/policy 配置值；
    // 算法面已统一，任何旧值都不得改变协商集合。
    let connection = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "unified-algos",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": "secret",
            "external_config": { "ssh_algorithm_profile": "legacy" }
        }
    }))
    .unwrap();
    let config = ssh_client_config(&connection);
    assert_eq!(config.preferred.mac.first(), Some(&mac::HMAC_SHA512_ETM));
    assert!(config.preferred.mac.contains(&mac::HMAC_SHA1));
    assert!(config.preferred.kex.contains(&kex::DH_G1_SHA1));
    assert!(config.preferred.cipher.contains(&cipher::AES_128_CBC));
    assert!(config.preferred.cipher.contains(&cipher::TRIPLE_DES_CBC));
    assert_eq!(config.gex.min_group_size(), 2048);
    assert_eq!(config.gex.preferred_group_size(), 3072);
    assert_eq!(config.gex.max_group_size(), 8192);
}

#[test]
fn client_config_advertises_a_large_receive_window_for_transfers() {
    // 下载吞吐上限 ≈ 接收窗口 / RTT：russh 默认 2MiB 在高 RTT 链路上把
    // 单文件下载压到 ~17MB/s，16MiB 才够喂满 40ms RTT 下的几百 MB/s。
    let connection = StoredConnection::from_lifecycle_params(&json!({
        "connection": { "id": "window", "host": "example.com", "port": 22, "username": "user", "password": "secret" }
    }))
    .unwrap();
    assert_eq!(ssh_client_config(&connection).window_size, 16 * 1024 * 1024);
}

// —— 私钥来源解析：粘贴内容优先于路径（connection_secrets.private_key）———

#[test]
fn private_key_text_normalizes_carriage_returns() {
    assert_eq!(
        normalize_private_key_text("a\r\nb\rc\nd"),
        "a\nb\nc\nd".to_string()
    );
    assert_eq!(normalize_private_key_text("plain\n"), "plain\n".to_string());
}

#[test]
fn private_key_text_normalizes_windows_bom_and_leading_whitespace() {
    let key = "\u{feff} \r\n-----BEGIN PRIVATE KEY-----\r\n\
        MC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/nwoy/HU++CXqI9EdVhC\r\n\
        -----END PRIVATE KEY-----\r\n";
    let normalized = normalize_private_key_text(key);

    assert!(normalized.starts_with("-----BEGIN PRIVATE KEY-----\n"));
    assert!(!normalized.contains('\r'));
    assert!(decode_secret_key(&normalized, None).is_ok());
}

// —— #21: 私钥解码错误分类与粘贴污损修复 ————————————————————————————
// 以下密钥全部为本仓库测试现场生成的废弃密钥或 ssh-key 上游公开测试
// fixture,绝不包含真实凭据。口令仅为测试值。

#[test]
fn paste_artifacts_are_normalized_and_then_decode() {
    let cases: Vec<(&str, String)> = vec![
        (
            "marker trailing space",
            OPENSSH_ED25519_PLAIN.replace(
                "-----BEGIN OPENSSH PRIVATE KEY-----",
                "-----BEGIN OPENSSH PRIVATE KEY----- ",
            ),
        ),
        (
            "marker inner double space",
            OPENSSH_ED25519_PLAIN.replace(
                "-----BEGIN OPENSSH PRIVATE KEY-----",
                "-----BEGIN  OPENSSH  PRIVATE  KEY-----",
            ),
        ),
        (
            "smart dashes",
            OPENSSH_ED25519_PLAIN
                .replace("-----BEGIN", "——–BEGIN")
                .replace("-----END", "——–END"),
        ),
        ("zero-width characters", {
            let mut text = OPENSSH_ED25519_PLAIN
                .replace("-----BEGIN OPENSSH", "-----\u{200b}BEGIN\u{200b} OPENSSH");
            text.insert(40, '\u{200b}');
            text
        }),
        (
            "web-page <br> tags",
            OPENSSH_ED25519_PLAIN
                .replace('\n', "<br>\n")
                .replacen("<br>", "<BR>", 1),
        ),
        ("re-wrapped body lines", {
            let mut lines: Vec<String> =
                OPENSSH_ED25519_PLAIN.lines().map(str::to_string).collect();
            lines[1].insert_str(24, "  ");
            lines[2].insert(24, '\t');
            lines.join("\n")
        }),
        (
            "crlf plus bom plus indent",
            format!(
                "\u{feff}   \r\n{}\r\n",
                OPENSSH_ED25519_PLAIN.replace('\n', "\r\n")
            ),
        ),
    ];
    for (label, polluted) in &cases {
        let normalized = normalize_private_key_text(polluted);
        let decoded = decode_secret_key(&normalized, None);
        assert!(decoded.is_ok(), "{label}: {normalized:?}");
    }
}

#[test]
fn paste_artifact_normalization_preserves_directive_lines() {
    // PKCS#5 传统加密 PEM 的 DEK-Info 指令行必须原样保留(含 IV 十六进制)。
    let text = "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\n\
        DEK-Info: AES-128-CBC,0123456789ABCDEF0123456789ABCDEF \n\nabc= \n\
        -----END RSA PRIVATE KEY-----\n";
    let normalized = normalize_private_key_text(text);
    assert!(normalized.contains("DEK-Info: AES-128-CBC,0123456789ABCDEF0123456789ABCDEF\n"));
    assert!(normalized.contains("Proc-Type: 4,ENCRYPTED\n"));
    assert!(normalized.contains("\nabc=\n"));
}

#[test]
fn decode_errors_are_classified_into_actionable_guidance() {
    // 加密 OpenSSH 密钥、未填口令。
    assert_actionable(
        OPENSSH_ED25519_ENCRYPTED,
        &decode_error_of(OPENSSH_ED25519_ENCRYPTED, None),
        false,
        "fill in the private key passphrase field",
    );
    // 加密 OpenSSH 密钥、口令错误。
    assert_actionable(
        OPENSSH_ED25519_ENCRYPTED,
        &decode_error_of(OPENSSH_ED25519_ENCRYPTED, Some("wrong-phrase")),
        true,
        "did not decrypt with the stored passphrase",
    );
    // 公钥单行形态被误当私钥。
    assert_actionable(
        ED25519_PUBLIC_ONELINER,
        &decode_error_of(ED25519_PUBLIC_ONELINER, None),
        false,
        "looks like a PUBLIC key or certificate",
    );
    assert_actionable(
        "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=\n-----END PUBLIC KEY-----",
        &decode_error_of(
            "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=\n-----END PUBLIC KEY-----",
            None,
        ),
        false,
        "looks like a PUBLIC key or certificate",
    );
    // 完全不是密钥文本。
    assert_actionable(
        "hello world, this is not a key",
        &decode_error_of("hello world, this is not a key", None),
        false,
        "no PEM private-key header",
    );
    // HTML 转义污染(无法自动修复,指引重新复制)。
    let html_escaped = "&lt;p&gt;-----BEGIN OPENSSH PRIVATE KEY-----&lt;/p&gt;";
    assert_actionable(
        html_escaped,
        &decode_error_of(html_escaped, None),
        false,
        "looks HTML-escaped",
    );
    // BEGIN 存在但标记无法修复(类型名内部断行)。
    let broken_marker =
        "-----BEGIN OPENSSH PRIVATE-KEY-----\nbody\n-----END OPENSSH PRIVATE-KEY-----";
    assert_actionable(
        broken_marker,
        &decode_error_of(broken_marker, None),
        false,
        "mangled by rich-text copy",
    );
    // SEC1 传统 EC PEM 自 russh 0.62 起已可直接解码，改归有效密钥用例。
    // PKCS#8 + 不支持的 PBKDF2 PRF。
    assert_actionable(
        PKCS8_PBES2_ENCRYPTED,
        &decode_error_of(PKCS8_PBES2_ENCRYPTED, Some(ENCRYPTED_TEST_PASSPHRASE)),
        true,
        "PBKDF2 variant",
    );
    // PuTTY PPK 加密、未填口令。
    assert_actionable(
        PPK3_ED25519_ENCRYPTED,
        &decode_error_of(PPK3_ED25519_ENCRYPTED, None),
        false,
        "this PuTTY PPK key is encrypted",
    );
    // PuTTY PPK 口令错误(MAC 校验失败)。
    assert_actionable(
        PPK3_ED25519_ENCRYPTED,
        &decode_error_of(PPK3_ED25519_ENCRYPTED, Some("wrong")),
        true,
        "the passphrase appears to be incorrect",
    );
    // PuTTY PPK 结构损坏。
    let corrupt_ppk = PPK3_ED25519_PLAIN.replace("Public-Lines: 2", "Public-Lines: 3");
    assert_actionable(
        &corrupt_ppk,
        &decode_error_of(&corrupt_ppk, None),
        false,
        "export it from PuTTYgen",
    );
    // base64 主体损坏(终端复制插空格、未走归一化的原始文本)。
    let mut lines: Vec<&str> = OPENSSH_ED25519_PLAIN.lines().collect();
    let mut second = lines[1].to_string();
    second.insert(20, ' ');
    lines[1] = &second;
    let spaced_body = lines.join("\n");
    assert_actionable(
        &spaced_body,
        &decode_error_of(&spaced_body, None),
        false,
        "not valid base64",
    );
    // 兜底:未知错误仍保留原文,只加一句说明。
    let junk_key = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----";
    assert_actionable(
        junk_key,
        &decode_error_of(junk_key, None),
        false,
        "the key could not be decoded",
    );
}

#[test]
fn valid_keys_still_decode_after_normalization() {
    assert!(decode_secret_key(&normalize_private_key_text(OPENSSH_ED25519_PLAIN), None).is_ok());
    assert!(decode_secret_key(
        &normalize_private_key_text(OPENSSH_ED25519_ENCRYPTED),
        Some(ENCRYPTED_TEST_PASSPHRASE)
    )
    .is_ok());
    assert!(decode_secret_key(&normalize_private_key_text(PPK3_ED25519_PLAIN), None).is_ok());
    assert!(decode_secret_key(
        &normalize_private_key_text(PPK3_ED25519_ENCRYPTED),
        Some("123")
    )
    .is_ok());
    // russh 0.62 起支持 SEC1 传统 EC PEM，不再作为解码失败场景。
    assert!(decode_secret_key(&normalize_private_key_text(EC_SEC1_PEM), None).is_ok());
    assert!(!looks_like_public_key(OPENSSH_ED25519_PLAIN));
    assert!(looks_like_public_key(ED25519_PUBLIC_ONELINER));
}

#[tokio::test]
async fn pasted_key_content_wins_over_key_path() {
    // 路径指向不存在的文件也能取到内容：content 非空时不碰文件系统。
    let connection = key_connection(
        "/definitely/missing/id_test",
        "-----BEGIN OPENSSH PRIVATE KEY-----\nbody\n-----END OPENSSH PRIVATE KEY-----\n",
    );
    let text = resolve_private_key_text(&connection).await.unwrap();
    assert!(text.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
    assert!(text.ends_with("-----END OPENSSH PRIVATE KEY-----\n"));
}

#[tokio::test]
async fn key_path_fallback_reads_and_normalizes_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crlf_key");
    std::fs::write(
        &path,
        "-----BEGIN OPENSSH PRIVATE KEY-----\r\nbody\r\n-----END OPENSSH PRIVATE KEY-----\r\n",
    )
    .unwrap();
    let connection = key_connection(&path.display().to_string(), "");
    let text = resolve_private_key_text(&connection).await.unwrap();
    assert!(text.contains("\nbody\n"));
    assert!(!text.contains('\r'));
}

#[tokio::test]
async fn missing_key_without_content_reports_the_path() {
    let connection = key_connection("/definitely/missing/id_test", "");
    let error = resolve_private_key_text(&connection).await.unwrap_err();
    assert!(error.contains("Failed to read SSH private key"), "{error}");
}

// —— F1: 断点续传（spool meta / resume 校验 / resumable 扫描）———

#[test]
fn host_key_verdicts_map_check_results() {
    assert_eq!(
        host_key_verdict(Ok(HostKeyState::Trusted)),
        HostKeyVerdict::Trusted
    );
    assert_eq!(
        host_key_verdict(Ok(HostKeyState::Unknown)),
        HostKeyVerdict::Unknown
    );
    let changed = host_key_verdict(Err(std::io::Error::other(
        "Host key for h:22 changed (recorded at known_hosts, line 3)",
    )));
    assert!(matches!(&changed, HostKeyVerdict::Changed(message) if message.contains("line 3")));
}

#[test]
fn host_key_check_responses_carry_state_and_key_identity() {
    let trusted = host_key_check_response(&HostKeyVerdict::Trusted, "ssh-ed25519", "SHA256:abcdef");
    assert_eq!(trusted["state"], "trusted");
    assert_eq!(trusted["keyType"], "ssh-ed25519");
    assert_eq!(trusted["fingerprint"], "SHA256:abcdef");

    assert_eq!(
        host_key_check_response(&HostKeyVerdict::Unknown, "rsa", "f")["state"],
        "unknown"
    );
    // A changed key keeps the identity of the presented key so the UI
    // can show the new fingerprint next to the mismatch reason.
    let changed = host_key_check_response(
        &HostKeyVerdict::Changed("key mismatch".to_string()),
        "rsa",
        "f",
    );
    assert_eq!(changed["state"], "changed");
    assert!(
        changed.get("error").is_none(),
        "reason travels in the notice event, not the check response"
    );

    let unreachable = host_key_unreachable_response("SSH connection to h:22 timed out".to_string());
    assert_eq!(unreachable["state"], "unreachable");
    assert_eq!(unreachable["error"], "SSH connection to h:22 timed out");
}

/// Auto 按序回退的纯逻辑契约：固定顺序、阶段门控、全失败汇总。真实
/// 凭据流（密码/私钥/KI/agent）复用既有 helper，端到端由既有认证测试
/// 覆盖；这里只锁定编排器自身的决策面。
mod auto_auth {
    use super::*;

    #[test]
    fn auto_order_contract_lists_methods_in_fallback_order() {
        // 对标 Tabby 的认证依序回退：密码 → 私钥 → 交互式 KI → agent。
        assert_eq!(
            AUTO_AUTH_ORDER,
            ["password", "private-key", "keyboard-interactive", "agent"]
        );
    }

    #[test]
    fn auto_stage_skip_gates_password_on_config_and_advertised_methods() {
        // 无密码配置：跳过，无论服务器是否广告 password 方法。
        assert_eq!(
            auto_stage_skip("password", false, true, true).as_deref(),
            Some("no password configured for this connection")
        );
        // 有密码但服务器未广告 password 方法：不发送密码，跳过。
        assert_eq!(
            auto_stage_skip("password", true, false, true).as_deref(),
            Some("server does not advertise password authentication")
        );
        // 有密码且已广告：尝试。
        assert_eq!(auto_stage_skip("password", true, true, true), None);
    }

    #[test]
    fn auto_stage_skip_gates_private_key_on_configured_material() {
        assert_eq!(
            auto_stage_skip("private-key", true, true, false).as_deref(),
            Some("no private key configured for this connection")
        );
        // 路径或粘贴内容任一存在即算已配置。
        assert_eq!(auto_stage_skip("private-key", false, false, true), None);
    }

    #[test]
    fn auto_stage_skip_never_gates_interactive_or_agent_stages() {
        // KI 与 agent 没有本地前置凭据：提问可交互作答，agent 套接字
        // 来自环境（挑战流中立——Auto 不改变 host key / 2FA 交互行为）。
        for method in ["keyboard-interactive", "agent"] {
            assert_eq!(
                auto_stage_skip(method, false, false, false),
                None,
                "{method} must never be skipped by local prerequisites"
            );
        }
    }

    #[test]
    fn auto_auth_failure_message_aggregates_attempts_in_order() {
        let attempts = vec![
            auto_auth_skip(
                "password",
                "no password configured for this connection".to_string(),
            ),
            auto_auth_attempt("private-key", "rejected by the server".to_string()),
            auto_auth_attempt(
                "keyboard-interactive",
                "SSH keyboard-interactive authentication was rejected".to_string(),
            ),
            auto_auth_attempt("agent", "no SSH Agent identity was accepted".to_string()),
        ];
        // 汇总保持固定顺序，逐方式带原因；skipped 有显式标记。
        assert_eq!(
            auto_auth_failure_message(&attempts),
            "SSH authentication failed in Auto mode, tried in order — \
             password (skipped: no password configured for this connection); \
             private-key (rejected by the server); \
             keyboard-interactive (SSH keyboard-interactive authentication was rejected); \
             agent (no SSH Agent identity was accepted)"
        );
    }

    #[test]
    fn auto_auth_failure_message_without_attempts_names_the_chain() {
        assert_eq!(
            auto_auth_failure_message(&[]),
            "SSH authentication failed in Auto mode: no method could be attempted"
        );
    }

    #[test]
    fn merge_auto_auth_attempt_keeps_one_entry_per_method_in_try_order() {
        // 双重 KI 场景（private-key partial-success 续答失败 + agent 续答
        // 失败 + 专属 KI 阶段 skip）：同 method 必须合并 detail 而不是重复
        // push，否则"每方法一条、按首试顺序"的不变量（authenticate_auto
        // 末尾的 debug_assert_eq）在 debug 构建必假，聚合消息也会把同一
        // 方法列两遍。
        let mut attempts = Vec::new();
        merge_auto_auth_attempt(
            &mut attempts,
            auto_auth_attempt("password", "rejected by the server".to_string()),
        );
        merge_auto_auth_attempt(
            &mut attempts,
            auto_auth_attempt("keyboard-interactive", "KI round one rejected".to_string()),
        );
        merge_auto_auth_attempt(
            &mut attempts,
            auto_auth_skip(
                "keyboard-interactive",
                "already answered as the MFA follow-up".to_string(),
            ),
        );
        merge_auto_auth_attempt(
            &mut attempts,
            auto_auth_attempt("keyboard-interactive", "KI round two rejected".to_string()),
        );
        merge_auto_auth_attempt(
            &mut attempts,
            auto_auth_attempt("agent", "no SSH Agent identity was accepted".to_string()),
        );

        let methods: Vec<_> = attempts.iter().map(|attempt| attempt.method).collect();
        assert_eq!(methods, ["password", "keyboard-interactive", "agent"]);
        let ki = &attempts[1];
        assert!(
            !ki.skipped,
            "an actual attempt clears the earlier skip mark"
        );
        assert_eq!(
            ki.detail,
            "KI round one rejected; already answered as the MFA follow-up; KI round two rejected"
        );
        // authenticate_auto 末尾的顺序断言在此场景下必须还原成立。
        assert_eq!(
            attempts
                .iter()
                .map(|attempt| attempt.method)
                .collect::<Vec<_>>(),
            AUTO_AUTH_ORDER
                .iter()
                .filter(|method| attempts.iter().any(|attempt| attempt.method == **method))
                .copied()
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn merge_auto_auth_attempt_appends_new_methods_in_arrival_order() {
        let mut attempts = Vec::new();
        merge_auto_auth_attempt(
            &mut attempts,
            auto_auth_attempt("keyboard-interactive", "rejected".to_string()),
        );
        merge_auto_auth_attempt(
            &mut attempts,
            auto_auth_attempt("agent", "no identity".to_string()),
        );
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[1].method, "agent");
        assert!(!attempts[1].skipped);
    }

    #[test]
    fn auto_auth_attempt_helpers_mark_skip_state() {
        let skipped = auto_auth_skip("password", "reason".to_string());
        assert!(skipped.skipped);
        assert_eq!(skipped.method, "password");
        let attempted = auto_auth_attempt("agent", "detail".to_string());
        assert!(!attempted.skipped);
        assert_eq!(attempted.method, "agent");
    }
}
