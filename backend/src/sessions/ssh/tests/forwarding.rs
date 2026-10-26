use super::*;

#[test]
fn agent_terminal_mode_store_round_trips_per_connection() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    // Unknown connections read the default off.
    assert_eq!(
        runtime.agent_terminal_mode("conn-1"),
        AgentTerminalMode::Off
    );
    runtime
        .set_agent_terminal_mode("conn-1", AgentTerminalMode::parse("auto"))
        .expect("mode");
    runtime
        .set_agent_terminal_mode("conn-2", AgentTerminalMode::Strict)
        .expect("mode");
    assert_eq!(
        runtime.agent_terminal_mode("conn-1"),
        AgentTerminalMode::Auto
    );
    assert_eq!(
        runtime.agent_terminal_mode("conn-2"),
        AgentTerminalMode::Strict
    );
    assert_eq!(
        runtime.agent_terminal_mode("conn-3"),
        AgentTerminalMode::Off
    );
}

#[tokio::test]
async fn agent_mode_get_reports_mode_and_live_session_presence() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());

    // Unknown connection: degrades to off with no live session — the
    // bridge caller treats this as "stay on the silent path" instead of
    // erroring.
    let probe = runtime.agent_mode_get("conn-ghost").await.expect("probe");
    assert_eq!(probe["agentTerminalMode"], "off");
    assert_eq!(probe["hasTerminalSession"], false);

    // A mode set through the settings surface is reflected verbatim.
    runtime
        .set_agent_terminal_mode("conn-1", AgentTerminalMode::Strict)
        .expect("mode");
    let probe = runtime.agent_mode_get("conn-1").await.expect("probe");
    assert_eq!(probe["agentTerminalMode"], "strict");
    // No session was opened in this test, so presence stays false even
    // though the mode is on.
    assert_eq!(probe["hasTerminalSession"], false);
}

#[test]
fn agent_challenges_resolve_once_and_report_unknown() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());

    // Unknown / already resolved challenges are refused.
    assert!(runtime
        .resolve_agent_challenge("ghost", "approve", None, false)
        .is_err());

    // Approve delivers the (possibly edited) command once, then the
    // challenge is gone. The registry guard is dropped before each
    // resolve so the std Mutex never re-enters on the same thread.
    let (sender, receiver) = oneshot::channel();
    runtime.agent_challenges.lock().expect("challenges").insert(
        "c-1".to_string(),
        PendingChallenge {
            sender,
            connection_id: "conn-1".to_string(),
            tool: "ssh_exec".to_string(),
            command: "echo edited".to_string(),
            raised_at_ms: 0,
        },
    );
    runtime
        .resolve_agent_challenge("c-1", "approve", Some("echo edited"), false)
        .expect("resolve");
    let decision = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(receiver)
        .expect("decision");
    assert_eq!(
        decision,
        AgentDecision::Approve {
            command: Some("echo edited".to_string())
        }
    );
    assert!(runtime
        .resolve_agent_challenge("c-1", "approve", None, false)
        .is_err());

    // Deny decisions and unknown decision names are handled too.
    let (sender, receiver) = oneshot::channel();
    runtime.agent_challenges.lock().expect("challenges").insert(
        "c-2".to_string(),
        PendingChallenge {
            sender,
            connection_id: "conn-1".to_string(),
            tool: "ssh_exec".to_string(),
            command: "echo hi".to_string(),
            raised_at_ms: 0,
        },
    );
    runtime
        .resolve_agent_challenge("c-2", "deny", None, false)
        .expect("resolve");
    let decision = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(receiver)
        .expect("decision");
    assert_eq!(decision, AgentDecision::Deny);
    assert!(runtime
        .resolve_agent_challenge("c-2", "maybe", None, false)
        .is_err());
}

/// Reliability round 5 (churn): 500 raise→resolve cycles must leave the
/// challenge registry empty (no leak), every consumed id must stay
/// unknown afterwards (one-shot semantics survive churn), and each
/// decision must reach its waiter exactly once.
#[test]
fn agent_challenges_survive_high_volume_resolve_churn() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    const CYCLES: usize = 500;

    for index in 0..CYCLES {
        let id = format!("churn-{index}");
        let (sender, mut receiver) = oneshot::channel();
        runtime.agent_challenges.lock().expect("challenges").insert(
            id.clone(),
            PendingChallenge {
                sender,
                connection_id: "conn-1".to_string(),
                tool: "ssh_exec".to_string(),
                command: format!("echo {index}"),
                raised_at_ms: 0,
            },
        );
        // The registry never grows beyond the live challenge: raising
        // one inserts one row, resolving consumes it.
        {
            let challenges = runtime.agent_challenges.lock().expect("challenges");
            assert_eq!(
                challenges.len(),
                1,
                "challenge registry leaked at cycle {index}"
            );
        }
        let decision = if index % 3 == 0 {
            runtime
                .resolve_agent_challenge(&id, "deny", None, false)
                .expect("deny");
            AgentDecision::Deny
        } else {
            let edited = format!("echo edited-{index}");
            runtime
                .resolve_agent_challenge(&id, "approve", Some(&edited), false)
                .expect("approve");
            AgentDecision::Approve {
                command: Some(edited),
            }
        };
        // One-shot: the consumed id is unknown, and a wrong decision
        // name is still refused.
        assert!(
            runtime
                .resolve_agent_challenge(&id, "approve", None, false)
                .is_err(),
            "consumed challenge came back at cycle {index}"
        );
        assert_eq!(
            receiver.try_recv().ok(),
            Some(decision),
            "decision must reach its waiter exactly once (cycle {index})"
        );
    }
    assert!(
        runtime
            .agent_challenges
            .lock()
            .expect("challenges")
            .is_empty(),
        "challenge registry must be empty after churn"
    );

    // The MCP confirm payload bakes its own timeout at issue time: two
    // challenges raised with different budgets each carry theirs (the
    // "不追溯" contract — later changes never rewrite live payloads).
    let short = SshRuntime::mcp_confirm_challenge_payload("s", "ssh_exec", "x", None, 10);
    let long = SshRuntime::mcp_confirm_challenge_payload("l", "ssh_exec", "x", None, 600);
    assert_eq!(short["timeoutSecs"], 10);
    assert_eq!(long["timeoutSecs"], 600);
}

#[test]
fn mcp_confirm_challenge_payload_shape() {
    let payload = SshRuntime::mcp_confirm_challenge_payload(
        "ch-1",
        "ssh_exec",
        "echo hi",
        Some("conn-9"),
        120,
    );
    assert_eq!(payload["challengeId"], "ch-1");
    assert_eq!(payload["kind"], "mcp-confirm");
    assert_eq!(payload["source"], "mcp");
    // `risk` is part of the decoder contract: the workbench drops every
    // `ssh/agent/prompt` event whose risk is missing/unknown, which used to
    // silently deny MCP confirms. MCP confirms are the elevated tier.
    assert_eq!(payload["risk"], "elevated");
    assert_eq!(payload["tool"], "ssh_exec");
    assert_eq!(payload["command"], "echo hi");
    assert_eq!(payload["connectionId"], "conn-9");
    assert_eq!(payload["timeoutSecs"], 120);
    assert!(payload["requestedAt"].is_u64());
    // Pin the exact field set so a future field cannot silently disappear
    // (the event is a wire contract with `decodeAgentPromptPayload`).
    let keys: std::collections::BTreeSet<&str> = payload
        .as_object()
        .expect("payload must be an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from([
            "challengeId",
            "command",
            "connectionId",
            "kind",
            "requestedAt",
            "risk",
            "source",
            "timeoutSecs",
            "tool",
        ])
    );

    // Without a connection reference the field is absent (optional —
    // not null) so legacy frontends keep their fallback rendering.
    let bare =
        SshRuntime::mcp_confirm_challenge_payload("ch-2", "sftp_remove", "path=/tmp/x", None, 120);
    assert!(bare.get("connectionId").is_none());
    assert_eq!(bare["kind"], "mcp-confirm");
    assert_eq!(bare["source"], "mcp");
    assert_eq!(bare["risk"], "elevated");
    // An empty reference is treated as absent too.
    let blank = SshRuntime::mcp_confirm_challenge_payload("ch-3", "ssh_exec", "ls", Some(""), 120);
    assert!(blank.get("connectionId").is_none());
}

#[test]
fn agent_remember_persists_approved_and_skips_destructive() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());

    // approve + remember persists the approved command on the
    // connection's remembered list (IMPL_PLAN §2.1).
    let (sender, _receiver) = oneshot::channel();
    runtime.agent_challenges.lock().expect("challenges").insert(
        "r-1".to_string(),
        PendingChallenge {
            sender,
            connection_id: "conn-1".to_string(),
            tool: "ssh_exec".to_string(),
            command: "systemctl restart nginx".to_string(),
            raised_at_ms: 0,
        },
    );
    runtime
        .resolve_agent_challenge("r-1", "approve", Some("systemctl restart nginx"), true)
        .expect("resolve");
    assert_eq!(
        agent_approvals::list_lines(&agent_approvals::load_store(data_dir.path()), "conn-1"),
        ["systemctl restart nginx"]
    );

    // A destructive text is approved (the decision still delivers) but
    // never remembered — the D2 double lock, second half.
    let (sender, receiver) = oneshot::channel();
    runtime.agent_challenges.lock().expect("challenges").insert(
        "r-2".to_string(),
        PendingChallenge {
            sender,
            connection_id: "conn-1".to_string(),
            tool: "ssh_exec".to_string(),
            command: "rm -rf /".to_string(),
            raised_at_ms: 0,
        },
    );
    runtime
        .resolve_agent_challenge("r-2", "approve", Some("rm -rf /"), true)
        .expect("resolve");
    let decision = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(receiver)
        .expect("decision");
    assert_eq!(
        decision,
        AgentDecision::Approve {
            command: Some("rm -rf /".to_string())
        }
    );
    assert_eq!(
        agent_approvals::list_lines(&agent_approvals::load_store(data_dir.path()), "conn-1"),
        ["systemctl restart nginx"],
        "destructive command must not join the remembered list"
    );
}

#[test]
fn agent_exec_lock_admits_one_holder_then_queues() {
    // Structural test for the per-session agent-execution lock: one
    // holder excludes a second contender and release lets it through.
    // The await-based acquisition itself (`agent_exec_guard`, held across
    // approval + run) is exercised by the container smoke's concurrent
    // pair, which needs a live PTY. `blocking_lock` is fine here — the
    // test runs outside any tokio runtime.
    let lock = Arc::new(AsyncMutex::new(()));
    let held = lock.clone();
    let guard = held.blocking_lock();
    assert!(
        lock.try_lock().is_err(),
        "a second agent command must queue behind the running one"
    );
    drop(guard);
    assert!(
        lock.try_lock().is_ok(),
        "release must admit the next agent command"
    );
}

#[test]
fn attach_target_requires_exact_workbench() {
    let sessions: Vec<(String, String, String, bool)> = vec![
        ("s-old".into(), "conn-1".into(), "wb-old".into(), true),
        ("s-new".into(), "conn-1".into(), "wb-new".into(), true),
        ("s-other".into(), "conn-2".into(), "wb-other".into(), true),
    ];
    // Exact double match wins when the workbench id is stable.
    assert_eq!(
        SshRuntime::pick_attach_target(&sessions, "conn-1", "wb-old").as_deref(),
        Some("s-old")
    );
    // A second workbench on the same saved connection gets its own exact
    // session rather than stealing the first tab's PTY.
    assert_eq!(
        SshRuntime::pick_attach_target(&sessions, "conn-1", "wb-new").as_deref(),
        Some("s-new")
    );
    assert_eq!(
        SshRuntime::pick_attach_target(&sessions, "conn-1", "wb-missing"),
        None
    );
    // Dead sessions never attach; other connections' sessions stay put.
    let dead: Vec<(String, String, String, bool)> =
        vec![("s-dead".into(), "conn-1".into(), "wb-old".into(), false)];
    assert_eq!(
        SshRuntime::pick_attach_target(&dead, "conn-1", "wb-new"),
        None
    );
    assert_eq!(
        SshRuntime::pick_attach_target(&sessions, "conn-3", "wb-new"),
        None
    );
}

// ===== issue #202：web 刷新后「从零 connect」路径的契约锁定 =====
//
// 背景：刷新后宿主（web 模式）不一定立即重推 connection/connect，恢复的
// 工作台会先打到 sidecar 的空连接注册表。前端按**确定的错误字符串**分类
// （isConnectionInactiveError / isSessionGoneError），这些字符串就是协议
// 的一部分——hang、静默或措辞漂移都会把工作台卡死在重试梯子上，因此逐
// 字锁定。重推 connect（store_connection）必须幂等并清扫旧注册的残留。

#[tokio::test]
async fn session_ops_fail_structured_when_connection_registry_is_empty() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = Arc::new(SshRuntime::new(data_dir.path().to_path_buf()));
    assert!(!runtime.has_connection("conn-1"));

    let request: SessionOpenRequest =
        serde_json::from_value(json!({ "connectionId": "conn-1", "workbenchId": "wb-1" }))
            .expect("parse open request");
    let error = runtime
        .open_session(&request, "op-1", structured_error_emitter())
        .await
        .expect_err("open without a registered connection must fail");
    assert_eq!(error, "Connection is not active; reopen it from DBX");

    let error = runtime
        .attach_session("conn-1", "wb-1", 0, &structured_error_emitter())
        .await
        .expect_err("attach without a live session must fail");
    assert_eq!(error, "No live SSH session is attached to this workbench");

    let error = runtime
        .close_session("s-missing")
        .await
        .expect_err("close of an unknown session must fail");
    assert_eq!(error, "SSH session was not found");
}

#[tokio::test]
async fn agent_exec_guard_reports_unknown_sessions() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let error = runtime
        .agent_exec_guard("no-such-session")
        .await
        .expect_err("unknown session must be refused");
    assert!(
        error.contains("not found") || error.contains("expired"),
        "unexpected error: {error}"
    );
}

mod host_key_prompt {
    use super::*;
    use dbx_plugin_sdk::{PluginTransport, UserInputAnswer, UserInputPrompt};
    use std::sync::atomic::Ordering;

    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn events(sink: &Arc<Mutex<Vec<u8>>>) -> Vec<serde_json::Value> {
        String::from_utf8(sink.lock().unwrap().clone())
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn emitter_for_test() -> (PluginEmitter, Arc<Mutex<Vec<u8>>>) {
        let sink = Arc::new(Mutex::new(Vec::new()));
        (
            dbx_plugin_sdk::PluginEmitter::for_tests(
                Arc::new(Mutex::new(Box::new(SharedSink(sink.clone())))),
                PluginTransport::JsonLines,
            ),
            sink,
        )
    }

    struct ScriptedGateway {
        supports: bool,
        answer: Mutex<Result<UserInputAnswer, PluginError>>,
        prompts_seen: Mutex<Vec<serde_json::Value>>,
    }

    impl ScriptedGateway {
        fn supports() -> Self {
            Self {
                supports: true,
                answer: Mutex::new(Ok(UserInputAnswer {
                    action: "submit".into(),
                    value: Some("remember".into()),
                })),
                prompts_seen: Mutex::new(Vec::new()),
            }
        }
        fn without_feature() -> Self {
            Self {
                supports: false,
                answer: Mutex::new(Ok(UserInputAnswer {
                    action: "submit".into(),
                    value: None,
                })),
                prompts_seen: Mutex::new(Vec::new()),
            }
        }
        fn answering(answer: Result<UserInputAnswer, PluginError>) -> Self {
            Self {
                supports: true,
                answer: Mutex::new(answer),
                prompts_seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl HostPromptGateway for ScriptedGateway {
        fn supports_request_user_input(&self) -> bool {
            self.supports
        }
        fn request_user_input(
            &self,
            prompt: &UserInputPrompt,
        ) -> Result<UserInputAnswer, PluginError> {
            self.prompts_seen
                .lock()
                .unwrap()
                .push(serde_json::to_value(prompt).unwrap());
            self.answer.lock().unwrap().clone()
        }
    }

    const HOST: &str = "server.example.com";
    const FINGERPRINT: &str = "SHA256:abcdefgh";

    async fn challenge_via(
        broker: &PromptBroker,
        emitter: &PluginEmitter,
    ) -> Option<PromptDecision> {
        broker
            .request(
                HOST,
                22,
                "ssh-ed25519".into(),
                FINGERPRINT.into(),
                "conn-1",
                "op-1",
                emitter,
            )
            .await
    }

    #[tokio::test]
    async fn host_dialog_submit_remember_maps_to_accept_and_remember() {
        let gateway = Arc::new(ScriptedGateway::supports());
        let broker = PromptBroker::with_gateway(gateway.clone());
        let (emitter, sink) = emitter_for_test();

        let decision = challenge_via(&broker, &emitter).await;

        assert_eq!(
            decision,
            Some(PromptDecision {
                accept: true,
                remember: true
            })
        );
        assert!(events(&sink)
            .iter()
            .all(|event| event["method"] != "connection/challenge"));
        let prompt = gateway.prompts_seen.lock().unwrap()[0].clone();
        assert_eq!(prompt["options"][0]["value"], "accept");
        assert_eq!(prompt["options"][1]["value"], "remember");
        assert!(prompt["prompt"].as_str().unwrap().contains(FINGERPRINT));
        assert!(broker.challenge_was_raised());
        // The dialog took the challenge: connection/test may extend its
        // budget for exactly this probe.
        assert!(broker.host_dialog_was_used());
    }

    #[tokio::test]
    async fn host_dialog_accept_maps_to_accept_without_remember() {
        let gateway = Arc::new(ScriptedGateway::answering(Ok(UserInputAnswer {
            action: "submit".into(),
            value: Some("accept".into()),
        })));
        let broker = PromptBroker::with_gateway(gateway);
        let (emitter, _sink) = emitter_for_test();
        assert_eq!(
            challenge_via(&broker, &emitter).await,
            Some(PromptDecision {
                accept: true,
                remember: false
            })
        );
    }

    #[tokio::test]
    async fn host_dialog_cancel_and_timeout_reject_fail_closed() {
        for action in ["cancel", "timeout"] {
            let gateway = Arc::new(ScriptedGateway::answering(Ok(UserInputAnswer {
                action: action.into(),
                value: None,
            })));
            let broker = PromptBroker::with_gateway(gateway);
            let (emitter, sink) = emitter_for_test();
            assert_eq!(
                challenge_via(&broker, &emitter).await,
                Some(PromptDecision {
                    accept: false,
                    remember: false
                })
            );
            assert!(events(&sink)
                .iter()
                .all(|event| event["method"] != "connection/challenge"));
        }
    }

    #[tokio::test]
    async fn host_without_feature_falls_back_to_workbench_event() {
        let gateway = Arc::new(ScriptedGateway::without_feature());
        let broker = PromptBroker::with_gateway(gateway);
        let (emitter, sink) = emitter_for_test();

        let pending = tokio::spawn({
            let broker = broker.clone();
            let emitter = emitter.clone();
            async move {
                broker
                    .request(
                        HOST,
                        22,
                        "ssh-ed25519".into(),
                        FINGERPRINT.into(),
                        "conn-1",
                        "op-1",
                        &emitter,
                    )
                    .await
            }
        });
        // 降级路径必须发出既有事件载荷(workbench 依赖 challengeId/kind 字段)。
        let mut challenge_id = None;
        for _ in 0..200 {
            if let Some(event) = events(&sink)
                .into_iter()
                .find(|event| event["method"] == "connection/challenge")
            {
                challenge_id = Some(event["params"]["challengeId"].as_str().unwrap().to_string());
                assert_eq!(event["params"]["kind"], "host-key");
                assert_eq!(event["params"]["fingerprint"], FINGERPRINT);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let challenge_id = challenge_id.expect("legacy challenge event must be emitted");
        broker
            .resolve(
                &challenge_id,
                "op-1",
                PromptDecision {
                    accept: true,
                    remember: true,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            pending.await.unwrap(),
            Some(PromptDecision {
                accept: true,
                remember: true
            })
        );
    }

    #[tokio::test]
    async fn no_ui_error_falls_back_to_workbench_event() {
        let gateway = Arc::new(ScriptedGateway::answering(Err(PluginError::new(
            -32001,
            "no user interface is attached",
        ))));
        let broker = PromptBroker::with_gateway(gateway);
        let (emitter, sink) = emitter_for_test();
        let pending = tokio::spawn({
            let broker = broker.clone();
            let emitter = emitter.clone();
            async move {
                broker
                    .request(
                        HOST,
                        22,
                        "ssh-ed25519".into(),
                        FINGERPRINT.into(),
                        "conn-1",
                        "op-1",
                        &emitter,
                    )
                    .await
            }
        });
        let mut challenge_id = None;
        for _ in 0..200 {
            if let Some(event) = events(&sink)
                .into_iter()
                .find(|event| event["method"] == "connection/challenge")
            {
                challenge_id = Some(event["params"]["challengeId"].as_str().unwrap().to_string());
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        broker
            .resolve(
                &challenge_id.expect("fallback event"),
                "op-1",
                PromptDecision {
                    accept: false,
                    remember: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            pending.await.unwrap(),
            Some(PromptDecision {
                accept: false,
                remember: false
            })
        );
        // A degradable failure fell back to the workbench event: the
        // dialog must not keep the extended test budget armed.
        assert!(!broker.host_dialog_was_used());
    }

    #[tokio::test]
    async fn host_dialog_marks_used_and_degraded_fallback_does_not() {
        // 粘性标志的语义:弹窗真正接管挑战时置位(connection/test 据此重臂
        // 预算);可降级错误回落 legacy 前必须清零,1.0 宿主保持短预算。
        let gateway = Arc::new(ScriptedGateway::answering(Err(PluginError::new(
            -32601,
            "method not found",
        ))));
        let broker = PromptBroker::with_gateway(gateway);
        let (emitter, sink) = emitter_for_test();
        let pending = tokio::spawn({
            let broker = broker.clone();
            let emitter = emitter.clone();
            async move {
                broker
                    .request(
                        HOST,
                        22,
                        "ssh-ed25519".into(),
                        FINGERPRINT.into(),
                        "conn-1",
                        "op-1",
                        &emitter,
                    )
                    .await
            }
        });
        let mut challenge_id = None;
        for _ in 0..200 {
            if let Some(event) = events(&sink)
                .into_iter()
                .find(|event| event["method"] == "connection/challenge")
            {
                challenge_id = Some(event["params"]["challengeId"].as_str().unwrap().to_string());
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        // Once the legacy event is out, the dialog flag must already be
        // cleared again.
        assert!(!broker.host_dialog_was_used());
        broker
            .resolve(
                &challenge_id.expect("fallback event"),
                "op-1",
                PromptDecision {
                    accept: true,
                    remember: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            pending.await.unwrap(),
            Some(PromptDecision {
                accept: true,
                remember: false
            })
        );
        assert!(!broker.host_dialog_was_used());
    }

    #[tokio::test]
    async fn host_silence_fails_closed_without_fallback() {
        // SDK 本地超时的错误以 -32001 + "did not answer" 表达;此时再降级会
        // 把总等待拖到 630s,必须直接拒绝。
        let gateway = Arc::new(ScriptedGateway::answering(Err(PluginError::new(
            -32001,
            "Host did not answer 'host/requestUserInput' in time",
        ))));
        let broker = PromptBroker::with_gateway(gateway);
        let (emitter, sink) = emitter_for_test();
        assert_eq!(challenge_via(&broker, &emitter).await, None);
        assert!(events(&sink)
            .iter()
            .all(|event| event["method"] != "connection/challenge"));
    }

    #[tokio::test]
    async fn host_dialog_submit_without_known_value_rejects_fail_closed() {
        // Only an explicit accept/remember value grants trust: a missing
        // or unknown value (host sent action=submit with nothing usable)
        // must be a rejection, never a guess.
        for value in [None, Some("maybe".to_string())] {
            let gateway = Arc::new(ScriptedGateway::answering(Ok(UserInputAnswer {
                action: "submit".into(),
                value,
            })));
            let broker = PromptBroker::with_gateway(gateway);
            let (emitter, sink) = emitter_for_test();
            assert_eq!(
                challenge_via(&broker, &emitter).await,
                Some(PromptDecision {
                    accept: false,
                    remember: false
                })
            );
            assert!(events(&sink)
                .iter()
                .all(|event| event["method"] != "connection/challenge"));
        }
    }

    #[tokio::test]
    async fn challenge_raised_flag_clears_between_probes() {
        let gateway = Arc::new(ScriptedGateway::supports());
        let broker = PromptBroker::with_gateway(gateway);
        let (emitter, _sink) = emitter_for_test();
        broker.challenge_raised.store(true, Ordering::Relaxed);
        broker.clear_challenge_raised();
        let _ = challenge_via(&broker, &emitter).await;
        assert!(broker.challenge_was_raised());
        assert!(broker.host_dialog_was_used());
        broker.clear_challenge_raised();
        // One clear entry resets both sticky challenge marks so the next
        // probe starts from a clean slate.
        assert!(!broker.challenge_was_raised());
        assert!(!broker.host_dialog_was_used());
    }

    #[test]
    fn challenge_flag_clears_between_probes() {
        let gateway = Arc::new(ScriptedGateway::without_feature());
        let broker = PromptBroker::with_gateway(gateway);
        broker
            .challenge_raised
            .store(true, std::sync::atomic::Ordering::Relaxed);
        broker.clear_challenge_raised();
        assert!(!broker.challenge_was_raised());
    }

    #[test]
    fn local_target_registry_follows_the_connection_lifecycle() {
        let dir = std::env::temp_dir().join(format!("dbx-ssh-local-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let runtime = SshRuntime::new(dir);
        // Unknown ids (SSH connections included) read as "not local" so
        // the task provider keeps its SSH path untouched.
        let runtime_ref = &runtime;
        let runtime_handle = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(runtime_handle
            .block_on(runtime_ref.local_target("conn-local-1"))
            .is_none());

        runtime
            .store_local_target(crate::sessions::local::target::LocalTarget {
                id: "conn-local-1".to_string(),
                name: Some("ops".to_string()),
                shell: "/bin/bash".to_string(),
                cwd: String::new(),
                env: Vec::new(),
            })
            .unwrap();
        let target = runtime_handle
            .block_on(runtime_ref.local_target("conn-local-1"))
            .expect("stored target");
        assert_eq!(target.shell, "/bin/bash");
        // Re-connect replaces the registration in place.
        runtime
            .store_local_target(crate::sessions::local::target::LocalTarget {
                id: "conn-local-1".to_string(),
                name: None,
                shell: String::new(),
                cwd: String::new(),
                env: Vec::new(),
            })
            .unwrap();
        let replaced = runtime_handle
            .block_on(runtime_ref.local_target("conn-local-1"))
            .unwrap();
        assert_eq!(replaced.shell, String::new());
        // Disconnect clears it; an SSH disconnect id is a harmless no-op.
        runtime.drop_local_target("conn-local-1");
        runtime.drop_local_target("some-ssh-connection");
        assert!(runtime_handle
            .block_on(runtime_ref.local_target("conn-local-1"))
            .is_none());
    }
}
