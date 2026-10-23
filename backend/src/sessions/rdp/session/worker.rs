//! Metrics support.

use super::*;

// —— worker (generation-guarded pump) ——

pub(crate) fn spawn_worker(
    session_id: String,
    entry: Arc<RdpSessionEntry>,
    emitter: PluginEmitter,
    sessions: Arc<tokio::sync::RwLock<HashMap<String, Arc<RdpSessionEntry>>>>,
) {
    tokio::spawn(async move {
        let emit_state = |state: &str, error: Option<(&str, &str)>, attempt: Option<(u32, u32)>| {
            let mut payload = json!({
                "sessionId": session_id,
                "workbenchId": entry.workbench_id,
                "state": state,
            });
            if let Some((kind, message)) = error {
                payload["errorKind"] = Value::String(kind.to_string());
                payload["error"] = Value::String(message.to_string());
            }
            if let Some((attempt, max)) = attempt {
                payload["attempt"] = Value::from(attempt);
                payload["maxAttempts"] = Value::from(max);
            }
            emitter.event("rdp/session/state", payload)
        };
        // D3 (rdp-security-review): ladder step + cumulative total budget.
        // An active generation resets the step only, so a hostile server
        // cannot keep the client dialling forever.
        let mut budget = ReconnectBudget::new();
        loop {
            if entry.close_requested.load(Ordering::Acquire) {
                return;
            }
            let generation = entry.generation.fetch_add(1, Ordering::AcqRel) + 1;
            let _ = emit_state("connecting", None, None);
            match run_generation(&session_id, &entry, generation, &emitter).await {
                GenerationEnd::Closed | GenerationEnd::Superseded => {
                    // Only the current generation may publish the terminal
                    // state (close() owns the closed state; a superseded
                    // generation's successor owns the session now).
                    if entry.generation.load(Ordering::Acquire) == generation
                        && !entry.close_requested.load(Ordering::Acquire)
                    {
                        let _ = emit_state("closed", None, None);
                        entry.frame_state.lock().await.clear();
                        sessions.write().await.remove(&session_id);
                    }
                    return;
                }
                GenerationEnd::Failed {
                    error,
                    error_kind,
                    retryable,
                    was_active,
                } => {
                    let _ = emit_state("error", Some((error_kind.as_str(), error.as_str())), None);
                    // An error invalidates the retained scene. Release the
                    // bounded buffer before retry backoff so a dead/retrying
                    // session never holds a stale 4K desktop in memory.
                    entry.frame_state.lock().await.clear();
                    if was_active {
                        // The session had been fully active in this
                        // generation: a fresh backoff ladder (NyaTerm resets
                        // the attempt counter on the first active frame).
                        // The cumulative budget is NOT reset (D3).
                        budget.on_active_generation();
                    }
                    if !budget.allows(entry.reconnect_attempts, retryable)
                        || entry.close_requested.load(Ordering::Acquire)
                        || entry.generation.load(Ordering::Acquire) != generation
                    {
                        let _ = emit_state("closed", None, None);
                        entry.frame_state.lock().await.clear();
                        sessions.write().await.remove(&session_id);
                        return;
                    }
                    budget.charge();
                    let _ = emit_state(
                        "reconnecting",
                        Some((error_kind.as_str(), error.as_str())),
                        Some((budget.attempt, entry.reconnect_attempts)),
                    );
                    tokio::time::sleep(reconnect_delay(budget.attempt)).await;
                }
            }
        }
    });
}

/// One dial → negotiate → pump cycle. Runs the IronRDP client on a dedicated
/// OS thread (current-thread runtime, NyaTerm's engine-thread shape) and
/// pumps its output events into binary frames / state events.
pub(crate) async fn run_generation(
    session_id: &str,
    entry: &Arc<RdpSessionEntry>,
    generation: u64,
    emitter: &PluginEmitter,
) -> GenerationEnd {
    if entry.generation.load(Ordering::Acquire) != generation {
        return GenerationEnd::Superseded;
    }
    let config = match build_ironrdp_config(session_id, entry, generation, emitter) {
        Ok(config) => config,
        Err(error) => {
            return GenerationEnd::Failed {
                error,
                error_kind: RdpErrorKind::Session,
                retryable: false,
                was_active: false,
            };
        }
    };
    let (output_sender, mut output_receiver) = mpsc::channel::<RdpOutputEvent>(2);
    let client = IronRdpClient::new(config, output_sender);
    let input_sender = client.input_sender();
    // 引擎中止通道：watch sender 与本代 run_generation 同生命周期。函数以任何
    // 原因返回（关闭/终态失败/换代/连接预算放弃）即 drop——引擎侧 changed()
    // 返回 Err，select 触发，client.run() future 被丢弃，其 TCP/TLS transport
    // 随之关闭。否则阻塞在握手等待里的引擎线程会永远抱着 ESTABLISHED socket
    //（连接预算放弃/连接中关闭/换代重连都会各泄漏一条，真机 wtr 实测复现）。
    let (_engine_abort_tx, mut engine_abort_rx) = tokio::sync::watch::channel(false);
    let mut join_handle = Some(std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        if let Ok(runtime) = runtime {
            runtime.block_on(async move {
                tokio::select! {
                    _ = client.run() => {}
                    // sender drop（或显式 send）即中止本代引擎。栈内连接/握手
                    // 全部是 tokio 异步等待点，select 能可靠打断并回收 socket。
                    _ = engine_abort_rx.changed() => {}
                }
            });
        }
    }));

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<RdpCommand>(COMMAND_CHANNEL_CAPACITY);
    *entry.command_sender.lock().await = Some(cmd_tx);

    let mut input_database = IronRdpInputDatabase::new();
    let mut was_active = false;
    let end = loop {
        if entry.generation.load(Ordering::Acquire) != generation {
            break GenerationEnd::Superseded;
        }
        // 预算只在首帧前生效；证书确认等待期（引擎阻塞在 verify 回调，无输出
        // 事件流动）豁免——豁免信号由 verifier 置位/清零，见 cert_wait。
        let budget_enabled = !was_active && !entry.cert_wait.load(Ordering::Acquire);
        tokio::select! {
            event = output_receiver.recv() => match event {
                Some(event) => {
                    if let Some(end) = handle_output_event(
                        session_id, entry, generation, event, emitter, &mut was_active,
                    )
                    .await
                    {
                        break end;
                    }
                }
                None => {
                    // The engine thread dropped its sender: it stopped.
                    if let Some(handle) = join_handle.take() {
                        let _ = handle.join();
                    }
                    break GenerationEnd::Failed {
                        error: "RDP engine stopped".to_string(),
                        error_kind: RdpErrorKind::Session,
                        retryable: true,
                        was_active,
                    };
                }
            },
            command = cmd_rx.recv() => match command {
                Some(RdpCommand::Input(input)) => {
                    if let Err(error) = send_wire_input(&input_sender, &mut input_database, input)
                    {
                        break GenerationEnd::Failed {
                            error,
                            error_kind: RdpErrorKind::Session,
                            retryable: false,
                            was_active,
                        };
                    }
                }
                // 注意：RdpCommand::Resize 已随 rdp/resize 的“重连生效”方案
                // 移除（见 RdpSessionRuntime::resize 文档）——会话内 display
                // control PDU 真机不生效，运行中改尺寸走 generation 重拨。
                Some(RdpCommand::ClipboardAdvertise) => {
                    let sent = input_sender.send(IronRdpInputEvent::Clipboard(
                        ClipboardMessage::SendInitiateCopy(vec![ClipboardFormat::new(
                            ClipboardFormatId::CF_UNICODETEXT,
                        )]),
                    ));
                    if sent.is_err() {
                        break GenerationEnd::Failed {
                            error: "RDP input channel is closed".to_string(),
                            error_kind: RdpErrorKind::Session,
                            retryable: false,
                            was_active,
                        };
                    }
                }
                Some(RdpCommand::Close) | None => break GenerationEnd::Closed,
            },
            // 首帧前连接进度预算（CONNECT_PROGRESS_BUDGET）：预算随每个循环
            // 重建（任一事件/命令即重置）。到期竞态兜底：证书窗恰在此刻开启
            // 则本轮豁免，回循环重算（guard 是进入 select 时的旧值）。
            _ = tokio::time::sleep(CONNECT_PROGRESS_BUDGET), if budget_enabled => {
                if entry.cert_wait.load(Ordering::Acquire) {
                    continue;
                }
                break GenerationEnd::Failed {
                    error: format!(
                        "RDP connection made no progress for {CONNECT_PROGRESS_BUDGET:?} before the first frame"
                    ),
                    error_kind: RdpErrorKind::Transport,
                    retryable: true,
                    was_active,
                };
            }
        }
    };
    // Clear only when this generation still owns the channel: a superseded
    // worker must not drop the successor's sender.
    if entry.generation.load(Ordering::Acquire) == generation {
        *entry.command_sender.lock().await = None;
    }
    // Tell the engine to finish; the abort channel's sender drops when this
    // function returns, which aborts a still-running engine (see the channel
    // comment) — a handshake-stalled engine would otherwise park forever.
    // Only join when it has already finished; never block the pump on it.
    let _ = input_sender.send(IronRdpInputEvent::Close);
    if let Some(handle) = join_handle.take() {
        if handle.is_finished() {
            let _ = handle.join();
        }
    }
    end
}

/// Handles one engine output event; `Some(end)` aborts the pump.
pub(crate) async fn handle_output_event(
    session_id: &str,
    entry: &Arc<RdpSessionEntry>,
    generation: u64,
    event: RdpOutputEvent,
    emitter: &PluginEmitter,
    was_active: &mut bool,
) -> Option<GenerationEnd> {
    if entry.generation.load(Ordering::Acquire) != generation {
        return Some(GenerationEnd::Superseded);
    }
    match event {
        RdpOutputEvent::ImagePatch {
            buffer,
            desktop_width,
            desktop_height,
            x,
            y,
            width,
            height,
        } => {
            if !*was_active {
                *was_active = true;
                let _ = emitter.event(
                    "rdp/session/state",
                    json!({
                        "sessionId": session_id,
                        "workbenchId": entry.workbench_id,
                        "state": "connected",
                    }),
                );
            }
            // The desktop is engine-supplied: bound it like every other
            // wire-derived value (fail the session on violation).
            if desktop_width > MAX_DESKTOP_WIDTH || desktop_height > MAX_DESKTOP_HEIGHT {
                return Some(GenerationEnd::Failed {
                    error: format!(
                        "RDP desktop {desktop_width}x{desktop_height} is outside the supported range (max {MAX_DESKTOP_WIDTH}x{MAX_DESKTOP_HEIGHT})"
                    ),
                    error_kind: RdpErrorKind::Session,
                    retryable: false,
                    was_active: false,
                });
            }
            let mut frame_state = entry.frame_state.lock().await;
            match frame_state.apply_and_encode(
                &buffer,
                desktop_width,
                desktop_height,
                x,
                y,
                width,
                height,
            ) {
                Ok(frame) => {
                    // Keep the shared frame-state lock through emission. This
                    // makes mutation, sequence allocation, replay construction
                    // and output publication one total order.
                    publish_frame(session_id, &frame, emitter);
                }
                Err(error) => {
                    return Some(GenerationEnd::Failed {
                        error,
                        error_kind: RdpErrorKind::Session,
                        retryable: false,
                        was_active: false,
                    });
                }
            }
            None
        }
        RdpOutputEvent::ConnectionFailure(error) => {
            let error_text = format!("{error:?}");
            let (kind, retryable) = classify_connector_error_text(&error_text);
            let message = connector_error_message(kind, &error_text);
            // 诊断通道（stderr 走 sidecar-exit 宿主日志，不进事件流）：vendored
            // 连接器的原始错误文本。认证类错误对用户只暴露统一文案（§红线），
            // 但运维排障需要区分 CredSSP 拒绝码（如 CBT/EPA vs 凭据错误）。
            // 连接器错误不含凭据（凭据只进 Zeroizing），可安全记录。
            eprintln!("[dbx-plugin-sshx] rdp connection raw error: {error_text}");
            eprintln!(
                "[dbx-plugin-sshx] rdp connection failed ({}): {}",
                kind.as_str(),
                message
            );
            Some(GenerationEnd::Failed {
                error: message,
                error_kind: kind,
                retryable,
                was_active: false,
            })
        }
        RdpOutputEvent::Terminated(result) => match result {
            Ok(reason) => Some(GenerationEnd::Failed {
                error: format!("RDP session disconnected: {reason:?}"),
                error_kind: RdpErrorKind::Session,
                // Graceful server-side disconnect: no auto-reconnect
                // (NyaTerm parity — the user decides via rdp/reconnect).
                retryable: false,
                was_active: *was_active,
            }),
            Err(error) => {
                let (kind, retryable) = classify_session_error_text(&format!("{error:?}"));
                let message = session_error_message(kind, &format!("{error:?}"));
                eprintln!(
                    "[dbx-plugin-sshx] rdp session failed ({}): {}",
                    kind.as_str(),
                    message
                );
                Some(GenerationEnd::Failed {
                    error: message,
                    error_kind: kind,
                    retryable,
                    was_active: *was_active,
                })
            }
        },
        RdpOutputEvent::PointerDefault => {
            let _ = emitter.event(
                "rdp/pointer",
                json!({ "type": "default", "sessionId": session_id }),
            );
            None
        }
        RdpOutputEvent::PointerHidden => {
            let _ = emitter.event(
                "rdp/pointer",
                json!({ "type": "hidden", "sessionId": session_id }),
            );
            None
        }
        RdpOutputEvent::PointerPosition { x, y } => {
            let _ = emitter.event(
                "rdp/pointer",
                json!({ "type": "position", "sessionId": session_id, "x": x, "y": y }),
            );
            None
        }
        RdpOutputEvent::PointerBitmap(pointer) => {
            let _ = emitter.event(
                "rdp/pointer",
                json!({
                    "type": "bitmap",
                    "sessionId": session_id,
                    "width": pointer.width,
                    "height": pointer.height,
                    "hotspotX": pointer.hotspot_x,
                    "hotspotY": pointer.hotspot_y,
                    "rgbaBase64": BASE64_STANDARD.encode(&pointer.bitmap_data),
                }),
            );
            None
        }
    }
}

pub(crate) fn publish_frame(session_id: &str, frame: &[u8], emitter: &PluginEmitter) {
    if let Err(error) = emitter.binary(&format!("rdp/frame/{session_id}"), frame) {
        // Frame loss is visual-only; the engine's own I/O errors tear the
        // session down, so a failing emit only needs a log line.
        eprintln!(
            "[dbx-plugin-sshx] rdp frame publish failed: {}",
            error.message
        );
    }
}
