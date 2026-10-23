use super::*;

// RFC 6238 appendix B vectors use the ASCII secret of "12345678901234567890".
const RFC_KEY: &[u8] = b"12345678901234567890";

#[test]
fn rfc6238_sha1_vectors() {
    // (counter, 8-digit code) from RFC 6238 appendix B.
    let vectors = [
        (1_u64, "94287082"),
        (0x23523EC, "07081804"),
        (0x23523ED, "14050471"),
        (0x273EF07, "89005924"),
        (0x3F940AA, "69279037"),
        (0x27BC86AA, "65353130"),
    ];
    for (counter, expected) in vectors {
        assert_eq!(hotp(RFC_KEY, counter, 8, TotpAlgorithm::Sha1), expected);
    }
}

#[test]
fn parses_base32_and_static_secrets() {
    let base32 = parse_totp_secret("JBSWY3DPEHPK3PXP").unwrap();
    match base32 {
        TotpSecret::Key { digits, period, .. } => {
            assert_eq!(digits, 6);
            assert_eq!(period, 30);
        }
        other => panic!("expected key, got {other:?}"),
    }
    match parse_totp_secret("12345678").unwrap() {
        TotpSecret::Static(code) => assert_eq!(code, "12345678"),
        other => panic!("expected static, got {other:?}"),
    }
    assert!(parse_totp_secret("  ").is_none());
    assert!(parse_totp_secret("not base32!!").is_none());
}

#[test]
fn parses_otpauth_uri() {
    let secret = parse_totp_secret(
        "otpauth://totp/ACME:alice?secret=JBSWY3DPEHPK3PXP&issuer=ACME&digits=8&period=60&algorithm=SHA256",
    )
    .unwrap();
    match secret {
        TotpSecret::Key {
            key,
            digits,
            period,
            algorithm,
        } => {
            assert_eq!(key, b"Hello!\xde\xad\xbe\xef");
            assert_eq!(digits, 8);
            assert_eq!(period, 60);
            assert_eq!(algorithm, TotpAlgorithm::Sha256);
        }
        other => panic!("expected key, got {other:?}"),
    }
    assert!(parse_totp_secret("otpauth://hotp/x?secret=JBSWY3DPEHPK3PXP").is_none());
}

#[test]
fn sanitizes_sudo_commands() {
    assert_eq!(sanitize_sudo_command("ls -la"), "sh -c 'ls -la'");
    assert_eq!(
        sanitize_sudo_command("echo 'hi'; rm -rf /tmp/x"),
        "sh -c 'echo '\\''hi'\\''; rm -rf /tmp/x'"
    );
}

#[test]
fn sanitizes_prompt_hints_against_adversarial_input() {
    // ANSI sequences and control characters are stripped, not stored.
    assert_eq!(
        sanitize_prompt_hint("\x1b[1midentity token\x1b[0m\u{7}"),
        "identity token"
    );
    assert_eq!(
        sanitize_prompt_hint("  duo \r passcode \n"),
        "duo \n passcode"
    );
    // Overlong hints are capped on a char boundary.
    let long = "a".repeat(MAX_PROMPT_HINT_LEN + 4096);
    let sanitized = sanitize_prompt_hint(&long);
    assert!(sanitized.len() <= MAX_PROMPT_HINT_LEN);
    let multibyte = "密".repeat(MAX_PROMPT_HINT_LEN / 2 + 16);
    assert!(sanitize_prompt_hint(&multibyte).is_char_boundary(0));
    assert!(sanitize_prompt_hint(&multibyte).len() <= MAX_PROMPT_HINT_LEN);
    // Clean input passes through unchanged.
    assert_eq!(
        sanitize_prompt_hint("verification code"),
        "verification code"
    );
}

#[test]
fn classify_matches_hints_despite_control_characters() {
    // A hint stored before sanitization existed (or delivered by an older
    // host) still matches the normalized prompt text.
    let auth = SudoAuth {
        password: "pw".into(),
        totp_secrets: Vec::new(),
        password_prompt_hint: "\x1b[1midentity token\x1b[0m".into(),
        totp_prompt_hint: String::new(),
        flow_mode: None,
        ..Default::default()
    };
    assert_eq!(
        auth.classify("Enter identity token:"),
        Some(PromptKind::Password)
    );
}

#[test]
fn shared_prompt_corpus_matches_backend_vocab() {
    // 架构评审 WATCH：前后端提示词词表防漂移——语料在 shared/prompt-corpus.json，
    // 前端 promptCorpus.spec.ts 加载同一份；改词表必须同步语料，任一侧先改
    // 即在此失败。
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../../../shared/prompt-corpus.json"))
            .expect("shared/prompt-corpus.json parses");
    let auth = SudoAuth::default();
    for case in corpus["cases"].as_array().expect("cases array") {
        let text = case["text"].as_str().expect("case text");
        let expected = match case["backend"].as_str().expect("backend expectation") {
            "password" => Some(PromptKind::Password),
            "totp" => Some(PromptKind::Totp),
            "combined" => Some(PromptKind::Combined),
            "none" => None,
            other => panic!("unknown backend expectation: {other}"),
        };
        assert_eq!(
            classify_auth_prompt(text, &auth),
            expected,
            "case {text:?} diverges from the shared prompt corpus"
        );
    }
}

#[test]
fn auth_flow_mode_parse_falls_back_for_adversarial_values() {
    // Unknown / hostile values degrade to the default flow instead of
    // panicking or producing an invalid mode.
    for value in [
        "",
        "garbage",
        "password\u{0}otp",
        "PASSWORD+OTP ",
        " password_only\t",
    ] {
        let _ = AuthFlowMode::parse(value);
    }
    assert_eq!(
        AuthFlowMode::parse("garbage"),
        AuthFlowMode::PasswordThenOtp
    );
    assert_eq!(
        AuthFlowMode::parse("PASSWORD+OTP"),
        AuthFlowMode::PasswordPlusOtp
    );
    assert_eq!(
        AuthFlowMode::parse(" password_only\t"),
        AuthFlowMode::PasswordOnly
    );
}

#[test]
fn off_flow_keeps_password_and_never_spends_otp() {
    // `off`（0.4.77 起连接表单默认）继续应答密码类提示，但 OTP 密钥哪怕
    // 已配置也绝不自动回码（登录 keyboard-interactive、sudo watcher、
    // 组合提示三条路径一致），且纯 OTP 凭据不再视作「有用」。
    let totp_only = SudoAuth::new(
        "",
        "",
        "JBSWY3DPEHPK3PXP\n123456",
        Hints {
            password: String::new(),
            totp: String::new(),
            flow_mode: Some(AuthFlowMode::Off),
        },
    );
    assert!(totp_only.totp_configured());
    assert!(
        !totp_only.useful(),
        "totp-only auth must not arm the watcher when off"
    );
    assert!(totp_only.answer_for(PromptKind::Totp).is_none());
    let (answer, retry_at) = totp_only.answer_for_with_retry(PromptKind::Totp);
    assert!(answer.is_none());
    assert!(retry_at.is_none(), "off must not schedule an OTP retry");
    // 密码路径不受 off 影响：组合提示仍回密码半段（拒答会晾死会话）。
    let with_password = SudoAuth::new(
        "",
        "login",
        "JBSWY3DPEHPK3PXP",
        Hints {
            password: String::new(),
            totp: String::new(),
            flow_mode: Some(AuthFlowMode::Off),
        },
    );
    assert!(with_password.useful());
    assert_eq!(
        with_password.answer_for(PromptKind::Combined).as_deref(),
        Some("login")
    );
    assert!(!can_respond_to_prompt(
        AuthFlowMode::Off,
        PromptKind::Totp,
        true
    ));
    assert!(can_respond_to_prompt(
        AuthFlowMode::Off,
        PromptKind::Password,
        false
    ));
    assert!(can_respond_to_prompt(
        AuthFlowMode::Off,
        PromptKind::Combined,
        false
    ));
    assert_eq!(AuthFlowMode::parse("off"), AuthFlowMode::Off);
    assert_eq!(AuthFlowMode::parse("Disabled"), AuthFlowMode::Off);
    assert_eq!(AuthFlowMode::Off.name(), "off");
}

#[test]
fn totp_secret_parsing_degrades_for_malformed_input() {
    // Multi-line input keeps the usable lines and drops the junk.
    let secrets = parse_totp_secrets("JBSWY3DPEHPK3PXP\nnot base32!!\n\n123456");
    assert_eq!(secrets.len(), 2);
    assert!(matches!(secrets[0], TotpSecret::Key { .. }));
    assert!(matches!(secrets[1], TotpSecret::Static(_)));
    // A space-laden pseudo-base32 line is rejected rather than decoded.
    assert!(parse_totp_secret("JBSW Y3DP EHPK 3PXP").is_none());
    // Semicolon-separated multi-secret input behaves like newlines.
    assert_eq!(parse_totp_secrets("JBSWY3DPEHPK3PXP;;bad value!").len(), 1);
    assert!(parse_totp_secrets("").is_empty());
}

#[test]
fn classifies_prompts_with_hints_and_defaults() {
    let auth = SudoAuth {
        password: "pw".into(),
        totp_secrets: parse_totp_secrets("JBSWY3DPEHPK3PXP"),
        password_prompt_hint: String::new(),
        totp_prompt_hint: String::new(),
        flow_mode: None,
        ..Default::default()
    };
    assert_eq!(
        auth.classify("[sudo] password for user:"),
        Some(PromptKind::Password)
    );
    assert_eq!(auth.classify("Verification code:"), Some(PromptKind::Totp));
    assert_eq!(auth.classify("密码："), Some(PromptKind::Password));
    assert_eq!(auth.classify("some regular output"), None);

    let hinted = SudoAuth {
        password: "pw".into(),
        totp_secrets: Vec::new(),
        password_prompt_hint: "identity token".into(),
        totp_prompt_hint: "duo passcode".into(),
        flow_mode: None,
        ..Default::default()
    };
    assert_eq!(
        hinted.classify("Enter identity token:"),
        Some(PromptKind::Password)
    );
    assert_eq!(hinted.classify("Duo passcode:"), Some(PromptKind::Totp));
}

#[test]
fn ansi_and_control_sequences_are_stripped() {
    assert_eq!(
        normalize_auth_prompt_text("\x1b[1m[sudo] password\x1b[0m for u: "),
        "[sudo] password for u:"
    );
    assert_eq!(normalize_auth_prompt_text("\r\nOTP:\u{7}"), "OTP:");
}

#[test]
fn flow_modes_gate_otp_answers() {
    assert!(can_respond_to_prompt(
        AuthFlowMode::PasswordThenOtp,
        PromptKind::Totp,
        true
    ));
    assert!(!can_respond_to_prompt(
        AuthFlowMode::PasswordThenOtp,
        PromptKind::Totp,
        false
    ));
    assert!(!can_respond_to_prompt(
        AuthFlowMode::PasswordOnly,
        PromptKind::Totp,
        true
    ));
    assert!(can_respond_to_prompt(
        AuthFlowMode::PasswordPlusOtp,
        PromptKind::Totp,
        false
    ));
    assert!(can_respond_to_prompt(
        AuthFlowMode::PasswordOnly,
        PromptKind::Combined,
        false
    ));
    // Piped-credential replay guard: a "merged prompt" surfacing after
    // the password went down the stdin pipe is command output spoofing
    // it — answering would echo the credential into the command's stdin.
    assert!(!can_respond_to_prompt(
        AuthFlowMode::PasswordOnly,
        PromptKind::Combined,
        true
    ));
    assert!(!can_respond_to_prompt(
        AuthFlowMode::PasswordPlusOtp,
        PromptKind::Combined,
        true
    ));
}

#[test]
fn piped_password_is_not_reanswered_on_visible_prompt() {
    // exec_with_sudo pipes the password before watching prompts; a second
    // copy would desync the stdin line stream against the OTP answer.
    assert!(!can_respond_to_prompt(
        AuthFlowMode::PasswordThenOtp,
        PromptKind::Password,
        true
    ));
    assert!(can_respond_to_prompt(
        AuthFlowMode::PasswordThenOtp,
        PromptKind::Password,
        false
    ));
}

#[test]
fn append_capped_truncates_at_the_ceiling_and_keeps_head_bytes() {
    let mut buffer = vec![b'x'; MAX_EXEC_OUTPUT_BYTES - 1];
    append_capped(&mut buffer, &[b'a'; 16]);
    assert_eq!(buffer.len(), MAX_EXEC_OUTPUT_BYTES);
    assert_eq!(*buffer.last().unwrap(), b'a');
    // Past the cap the chunk is dropped entirely (drain keeps flowing).
    append_capped(&mut buffer, &[b'y'; 16]);
    assert_eq!(buffer.len(), MAX_EXEC_OUTPUT_BYTES);
    assert_eq!(*buffer.last().unwrap(), b'a');
}

#[test]
fn sudo_password_overrides_login_password() {
    let auth = SudoAuth::new("sudo-pw", "login-pw", "", Hints::default());
    assert_eq!(auth.password, "sudo-pw");
    let fallback = SudoAuth::new("", "login-pw", "", Hints::default());
    assert_eq!(fallback.password, "login-pw");
}

#[test]
fn combined_answers_follow_flow_mode() {
    let _otp_ledger = otp_ledger_test_guard();
    let mut auth = SudoAuth {
        password: "pw".into(),
        totp_secrets: parse_totp_secrets("123456"),
        password_prompt_hint: String::new(),
        totp_prompt_hint: String::new(),
        flow_mode: Some(AuthFlowMode::PasswordThenOtp),
        ..Default::default()
    };
    // 合并提问（一条提问同时要密码与验证码）：两种 OTP 模式都拼接，
    // 只回密码对真正的合并提问无法通过。每个模式换一个码，避免进程级
    // 防重放台账把后一次调用吃掉。
    assert_eq!(auth.answer_for(PromptKind::Combined).unwrap(), "pw123456");
    auth.flow_mode = Some(AuthFlowMode::PasswordPlusOtp);
    auth.totp_secrets = parse_totp_secrets("654321");
    assert_eq!(auth.answer_for(PromptKind::Combined).unwrap(), "pw654321");
    // 密码类模式只回密码半边（永不自动回 OTP）。
    auth.flow_mode = Some(AuthFlowMode::PasswordOnly);
    assert_eq!(auth.answer_for(PromptKind::Combined).unwrap(), "pw");
    auth.flow_mode = Some(AuthFlowMode::Off);
    assert_eq!(auth.answer_for(PromptKind::Combined).unwrap(), "pw");
}

fn prompt(text: &str, echo: bool) -> russh::client::Prompt {
    russh::client::Prompt {
        prompt: text.to_string(),
        echo,
    }
}

fn terminal_auth() -> SudoAuth {
    SudoAuth {
        password: "pw".into(),
        totp_secrets: parse_totp_secrets("654321"),
        password_prompt_hint: String::new(),
        totp_prompt_hint: String::new(),
        flow_mode: None,
        ..Default::default()
    }
}

#[test]
fn keyboard_interactive_answers_follow_flow_mode() {
    let _otp_ledger = otp_ledger_test_guard();
    let mut state = KeyboardInteractiveState::default();
    let auth = terminal_auth();

    // Round 1: password question.
    let answers = keyboard_interactive_answers(
        &auth,
        &mut state,
        "",
        &[
            prompt("Password:", false),
            prompt("Enter account name:", true),
        ],
    );
    assert_eq!(answers, vec!["pw".to_string(), String::new()]);
    assert!(state.password_answered);

    // Round 2: TOTP is allowed because a password round already succeeded.
    let answers = keyboard_interactive_answers(
        &auth,
        &mut state,
        "",
        &[prompt("Verification code:", false)],
    );
    assert_eq!(answers, vec!["654321".to_string()]);

    // password+otp combined prompts concatenate when configured. A
    // distinct static code: the process-global replay guard already saw
    // "654321" go out in round 2, so the same code must not be answered
    // again inside its window.
    let mut plus = terminal_auth();
    plus.flow_mode = Some(AuthFlowMode::PasswordPlusOtp);
    plus.totp_secrets = parse_totp_secrets("998877");
    let answers = keyboard_interactive_answers(
        &plus,
        &mut KeyboardInteractiveState::default(),
        "",
        &[prompt("Password: otp:", false)],
    );
    assert_eq!(answers, vec!["pw998877".to_string()]);

    // password_only never answers OTP.
    let mut only = terminal_auth();
    only.flow_mode = Some(AuthFlowMode::PasswordOnly);
    let answers = keyboard_interactive_answers(
        &only,
        &mut KeyboardInteractiveState::default(),
        "",
        &[prompt("Verification code:", false)],
    );
    assert_eq!(answers, vec![String::new()]);
}

#[test]
fn keyboard_interactive_without_totp_leaves_prompts_blank() {
    let mut auth = terminal_auth();
    auth.totp_secrets = Vec::new();
    let answers = keyboard_interactive_answers(
        &auth,
        &mut KeyboardInteractiveState::default(),
        "",
        &[prompt("Verification code:", false)],
    );
    assert_eq!(answers, vec![String::new()]);
}

/// JumpServer/koko 的登录期 MFA：密码（或公钥）被 partial success 接受后，
/// 服务器用 keyboard-interactive 提问 "Please Enter MFA Code." + "[OTP Code]: "。
/// 这里逐条钉住修复后的行为（issue #17 / #30）。
#[test]
fn koko_mfa_prompt_is_answered_after_partial_success_password() {
    let _otp_ledger = otp_ledger_test_guard();
    let mut auth = terminal_auth();
    auth.flow_mode = Some(AuthFlowMode::PasswordThenOtp);
    let prompt = prompt("[OTP Code]: ", false);

    // 未播种（密码尚未提交）：先密码再 OTP 的语义仍然拦住裸 OTP 提问。
    let answers = keyboard_interactive_answers(
        &auth,
        &mut KeyboardInteractiveState::default(),
        "",
        std::slice::from_ref(&prompt),
    );
    assert_eq!(answers, vec![String::new()]);

    // 前置认证 partial success（koko 的密码已通过）后必须自动回码。
    let answers = keyboard_interactive_answers(
        &auth,
        &mut KeyboardInteractiveState::first_factor_accepted(),
        "",
        std::slice::from_ref(&prompt),
    );
    assert_eq!(answers, vec!["654321".to_string()]);
}

#[test]
fn keyboard_interactive_falls_back_to_challenge_instructions() {
    let _otp_ledger = otp_ledger_test_guard();
    let mut auth = terminal_auth();
    auth.flow_mode = Some(AuthFlowMode::PasswordThenOtp);
    // 提问文案本身认不出来时，用 koko 的指令行兜底（用户照屏幕抄进
    // OTP 提示词的也是这句话）。
    let context = keyboard_interactive_challenge_context("jumper", "Please Enter MFA Code.");
    let answers = keyboard_interactive_answers(
        &auth,
        &mut KeyboardInteractiveState::first_factor_accepted(),
        &context,
        &[prompt("Code: ", false)],
    );
    assert_eq!(answers, vec!["654321".to_string()]);

    // 指令行缺省时不得凭空回码。
    let answers = keyboard_interactive_answers(
        &auth,
        &mut KeyboardInteractiveState::first_factor_accepted(),
        &keyboard_interactive_challenge_context("jumper", ""),
        &[prompt("Code: ", false)],
    );
    assert_eq!(answers, vec![String::new()]);
}

/// issue #30：用户把 "OTP Code" 填进了密码提示词字段。旧逻辑把它判成密码
/// 提问，于是把登录密码当验证码回给服务器（必然被拒）。OTP 信号必须优先。
#[test]
fn password_hint_cannot_hijack_an_otp_prompt() {
    let _otp_ledger = otp_ledger_test_guard();
    let mut auth = terminal_auth();
    auth.flow_mode = Some(AuthFlowMode::PasswordThenOtp);
    auth.password_prompt_hint = "OTP Code".into();
    assert_eq!(
        classify_auth_prompt("[OTP Code]: ", &auth),
        Some(PromptKind::Totp)
    );
    let answers = keyboard_interactive_answers(
        &auth,
        &mut KeyboardInteractiveState::first_factor_accepted(),
        "",
        &[prompt("[OTP Code]: ", false)],
    );
    assert_eq!(answers, vec!["654321".to_string()]);
    assert_ne!(answers[0], auth.password);

    // 内置密码模式仍然把真正的密码提问判成密码（提示词重叠不影响）。
    assert_eq!(
        classify_auth_prompt("<user>@root@host's password: ", &auth),
        Some(PromptKind::Password)
    );
}

/// 阿里云 Bastionhost 的官方 SSH 运维形态：说明行 "Two-Step Verification
/// required" + 提问行 "Please Input Mfa Code (SMS):"。提问行必须凭内置
/// pattern 识别（不经提示词）；说明行是上下文——既不构成验证码提问，
/// 也不得让文案未知的提问回码。
#[test]
fn aliyun_bastionhost_prompt_shapes() {
    let _otp_ledger = otp_ledger_test_guard();
    let mut auth = terminal_auth();
    auth.flow_mode = Some(AuthFlowMode::PasswordThenOtp);

    // 提问行原文命中，无需任何提示词。
    assert_eq!(
        classify_auth_prompt("Please Input Mfa Code (SMS): ", &auth),
        Some(PromptKind::Totp)
    );

    // 说明行单独出现不构成验证码提问；未知提问借它兜底也不得回码。
    assert_eq!(
        classify_auth_prompt("Two-Step Verification required", &auth),
        None
    );
    let context =
        keyboard_interactive_challenge_context("jumper", "Two-Step Verification required");
    let answers = keyboard_interactive_answers(
        &auth,
        &mut KeyboardInteractiveState::first_factor_accepted(),
        &context,
        &[prompt("Code: ", false)],
    );
    assert_eq!(answers, vec![String::new()]);

    // 完整一问（真实说明 + 真实提问）照常回码。
    let answers = keyboard_interactive_answers(
        &auth,
        &mut KeyboardInteractiveState::first_factor_accepted(),
        &context,
        &[prompt("Please Input Mfa Code (SMS): ", true)],
    );
    assert_eq!(answers, vec!["654321".to_string()]);
}

/// 登录期 2FA 组合矩阵：流程模式 × 服务器提问形态 × 首因子是否已过。
/// 每个用例用各自独立的静态码，避免进程级防重放台账把后一个用例吃掉。
#[test]
fn keyboard_interactive_combination_matrix() {
    let _otp_ledger = otp_ledger_test_guard();
    struct Case {
        mode: AuthFlowMode,
        first_factor: bool,
        context: &'static str,
        prompts: Vec<&'static str>,
        expect: Vec<&'static str>,
        code: &'static str,
    }
    let cases = vec![
        // off：密码类提问照答，OTP 永不自动回码（即使首因子已过）。
        Case {
            mode: AuthFlowMode::Off,
            first_factor: true,
            context: "",
            prompts: vec!["Password:"],
            expect: vec!["pw"],
            code: "111111",
        },
        Case {
            mode: AuthFlowMode::Off,
            first_factor: true,
            context: "",
            prompts: vec!["[OTP Code]: "],
            expect: vec![""],
            code: "222222",
        },
        // password_only：同上，只服务密码。
        Case {
            mode: AuthFlowMode::PasswordOnly,
            first_factor: true,
            context: "",
            prompts: vec!["[OTP Code]: "],
            expect: vec![""],
            code: "333333",
        },
        // 先密码再 OTP：首因子已过则回码；未过则留空（保护）。
        Case {
            mode: AuthFlowMode::PasswordThenOtp,
            first_factor: true,
            context: "",
            prompts: vec!["[OTP Code]: "],
            expect: vec!["444444"],
            code: "444444",
        },
        Case {
            mode: AuthFlowMode::PasswordThenOtp,
            first_factor: false,
            context: "",
            prompts: vec!["[OTP Code]: "],
            expect: vec![""],
            code: "555555",
        },
        // 合并提问（服务器把密码与验证码放一行）：两种 OTP 模式都拼接——
        // 只回密码对真正的合并提问无法通过。
        Case {
            mode: AuthFlowMode::PasswordPlusOtp,
            first_factor: false,
            context: "",
            prompts: vec!["Password: OTP Code: "],
            expect: vec!["pw888888"],
            code: "888888",
        },
        Case {
            mode: AuthFlowMode::PasswordThenOtp,
            first_factor: true,
            context: "",
            prompts: vec!["Password: OTP Code: "],
            expect: vec!["pw666666"],
            code: "666666",
        },
        // 密码类模式仍只回密码半边（永不自动回 OTP）。
        Case {
            mode: AuthFlowMode::PasswordOnly,
            first_factor: true,
            context: "",
            prompts: vec!["Password: OTP Code: "],
            expect: vec!["pw"],
            code: "161616",
        },
        Case {
            mode: AuthFlowMode::Off,
            first_factor: true,
            context: "",
            prompts: vec!["Password: OTP Code: "],
            expect: vec!["pw"],
            code: "171717",
        },
        // MFA 先问（反问顺序）：先密码再 OTP 留空；+合并模式不设密码门槛，回码。
        Case {
            mode: AuthFlowMode::PasswordThenOtp,
            first_factor: false,
            context: "",
            prompts: vec!["Please enter OTP code"],
            expect: vec![""],
            code: "121212",
        },
        Case {
            mode: AuthFlowMode::PasswordPlusOtp,
            first_factor: false,
            context: "",
            prompts: vec!["Please enter OTP code"],
            expect: vec!["131313"],
            code: "131313",
        },
        // 提问文案认不出时用挑战 name/instructions 兜底（koko 形状）。
        Case {
            mode: AuthFlowMode::PasswordThenOtp,
            first_factor: true,
            context: "Please Enter MFA Code.",
            prompts: vec!["Code: "],
            expect: vec!["141414"],
            code: "141414",
        },
    ];
    for case in cases {
        let auth = SudoAuth {
            password: "pw".into(),
            totp_secrets: parse_totp_secrets(case.code),
            password_prompt_hint: String::new(),
            totp_prompt_hint: String::new(),
            flow_mode: Some(case.mode),
            ..Default::default()
        };
        let mut state = if case.first_factor {
            KeyboardInteractiveState::first_factor_accepted()
        } else {
            KeyboardInteractiveState::default()
        };
        let prompts: Vec<_> = case
            .prompts
            .iter()
            .map(|text| prompt(text, false))
            .collect();
        let answers = keyboard_interactive_answers(&auth, &mut state, case.context, &prompts);
        let expected: Vec<String> = case.expect.iter().map(|text| text.to_string()).collect();
        assert_eq!(
            answers, expected,
            "mode={:?} first_factor={} prompts={:?} context={:?}",
            case.mode, case.first_factor, case.prompts, case.context
        );
    }

    // 同一轮内先密码、后验证码：in-round 提升，不依赖 partial success
    // （PAM 风格主机把密码放在 KI 里问）。
    let auth = SudoAuth {
        password: "pw".into(),
        totp_secrets: parse_totp_secrets("151515"),
        password_prompt_hint: String::new(),
        totp_prompt_hint: String::new(),
        flow_mode: Some(AuthFlowMode::PasswordThenOtp),
        ..Default::default()
    };
    let mut state = KeyboardInteractiveState::default();
    let answers = keyboard_interactive_answers(
        &auth,
        &mut state,
        "",
        &[prompt("Password:", false), prompt("OTP Code: ", false)],
    );
    assert_eq!(answers, vec!["pw".to_string(), "151515".to_string()]);
}

#[test]
fn terminal_auto_sudo_answers_sudo_password() {
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(terminal_auth())));
    assert_eq!(
        auto.observe("[sudo] password for user: "),
        Some((AutoSudoKind::Password, "pw".to_string()))
    );
    // A shell prompt resets the auth sequence.
    assert_eq!(auto.observe("user@host:~$ "), None);
    // Direct sudo prompts keep working after the reset.
    assert_eq!(
        auto.observe("We trust you... [sudo] password for user:"),
        Some((AutoSudoKind::Password, "pw".to_string()))
    );
}

#[test]
fn terminal_auto_sudo_ignores_fullscreen_redraws() {
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(terminal_auth())));
    // Entering the alternate screen buffer (vim/less/top) must disable the
    // watcher: a redraw echoes file content, including lines that read
    // exactly like a sudo prompt.
    assert_eq!(auto.observe("\x1b[?1049h\x1b[H\x1b[2J"), None);
    assert_eq!(auto.observe("\x1b[3;1H[sudo] password for user:"), None);
    assert_eq!(
        auto.observe("\x1b[4;1H    password:\x1b[5;1H\"conf.yml\" 12L, 340B"),
        None
    );
    // Leaving the alternate screen re-arms the watcher.
    assert_eq!(auto.observe("\x1b[?1049l"), None);
    assert_eq!(
        auto.observe("[sudo] password for user: "),
        Some((AutoSudoKind::Password, "pw".to_string()))
    );
}

#[test]
fn terminal_auto_sudo_tracks_mode_switches_split_across_chunks() {
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(terminal_auth())));
    assert_eq!(auto.observe("\x1b[?10"), None);
    assert_eq!(auto.observe("49h"), None);
    assert_eq!(auto.observe("[sudo] password for user: "), None);
    assert_eq!(auto.observe("\x1b[?1049l"), None);
    assert_eq!(
        auto.observe("[sudo] password for user: "),
        Some((AutoSudoKind::Password, "pw".to_string()))
    );
}

#[test]
fn terminal_auto_sudo_ignores_file_content_that_mentions_passwords() {
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(terminal_auth())));
    // Config key with a value: the line does not end with the prompt colon.
    assert_eq!(auto.observe("password: hunter2"), None);
    // Nested YAML key: indented, so it never sits in the prompt column.
    assert_eq!(auto.observe("      password:"), None);
    // A cat/grep dump is long, so it cannot be a live prompt.
    let dump = "spring:\n  datasource:\n    url: jdbc:mysql://db:3306/app\n    username: app\n    password:\n    driver: com.zaxxer.hikari.HikariDataSource\n    lazy: true\n    pool-name: main\npassword:\n";
    assert_eq!(auto.observe(dump), None);
    // A real prompt, even behind the echoed command, still answers.
    assert_eq!(
        auto.observe("user@host:~$ sudo ls\n[sudo] password for user:"),
        Some((AutoSudoKind::Password, "pw".to_string()))
    );
}

#[test]
fn terminal_auto_sudo_requires_hints_for_generic_otp_prompts() {
    let _otp_ledger = otp_ledger_test_guard();
    // Without a custom hint, generic OTP prompts stay untouched.
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(terminal_auth())));
    assert_eq!(auto.observe("Verification code: "), None);

    // With a hint but password_then_otp mode, a standalone OTP prompt is
    // still skipped: the password must come first.
    let mut hinted = terminal_auth();
    hinted.totp_prompt_hint = "verification code".into();
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(hinted.clone())));
    assert_eq!(auto.observe("sudo: verification code: "), None);

    // password_plus_otp hosts ask for the OTP directly.
    hinted.flow_mode = Some(AuthFlowMode::PasswordPlusOtp);
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(hinted)));
    assert_eq!(
        auto.observe("sudo: verification code: "),
        Some((AutoSudoKind::Totp, "654321".to_string()))
    );
    // The OTP is only answered once per auth sequence.
    assert_eq!(auto.observe("sudo: verification code: "), None);
}

#[test]
fn terminal_auto_sudo_answers_merged_prompt() {
    let _otp_ledger = otp_ledger_test_guard();
    // 终端内的合并提问（密码与验证码同一条）：OTP 模式拼接应答，不必先
    // 出现过密码提示——提问本身就带着密码。每个用例换一个码，避免进程级
    // 防重放台账吃掉后续断言。
    let merged = |mode: AuthFlowMode, code: &str| {
        let mut auth = terminal_auth();
        auth.flow_mode = Some(mode);
        auth.totp_secrets = parse_totp_secrets(code);
        auth
    };
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(merged(
        AuthFlowMode::PasswordThenOtp,
        "654321",
    ))));
    assert_eq!(
        auto.observe("[sudo] password for user: OTP Code: "),
        Some((AutoSudoKind::Password, "pw654321".to_string()))
    );
    // 密码类模式仍只回密码半边（永不自动回 OTP）。
    let mut only = TerminalAutoSudo::new(Arc::new(RwLock::new(merged(
        AuthFlowMode::PasswordOnly,
        "654322",
    ))));
    assert_eq!(
        only.observe("[sudo] password for user: OTP Code: "),
        Some((AutoSudoKind::Password, "pw".to_string()))
    );
}

#[test]
fn terminal_auto_sudo_answers_totp_after_sudo_password() {
    let _otp_ledger = otp_ledger_test_guard();
    let mut auth = terminal_auth();
    auth.totp_prompt_hint = "verification code".into();
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(auth)));
    assert_eq!(
        auto.observe("[sudo] password for user: "),
        Some((AutoSudoKind::Password, "pw".to_string()))
    );
    assert_eq!(
        auto.observe("verification code: "),
        Some((AutoSudoKind::Totp, "654321".to_string()))
    );
    // Shell prompt closes the sequence and re-arms everything.
    assert_eq!(auto.observe("user@host:~$ "), None);
    assert_eq!(
        auto.observe("[sudo] password for user: "),
        Some((AutoSudoKind::Password, "pw".to_string()))
    );
}

#[test]
fn terminal_auto_sudo_without_password_stays_silent() {
    let mut auth = terminal_auth();
    auth.password = String::new();
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(auth)));
    assert_eq!(auto.observe("[sudo] password for user: "), None);
}

// 并发同靶 sudo：同一 otp_ledger_scope 的另一个会话已提交当前窗口唯一的码，
// 后到的 OTP 提示应推迟到窗口滚动后补答，而不是永远晾在提示符上。
#[test]
fn concurrent_same_target_otp_prompt_defers_then_answers_next_window() {
    let _otp_ledger = otp_ledger_test_guard();
    let mut auth = terminal_auth();
    auth.totp_prompt_hint = "verification code".into();
    auth.totp_secrets = vec![TotpSecret::Key {
        key: RFC_KEY.to_vec(),
        digits: 6,
        period: 30,
        algorithm: TotpAlgorithm::Sha1,
    }];
    let scope = auth.otp_ledger_scope.clone();
    let fingerprint = otp_secret_fingerprint(&auth.totp_secrets[0]);

    // 模拟会话 A：已提交当前窗口的码（提交账本在重放窗口内）。
    let now = unix_now();
    let period = 30u64;
    let counter = now / period;
    let code = hotp(RFC_KEY, counter, 6, TotpAlgorithm::Sha1);
    committed_otp_ledger()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(
            format!("{scope}|{fingerprint}|{code}"),
            totp_replay_window_expiry((counter + 1) * period, period),
        );

    // 会话 B：密码已答，OTP 提示撞上重放保护 → 跳过并进入推迟态。
    let mut auto = TerminalAutoSudo::new(Arc::new(RwLock::new(auth)));
    assert_eq!(
        auto.observe("[sudo] password for user: "),
        Some((AutoSudoKind::Password, "pw".to_string()))
    );
    assert_eq!(auto.observe("verification code: "), None);
    // 窗口未滚动前不得补答。
    assert_eq!(auto.take_deferred_otp(now), None);

    // 窗口滚动：旧提交过期（回拨账本时间模拟时钟前进），推迟重试到期。
    let boundary = (counter + 1) * period + 1;
    committed_otp_ledger()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(
            format!("{scope}|{fingerprint}|{code}"),
            now.saturating_sub(1),
        );
    let answered = auto.take_deferred_otp(boundary);
    assert_eq!(answered.map(|(kind, _)| kind), Some(AutoSudoKind::Totp));
    // 推迟态一次性：补答后不再重复注入。
    assert_eq!(auto.take_deferred_otp(boundary), None);
}

#[test]
fn shell_prompt_detection_matches_last_line() {
    assert!(has_shell_prompt("output line\nuser@host:~$ "));
    assert!(has_shell_prompt("root@host:~# "));
    assert!(!has_shell_prompt("[sudo] password for user:"));
    assert!(!has_shell_prompt(""));
}

#[test]
fn parses_server_metrics_output() {
    let output = "\
hostname=web-01
kernel=6.1.0-18-amd64
loadavg=0.28 0.42 0.35 1/887 23456
uptime=987654.32 456789.01
nproc=8
--mem--
MemTotal:       16308856 kB
MemFree:        1234567 kB
MemAvailable:   12000000 kB
SwapTotal:      2047996 kB
SwapFree:       2047996 kB
--cpu--
cpu  100 0 200 8000 40 0 0 0 0 0
cpu  200 0 300 8100 40 0 0 0 0 0
--df--
/dev/sda1 51469868 23456780 25879924 48% /
tmpfs 8154428 0 8154428 0% /dev/shm
/dev/sdb1 103080888 53456780 44349988 55% /data
";
    let metrics = parse_metrics_output(output);
    assert_eq!(metrics["hostname"], "web-01");
    assert_eq!(metrics["cpu"]["cores"], 8);
    assert_eq!(metrics["cpu"]["load1"], 0.28);
    assert_eq!(metrics["uptimeSeconds"], 987654);
    // busy delta = (200-100)+(300-200) = 200; total delta = 200+100 = 300 → 66.7%
    assert_eq!(metrics["cpu"]["percent"], 66.7);
    assert_eq!(metrics["memory"]["totalBytes"], 16_308_856_u64 * 1024);
    assert_eq!(
        metrics["memory"]["usedBytes"],
        (16_308_856_u64 - 12_000_000) * 1024
    );
    assert_eq!(metrics["memory"]["swapUsedBytes"], 0);
    let disks = metrics["disks"].as_array().unwrap();
    assert_eq!(disks.len(), 3);
    assert_eq!(disks[1]["mount"], "/dev/shm");
    assert_eq!(disks[2]["percentUsed"], 55.0);
}

#[test]
fn meminfo_without_mem_available_falls_back_to_free() {
    // MSYS（Windows 上 Git-Bash sh 的 /proc 垫片）只有 MemTotal/MemFree：
    // 旧内核同形。缺 MemAvailable 时回退 classic free+buffers+cached，
    // 否则 used=total，本地终端信息带 MEM 恒定 100%。
    let output = "\
hostname=win-pc
--mem--
MemTotal:       16772124 kB
MemFree:        10700520 kB
SwapTotal:       2490368 kB
SwapFree:        2490368 kB
--cpu--
cpu  100 0 200 8000
cpu  200 0 300 8100
";
    let metrics = parse_metrics_output(output);
    assert_eq!(metrics["memory"]["totalBytes"], 16_772_124_u64 * 1024);
    assert_eq!(metrics["memory"]["availableBytes"], 10_700_520_u64 * 1024);
    assert_eq!(
        metrics["memory"]["usedBytes"],
        (16_772_124_u64 - 10_700_520) * 1024
    );
    // 老内核的 Buffers/Cached 也在估计之列。
    let with_buffers = parse_metrics_output(
        "--mem--\nMemTotal: 100 kB\nMemFree: 10 kB\nBuffers: 5 kB\nCached: 8 kB\n",
    );
    assert_eq!(with_buffers["memory"]["availableBytes"], 23 * 1024);
}

#[test]
fn meminfo_fallback_is_clamped_to_total() {
    // 异常数据源可能让 free+buffers+cached 超过 MemTotal：available 钳到
    // total、used 归零，MEM 带不至于倒挂（classic free 同样夹紧）。
    let metrics = parse_metrics_output("--mem--\nMemTotal: 100 kB\nMemFree: 900 kB\n");
    assert_eq!(metrics["memory"]["totalBytes"], 100 * 1024);
    assert_eq!(metrics["memory"]["availableBytes"], 100 * 1024);
    assert_eq!(metrics["memory"]["usedBytes"], 0);
}

#[test]
fn df_section_rejects_os_release_bleed() {
    // 段切换认所有 `--name--` 标记，--os-- 之后的 os-release 行整体跳过；
    // 行形校验只兜底非标记形状的杂散行。VERSION_ID 一行在形状上完全合法
    // （末 token 以 / 开头、三列数字、% 后缀）——没有标记识别时它会变成
    // 挂载点 /ghost" 的幽灵磁盘行。
    let output = "\
--df--
/dev/vda1 41022688 16785408 22573568 44% /
--os--
NAME=\"Alibaba Cloud Linux\"
PRETTY_NAME=\"Alibaba Cloud Linux release 3 (OpenAnolis)\"
VERSION_ID=\"3 2 4096 100 55% /ghost\"
";
    let metrics = parse_metrics_output(output);
    let disks = metrics["disks"].as_array().unwrap();
    assert_eq!(disks.len(), 1);
    assert_eq!(disks[0]["mount"], "/");
}

#[test]
fn df_row_with_spaces_in_filesystem_parses() {
    // Windows 本地采集（Git-Bash sh → MSYS df）：文件系统名含空格
    // （`C:/Program Files/Git` 即 Git 安装根/系统盘）。列右锚定后整行
    // 可解析，左锚定会把 C 盘从本地终端的磁盘列表里丢掉。
    let output = "\
--df--
E:                     262126588 184832468  77294120      71% /e
C:/Program Files/Git   100955784  60314920  40640864      60% /
";
    let metrics = parse_metrics_output(output);
    let disks = metrics["disks"].as_array().unwrap();
    assert_eq!(disks.len(), 2);
    assert_eq!(disks[0]["filesystem"], "E:");
    assert_eq!(disks[0]["mount"], "/e");
    assert_eq!(disks[1]["filesystem"], "C:/Program Files/Git");
    assert_eq!(disks[1]["mount"], "/");
    assert_eq!(disks[1]["totalBytes"], 100_955_784_u64 * 1024);
    assert_eq!(disks[1]["percentUsed"], 60.0);
}

#[test]
fn parses_disk_usage_for_a_path() {
    let usage = parse_disk_usage(
        "Filesystem     1024-blocks      Used Available Capacity Mounted on\n/dev/sda1         51469868  23456780  25879924      48% /",
    )
    .unwrap();
    assert_eq!(usage["filesystem"], "/dev/sda1");
    assert_eq!(usage["mount"], "/");
    assert_eq!(usage["totalBytes"], 51_469_868_u64 * 1024);
    assert_eq!(usage["percentUsed"], 48.0);
    assert!(parse_disk_usage("no output").is_none());
}

#[test]
fn parses_disk_usage_for_non_device_filesystems() {
    // Containers (overlay) and tmpfs mounts do not start with '/' —
    // found by the local_ubuntu MCP coverage pass against the
    // dbx-ssh-test container.
    let usage = parse_disk_usage(
        "Filesystem     1024-blocks      Used Available Capacity Mounted on\noverlay           91029504  76153516  14875988      84% /",
    )
    .unwrap();
    assert_eq!(usage["filesystem"], "overlay");
    assert_eq!(usage["mount"], "/");
    assert_eq!(usage["totalBytes"], 91_029_504_u64 * 1024);
    assert_eq!(usage["availableBytes"], 14_875_988_u64 * 1024);
    assert_eq!(usage["percentUsed"], 84.0);
}

#[test]
fn shell_quotes_paths() {
    assert_eq!(shell_quote("/var/log/it's"), "'/var/log/it'\\''s'");
}

#[test]
fn keepalive_failure_step_stops_after_two_consecutive_failures() {
    // Success resets the counter.
    assert_eq!(keepalive_failure_step(0, true), Some(0));
    assert_eq!(keepalive_failure_step(1, true), Some(0));
    // First failure keeps the loop alive, second one stops it.
    assert_eq!(keepalive_failure_step(0, false), Some(1));
    assert_eq!(keepalive_failure_step(1, false), None);
}

#[test]
fn totp_replay_window_covers_the_one_step_slack() {
    // A 30s code valid until T stays blocked until T + one period.
    assert_eq!(totp_replay_window_expiry(1_000, 30), 1_030);
    assert_eq!(totp_replay_window_expiry(60, 90), 150);
    assert!(is_totp_in_replay_window(1_030, 1_030));
    assert!(!is_totp_in_replay_window(1_031, 1_030));
    assert!(is_totp_in_replay_window(500, 1_030));
}

#[test]
fn take_totp_answer_skips_recommitted_codes_within_window() {
    let _otp_ledger = otp_ledger_test_guard();
    let auth = SudoAuth {
        password: "pw".into(),
        totp_secrets: parse_totp_secrets("654321"),
        ..Default::default()
    };
    let first = auth.take_totp_answer().expect("first submission");
    assert_eq!(first, "654321");
    let error = auth.take_totp_answer().expect_err("replay must be skipped");
    assert!(
        error.contains("already submitted") && error.contains("replay window"),
        "{error}"
    );
    // answer_for routes through the same guard, so prompts inside the
    // window get no answer instead of a duplicated injection.
    assert_eq!(auth.answer_for(PromptKind::Totp), None);
}

#[test]
fn committed_otp_state_shares_across_clones() {
    let _otp_ledger = otp_ledger_test_guard();
    let auth = SudoAuth {
        totp_secrets: parse_totp_secrets("334455"),
        ..Default::default()
    };
    assert!(auth.take_totp_answer().is_ok());
    let clone = auth.clone();
    let error = clone
        .take_totp_answer()
        .expect_err("clone shares the state");
    assert!(error.contains("already submitted"), "{error}");
}
