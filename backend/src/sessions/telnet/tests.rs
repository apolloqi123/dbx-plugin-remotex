use super::*;
use crate::core::model::TerminalFrame;
use serde_json::json;

const WILL_ECHO: &[u8] = &[IAC, WILL, OPT_ECHO];
const DO_NAWS: &[u8] = &[IAC, DO, OPT_NAWS];

fn feed_all(chunks: &[&[u8]]) -> (Vec<u8>, Vec<u8>) {
    let mut parser = TelnetParser::new();
    let mut payload = Vec::new();
    let mut replies = Vec::new();
    for chunk in chunks {
        let (chunk_payload, chunk_replies) = parser.feed(chunk);
        payload.extend_from_slice(&chunk_payload);
        replies.extend_from_slice(&chunk_replies);
    }
    (payload, replies)
}

// —— 协商应答矩阵 ——————————————————————————————————————

#[test]
fn negotiation_replies_follow_the_matrix() {
    // WILL ECHO / WILL SGA → DO；WILL 其他 → DONT。
    let (_, replies) = feed_all(&[WILL_ECHO]);
    assert_eq!(replies, vec![IAC, DO, OPT_ECHO]);
    let (_, replies) = feed_all(&[&[IAC, WILL, OPT_SGA]]);
    assert_eq!(replies, vec![IAC, DO, OPT_SGA]);
    let (_, replies) = feed_all(&[&[IAC, WILL, 24]]);
    assert_eq!(replies, vec![IAC, DONT, 24]);
    // DO NAWS → WILL NAWS；DO 其他 → WONT。
    let (_, replies) = feed_all(&[DO_NAWS]);
    assert_eq!(replies, vec![IAC, WILL, OPT_NAWS]);
    let (_, replies) = feed_all(&[&[IAC, DO, OPT_ECHO]]);
    assert_eq!(replies, vec![IAC, WONT, OPT_ECHO]);
    // WONT/DONT 无应答；单字节命令（NOP/AYT）静默吞掉。
    for sequence in [
        vec![IAC, WONT, OPT_ECHO],
        vec![IAC, DONT, OPT_NAWS],
        vec![IAC, 241], // NOP
        vec![IAC, 246], // AYT
    ] {
        let (payload, replies) = feed_all(&[&sequence]);
        assert!(replies.is_empty(), "no reply for {sequence:?}");
        assert!(payload.is_empty());
    }
}

#[test]
fn strip_removes_iac_and_keeps_payload_bytes() {
    let wire = [b'a', b'b', IAC, WILL, OPT_ECHO, b'c', IAC, 241, b'd'];
    let (payload, replies) = feed_all(&[&wire]);
    assert_eq!(payload, b"abcd");
    assert_eq!(replies, vec![IAC, DO, OPT_ECHO]);
}

#[test]
fn iac_iac_unescapes_to_a_literal_ff() {
    let wire = [b'x', IAC, IAC, 0xfe, b'y'];
    let (payload, replies) = feed_all(&[&wire]);
    assert_eq!(payload, vec![b'x', 0xff, 0xfe, b'y']);
    assert!(replies.is_empty());
}

#[test]
fn iac_split_across_chunks_still_parses() {
    // IAC 断在块边界：首块以 IAC 结尾，次块补完 WILL ECHO。
    let (payload, replies) = feed_all(&[&[b'o', b'k', IAC], &[WILL, OPT_ECHO, b'!']]);
    assert_eq!(payload, b"ok!");
    assert_eq!(replies, vec![IAC, DO, OPT_ECHO]);
    // 子协商断块同样成立。
    let (payload, replies) = feed_all(&[&[b'a', IAC, SB, 24, b'x'], &[b'y', IAC], &[SE, b'b']]);
    assert_eq!(payload, b"ab");
    assert!(replies.is_empty());
}

#[test]
fn subnegotiation_content_is_swallowed() {
    // TTYPE (24) 子协商内容（含 0xFF 之外的任意字节）不进 payload。
    let wire = [IAC, SB, 24, 0, b'x', 0xff, b't', IAC, SE, b'z'];
    let (payload, replies) = feed_all(&[&wire]);
    assert_eq!(payload, b"z");
    assert!(replies.is_empty());
    // 子协商里的 IAC IAC 是字面 0xFF 值字节，仍留在 SB 内。
    let wire = [IAC, SB, OPT_NAWS, 0x00, IAC, IAC, 0x78, IAC, SE, b'!'];
    let (payload, _) = feed_all(&[&wire]);
    assert_eq!(payload, b"!");
}

#[test]
fn malformed_subnegotiation_resynchronizes() {
    // 子协商中 IAC 后跟非法字节 → 回到 Ground，后续 payload 恢复透传。
    let wire = [IAC, SB, 24, IAC, b'q', b'!'];
    let (payload, _) = feed_all(&[&wire]);
    assert_eq!(payload, b"!");
}

#[test]
fn dropped_output_chunk_cannot_desync_the_reader_parser() {
    // Regression: the IAC parser used to live in the pump and only saw
    // chunks that survived the bounded output queue. A dropped chunk ending
    // mid-negotiation left the parser in Ground for the next chunk, so
    // `ECHO`/payload bytes were mangled. Parsing now happens in the reader,
    // so even a dropped chunk advances the state machine.
    let mut parser = TelnetParser::new();
    let dropped = super::pump::parse_telnet_output(&mut parser, &[b'a', IAC, WILL]);
    // The payload of this chunk would be lost, but the parser consumed it.
    assert_eq!(dropped.payload, b"a");
    let next = super::pump::parse_telnet_output(&mut parser, &[OPT_ECHO, b'b']);
    assert_eq!(next.payload, b"b", "payload after a dropped IAC prefix");
    assert_eq!(
        next.replies,
        vec![IAC, DO, OPT_ECHO],
        "the negotiation split across the dropped chunk still replies"
    );
}

// —— NAWS / 输入转换 ————————————————————————————————————

#[test]
fn naws_encodes_big_endian_columns_and_rows() {
    assert_eq!(
        naws_frame(120, 32),
        vec![IAC, SB, OPT_NAWS, 0, 120, 0, 32, IAC, SE]
    );
    // 0xFF 值字节按 Telnet 规则转义为 IAC IAC。
    assert_eq!(
        naws_frame(0x00ff, 0x0102),
        vec![IAC, SB, OPT_NAWS, 0x00, IAC, IAC, 0x01, 0x02, IAC, SE]
    );
}

#[test]
fn enter_mode_converts_carriage_returns() {
    // crlf：\r → \r\n（默认）。
    assert_eq!(
        transform_input(b"a\rb", EnterMode::Crlf, BackspaceMode::Del),
        b"a\r\nb"
    );
    // 已成对的 \r\n 只算一次回车，不翻倍。
    assert_eq!(
        transform_input(b"a\r\nb", EnterMode::Crlf, BackspaceMode::Del),
        b"a\r\nb"
    );
    // cr：原样；lf：\r → \n。
    assert_eq!(
        transform_input(b"a\rb", EnterMode::Cr, BackspaceMode::Del),
        b"a\rb"
    );
    assert_eq!(
        transform_input(b"a\rb", EnterMode::Lf, BackspaceMode::Del),
        b"a\nb"
    );
}

#[test]
fn backspace_mode_maps_del_to_ctrl_h() {
    let input = [b'a', 0x7f, b'b'];
    assert_eq!(
        transform_input(&input, EnterMode::Cr, BackspaceMode::Del),
        input
    );
    assert_eq!(
        transform_input(&input, EnterMode::Cr, BackspaceMode::CtrlH),
        [b'a', 0x08, b'b']
    );
    // 组合生效：ctrl_h 换映射，crlf 换回车。
    assert_eq!(
        transform_input(&input, EnterMode::Crlf, BackspaceMode::CtrlH),
        [b'a', 0x08, b'b']
    );
}

// —— 自动登录（规则形态复用 triggers 规则引擎）———————————
// 与声明式形态（NyaTerm 对齐）共用 parse_auto_login → AutoLoginPlan。

fn slot_secrets() -> Vec<String> {
    vec!["s3cret-one".to_string(), "s3cret-two".to_string()]
}

fn rule_spec(rules: serde_json::Value) -> AutoLoginSpec {
    AutoLoginSpec {
        rules: Some(rules),
        secrets: slot_secrets(),
        declarative: None,
    }
}

/// 声明式规格构造：正则全部留空 → 走内置默认提示词表。
fn decl_spec(username: &str, password: &str, max_retries: u8) -> AutoLoginSpec {
    AutoLoginSpec {
        rules: None,
        secrets: Vec::new(),
        declarative: Some(DeclarativeAutoLogin {
            username: (!username.is_empty()).then(|| username.to_string()),
            password: (!password.is_empty()).then(|| password.to_string()),
            username_prompt_regex: None,
            password_prompt_regex: None,
            success_regex: None,
            failure_regex: None,
            max_retries: Some(max_retries),
        }),
    }
}

#[test]
fn auto_login_parses_rules_and_answers_fake_output() {
    // JSON 对象形态：明文 + 密文槽两阶段（sendSecretKey 槽引用）。
    // JSON 形态按 D7 严格校验（`*assword` 只在 tssh 文本形态有字面量兜底）。
    let spec = rule_spec(json!(
        r#"{"stages":[{"pattern":"ogin:","sendText":"myuser\r"},{"pattern":"assword","sendSecretKey":"trigger_answer_1"}]}"#
    ));
    let plan = parse_auto_login(&spec)
        .expect("rules parse")
        .expect("enabled");
    assert_eq!(plan.triggers.stages.len(), 2);
    assert!(plan.watch.is_none(), "rule form has no declarative watch");

    let mut engine = triggers::TriggerEngine::new(
        plan.triggers,
        triggers::CommandPlaceholders::new("bbs.example", "", 23, "bbs.example"),
    );
    // 喂假输出流：login 提示命中阶段 1，明文应答。
    let decision = engine
        .observe("Welcome!\r\nlogin: ", 1_000)
        .expect("stage 1 must answer");
    assert_eq!(decision.stage, 1);
    assert_eq!(decision.kind, triggers::TriggerKind::Text);
    assert_eq!(decision.segments, vec![(b"myuser\r".to_vec(), 0)]);

    // 随后 password 提示命中阶段 2，密文槽应答（槽 1 值）。
    let decision = engine
        .observe("Password: ", 2_000)
        .expect("stage 2 must answer");
    assert_eq!(decision.stage, 2);
    assert_eq!(decision.kind, triggers::TriggerKind::Secret);
    assert_eq!(
        decision.segments,
        triggers::pass_sleep_segments("s3cret-one", 100, triggers::PassSleep::None)
    );
}

#[test]
fn auto_login_accepts_the_tssh_text_form() {
    // tssh 文本形态同样全量可用（Expect* 指令）；注意 tssh 语法里没有
    // sendSecretKey——槽引用属于 JSON 形态的 sendSecretKey 字段。
    let spec = rule_spec(json!(
        "#!! ExpectCount 1\n#!! ExpectPattern1 ogin:\n#!! ExpectSendText1 myuser\\r"
    ));
    let plan = parse_auto_login(&spec)
        .expect("rules parse")
        .expect("enabled");
    let mut engine = triggers::TriggerEngine::new(
        plan.triggers,
        triggers::CommandPlaceholders::new("bbs", "", 23, "bbs"),
    );
    let decision = engine.observe("login: ", 1_000).expect("stage 1");
    assert_eq!(decision.segments, vec![(b"myuser\r".to_vec(), 0)]);
}

#[test]
fn auto_login_maps_both_secret_slots() {
    let spec = rule_spec(json!(
        r#"{"stages":[{"pattern":"a","sendSecretKey":"trigger_answer_1"},{"pattern":"b","sendSecretKey":"trigger_answer_2"}]}"#
    ));
    let plan = parse_auto_login(&spec)
        .expect("rules parse")
        .expect("enabled");
    let mut engine = triggers::TriggerEngine::new(
        plan.triggers,
        triggers::CommandPlaceholders::new("h", "", 23, "h"),
    );
    let decision = engine.observe("aaa", 1_000).expect("stage 1");
    assert_eq!(decision.segments[0].0, b"s3cret-one\r".to_vec());
    let decision = engine.observe("bbb", 2_000).expect("stage 2");
    assert_eq!(decision.segments[0].0, b"s3cret-two\r".to_vec());
}

#[test]
fn auto_login_rejects_invalid_rules_and_empty_slots() {
    // 非法 JSON 且不含 Expect 指令 → 沿用 triggers 的 JSON 报错。
    let spec = rule_spec(json!("{not json"));
    let error = parse_auto_login(&spec).expect_err("must fail");
    assert!(error.contains("invalid JSON"), "{error}");
    // 槽位为空 = 配置错误（D7 同款）。
    let spec = AutoLoginSpec {
        rules: Some(json!(
            r#"{"stages":[{"pattern":"a","sendSecretKey":"trigger_answer_1"}]}"#
        )),
        secrets: Vec::new(),
        declarative: None,
    };
    let error = parse_auto_login(&spec).expect_err("must fail");
    assert!(error.contains("is empty"), "{error}");
    // ExpectCount 0 = 显式关闭。
    let spec = rule_spec(json!("ExpectCount 0\nExpectPattern1 a\nExpectSendText1 x"));
    assert!(parse_auto_login(&spec).expect("rules parse").is_none());
}

// —— 自动登录（声明式形态，NyaTerm 对齐 P0-1）————————————

#[test]
fn declarative_builds_stages_and_watch_supervises_success() {
    let spec = AutoLoginSpec {
        rules: None,
        secrets: Vec::new(),
        declarative: Some(DeclarativeAutoLogin {
            username: Some("myuser".to_string()),
            password: Some("s3cret-pass".to_string()),
            username_prompt_regex: Some("ogin:".to_string()),
            password_prompt_regex: Some("assword".to_string()),
            success_regex: Some(r"\$ $".to_string()),
            failure_regex: Some("incorrect".to_string()),
            max_retries: Some(2),
        }),
    };
    let mut plan = parse_auto_login(&spec)
        .expect("declarative parse")
        .expect("enabled");
    assert_eq!(plan.triggers.stages.len(), 2);
    let mut watch = plan.watch.take().expect("declarative form has a watch");

    // 提示应答走共享引擎：用户名明文阶段 + 密码密文槽阶段。
    let mut engine = triggers::TriggerEngine::new(
        plan.triggers,
        triggers::CommandPlaceholders::new("host", "", 23, "host"),
    );
    let decision = engine
        .observe("Welcome!\r\nlogin: ", 1_000)
        .expect("stage 1");
    assert_eq!(decision.kind, triggers::TriggerKind::Text);
    assert_eq!(decision.segments, vec![(b"myuser\r".to_vec(), 0)]);
    let decision = engine.observe("Password: ", 2_000).expect("stage 2");
    assert_eq!(decision.kind, triggers::TriggerKind::Secret);
    assert_eq!(
        decision.segments,
        triggers::pass_sleep_segments("s3cret-pass", 100, triggers::PassSleep::None)
    );

    // 未发出任何应答前，成功正则不生效（NyaTerm sent_* 守卫）。
    assert!(watch.observe("$ ", false).is_none());
    // 发出应答后命中成功正则 → Success，之后停止监督。
    assert_eq!(
        watch.observe("user@host:~$ ", true),
        Some(WatchEvent::Success)
    );
    assert!(watch.observe("anything", true).is_none());
}

#[test]
fn declarative_defaults_cover_common_prompts() {
    let plan = parse_auto_login(&decl_spec("dev", "pw", 0))
        .expect("declarative parse")
        .expect("enabled");
    // 内置用户名提示默认：英文与中文。每次用全新引擎，避免上一提示命中
    // 后游标停在密码阶段。
    for prompt in ["login: ", "Username: ", "用户名：", "帐号:"] {
        let mut engine = triggers::TriggerEngine::new(
            plan.triggers.clone(),
            triggers::CommandPlaceholders::new("host", "", 23, "host"),
        );
        let decision = engine
            .observe(prompt, 1_000)
            .unwrap_or_else(|| panic!("default username prompt must match {prompt:?}"));
        assert_eq!(
            decision.segments,
            vec![(b"dev\r".to_vec(), 0)],
            "{prompt:?}"
        );
    }
    // 内置密码提示默认：密码-only 规格（引擎游标从密码阶段开始）。
    let plan = parse_auto_login(&decl_spec("", "pw", 0))
        .expect("declarative parse")
        .expect("enabled");
    assert_eq!(
        plan.triggers.stages.len(),
        1,
        "password-only keeps one stage"
    );
    let mut engine = triggers::TriggerEngine::new(
        plan.triggers,
        triggers::CommandPlaceholders::new("host", "", 23, "host"),
    );
    let decision = engine
        .observe("Password: ", 2_000)
        .expect("password prompt");
    assert_eq!(decision.kind, triggers::TriggerKind::Secret);
    // 内置失败默认 + max_retries=0：首次失败即耗尽。
    let mut watch = plan.watch.expect("watch present");
    assert_eq!(
        watch.observe("Login incorrect", false),
        Some(WatchEvent::Exhausted)
    );
    // 内置成功默认（发出应答后生效）。
    let plan = parse_auto_login(&decl_spec("dev", "pw", 0))
        .expect("declarative parse")
        .expect("enabled");
    let mut watch = plan.watch.expect("watch present");
    assert_eq!(
        watch.observe("dev@host:~$ ", true),
        Some(WatchEvent::Success)
    );
}

#[test]
fn declarative_default_prompts_ignore_last_login_banner() {
    // E1 回归：行尾锚定后，`Last login: <time>` MOTD 横幅不再命中内置
    // 用户名提示（旧行为会把用户名提前打进远端并回显）。
    let plan = parse_auto_login(&decl_spec("dev", "pw", 0))
        .expect("declarative parse")
        .expect("enabled");
    let mut engine = triggers::TriggerEngine::new(
        plan.triggers.clone(),
        triggers::CommandPlaceholders::new("host", "", 23, "host"),
    );
    assert!(
        engine
            .observe(
                "Welcome to host\r\nLast login: 10.0.0.1 on Mon Sep 24 22:00:00 2026\r\n",
                1_000
            )
            .is_none(),
        "MOTD banner must not answer the username prompt"
    );
    // 横幅之后的真实 login: 提示照常命中（跨 chunk 场景：引擎按 chunk
    // 归一化拼接，提示符在缓冲末尾即活提示）。
    let decision = engine.observe("login: ", 1_001).expect("username prompt");
    assert_eq!(decision.segments, vec![(b"dev\r".to_vec(), 0)]);
    // 内置密码提示同源修复：帮助文本中的 "password:" 子串不再误发密码。
    let plan = parse_auto_login(&decl_spec("", "pw", 0))
        .expect("declarative parse")
        .expect("enabled");
    let mut engine = triggers::TriggerEngine::new(
        plan.triggers,
        triggers::CommandPlaceholders::new("host", "", 23, "host"),
    );
    assert!(
        engine
            .observe("Type 'run password: reset' to rotate it\r\n", 1_000)
            .is_none(),
        "embedded 'password:' text must not answer the password stage"
    );
    let decision = engine
        .observe("Password: ", 1_001)
        .expect("password prompt");
    assert_eq!(decision.kind, triggers::TriggerKind::Secret);
}

#[test]
fn declarative_watch_strips_ansi_before_matching_success() {
    // E2 回归：带颜色的 `$ ` 成功提示（ANSI 包裹）必须先剥离再匹配
    // （NyaTerm strip_ansi_escapes 同语义；未剥离时默认成功正则不命中）。
    let colored = "\x1b[32mdev@host:~$ \x1b[0m";
    // 旧行为锚定：原始转义字节尾随 `m`，默认成功正则不命中。
    let mut plan = parse_auto_login(&decl_spec("dev", "pw", 0))
        .expect("declarative parse")
        .expect("enabled");
    let mut watch = plan.watch.take().expect("watch present");
    assert!(
        watch.observe(colored, true).is_none(),
        "raw escapes must not match"
    );
    // 读循环喂给 watch 前剥离（publish 的透传字节不动）。
    let mut plan = parse_auto_login(&decl_spec("dev", "pw", 0))
        .expect("declarative parse")
        .expect("enabled");
    let mut watch = plan.watch.take().expect("watch present");
    assert_eq!(
        watch.observe(&strip_ansi_control_sequences(colored), true),
        Some(WatchEvent::Success)
    );
}

#[test]
fn declarative_tail_refills_after_retry_so_each_failure_counts_once() {
    let mut plan = parse_auto_login(&decl_spec("dev", "pw", 1))
        .expect("declarative parse")
        .expect("enabled");
    let mut watch = plan.watch.take().expect("watch present");
    assert_eq!(
        watch.observe("Login incorrect", false),
        Some(WatchEvent::Retry { attempt: 1 })
    );
    // tail 已清空后同文本再次到达 = 宿主再次拒绝 → 重新计数并立即耗尽
    // （budget=1）。
    assert_eq!(
        watch.observe("Login incorrect", false),
        Some(WatchEvent::Exhausted)
    );
    assert_eq!(watch.attempts_used(), 2, "初始 1 次 + 重试 1 次");
    // 断言清空语义的另一半：失败文本被无关输出稀释时不得连击。
    let mut plan = parse_auto_login(&decl_spec("dev", "pw", 1))
        .expect("declarative parse")
        .expect("enabled");
    let mut watch = plan.watch.take().expect("watch present");
    assert_eq!(
        watch.observe("Login incorrect", false),
        Some(WatchEvent::Retry { attempt: 1 })
    );
    assert!(watch.observe("welcome banner", false).is_none());
}

#[test]
fn declarative_retry_budget_exhausts_and_reports_attempts() {
    let mut plan = parse_auto_login(&decl_spec("dev", "pw", 2))
        .expect("declarative parse")
        .expect("enabled");
    let mut watch = plan.watch.take().expect("watch present");
    assert_eq!(
        watch.observe("Authentication failed", false),
        Some(WatchEvent::Retry { attempt: 1 })
    );
    assert!(watch.observe("ok", false).is_none());
    assert_eq!(
        watch.observe("Authentication failed", false),
        Some(WatchEvent::Retry { attempt: 2 })
    );
    assert_eq!(
        watch.observe("Authentication failed", false),
        Some(WatchEvent::Exhausted)
    );
    assert_eq!(watch.attempts_used(), 3);
}

#[test]
fn declarative_rejects_invalid_config_before_start() {
    // 非法成功正则 → 可读错误带字段名。
    let mut spec = decl_spec("dev", "pw", 0);
    spec.declarative.as_mut().unwrap().success_regex = Some("(unclosed".to_string());
    let error = parse_auto_login(&spec).expect_err("invalid success regex must fail");
    assert!(error.contains("autoLogin.successRegex"), "{error}");
    // 非法用户名提示正则同理。
    let mut spec = decl_spec("dev", "pw", 0);
    spec.declarative.as_mut().unwrap().username_prompt_regex = Some("[".to_string());
    let error = parse_auto_login(&spec).expect_err("invalid prompt regex must fail");
    assert!(error.contains("autoLogin.usernamePromptRegex"), "{error}");
    // 无凭据可发 = 配置错误。
    let error = parse_auto_login(&decl_spec("", "", 0)).expect_err("no credentials must fail");
    assert!(error.contains("username or a password"), "{error}");
    // 重试预算超上限。
    let error = parse_auto_login(&decl_spec("dev", "pw", 11)).expect_err("retries cap");
    assert!(error.contains("autoLogin.maxRetries"), "{error}");
    // 两种形态互斥。
    let spec = AutoLoginSpec {
        rules: Some(json!({"stages": []})),
        secrets: Vec::new(),
        declarative: Some(DeclarativeAutoLogin {
            username: Some("dev".to_string()),
            password: None,
            username_prompt_regex: None,
            password_prompt_regex: None,
            success_regex: None,
            failure_regex: None,
            max_retries: None,
        }),
    };
    let error = parse_auto_login(&spec).expect_err("both forms must fail");
    assert!(error.contains("mutually exclusive"), "{error}");
}

#[test]
fn declarative_credentials_never_reach_debug_output() {
    // 安全红线：密码/密文槽值不得出现在任何 Debug 输出（日志/panic 的载体）。
    let spec = decl_spec("dev", "s3cret-pass", 1);
    assert!(!format!("{spec:?}").contains("s3cret-pass"));
    let plan = parse_auto_login(&spec).expect("parse").expect("enabled");
    // TriggersConfig 的 Debug 对 Secret 脱敏，Watch 不含凭据。
    assert!(!format!("{plan:?}").contains("s3cret-pass"));
    let rendered = format!("{:?}", plan.triggers.stages[1].answer);
    assert!(rendered.contains("redacted"), "{rendered}");
}

#[test]
fn mode_enums_accept_wire_names_only() {
    assert_eq!(EnterMode::parse("crlf"), Some(EnterMode::Crlf));
    assert_eq!(EnterMode::parse("cr"), Some(EnterMode::Cr));
    assert_eq!(EnterMode::parse("lf"), Some(EnterMode::Lf));
    assert_eq!(EnterMode::parse("cr lf"), None);
    assert_eq!(BackspaceMode::parse("del"), Some(BackspaceMode::Del));
    assert_eq!(BackspaceMode::parse("ctrl_h"), Some(BackspaceMode::CtrlH));
    assert_eq!(BackspaceMode::parse("ctrlH"), Some(BackspaceMode::CtrlH));
    assert_eq!(BackspaceMode::parse("backspace"), None);
}

#[test]
fn write_payload_requires_base64() {
    assert_eq!(decode_write_payload("aGk=").unwrap(), b"hi");
    assert!(decode_write_payload("!!").is_err());
}

#[test]
fn write_payload_enforces_size_limit() {
    // 边界值通过，超限拒绝——键盘输入的现实尺寸远小于该帽，帽只挡
    // 异常/恶意的大 payload（对齐 serial 上传分块的 64KiB 纪律）。
    let boundary = vec![b'a'; WRITE_PAYLOAD_LIMIT];
    let encoded = base64::engine::general_purpose::STANDARD.encode(&boundary);
    assert_eq!(decode_write_payload(&encoded).unwrap(), boundary);
    let oversized = vec![b'a'; WRITE_PAYLOAD_LIMIT + 1];
    let encoded = base64::engine::general_purpose::STANDARD.encode(&oversized);
    let error = decode_write_payload(&encoded).unwrap_err();
    assert!(error.contains(&WRITE_PAYLOAD_LIMIT.to_string()), "{error}");
}

#[test]
fn telnet_state_frame_roundtrips_through_terminal_frame() {
    let frame = TerminalFrame {
        sequence: 3,
        stream: TerminalStream::State,
        data: b"telnet-session-closed".to_vec(),
    };
    let encoded = frame.encode();
    assert_eq!(encoded[0], 2);
    assert_eq!(&encoded[1..9], &3u64.to_be_bytes());
}
