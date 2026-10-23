//! SSH connection support.

use super::*;

impl SshRuntime {
    pub async fn open_session(
        self: &Arc<Self>,
        request: &SessionOpenRequest,
        operation_id: &str,
        emitter: PluginEmitter,
    ) -> Result<Value, String> {
        // WT-4 (WezTerm `spawn` parity): resolve the optional command-session
        // request BEFORE any dial or transport lease, so a malformed command
        // fails closed without touching the shared transport at all. The
        // normalized command is wrapped as `sh -c '<escaped>'` here so the
        // exec branch stays a single `Some`/`None` decision.
        let spawn_exec = normalized_spawn_command(request.spawn_command.as_deref())?
            .map(|command| spawn_exec_payload(&command));
        let connection_id = request.connection_id.as_str();
        let workbench_id = request.workbench_id.as_str();
        let reuse_authenticated_transport = request.reuse_authenticated_transport;
        let cols = request.cols;
        let rows = request.rows;
        let connection = self
            .connections
            .read()
            .map_err(|_| "Connection registry is poisoned".to_string())?
            .get(connection_id)
            .cloned()
            .ok_or("Connection is not active; reopen it from DBX")?;
        // A copied workbench gets its own PTY channel, replay buffer and
        // terminal task, but deliberately shares the already authenticated
        // SSH transport. This avoids replaying an MFA challenge without ever
        // caching or reusing the OTP itself. A normal "new session" keeps the
        // old behavior and establishes a fully independent transport.
        let reuse_source = if reuse_authenticated_transport {
            let sessions = self.sessions.read().await;
            let source = if let Some(source_session_id) =
                request.reuse_authenticated_session_id.as_deref()
            {
                sessions
                    .get(source_session_id)
                    .filter(|entry| {
                        entry.connection_id == connection_id
                            && entry.connected.load(Ordering::Acquire)
                    })
                    .cloned()
            } else {
                // Compatibility for direct callers that know only the older
                // boolean contract: choose a deterministic live source.
                sessions
                    .values()
                    .filter(|entry| {
                        entry.connection_id == connection_id
                            && entry.connected.load(Ordering::Acquire)
                    })
                    .min_by_key(|entry| entry.created_seq)
                    .cloned()
            };
            source
                .map(|source| {
                    let orchestration = source
                        .orchestration
                        .read()
                        .map_err(|_| "SSH authentication state is unavailable".to_string())?
                        .clone();
                    // Reserve while the sessions read lock still prevents
                    // close_session from removing/releasing the source.
                    source.transport_lease.retain();
                    Ok::<_, String>((
                        Arc::clone(&source.handle),
                        Arc::clone(&source.transport_lease),
                        orchestration,
                    ))
                })
                .transpose()?
        } else {
            None
        };
        if reuse_authenticated_transport && reuse_source.is_none() {
            return Err(
                "No live authenticated SSH connection is available to duplicate; use New session to reconnect"
                    .to_string(),
            );
        }
        let (connection, handle, transport_lease, inherited_orchestration, reused_transport) =
            if let Some((handle, transport_lease, orchestration)) = reuse_source {
                eprintln!(
                    "[ssh-trace] open_session connection_id={connection_id} workbench_id={workbench_id} reuse_authenticated_transport=true"
                );
                (
                    connection,
                    handle,
                    transport_lease,
                    Some(orchestration),
                    true,
                )
            } else {
                eprintln!(
                    "[ssh-trace] open_session connection_id={connection_id} workbench_id={workbench_id} -> {}:{} auth={:?} reuse_authenticated_transport=false",
                    connection.host, connection.port, connection.authentication
                );
                // D9: 拨号前解析 password_command，使取回的凭据同时作用于登录链与
                // 本会话的 sudo 编排（下游 dial_and_authenticate 看到非空密码即不再
                // 重复执行——单次执行语义）。
                let connection = resolve_password_command(&connection).await;
                let (handle, jump_chain) = self
                    .connect_authenticated(&connection, operation_id, Some(emitter.clone()))
                    .await?;
                eprintln!("[ssh-trace] open_session dial+auth ok");
                (
                    connection,
                    Arc::new(handle),
                    Arc::new(SharedTransportLease::new(
                        jump_chain.into_iter().map(Arc::new).collect::<Vec<_>>(),
                    )),
                    None,
                    false,
                )
            };
        let terminal = async {
            let remote_shell = RemoteShell::Unknown;
            let directory_tracking_supported = true;
            let mut channel = handle.channel_open_session().await.map_err(|error| {
                if reused_transport {
                    format!("The authenticated SSH connection can no longer be reused; use New session to reconnect: {error}")
                } else {
                    format!("Failed to open terminal channel: {error}")
                }
            })?;
            channel
                .request_pty(true, "xterm-256color", cols.max(1), rows.max(1), 0, 0, &[])
                .await
                .map_err(|error| format!("Failed to request SSH PTY: {error}"))?;
            // Client-specified SetEnv rides on the interactive session too,
            // in ssh(1) order: PTY first, env next, shell/exec last.
            exec::apply_connection_env(&mut channel, &connection.set_env).await?;
            // X11 forwarding (OpenSSH -X parity): gated on the connection-scoped
            // preference; a refused request only disables X11, never the shell.
            if crate::features::forwarding::x11::enabled_from(&self.data_dir) {
                match crate::features::forwarding::x11::arm_session() {
                    Ok(cookie_hex) => {
                        if let Err(error) = channel
                            .request_x11(true, false, crate::features::forwarding::x11::X11_AUTH_PROTOCOL, cookie_hex, 0)
                            .await
                        {
                            eprintln!("[x11] x11-req refused by server: {error}");
                        }
                    }
                    Err(error) => eprintln!("[x11] cannot arm the forwarding gate: {error}"),
                }
            }
            if let Some(command) = spawn_exec.as_deref() {
                // WT-4 (command session): exec `sh -c '<escaped command>'`
                // instead of a shell — see docs/architecture/PROTOCOL.zh-CN.md「同
                // transport 命令会话」. Explicit per-open request wins over
                // the connection-level remote_command.
                channel
                    .exec(true, command.as_bytes())
                    .await
                    .map_err(|error| format!("Failed to start remote command: {error}"))?;
            } else if connection.remote_command.is_empty() {
                channel
                    .request_shell(true)
                    .await
                    .map_err(|error| format!("Failed to start SSH shell: {error}"))?;
            } else {
                channel
                    .exec(true, connection.remote_command.as_bytes())
                    .await
                    .map_err(|error| format!("Failed to start remote command: {error}"))?;
            }
            Ok::<_, String>((remote_shell, directory_tracking_supported, channel))
        }
        .await;
        let (mut remote_shell, directory_tracking_supported, mut channel) = match terminal {
            Ok(terminal) => terminal,
            Err(error) => {
                transport_lease.release().await;
                return Err(error);
            }
        };

        let mut session_id = request
            .requested_session_id
            .as_deref()
            .filter(|id| valid_requested_session_id(id))
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let (terminal_tx, mut terminal_rx) = mpsc::channel(256);
        let replay = Arc::new(AsyncMutex::new(ReplayBuffer::default()));
        let bound_profile = {
            let store = sudo_profiles::load_store(&self.data_dir);
            effective_sudo_profile(&connection, &store)
        };
        let orchestration =
            Arc::new(RwLock::new(inherited_orchestration.unwrap_or_else(|| {
                resolved_sudo_auth(&connection, bound_profile.as_ref())
            })));
        // In-terminal Quick Sudo: answers sudo password / 2FA prompts while
        // the user keeps typing normal commands (ported from tiny-rdm). The
        // watcher is attached here and re-synced on every runtime settings
        // update, so configuring Quick Sudo after connecting still arms it.
        let entry = Arc::new(SessionEntry {
            connection_id: connection.id.clone(),
            workbench_id: RwLock::new(workbench_id.to_string()),
            read_only: connection.read_only,
            connected: AtomicBool::new(true),
            created_at_secs: unix_now_secs(),
            created_seq: self.session_seq.fetch_add(1, Ordering::Relaxed),
            handle,
            transport_lease,
            orchestration: orchestration.clone(),
            auto_sudo: Arc::new(Mutex::new(None)),
            triggers: Arc::new(Mutex::new(None)),
            terminal_tx,
            replay: replay.clone(),
            sftp: AsyncMutex::new(None),
            agent_recorder: Arc::new(Mutex::new(None)),
            session_recorder: Arc::new(Mutex::new(None)),
            agent_exec_lock: Arc::new(AsyncMutex::new(())),
        });
        Self::sync_auto_sudo(&entry, &connection);
        Self::sync_triggers(&entry, &connection);
        let mut sessions = self.sessions.write().await;
        if sessions.contains_key(&session_id) {
            session_id = Uuid::new_v4().to_string();
        }
        sessions.insert(session_id.clone(), entry.clone());

        // Terminal activity keepalive (opt-in per connection): resets
        // server-side idle policies that watch PTY input, which protocol
        // keepalives don't satisfy. Holds only the command sender, so the
        // loop ends as soon as the session's read loop drops its receiver.
        if connection.terminal_keepalive_secs > 0 {
            let terminal_tx = entry.terminal_tx.clone();
            let interval_secs = connection.terminal_keepalive_secs;
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                // The interval's first tick fires immediately; the session
                // just opened, so nothing needs resetting yet.
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    if terminal_tx
                        .send(TerminalCommand::Input(TERMINAL_KEEPALIVE_INPUT.to_vec()))
                        .await
                        .is_err()
                    {
                        // Read loop gone — the session is closed.
                        break;
                    }
                }
            });
        }

        // Prompt-hook arming (issue t8y2/dbx#10750): every interactive shell
        // session arms the per-prompt OSC 7 hook for its whole lifetime so the
        // frontend can keep the next prompt off the tail of un-newlined output
        // (FinalShell/WindTerm behavior); the SFTP directory-follow toggle
        // consumes the same frames. Detection runs off the read loop — probing
        // inside a select! arm can stall the loop for the detection timeout —
        // and the resolved shell rides the command queue into the loop, FIFO
        // with the startup-commands input lines below. Only real shell
        // sessions: spawn exec / remote_command replace the shell, so typing
        // the hook line there would feed the command's stdin.
        if interactive_shell_session(spawn_exec.as_deref(), &connection.remote_command) {
            let hook_handle = entry.handle.clone();
            let hook_tx = entry.terminal_tx.clone();
            tokio::spawn(async move {
                let shell = detect_remote_shell(&hook_handle).await;
                let _ = hook_tx
                    .send(TerminalCommand::InstallPromptHook { shell })
                    .await;
            });
        }

        // Startup commands (Tabby "Login scripts" parity, M7 P0-4): after the
        // shell is up, type the connection's pre-configured command sequence
        // through the same input channel the keepalive uses. Only for real
        // shell sessions — a RemoteCommand or WT-4 spawn exec replaces the
        // shell, so typing into it is a semantic conflict (documented in
        // PROTOCOL.zh-CN.md). The event reports only the count and completion:
        // command contents can carry secrets and never reach logs or events.
        if spawn_exec.is_none() && startup_commands::executes_for(&connection.remote_command) {
            let plan = startup_commands::load_plan(&self.data_dir, &connection.id);
            if !plan.is_empty() {
                let terminal_tx = entry.terminal_tx.clone();
                let startup_emitter = emitter.clone();
                let startup_session_id = session_id.clone();
                let startup_count = plan.len();
                tokio::spawn(async move {
                    let completed = startup_commands::inject_sequence(&plan, |payload| {
                        let terminal_tx = terminal_tx.clone();
                        async move {
                            terminal_tx
                                .send(TerminalCommand::Input(payload))
                                .await
                                .is_ok()
                        }
                    })
                    .await;
                    let _ = startup_emitter.event(
                        "ssh/startup",
                        json!({
                            "sessionId": startup_session_id,
                            "count": startup_count,
                            "completed": completed,
                        }),
                    );
                });
            }
        }

        let task_id = session_id.clone();
        let directory_marker_id = session_id.clone();
        let sessions = self.sessions.clone();
        // 自发断开收尾要清传输登记（'static spawn 拿不到 &self）：克隆运行时
        // 句柄进读循环任务。
        let runtime_for_cleanup = Arc::clone(self);
        // Auto-record 提示事件在 spawn 之后仍要用 emitter（主闭包已 move 走
        // 原值），提前留一个克隆；录制器槽同理（entry 已 move 进读循环任务）。
        let auto_record_emitter = emitter.clone();
        let auto_record_slot = Arc::clone(&entry.session_recorder);
        let auto_record_connection_id = entry.connection_id.clone();
        tokio::spawn(async move {
            let mut directory_filter = DirectoryHandshakeFilter::default();
            let mut directory_tracking_enabled = false;
            // #90：ZMODEM 触发检测（远端 sz 发起的 ZRQINIT）。字节全透传，
            // 检测器只产出事件边沿；事件窗口超时后自动复位（详见
            // zmodem_detect 模块注释）。
            let mut zmodem_detector = crate::sessions::serial::detect::Detector::new();
            let mut directory_timeout = tokio::time::interval(Duration::from_millis(250));
            directory_timeout.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // 跨帧 UTF-8 增量解码的尾部残留：多字节字符恰跨 SSH 包边界时，
            // 逐帧独立 lossy 解码会把字符劈成两个 U+FFFD，中文提示词的触发器/
            // auto-sudo 匹配稳定 miss（帧边界取决于远端写盘分片，非确定）。
            // 只有 stdout 进文本观察链（stderr 直喂 recorder 原始字节）。
            let mut stdout_utf8_carry: Vec<u8> = Vec::new();
            loop {
                tokio::select! {
                    _ = directory_timeout.tick() => {
                        if let Some(data) = directory_filter.flush_if_timed_out() {
                            // 与主数据路径同一检测器：握手窗口里冲刷的字节同样
                            // 参与 ZMODEM 触发检测（#90）。字节照常发布，检测器
                            // 只产出事件边沿。
                            if zmodem_detector.feed(&data, unix_now_ms()) {
                                let _ = emitter.event(
                                    "ssh/zmodem",
                                    json!({ "sessionId": task_id, "kind": "zrqinit" }),
                                );
                            }
                            if !data.is_empty() {
                                publish_terminal(&task_id, TerminalStream::Stdout, data, &replay, &emitter).await;
                            }
                        }
                        if directory_filter.take_failed() {
                            directory_tracking_enabled = false;
                            publish_terminal(
                                &task_id,
                                TerminalStream::State,
                                b"directory-tracking-unavailable".to_vec(),
                                &replay,
                                &emitter,
                            ).await;
                        }
                        // 推迟的 OTP 应答：TOTP 窗口滚动后自动补答（并发同靶
                        // sudo 提示撞重放保护时的排队语义）。先取值再写通道，
                        // 让互斥锁守卫在 .await 前释放。
                        let deferred_otp = entry
                            .auto_sudo
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner())
                            .as_mut()
                            .and_then(|auto| auto.take_deferred_otp(unix_now_secs()));
                        if let Some((kind, answer)) = deferred_otp {
                            let mut payload = answer.into_bytes();
                            payload.push(b'\r');
                            if channel.data(&payload[..]).await.is_err() { break; }
                            let _ = emitter.event("ssh/auto-sudo", json!({
                                "sessionId": task_id,
                                "kind": if kind == exec::AutoSudoKind::Totp { "otp" } else { "password" },
                            }));
                        }
                    }
                    command = terminal_rx.recv() => match command {
                        Some(TerminalCommand::Input(data)) => {
                            if channel.data(&data[..]).await.is_err() { break; }
                        }
                        Some(TerminalCommand::Resize { cols, rows }) => {
                            let _ = channel.window_change(cols.max(1), rows.max(1), 0, 0).await;
                        }
                        Some(TerminalCommand::DirectoryTracking { enabled }) => {
                            if !enabled {
                                // Session-open arming made the hook session
                                // infrastructure: turning SFTP directory-follow
                                // off stops only the frontend navigation, while
                                // the per-prompt hook (and the prompt-newline
                                // fix built on it) stays armed for the session's
                                // lifetime. Tearing it down here would silently
                                // regress #10750 for follow-off users.
                                continue;
                            }
                            if remote_shell == RemoteShell::Unknown {
                                remote_shell = detect_remote_shell(&entry.handle).await;
                            }
                            if remote_shell == RemoteShell::Other {
                                publish_terminal(
                                    &task_id,
                                    TerminalStream::State,
                                    b"directory-tracking-unavailable".to_vec(),
                                    &replay,
                                    &emitter,
                                ).await;
                                continue;
                            }
                            if directory_tracking_enabled {
                                continue;
                            }
                            directory_tracking_enabled = true;
                            directory_filter.begin(directory_tracking_marker(&directory_marker_id));
                            publish_terminal(
                                &task_id,
                                TerminalStream::Stdout,
                                b"\r\x1b[2K".to_vec(),
                                &replay,
                                &emitter,
                            ).await;
                            let script = directory_tracking_script(&directory_marker_id, remote_shell);
                            if channel.data(script.as_bytes()).await.is_err() { break; }
                        }
                        Some(TerminalCommand::InstallPromptHook { shell }) => {
                            // Silent session-open arming: unsupported shells are
                            // skipped without the toggle's unavailable event,
                            // and an already-armed hook (user-facing toggle
                            // raced ahead) is left untouched. No `\r\x1b[2K`
                            // line clear either — the typed line lands before
                            // the first prompt and the handshake filter hides
                            // its echo, so nothing on screen is disturbed.
                            if !matches!(shell, RemoteShell::Bash | RemoteShell::Zsh) {
                                continue;
                            }
                            if directory_tracking_enabled {
                                continue;
                            }
                            remote_shell = shell;
                            directory_tracking_enabled = true;
                            directory_filter.begin(directory_tracking_marker(&directory_marker_id));
                            let script = directory_tracking_script(&directory_marker_id, shell);
                            if channel.data(script.as_bytes()).await.is_err() { break; }
                        }
                        Some(TerminalCommand::Close) | None => {
                            let _ = channel.close().await;
                            break;
                        }
                    },
                    message = channel.wait() => {
                        let (data, stream) = match message {
                            Some(ChannelMsg::Data { data }) => (data.to_vec(), TerminalStream::Stdout),
                            Some(ChannelMsg::ExtendedData { data, .. }) => (data.to_vec(), TerminalStream::Stderr),
                            Some(ChannelMsg::Eof | ChannelMsg::Close) | None => break,
                            _ => continue,
                        };
                        if stream == TerminalStream::Stdout {
                            let carry = &mut stdout_utf8_carry;
                            let chunk_text = decode_stream_text(carry, &data);
                            // Expect 式触发器先于终端 auto-sudo 观察（契约 D1
                            // 互斥防双答）：本 chunk 被触发器应答则跳过
                            // auto-sudo，反之亦然。锁在 .await 前释放。
                            let trigger_hit = {
                                let mut slot = entry
                                    .triggers
                                    .lock()
                                    .unwrap_or_else(|poison| poison.into_inner());
                                slot.as_mut().and_then(|engine| {
                                    engine.observe(&chunk_text, unix_now_ms()).map(|decision| {
                                        (
                                            decision,
                                            engine.placeholders().clone(),
                                            engine.pacing(),
                                        )
                                    })
                                })
                            };
                            let auto_answer = if trigger_hit.is_none() {
                                entry
                                    .auto_sudo
                                    .lock()
                                    .unwrap_or_else(|poison| poison.into_inner())
                                    .as_mut()
                                    .and_then(|auto| auto.observe(&chunk_text))
                            } else {
                                None
                            };
                            if let Some((decision, placeholders, (sleep_ms, pass_sleep))) =
                                trigger_hit
                            {
                                // command 应答在本地先执行（10s 超时、退出码
                                // 校验；日志只记失败原因，绝不落命令输出）。
                                let segments = match decision.kind {
                                    triggers::TriggerKind::Command => {
                                        match decision.command.as_deref().map(|command| {
                                            triggers::run_credential_command(
                                                command,
                                                &placeholders,
                                            )
                                        }) {
                                            Some(runner) => match runner.await {
                                                Ok(answer) => triggers::pass_sleep_segments(
                                                    &answer,
                                                    sleep_ms,
                                                    pass_sleep,
                                                ),
                                                Err(error) => {
                                                    eprintln!(
                                                        "[ssh] trigger stage {}: {error}",
                                                        decision.stage
                                                    );
                                                    Vec::new()
                                                }
                                            },
                                            None => Vec::new(),
                                        }
                                    }
                                    _ => decision.segments,
                                };
                                // 分段写回：段间按 sleepMs 停顿（契约 §2.1）。
                                // timeout 决策没有可发送段但视为已处理；命令
                                // 执行失败则什么都没发。
                                let mut answered =
                                    matches!(decision.kind, triggers::TriggerKind::Timeout);
                                let mut channel_dead = false;
                                for (payload, delay_ms) in &segments {
                                    if *delay_ms > 0 {
                                        tokio::time::sleep(Duration::from_millis(*delay_ms))
                                            .await;
                                    }
                                    if channel.data(&payload[..]).await.is_err() {
                                        channel_dead = true;
                                        break;
                                    }
                                    answered = true;
                                }
                                if channel_dead {
                                    break;
                                }
                                // D6：事件只带 {sessionId, stage, kind}，永不
                                // 携带应答内容。命令执行失败时不发事件（不能
                                // 让宿主提示一次不存在的应答）。
                                if answered {
                                    let _ = emitter.event(
                                        "ssh/trigger",
                                        json!({
                                            "sessionId": task_id,
                                            "stage": decision.stage,
                                            "kind": decision.kind.name(),
                                        }),
                                    );
                                }
                            } else if let Some((kind, answer)) = auto_answer {
                                let mut payload = answer.into_bytes();
                                payload.push(b'\r');
                                if channel.data(&payload[..]).await.is_err() { break; }
                                let _ = emitter.event("ssh/auto-sudo", json!({
                                    "sessionId": task_id,
                                    "kind": if kind == exec::AutoSudoKind::Totp { "otp" } else { "password" },
                                }));
                            }
                            // Agent terminal capture: feed the recorder
                            // installed by `exec_in_terminal` so the AI sees
                            // what the terminal shows.
                            if let Ok(mut slot) = entry.agent_recorder.lock() {
                                if let Some(recorder) = slot.as_mut() {
                                    recorder.observe(&chunk_text);
                                }
                            }
                        }
                        let Some(data) = directory_filter.filter(&data) else { continue; };
                        // #90 ZMODEM 触发检测：远端 `sz` 用 ZRQINIT 开启下载会
                        // 话。ZMODEM 下载接收已由前端 sentry（zmodem.js）承接，
                        // 字节一律原样透传给终端路径（sentry 会消费协议帧）；
                        // 检测器只产出事件边沿（40s 事件窗口内去重重试），供
                        // UX 提示/观测。只在 Stdout 上扫（sz 的协议帧走
                        // stdout；stderr 原样直通）。ZRINIT（rz 上传）不触发；
                        // 窗口超时后自动复位，后续 `sz` 会再次上报。
                        let zmodem_detected =
                            stream == TerminalStream::Stdout && zmodem_detector.feed(&data, unix_now_ms());
                        if zmodem_detected {
                            let _ = emitter.event(
                                "ssh/zmodem",
                                json!({ "sessionId": task_id, "kind": "zrqinit" }),
                            );
                        }
                        if !data.is_empty() {
                            // Session recording capture: everything the terminal
                            // shows (stdout + stderr, post-filter) lands in the
                            // cast file when a recording is active.
                            if stream != TerminalStream::State {
                                if let Ok(mut slot) = entry.session_recorder.lock() {
                                    if let Some(recorder) = slot.as_mut() {
                                        recorder.observe(&data);
                                    }
                                }
                            }
                            publish_terminal(&task_id, stream, data, &replay, &emitter).await;
                        }
                        if directory_filter.take_failed() {
                            directory_tracking_enabled = false;
                            publish_terminal(
                                &task_id,
                                TerminalStream::State,
                                b"directory-tracking-unavailable".to_vec(),
                                &replay,
                                &emitter,
                            ).await;
                        }
                    }
                }
            }
            entry.connected.store(false, Ordering::Release);
            // Auto-stop an active session recording: the file is finalized
            // even when the workbench never sent `ssh/recording/stop`.
            if let Ok(mut slot) = entry.session_recorder.lock() {
                if let Some(recorder) = slot.take() {
                    if let Err(error) = recorder.finish() {
                        eprintln!("[ssh-trace] recording auto-stop failed: {error}");
                    }
                }
            }
            publish_terminal(
                &task_id,
                TerminalStream::State,
                b"ssh-transport-disconnected".to_vec(),
                &replay,
                &emitter,
            )
            .await;
            let _ = emitter.event(
                "ssh/session/state",
                json!({
                    "sessionId": task_id,
                    "connectionId": entry.connection_id,
                    "state": "disconnected"
                }),
            );
            let removed = sessions.write().await.remove(&task_id);
            if let Some(removed) = removed {
                removed.transport_lease.release().await;
            }
            // 会话消失也要清传输登记（upload/download 表项、.part/spool 暂存、
            // metrics 缓存等）：它们此前只在 close_session 清——网络自发断开
            // 路径漏掉，表项与暂存文件会一直挂到 sidecar 退出。
            if let Err(error) = runtime_for_cleanup
                .cleanup_session_transfers(&task_id)
                .await
            {
                eprintln!("[ssh-trace] transfer cleanup after disconnect failed: {error}");
            }
        });

        // Auto-record（M14）：偏好开启时对每个新会话自动挂录制器。只读连接
        // 不禁用（录制是被动输出捕获）；已有录制进行中则跳过——两种情形都
        // 经 `ssh/recording/auto` 事件提示一次，负载只带 id 不带内容。
        let emitter = auto_record_emitter;
        if session_recording::auto_record_enabled() {
            let mut slot = auto_record_slot
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if slot.is_some() {
                let _ = emitter.event(
                    "ssh/recording/auto",
                    json!({
                        "sessionId": session_id,
                        "skipped": true,
                    }),
                );
            } else {
                let recording_id = Uuid::new_v4().to_string();
                match session_recording::SessionRecorder::start(
                    &self.data_dir,
                    &recording_id,
                    &auto_record_connection_id,
                    &connection.host,
                    &session_id,
                    80,
                    24,
                ) {
                    Ok(recorder) => {
                        *slot = Some(recorder);
                        let _ = emitter.event(
                            "ssh/recording/auto",
                            json!({
                                "sessionId": session_id,
                                "recordingId": recording_id,
                            }),
                        );
                    }
                    Err(error) => {
                        eprintln!("[ssh-trace] auto-record start failed: {error}");
                    }
                }
            }
        }

        Ok(json!({
            "sessionId": session_id,
            "connectionId": connection.id,
            "connected": true,
            "sequence": 0,
            "chunkSize": TRANSFER_CHUNK_SIZE,
            "directoryTrackingSupported": directory_tracking_supported
        }))
    }
}
