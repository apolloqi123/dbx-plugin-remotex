use super::*;

#[test]
fn keyboard_interactive_round_budget_is_bounded() {
    // 防御性锁定：服务器循环重问时必须有确定性失败，而不是无限轮询；
    // 静默改动这个预算会改变堡垒机（koko MFA 多轮）的登录语义。
    assert_eq!(KEYBOARD_INTERACTIVE_MAX_ROUNDS, 4);
}

#[test]
fn keyboard_interactive_guidance_explains_unanswered_and_rejected() {
    // 没有回答：指引配置 TOTP 种子 / OTP 提示词（Quick Sudo 2FA 设置）。
    let unanswered = keyboard_interactive_guidance(&["\"[OTP Code]: \"".to_string()], false);
    assert!(unanswered.contains("the server asked for"), "{unanswered}");
    assert!(unanswered.contains("nothing was answered"), "{unanswered}");
    assert!(unanswered.contains("TOTP secret"), "{unanswered}");

    // 回答被拒：不再引导去配置（已配置但服务器拒收），只报拒绝事实。
    let rejected = keyboard_interactive_guidance(&["\"Password: \"".to_string()], true);
    assert!(
        rejected.contains("rejected the submitted answer"),
        "{rejected}"
    );
    assert!(!rejected.contains("TOTP secret"), "{rejected}");

    // 空提问列表：不追加任何子句（连接错误保持精简）。
    assert_eq!(keyboard_interactive_guidance(&[], false), "");
}

#[test]
fn keyboard_interactive_manual_challenge_dedupes_and_sanitizes() {
    // 服务器可控文本：ANSI 转义剥离（着色 prompt 还原成纯文本，转义序列
    // 进不了对话框）、重复部分去重（koko 的 instructions 与 prompt 常语义
    // 重复）、空输入为空串。
    let challenge = keyboard_interactive_manual_challenge(
        "Please Enter MFA Code.",
        "\u{1b}[1m[OTP Code]: \u{1b}[0m",
    );
    assert!(challenge.contains("Please Enter MFA Code."), "{challenge}");
    // normalize 会对每段 trim，行尾空格被剥掉是既有归一化语义。
    assert!(challenge.contains("[OTP Code]:"), "{challenge}");
    assert!(!challenge.contains('\u{1b}'), "{challenge}");

    let deduped = keyboard_interactive_manual_challenge("Same line.", "Same line.");
    assert_eq!(deduped, "Same line.");
    assert_eq!(keyboard_interactive_manual_challenge("  ", "\t"), "");
}

#[test]
fn challenge_display_text_dedupes_seen_wording() {
    let mut seen: Vec<String> = Vec::new();
    let first = challenge_display_text("Please Enter MFA Code.", &seen);
    assert_eq!(first.as_deref(), Some("Please Enter MFA Code."));
    seen.push(first.expect("first display"));
    // 同一段文案第二次出现（instructions 与 prompt 重复）不再上屏。
    assert_eq!(
        challenge_display_text("Please Enter MFA Code.", &seen),
        None
    );
    // 空文案与纯控制字符文案都不产生展示片段。
    assert_eq!(challenge_display_text("", &[]), None);
    assert_eq!(challenge_display_text("\u{1b}\u{7}", &[]), None);
}

/// 登录期 2FA 的端到端回归（issue #17 / #30）：密码/公钥先被接受后服务器
/// 用 keyboard-interactive 问 MFA（koko 形状）。mock 服务端按 `Shape`
/// 参数化，覆盖"密码方法 + partial success"、"KI 里先密码再 MFA"、
/// "只问 MFA"、"一个合并提问"四种提问形态。
mod koko_login {
    use super::*;
    use dbx_plugin_sdk::PluginTransport;
    use russh::keys::ssh_key::private::{Ed25519Keypair, Ed25519PrivateKey};
    use russh::server::{Auth, ChannelOpenHandle, Msg, Response, Server as _, Session};
    use russh::{Channel, ChannelId, MethodKind, MethodSet, Pty};
    use std::net::SocketAddr;

    const LOGIN_PASSWORD: &str = "jump-pw";
    const SUDO_PASSWORD: &str = "asset-pw";
    const MFA_CODE: &str = "654321";
    // 逐字取自 koko pkg/auth/mfa_option.go（mfaOptionInstruction /
    // mfaOptionQuestion，MFA 类型 otp）。
    const MFA_INSTRUCTION: &str = "Please Enter MFA Code.";
    const MFA_QUESTION: &str = "[OTP Code]: ";

    /// mock 堡垒机的认证形态。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Shape {
        /// 开放 password 方法：密码通过后 partial success → KI 问 MFA（koko 默认）。
        PasswordThenMfa,
        /// 开放 password 方法：密码全拒（无 partial success）→ KI 仍问
        /// MFA（PAM 栈不置 partial_success 位的形状）。
        PasswordRejectedThenMfa,
        /// 只开放 keyboard-interactive：KI 第一轮问密码，第二轮问 MFA（PAM 风格）。
        KiPasswordThenMfa,
        /// 只开放 keyboard-interactive：只问 MFA（反问顺序主机）。
        KiMfaOnly,
        /// 只开放 keyboard-interactive：一个提问同时要密码与验证码。
        KiCombined,
    }

    impl Shape {
        fn allows_password_method(self) -> bool {
            matches!(
                self,
                Shape::PasswordThenMfa | Shape::PasswordRejectedThenMfa
            )
        }
    }

    /// 测试专用 Ed25519 密钥：由固定种子展开，不依赖 RNG 版本，也绝不
    /// 涉及真实私钥（仅供本机 mock 服务器与客户端握手）。
    fn test_key(seed: u8) -> russh::keys::PrivateKey {
        russh::keys::PrivateKey::from(Ed25519Keypair::from_seed(
            &[seed; Ed25519PrivateKey::BYTE_SIZE],
        ))
    }

    #[derive(Clone)]
    struct MockKoko {
        shape: Shape,
        instruction: &'static str,
        prompt: &'static str,
        answers: Arc<Mutex<Vec<String>>>,
        exec_seen: Arc<Mutex<Vec<String>>>,
    }

    impl MockKoko {
        fn new(shape: Shape, instruction: &'static str, prompt: &'static str) -> Self {
            Self {
                shape,
                instruction,
                prompt,
                answers: Arc::new(Mutex::new(Vec::new())),
                exec_seen: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    struct MockKokoSession {
        shape: Shape,
        instruction: &'static str,
        prompt: &'static str,
        answers: Arc<Mutex<Vec<String>>>,
        exec_seen: Arc<Mutex<Vec<String>>>,
        ki_round: usize,
    }

    impl russh::server::Handler for MockKokoSession {
        type Error = russh::Error;

        async fn channel_open_session(
            &mut self,
            _channel: Channel<Msg>,
            reply: ChannelOpenHandle,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            reply.accept().await;
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            self.exec_seen
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(data).into_owned());
            session.channel_success(channel)?;
            session.data(channel, b"/bin/bash".to_vec())?;
            session.eof(channel)?;
            session.close(channel)?;
            Ok(())
        }

        #[allow(clippy::too_many_arguments)]
        async fn pty_request(
            &mut self,
            channel: ChannelId,
            _term: &str,
            _col_width: u32,
            _row_height: u32,
            _pix_width: u32,
            _pix_height: u32,
            _modes: &[(Pty, u32)],
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            session.channel_success(channel)?;
            Ok(())
        }

        async fn shell_request(
            &mut self,
            channel: ChannelId,
            session: &mut Session,
        ) -> Result<(), Self::Error> {
            session.channel_success(channel)?;
            Ok(())
        }

        async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
            Ok(Auth::reject())
        }

        async fn auth_password(
            &mut self,
            _user: &str,
            password: &str,
        ) -> Result<Auth, Self::Error> {
            if password != LOGIN_PASSWORD {
                return Ok(Auth::reject());
            }
            if self.shape == Shape::PasswordRejectedThenMfa {
                // 全拒形状：第一因子从未通过（无 partial success），但
                // 服务器仍把 keyboard-interactive 列为可继续的方法。
                return Ok(Auth::reject());
            }
            // koko: 第一因子通过 → PartialSuccessError，下一步
            // keyboard-interactive 问 MFA。
            let mut methods = MethodSet::empty();
            methods.push(MethodKind::KeyboardInteractive);
            Ok(Auth::Reject {
                proceed_with_methods: Some(methods),
                partial_success: true,
            })
        }

        async fn auth_publickey(
            &mut self,
            _user: &str,
            _public_key: &russh::keys::PublicKey,
        ) -> Result<Auth, Self::Error> {
            // koko 的私钥 + MFA：公钥被接受后仍要二次认证。
            let mut methods = MethodSet::empty();
            methods.push(MethodKind::KeyboardInteractive);
            Ok(Auth::Reject {
                proceed_with_methods: Some(methods),
                partial_success: true,
            })
        }

        async fn auth_keyboard_interactive<'a>(
            &'a mut self,
            _user: &str,
            _submethods: &str,
            response: Option<Response<'a>>,
        ) -> Result<Auth, Self::Error> {
            let Some(mut response) = response else {
                // 开局提问：PAM 形状先问密码；合并形状一个提问问两样。
                let (instruction, prompt) = match self.shape {
                    Shape::KiPasswordThenMfa => ("Please enter your password.", "Password: "),
                    Shape::KiCombined => ("Password and MFA required", "Password: OTP Code: "),
                    _ => (self.instruction, self.prompt),
                };
                return Ok(Auth::Partial {
                    name: "jumper".into(),
                    instructions: instruction.into(),
                    prompts: vec![(prompt.into(), self.shape == Shape::KiPasswordThenMfa)].into(),
                });
            };
            let answer = response
                .next()
                .map(|bytes| String::from_utf8_lossy(bytes.as_ref()).into_owned())
                .unwrap_or_default();
            self.answers.lock().unwrap().push(answer.clone());
            self.ki_round += 1;
            match self.shape {
                // 第一轮是密码：答对后继续问 MFA。
                Shape::KiPasswordThenMfa if self.ki_round == 1 => {
                    if answer != LOGIN_PASSWORD {
                        return Ok(Auth::reject());
                    }
                    Ok(Auth::Partial {
                        name: "jumper".into(),
                        instructions: self.instruction.into(),
                        prompts: vec![(self.prompt.into(), true)].into(),
                    })
                }
                // 合并提问：一条应答里必须同时有密码与验证码。
                Shape::KiCombined => Ok(if answer == format!("{LOGIN_PASSWORD}{MFA_CODE}") {
                    Auth::Accept
                } else {
                    Auth::reject()
                }),
                _ => Ok(if answer == MFA_CODE {
                    Auth::Accept
                } else {
                    Auth::reject()
                }),
            }
        }

        async fn auth_succeeded(&mut self, _session: &mut Session) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl russh::server::Server for MockKoko {
        type Handler = MockKokoSession;

        fn new_client(&mut self, _peer: Option<SocketAddr>) -> Self::Handler {
            MockKokoSession {
                shape: self.shape,
                instruction: self.instruction,
                prompt: self.prompt,
                answers: self.answers.clone(),
                exec_seen: self.exec_seen.clone(),
                ki_round: 0,
            }
        }
    }

    async fn spawn_mock_koko(
        shape: Shape,
        instruction: &'static str,
        prompt: &'static str,
    ) -> (
        u16,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Vec<String>>>,
        tokio::task::JoinHandle<()>,
    ) {
        // Unique per test run: OTP replay ledgers are scoped by user@host:port
        // and outlive the test, so a recycled port would make this test skip
        // its code as already submitted (see test_env).
        let listener = crate::test_env::bind_unique_loopback()
            .await
            .expect("bind mock koko");
        let port = listener.local_addr().expect("mock koko address").port();
        let mut methods = MethodSet::empty();
        if shape.allows_password_method() {
            methods.push(MethodKind::Password);
        }
        methods.push(MethodKind::KeyboardInteractive);
        let config = Arc::new(russh::server::Config {
            methods,
            keys: vec![test_key(1)],
            auth_rejection_time: Duration::from_millis(10),
            auth_rejection_time_initial: Some(Duration::from_millis(10)),
            ..Default::default()
        });
        let mut server = MockKoko::new(shape, instruction, prompt);
        let answers = server.answers.clone();
        let exec_seen = server.exec_seen.clone();
        let task = tokio::spawn(async move {
            let _ = server.run_on_socket(config, &listener).await;
        });
        (port, answers, exec_seen, task)
    }

    fn koko_connection(port: u16, secrets: Value, external: Value) -> StoredConnection {
        StoredConnection::from_lifecycle_params(&json!({
            "connection": {
                "id": "koko-login",
                "name": "koko",
                "db_type": "ssh",
                "host": "127.0.0.1",
                "port": port,
                "username": "jumper",
                "password": LOGIN_PASSWORD,
                "connection_secrets": secrets,
                "external_config": external,
            }
        }))
        .expect("parse koko connection")
    }

    fn test_runtime() -> Arc<SshRuntime> {
        // open_session 以 `self: &Arc<Self>` 接收者注册自发断开清理所需
        // 的运行时克隆，测试统一从 Arc 出发。
        let data_dir = std::env::temp_dir().join(format!("dbx-koko-e2e-{}", Uuid::new_v4()));
        Arc::new(SshRuntime::new(data_dir).with_auto_trust_keys())
    }

    fn test_emitter() -> PluginEmitter {
        let output: Arc<Mutex<Box<dyn std::io::Write + Send>>> =
            Arc::new(Mutex::new(Box::new(Vec::<u8>::new())));
        PluginEmitter::for_tests(output, PluginTransport::JsonLines)
    }

    struct ManualOtpGateway {
        answer: UserInputAnswer,
        prompts_seen: Mutex<Vec<Value>>,
    }

    impl ManualOtpGateway {
        fn submitting(code: &str) -> Self {
            Self {
                answer: UserInputAnswer {
                    action: "submit".into(),
                    value: Some(code.into()),
                },
                prompts_seen: Mutex::new(Vec::new()),
            }
        }

        fn cancelling() -> Self {
            Self {
                answer: UserInputAnswer {
                    action: "cancel".into(),
                    value: None,
                },
                prompts_seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl HostPromptGateway for ManualOtpGateway {
        fn supports_request_user_input(&self) -> bool {
            true
        }

        fn request_user_input(
            &self,
            prompt: &UserInputPrompt,
        ) -> Result<UserInputAnswer, PluginError> {
            self.prompts_seen
                .lock()
                .unwrap()
                .push(serde_json::to_value(prompt).unwrap());
            Ok(self.answer.clone())
        }
    }

    #[tokio::test]
    async fn unknown_chinese_mfa_prompt_requests_current_code_from_user() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::PasswordThenMfa, "请输入6位数字。", "[MFA认证]：").await;
        let gateway = Arc::new(ManualOtpGateway::submitting(MFA_CODE));
        let mut runtime = test_runtime();
        Arc::get_mut(&mut runtime).unwrap().prompts = PromptBroker::with_gateway(gateway.clone());
        let connection = koko_connection(
            port,
            json!({}),
            json!({
                "authentication": "password",
                "auth_flow_mode": "password_then_otp",
            }),
        );

        let result = runtime.connect_headless(&connection).await;
        server.abort();

        result.expect("unanswered MFA challenge must open a one-time secret prompt");
        assert_eq!(answers.lock().unwrap().as_slice(), [MFA_CODE.to_string()]);
        let prompts = gateway.prompts_seen.lock().unwrap();
        assert_eq!(prompts.len(), 1, "exactly one OTP dialog is expected");
        assert_eq!(prompts[0]["echo"], false, "the OTP input must be masked");
        assert!(prompts[0]["title"]
            .as_str()
            .unwrap()
            .starts_with("动态令牌验证"));
        assert!(prompts[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("本次输入仅用于此次登录，不会保存"));
        assert!(prompts[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("请输入6位数字。"));
        assert!(prompts[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("[MFA认证]："));
    }

    #[tokio::test]
    async fn configured_totp_keeps_login_automatic_without_user_prompt() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::PasswordThenMfa, MFA_INSTRUCTION, MFA_QUESTION).await;
        let gateway = Arc::new(ManualOtpGateway::submitting("should-not-be-used"));
        let mut runtime = test_runtime();
        Arc::get_mut(&mut runtime).unwrap().prompts = PromptBroker::with_gateway(gateway.clone());
        let connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE }),
            json!({
                "authentication": "password",
                "auth_flow_mode": "password_then_otp",
            }),
        );

        let result = runtime.connect_headless(&connection).await;
        server.abort();

        result.expect("configured TOTP must keep the existing automatic login path");
        assert_eq!(answers.lock().unwrap().as_slice(), [MFA_CODE.to_string()]);
        assert!(
            gateway.prompts_seen.lock().unwrap().is_empty(),
            "automatic TOTP must not open a redundant dialog"
        );
    }

    #[tokio::test]
    async fn cancelling_manual_mfa_prompt_fails_closed() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::PasswordThenMfa, "请输入6位数字。", "[MFA认证]：").await;
        let gateway = Arc::new(ManualOtpGateway::cancelling());
        let mut runtime = test_runtime();
        Arc::get_mut(&mut runtime).unwrap().prompts = PromptBroker::with_gateway(gateway.clone());
        let connection = koko_connection(
            port,
            json!({}),
            json!({
                "authentication": "password",
                "auth_flow_mode": "password_then_otp",
            }),
        );

        let result = runtime.connect_headless(&connection).await;
        server.abort();

        assert_eq!(
            result.err().as_deref(),
            Some("SSH keyboard-interactive authentication was cancelled")
        );
        assert!(
            answers.lock().unwrap().is_empty(),
            "cancelled prompts must not send an empty or guessed answer"
        );
        assert_eq!(gateway.prompts_seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn second_terminal_session_reuses_authenticated_transport() {
        let (port, answers, _exec, server) = spawn_mock_koko(
            Shape::PasswordThenMfa,
            "Please enter 6 digits.",
            "[MFA auth]:",
        )
        .await;
        let gateway = Arc::new(ManualOtpGateway::submitting(MFA_CODE));
        let mut runtime = test_runtime();
        Arc::get_mut(&mut runtime).unwrap().prompts = PromptBroker::with_gateway(gateway.clone());
        let connection = koko_connection(
            port,
            json!({}),
            json!({
                "authentication": "password",
                "auth_flow_mode": "off",
            }),
        );
        runtime.store_connection(connection).unwrap();

        let first = runtime
            .open_session(
                &SessionOpenRequest {
                    connection_id: "koko-login".into(),
                    workbench_id: "workbench-1".into(),
                    reuse_authenticated_transport: false,
                    reuse_authenticated_session_id: None,
                    requested_session_id: None,
                    spawn_command: None,
                    cols: 80,
                    rows: 24,
                },
                "open-1",
                test_emitter(),
            )
            .await
            .expect("first terminal session");
        let second = runtime
            .open_session(
                &SessionOpenRequest {
                    connection_id: "koko-login".into(),
                    workbench_id: "workbench-2".into(),
                    reuse_authenticated_transport: true,
                    reuse_authenticated_session_id: first["sessionId"].as_str().map(str::to_string),
                    requested_session_id: None,
                    spawn_command: None,
                    cols: 80,
                    rows: 24,
                },
                "open-2",
                test_emitter(),
            )
            .await
            .expect("second terminal session");

        assert_eq!(
            gateway.prompts_seen.lock().unwrap().len(),
            1,
            "a second PTY on the same live connection must not ask for MFA again"
        );
        assert_eq!(
            answers.lock().unwrap().as_slice(),
            [MFA_CODE.to_string()],
            "the bastion must authenticate only the first SSH transport"
        );

        for opened in [first, second] {
            let session_id = opened["sessionId"].as_str().unwrap();
            runtime.close_session(session_id).await.unwrap();
        }
        server.abort();
    }

    #[tokio::test]
    async fn spawn_command_session_runs_on_shared_transport_with_escaped_exec() {
        // WT-4 contract: a spawnCommand open on the reused transport must
        // NOT re-authenticate (bastion MFA is answered exactly once) and
        // must exec the single-quote-wrapped `sh -c '...'` payload.
        let (port, answers, exec_seen, server) = spawn_mock_koko(
            Shape::PasswordThenMfa,
            "Please enter 6 digits.",
            "[MFA auth]:",
        )
        .await;
        let gateway = Arc::new(ManualOtpGateway::submitting(MFA_CODE));
        let mut runtime = test_runtime();
        Arc::get_mut(&mut runtime).unwrap().prompts = PromptBroker::with_gateway(gateway.clone());
        let connection = koko_connection(
            port,
            json!({}),
            json!({
                "authentication": "password",
                "auth_flow_mode": "off",
            }),
        );
        runtime.store_connection(connection).unwrap();

        let base = runtime
            .open_session(
                &SessionOpenRequest {
                    connection_id: "koko-login".into(),
                    workbench_id: "workbench-1".into(),
                    reuse_authenticated_transport: false,
                    reuse_authenticated_session_id: None,
                    requested_session_id: None,
                    spawn_command: None,
                    cols: 80,
                    rows: 24,
                },
                "open-base",
                test_emitter(),
            )
            .await
            .expect("base shell session");
        let spawned = runtime
            .open_session(
                &SessionOpenRequest {
                    connection_id: "koko-login".into(),
                    workbench_id: "workbench-2".into(),
                    reuse_authenticated_transport: true,
                    reuse_authenticated_session_id: base["sessionId"].as_str().map(str::to_string),
                    requested_session_id: None,
                    spawn_command: Some("htop --tree".into()),
                    cols: 80,
                    rows: 24,
                },
                "open-spawn",
                test_emitter(),
            )
            .await
            .expect("spawn command session on the shared transport");

        assert_ne!(
            spawned["sessionId"].as_str().unwrap(),
            base["sessionId"].as_str().unwrap(),
            "the command session owns its own sessionId"
        );
        assert_eq!(
            gateway.prompts_seen.lock().unwrap().len(),
            1,
            "a command session on the shared transport must not ask for MFA again"
        );
        assert_eq!(
            answers.lock().unwrap().as_slice(),
            [MFA_CODE.to_string()],
            "the bastion must authenticate only the first SSH transport"
        );
        // russh 0.62 channel requests are fire-and-forget: `exec` only
        // queues the message, so the mock server may not have handled it
        // by the time `open_session` returns. Poll briefly instead of
        // asserting immediately.
        let exec_payload = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let seen = exec_seen.lock().unwrap().clone();
                // #10750: the base shell session now arms the prompt hook,
                // which probes `$SHELL` once via exec; the spawn session
                // execs the wrapped command. The two are independent tasks
                // on the shared transport, so neither arrival order nor
                // which lands first is deterministic — wait for both.
                let has_command = seen.iter().any(|command| command == "sh -c 'htop --tree'");
                let has_probe = seen
                    .iter()
                    .any(|command| command == "printf '%s' \"$SHELL\"");
                if has_command && has_probe {
                    break seen;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the mock server must receive the exec requests");
        assert_eq!(
            exec_payload.len(),
            2,
            "exactly one shell probe (interactive session arming) and one command exec"
        );
        assert!(
            exec_payload.contains(&"sh -c 'htop --tree'".to_string()),
            "the command is wrapped as one single-quoted sh -c argument"
        );
        assert!(
            exec_payload.contains(&"printf '%s' \"$SHELL\"".to_string()),
            "the interactive base session arms its prompt hook with one $SHELL probe"
        );

        for opened in [base, spawned] {
            let session_id = opened["sessionId"].as_str().unwrap();
            // The mock exec handler closes the channel right after the
            // reply, so the spawned session's read loop may already have
            // reaped it ("SSH session was not found") — that self-cleanup
            // on command exit is the expected command-session behavior.
            let _ = runtime.close_session(session_id).await;
        }
        server.abort();
    }

    #[test]
    fn pending_copy_reservation_keeps_transport_alive_before_session_registration() {
        let uses = TransportLeaseCounter::new();
        uses.retain(); // pending copied-session open

        assert_eq!(uses.count(), 2);
        assert!(
            !uses.release(),
            "closing the source must not release a transport reserved by a pending copy"
        );
        assert_eq!(uses.count(), 1);
        assert!(
            uses.release(),
            "the final copied session owns the last transport lease"
        );
    }

    #[tokio::test]
    async fn copied_session_without_a_live_transport_does_not_reprompt_for_mfa() {
        let gateway = Arc::new(ManualOtpGateway::submitting(MFA_CODE));
        let mut runtime = test_runtime();
        Arc::get_mut(&mut runtime).unwrap().prompts = PromptBroker::with_gateway(gateway.clone());
        runtime
            .store_connection(koko_connection(
                1,
                json!({}),
                json!({
                    "authentication": "password",
                    "auth_flow_mode": "off",
                }),
            ))
            .unwrap();

        let error = runtime
            .open_session(
                &SessionOpenRequest {
                    connection_id: "koko-login".into(),
                    workbench_id: "copied-workbench".into(),
                    reuse_authenticated_transport: true,
                    reuse_authenticated_session_id: Some("missing-session".into()),
                    requested_session_id: None,
                    spawn_command: None,
                    cols: 80,
                    rows: 24,
                },
                "copy-without-source",
                test_emitter(),
            )
            .await
            .expect_err("copying without a live source must fail before authentication");

        assert!(error.contains("use New session to reconnect"));
        assert!(gateway.prompts_seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn mfa_code_is_answered_after_partial_success_password() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::PasswordThenMfa, MFA_INSTRUCTION, MFA_QUESTION).await;
        let runtime = test_runtime();
        let connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE }),
            json!({ "authentication": "password", "auth_flow_mode": "password_then_otp" }),
        );
        let result = runtime.connect_headless(&connection).await;
        server.abort();
        result.expect("koko MFA login must succeed");
        assert_eq!(
            answers.lock().unwrap().as_slice(),
            [MFA_CODE.to_string()],
            "the MFA question must be answered with the TOTP code"
        );
    }

    /// 提问文案认不出、但挑战指令行是 koko 的 "Please Enter MFA Code."：
    /// 用户照屏幕抄进 OTP 提示词的就是这句话（issue #17 的 martin-bian）。
    #[tokio::test]
    async fn mfa_is_answered_from_challenge_instructions() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::PasswordThenMfa, MFA_INSTRUCTION, "Code: ").await;
        let runtime = test_runtime();
        let connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE }),
            json!({
                "authentication": "password",
                "auth_flow_mode": "password_then_otp",
                "totp_prompt_hint": "Please Enter MFA Code.",
            }),
        );
        let result = runtime.connect_headless(&connection).await;
        server.abort();
        result.expect("instructions-only MFA prompt must be answered");
        assert_eq!(answers.lock().unwrap().as_slice(), [MFA_CODE.to_string()]);
    }

    /// 完全自定义的提问文案靠用户提示词命中（无内置模式可依赖）。
    #[tokio::test]
    async fn mfa_is_answered_from_user_hint() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::PasswordThenMfa, "", "Enter verification token: ").await;
        let runtime = test_runtime();
        let connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE }),
            json!({
                "authentication": "password",
                "auth_flow_mode": "password_then_otp",
                "totp_prompt_hint": "verification token",
            }),
        );
        let result = runtime.connect_headless(&connection).await;
        server.abort();
        result.expect("hint-matched MFA prompt must be answered");
        assert_eq!(answers.lock().unwrap().as_slice(), [MFA_CODE.to_string()]);
    }

    /// 只开 keyboard-interactive、直接问 MFA（反问顺序主机）+ 先密码再 OTP：
    /// 保护仍然生效——不回码，且失败信息点名服务器提问与配置入口。
    #[tokio::test]
    async fn bare_mfa_prompt_stays_unanswered_before_the_password() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::KiMfaOnly, MFA_INSTRUCTION, MFA_QUESTION).await;
        let runtime = test_runtime();
        let connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE }),
            json!({ "authentication": "password", "auth_flow_mode": "password_then_otp" }),
        );
        let result = runtime.connect_headless(&connection).await;
        server.abort();
        let error = result.err().expect("bare MFA prompt must not be answered");
        assert!(
            error.contains(MFA_QUESTION.trim()),
            "error must name the server prompt: {error}"
        );
        assert!(
            !answers
                .lock()
                .unwrap()
                .iter()
                .any(|answer| answer == MFA_CODE),
            "the OTP code must not be submitted before the password step"
        );
    }

    /// 同一形态改配「密码 + OTP 合并」：反问顺序主机应能登录（表单选型指引）。
    #[tokio::test]
    async fn bare_mfa_prompt_is_answered_in_combined_mode() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::KiMfaOnly, MFA_INSTRUCTION, MFA_QUESTION).await;
        let runtime = test_runtime();
        let connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE }),
            json!({ "authentication": "password", "auth_flow_mode": "password_plus_otp" }),
        );
        let result = runtime.connect_headless(&connection).await;
        server.abort();
        result.expect("combined mode answers a bare MFA prompt");
        assert_eq!(answers.lock().unwrap().as_slice(), [MFA_CODE.to_string()]);
    }

    /// PAM 风格主机在 KI 里问密码：登录提问必须收到**登录口令**，而不是
    /// 单独的 sudo 口令（凭据混用只会认证失败，还把特权口令送给主机）。
    #[tokio::test]
    async fn login_password_prompt_gets_the_login_password() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::KiPasswordThenMfa, MFA_INSTRUCTION, MFA_QUESTION).await;
        let runtime = test_runtime();
        let connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE, "sudo_password": SUDO_PASSWORD }),
            json!({ "authentication": "password", "auth_flow_mode": "password_then_otp" }),
        );
        let result = runtime.connect_headless(&connection).await;
        server.abort();
        result.expect("PAM-style KI login must succeed");
        assert_eq!(
            answers.lock().unwrap().as_slice(),
            [LOGIN_PASSWORD.to_string(), MFA_CODE.to_string()],
            "the KI password question must get the login password, never the sudo password"
        );
    }

    /// 合并提问（一条应答同时要密码与验证码）：+合并模式拼接后通过。
    #[tokio::test]
    async fn combined_prompt_gets_password_and_code() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::KiCombined, MFA_INSTRUCTION, MFA_QUESTION).await;
        let runtime = test_runtime();
        let connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE }),
            json!({ "authentication": "password", "auth_flow_mode": "password_plus_otp" }),
        );
        let result = runtime.connect_headless(&connection).await;
        server.abort();
        result.expect("combined prompt must be answered with password + code");
        assert_eq!(
            answers.lock().unwrap().as_slice(),
            [format!("{LOGIN_PASSWORD}{MFA_CODE}")]
        );
    }

    /// 同一条合并提问在「先密码，再 OTP」下也要拼接：只回密码对真正的
    /// 合并提问必然失败（模式差异只影响"OTP 能不能单独先答"，不影响
    /// 合并提问的应答内容）。
    #[tokio::test]
    async fn combined_prompt_gets_password_and_code_in_password_then_otp() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::KiCombined, MFA_INSTRUCTION, MFA_QUESTION).await;
        let runtime = test_runtime();
        let connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE }),
            json!({ "authentication": "password", "auth_flow_mode": "password_then_otp" }),
        );
        let result = runtime.connect_headless(&connection).await;
        server.abort();
        result.expect("merged prompt must be answered in the default mode too");
        assert_eq!(
            answers.lock().unwrap().as_slice(),
            [format!("{LOGIN_PASSWORD}{MFA_CODE}")]
        );
    }

    /// global（表单隐藏 2FA 四件套、契约写明凭据由全局配置接管）：登录期
    /// MFA 也必须能读到该配置的 TOTP 与流程模式。
    #[tokio::test]
    async fn global_profile_supplies_login_mfa_credentials() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::PasswordThenMfa, MFA_INSTRUCTION, MFA_QUESTION).await;
        let runtime = test_runtime();
        let mut store = sudo_profiles::SudoProfileStore::default();
        let (profile, _) = sudo_profiles::save_profile(
            &mut store,
            &json!({
                "name": "ops",
                "sudoPassword": SUDO_PASSWORD,
                "totpSecret": MFA_CODE,
                "authFlowMode": "password_then_otp",
            }),
        )
        .expect("save global profile");
        sudo_profiles::save_store(&runtime.data_dir(), &store).expect("persist profile store");
        let connection = koko_connection(
            port,
            json!({}),
            json!({
                "authentication": "password",
                "sudo_source": "global",
                "sudo_profile": profile.name,
            }),
        );
        let result = runtime.connect_headless(&connection).await;
        server.abort();
        result.expect("global-profile connection must answer login MFA");
        assert_eq!(answers.lock().unwrap().as_slice(), [MFA_CODE.to_string()]);
    }

    /// 密码被全拒（无 partial success）且服务器仍广告 keyboard-interactive：
    /// KI 以 `first_factor_accepted=true`（无条件 seed，ssh.rs 密码编排回退
    /// 分支）起步——PAM 形状的服务器根本不置 partial_success 位，若改成
    /// 严格判定（按 partial_success seed），OTP 提问将无人应答、会话晾死。
    /// 对抗评审 #7 的固化测试：改 seed 语义前必须先让本用例失败。
    #[tokio::test]
    async fn password_full_rejection_still_seeds_first_factor_for_ki() {
        let (port, answers, _exec, server) = spawn_mock_koko(
            Shape::PasswordRejectedThenMfa,
            MFA_INSTRUCTION,
            MFA_QUESTION,
        )
        .await;
        let runtime = test_runtime();
        // 提交错误密码：mock 全拒（Auth::reject()，不置 partial_success），
        // 但 keyboard-interactive 仍在服务器的可继续方法集里。
        let mut connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE }),
            json!({
                "authentication": "password",
                "auth_flow_mode": "password_then_otp",
            }),
        );
        connection.password = "definitely-wrong".to_string();

        let result = runtime.connect_headless(&connection).await;
        server.abort();
        result.expect(
            "a full password rejection must still seed the KI first factor so the MFA question is answered",
        );
        assert_eq!(
            answers.lock().unwrap().as_slice(),
            [MFA_CODE.to_string()],
            "the OTP question must be answered even though the password step fully failed"
        );
    }

    /// 私钥被接受、服务器还要 MFA：以前直接报 "private-key authentication
    /// was rejected"，现在继续答 keyboard-interactive 验证码（issue #30 的
    /// 私钥反馈）。
    ///
    /// 说明：russh 服务端（0.60/0.63 同）在处理 Auth::Reject 时会把
    /// `partial_success` 无条件清零后回包，mock 服务器因此无法向客户端
    /// 复现"公钥已通过、还差 MFA"这一步——该路径的端到端覆盖交给
    /// `scripts/smoke/smoke_login_mfa_test.py`（paramiko 会如实置位）。
    #[tokio::test]
    async fn publickey_partial_success_is_reported_by_russh_server_as_reject() {
        let (port, answers, _exec, server) =
            spawn_mock_koko(Shape::PasswordThenMfa, MFA_INSTRUCTION, MFA_QUESTION).await;
        let runtime = test_runtime();
        let key_text = test_key(2)
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .expect("encode client key");
        let connection = koko_connection(
            port,
            json!({ "totp_secret": MFA_CODE, "private_key": key_text.to_string() }),
            json!({ "authentication": "private-key", "auth_flow_mode": "password_then_otp" }),
        );
        let result = runtime.connect_headless(&connection).await;
        server.abort();
        assert_eq!(
            result.err().as_deref(),
            Some("SSH private-key authentication was rejected"),
            "russh 服务端清零 partial success，客户端据此拒绝继续"
        );
        assert!(answers.lock().unwrap().is_empty());
    }

    #[test]
    fn preserved_permissions_keep_mode_bits_and_drop_everything_else() {
        // Issue #37: the mode's permission bits (including setuid/setgid/
        // sticky) must ride onto the staged file; the file-type bits of
        // st_mode and the uid/gid/size/timestamps must not.
        let target = russh_sftp::protocol::FileAttributes {
            size: Some(12),
            uid: Some(1000),
            gid: Some(1000),
            permissions: Some(0o100755),
            atime: Some(1),
            mtime: Some(2),
            ..Default::default()
        };
        let preserved = preserved_target_permissions(&target).expect("permissions present");
        assert_eq!(preserved.permissions, Some(0o755));
        // Only permission bits travel: the staged file already has its own
        // identity, and SETSTAT on uid/gid needs elevated privileges.
        assert_eq!(preserved.size, None);
        assert_eq!(preserved.uid, None);
        assert_eq!(preserved.user, None);
        assert_eq!(preserved.gid, None);
        assert_eq!(preserved.group, None);
        assert_eq!(preserved.atime, None);
        assert_eq!(preserved.mtime, None);
        let sticky = russh_sftp::protocol::FileAttributes {
            permissions: Some(0o101755),
            ..Default::default()
        };
        let preserved = preserved_target_permissions(&sticky).expect("permissions present");
        assert_eq!(preserved.permissions, Some(0o1755));
    }

    #[test]
    fn preserved_permissions_skip_targets_without_mode_information() {
        // Servers may omit permission attributes entirely; without a mode
        // to preserve there is nothing to SETSTAT.
        let target = russh_sftp::protocol::FileAttributes {
            size: Some(3),
            ..Default::default()
        };
        assert!(preserved_target_permissions(&target).is_none());
    }
}
