//! Read loop and message dispatch for one open SSH session.
//!
//! The loop owns the channel after `open_session` returns: it multiplexes
//! terminal commands, remote output, the directory-tracking handshake filter,
//! ZMODEM edge detection, triggers, auto-sudo and the recorders, and tears the
//! session down when the channel dies.

use super::*;

use crate::core::time::{unix_now_ms, unix_now_secs};

use super::pty::TerminalChannel;

/// Lazily decodes one stdout frame for the text observers. The cell makes the
/// trigger engine, auto-sudo and agent capture share a single decode, and the
/// lazy form keeps sessions without observers free of the per-frame cost.
fn observer_text<'a>(
    cell: &'a std::cell::OnceCell<String>,
    carry: &mut Vec<u8>,
    data: &[u8],
) -> &'a str {
    cell.get_or_init(|| decode_stream_text(carry, data)).as_str()
}

/// State handed to the detached read loop.
pub(super) struct ReadLoop {
    /// Runtime handle for post-disconnect cleanup (a 'static spawn cannot
    /// borrow `&self`): transfer registry entries, staging files and metrics
    /// caches are cleared through it when the transport drops by itself.
    pub(super) runtime: Arc<SshRuntime>,
    pub(super) sessions: Arc<AsyncRwLock<HashMap<String, Arc<SessionEntry>>>>,
    pub(super) entry: Arc<SessionEntry>,
    pub(super) replay: Arc<AsyncMutex<ReplayBuffer>>,
    pub(super) emitter: PluginEmitter,
    /// Session id published with every event.
    pub(super) task_id: String,
    /// Session id embedded in the directory-tracking handshake marker.
    pub(super) directory_marker_id: String,
    pub(super) channel: TerminalChannel,
    pub(super) remote_shell: RemoteShell,
    pub(super) terminal_rx: mpsc::Receiver<TerminalCommand>,
}

/// Spawns the session read loop; it ends when the channel closes, the user
/// sends `Close`, or the terminal command queue is dropped.
pub(super) fn spawn_read_loop(state: ReadLoop) {
    let ReadLoop {
        runtime: runtime_for_cleanup,
        sessions,
        entry,
        replay,
        emitter,
        task_id,
        directory_marker_id,
        mut channel,
        mut remote_shell,
        mut terminal_rx,
    } = state;
    tokio::spawn(async move {
        // Every published chunk rides this one channel name: formatting it per
        // frame would allocate a string on the hottest terminal-output path.
        let terminal_channel = format!("ssh/terminal/out/{task_id}");
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
                            publish_terminal(&terminal_channel, TerminalStream::Stdout, data, &replay, &emitter).await;
                        }
                    }
                    if directory_filter.take_failed() {
                        directory_tracking_enabled = false;
                        publish_terminal(
                            &terminal_channel,
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
                            // Session-open arming probes the shell off this
                            // loop (once per transport) and delivers
                            // InstallPromptHook when it resolves. Probing here
                            // would stall terminal output behind a channel dial
                            // and burn a second probe channel per pane.
                            continue;
                        }
                        if remote_shell == RemoteShell::Other {
                            publish_terminal(
                                &terminal_channel,
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
                            &terminal_channel,
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
                        // raced ahead) is left untouched. The handshake
                        // filter publishes `\r\x1b[2K` when it drops the
                        // typed line's echo, so the first prompt after
                        // arming starts on a clean line instead of being
                        // appended to the pre-arming prompt.
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
                        // A terminal tab close must also end the remote shell:
                        // some sshd builds keep counting the channel until the
                        // child exits, and an interactive shell ignores TERM.
                        let _ = channel.signal(russh::Sig::HUP).await;
                        let _ = channel.close().await;
                        channel_ledger::closed(channel.id().number());
                        break;
                    }
                },
                message = channel.wait() => {
                    let (data, stream) = match message {
                        Some(ChannelMsg::Data { data }) => (data.to_vec(), TerminalStream::Stdout),
                        Some(ChannelMsg::ExtendedData { data, .. }) => (data.to_vec(), TerminalStream::Stderr),
                        Some(ChannelMsg::Eof | ChannelMsg::Close) | None => {
                            channel_ledger::closed(channel.id().number());
                            break;
                        }
                        _ => continue,
                    };
                    if stream == TerminalStream::Stdout {
                        // Text observers (triggers, auto-sudo, agent capture)
                        // need a UTF-8 decode per frame; most sessions have none
                        // installed, so the decode is lazy: it only runs when one
                        // of the guards below actually holds an observer. Sharing
                        // one OnceCell keeps a single decode for the two engines,
                        // and doing it inside the guard removes any window where
                        // an observer installed between a probe and its use would
                        // miss this frame (the old unconditional decode covered it).
                        let chunk_text = std::cell::OnceCell::new();
                        // Expect 式触发器先于终端 auto-sudo 观察（契约 D1
                        // 互斥防双答）：本 chunk 被触发器应答则跳过
                        // auto-sudo，反之亦然。锁在 .await 前释放。
                        let trigger_hit = {
                            let mut slot = entry
                                .triggers
                                .lock()
                                .unwrap_or_else(|poison| poison.into_inner());
                            slot.as_mut().and_then(|engine| {
                                let text = observer_text(
                                    &chunk_text,
                                    &mut stdout_utf8_carry,
                                    &data,
                                );
                                engine.observe(text, unix_now_ms()).map(|decision| {
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
                                .and_then(|auto| {
                                    let text = observer_text(
                                        &chunk_text,
                                        &mut stdout_utf8_carry,
                                        &data,
                                    );
                                    auto.observe(text)
                                })
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
                                let text =
                                    observer_text(&chunk_text, &mut stdout_utf8_carry, &data);
                                recorder.observe(text);
                            }
                        }
                    }
                    let Some(data) = directory_filter.filter(data) else { continue; };
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
                        publish_terminal(&terminal_channel, stream, data, &replay, &emitter).await;
                    }
                    if directory_filter.take_failed() {
                        directory_tracking_enabled = false;
                        publish_terminal(
                            &terminal_channel,
                            TerminalStream::State,
                            b"directory-tracking-unavailable".to_vec(),
                            &replay,
                            &emitter,
                        ).await;
                    }
                }
            }
        }
        channel_ledger::closed(channel.id().number());
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
            &terminal_channel,
            TerminalStream::State,
            b"ssh-transport-disconnected".to_vec(),
            &replay,
            &emitter,
        )
        .await;
        // Capture before the registry entry is dropped so cleanup can still
        // attribute failed transfer rows to the saved connection.
        let connection_id = entry.connection_id.clone();
        let _ = emitter.event(
            "ssh/session/state",
            json!({
                "sessionId": task_id,
                "connectionId": &connection_id,
                "state": "disconnected"
            }),
        );
        // Spontaneous transport loss bypasses close_session: fire the same
        // session-end hooks so feature runners release their cloned handles
        // (and registry slots) instead of waiting for a TTL.
        runtime_for_cleanup.notify_session_end(&task_id);
        let removed = sessions.write().await.remove(&task_id);
        if let Some(removed) = removed {
            removed.transport_lease.release().await;
        }
        // 会话消失也要清传输登记（upload/download 表项、.part/spool 暂存、
        // metrics 缓存等）：它们此前只在 close_session 清——网络自发断开
        // 路径漏掉，表项与暂存文件会一直挂到 sidecar 退出。
        if let Err(error) = runtime_for_cleanup
            .cleanup_session_transfers(&task_id, &connection_id)
            .await
        {
            eprintln!("[ssh-trace] transfer cleanup after disconnect failed: {error}");
        }
    });
}
