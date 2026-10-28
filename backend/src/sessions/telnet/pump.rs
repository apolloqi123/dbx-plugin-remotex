//! Telnet layer.

use super::*;
use crate::core::time::unix_now_ms;

pub(crate) async fn publish_telnet_output(
    channel: &str,
    stream: TerminalStream,
    data: Vec<u8>,
    replay: &Arc<tokio::sync::Mutex<ReplayBuffer>>,
    emitter: &PluginEmitter,
) {
    let frame = replay.lock().await.push_encoded(stream, data);
    if let Err(error) = emitter.binary(channel, &frame) {
        eprintln!(
            "[dbx-plugin-sshx] telnet output publish failed: {}",
            error.message
        );
    }
}

/// Applies one trigger decision's segments exactly like the SSH read loop:
/// command answers run locally first (their trimmed stdout becomes the
/// answer), then segments are written with `sleepMs` pacing. Returns `true`
/// when anything was sent (a timeout counts as handled; a failed command
/// sends nothing).
pub(crate) async fn apply_trigger_decision(
    write_half: &mut tokio::net::tcp::OwnedWriteHalf,
    decision: &triggers::TriggerDecision,
    placeholders: &triggers::CommandPlaceholders,
    pacing: (u64, triggers::PassSleep),
) -> std::io::Result<bool> {
    let segments = match decision.kind {
        triggers::TriggerKind::Command => {
            let Some(command) = decision.command.as_deref() else {
                return Ok(false);
            };
            match triggers::run_credential_command(command, placeholders).await {
                Ok(answer) => triggers::pass_sleep_segments(&answer, pacing.0, pacing.1),
                Err(error) => {
                    // Failure reasons only — never the answer content.
                    eprintln!("[telnet] trigger stage {}: {error}", decision.stage);
                    return Ok(false);
                }
            }
        }
        _ => decision.segments.clone(),
    };
    let mut answered = matches!(decision.kind, triggers::TriggerKind::Timeout);
    for (payload, delay_ms) in &segments {
        if *delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
        }
        write_half.write_all(payload).await?;
        answered = true;
    }
    Ok(answered)
}

/// One parsed TCP read: application payload plus negotiation replies. Parsing
/// happens in the reader task BEFORE the bounded queue, so a dropped chunk can
/// never leave the IAC state machine desynchronized (the parser already
/// consumed those bytes; only their display is lost).
pub(super) struct TelnetOutput {
    pub(super) payload: Vec<u8>,
    pub(super) replies: Vec<u8>,
}

/// Parses one reader chunk into a queue item. Pure apart from the parser state.
pub(super) fn parse_telnet_output(parser: &mut TelnetParser, data: &[u8]) -> TelnetOutput {
    let (payload, replies) = parser.feed(data);
    TelnetOutput { payload, replies }
}

/// Reader → pump output queue depth. Each slot is one TCP read (32 KiB), so
/// the queue is bounded at ~2 MiB per session. Overflow drops the newest
/// chunk (the reader never blocks and never grows) and logs a rate-limited
/// byte count; Telnet has no dropped-output protocol event.
const TELNET_READER_OUTPUT_QUEUE: usize = 64;

/// One socket read buffer; matches the SSH/local chunk size.
const TELNET_READ_BUFFER: usize = 32 * 1024;

/// Owns one Telnet TCP connection: dials with the 10s timeout, pumps output
/// through the IAC parser (plus the expect engine), serves workbench commands
/// and tears down on peer close/close command/any write failure.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_pump(
    session_id: String,
    workbench_id: String,
    host: String,
    port: u16,
    runtime_host: String,
    runtime_port: u16,
    cols: u16,
    rows: u16,
    enter_mode: EnterMode,
    backspace_mode: BackspaceMode,
    auto_login_plan: Option<AutoLoginPlan>,
    mut cmd_rx: mpsc::Receiver<TelnetCommand>,
    replay: Arc<tokio::sync::Mutex<ReplayBuffer>>,
    emitter: PluginEmitter,
    sessions: Arc<RwLock<HashMap<String, Arc<TelnetSession>>>>,
) {
    tokio::spawn(async move {
        // Hoisted channel name: per-frame formatting would allocate on the
        // hottest output path (same as the SSH/local pumps).
        let output_channel = format!("telnet/terminal/out/{session_id}");
        let emit_state = |state: &str, error: Option<String>| {
            let mut payload = json!({
                "sessionId": session_id,
                "workbenchId": workbench_id,
                "state": state,
            });
            if let Some(error) = error {
                payload["error"] = Value::String(error);
            }
            emitter.event("telnet/session/state", payload)
        };
        let _ = emit_state("connecting", None);
        let stream = match tokio::time::timeout(
            CONNECT_TIMEOUT,
            TcpStream::connect((runtime_host.as_str(), runtime_port)),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => {
                let _ = emit_state(
                    "error",
                    Some(format!("connect {runtime_host}:{runtime_port}: {error}")),
                );
                let _ = emit_state("closed", None);
                sessions.write().await.remove(&session_id);
                return;
            }
            Err(_) => {
                let _ = emit_state(
                    "error",
                    Some(format!(
                        "connect {runtime_host}:{runtime_port} timed out after 10s"
                    )),
                );
                let _ = emit_state("closed", None);
                sessions.write().await.remove(&session_id);
                return;
            }
        };
        let _ = emit_state("connected", None);

        let (mut read_half, mut write_half) = stream.into_split();
        // 首帧 NAWS：对端声明 DO NAWS 后窗口尺寸就有意义了，尽力而为
        // （未协商时多数实现直接忽略 SB NAWS）。
        let _ = write_half.write_all(&naws_frame(cols, rows)).await;
        // Reader task → bounded channel, so the select loop never blocks a
        // read against command handling (mirrors the local PTY pump). A full
        // queue drops the newest chunk instead of buffering without bound.
        // The IAC parser lives in the reader: dropped chunks are fully parsed
        // (only their payload is lost), so the state machine can never desync
        // at a dropped IAC sequence.
        let (out_tx, mut out_rx) = mpsc::channel::<TelnetOutput>(TELNET_READER_OUTPUT_QUEUE);
        tokio::spawn(async move {
            let mut parser = TelnetParser::new();
            let mut buffer = vec![0u8; TELNET_READ_BUFFER];
            let mut dropped_bytes = 0_u64;
            let mut last_drop_log = tokio::time::Instant::now();
            loop {
                match read_half.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let output = parse_telnet_output(&mut parser, &buffer[..n]);
                        let size = output.payload.len() + output.replies.len();
                        match out_tx.try_send(output) {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                dropped_bytes = dropped_bytes.saturating_add(size as u64);
                                if last_drop_log.elapsed() >= Duration::from_secs(5) {
                                    eprintln!(
                                        "[dbx-plugin-sshx] telnet output backlog; dropped {dropped_bytes} bytes (newest chunks)"
                                    );
                                    last_drop_log = tokio::time::Instant::now();
                                }
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => break,
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            if dropped_bytes > 0 {
                eprintln!(
                    "[dbx-plugin-sshx] telnet output ended; dropped {dropped_bytes} bytes in total"
                );
            }
        });

        let mut auto_login: Option<AutoLoginRuntime> =
            auto_login_plan.map(|plan| AutoLoginRuntime {
                engine: triggers::TriggerEngine::new(
                    plan.triggers,
                    triggers::CommandPlaceholders::new(&host, "", port, &host),
                ),
                watch: plan.watch,
            });
        // Armed once the first prompt answer goes out: the declarative watch
        // only treats a success pattern as "logged in" after we answered
        // something (NyaTerm's `sent_username || sent_password` guard).
        let mut answered_stages = false;
        let mut closing_reason: Option<String> = None;
        loop {
            tokio::select! {
                chunk = out_rx.recv() => {
                    let Some(chunk) = chunk else {
                        closing_reason.get_or_insert_with(|| "peer closed".to_string());
                        break;
                    };
                    if !chunk.replies.is_empty() && write_half.write_all(&chunk.replies).await.is_err() {
                        closing_reason.get_or_insert_with(|| "write failed".to_string());
                        break;
                    }
                    if chunk.payload.is_empty() {
                        continue;
                    }
                    let payload = chunk.payload;
                    // Only the auto-login watch and expect engine read the
                    // decoded text; a plain interactive Telnet session must not
                    // pay a UTF-8 decode + String allocation per chunk.
                    let chunk_text = auto_login
                        .as_ref()
                        .map(|_| String::from_utf8_lossy(&payload).into_owned());
                    publish_telnet_output(
                        &output_channel,
                        TerminalStream::Stdout,
                        payload,
                        &replay,
                        &emitter,
                    )
                    .await;
                    // Declarative supervision first: failure takes precedence
                    // over answering further prompts (NyaTerm order), so a
                    // rejection banner never feeds the expect engine. The
                    // watch matches anchored success/failure regexes on this
                    // text, so ANSI escapes are stripped first (NyaTerm's
                    // `strip_ansi_escapes`): a colored `$ ` prompt still hits
                    // `[$#>]\s*$`. Only the matching window is cleaned — the
                    // published payload above stays the raw bytes.
                    let watch_event = chunk_text.as_deref().and_then(|text| {
                        auto_login
                            .as_mut()
                            .and_then(|runtime| runtime.watch.as_mut())
                            .and_then(|watch| {
                                watch.observe(&strip_ansi_control_sequences(text), answered_stages)
                            })
                    });
                    match watch_event {
                        // D6: the event carries the outcome only — never the
                        // matched text, never an answer.
                        Some(WatchEvent::Retry { attempt }) => {
                            // 重试轮次立即把引擎游标归位到第一阶段（NyaTerm
                            // 重置 sent_username/sent_password 的对应物）：
                            // 失败可能发生在序列中途（用户名已发、密码未达），
                            // 归位后宿主重印 login: 即从用户名重新应答。
                            if let Some(runtime) = auto_login.as_mut() {
                                runtime.engine.reset(unix_now_ms());
                            }
                            let _ = emitter.event(
                                "telnet/auto_login",
                                json!({ "sessionId": session_id, "status": "retry", "attempt": attempt }),
                            );
                        }
                        Some(WatchEvent::Success) => {
                            let _ = emitter.event(
                                "telnet/auto_login",
                                json!({ "sessionId": session_id, "status": "success" }),
                            );
                            // Login complete: stop answering and supervising
                            // (NyaTerm's Complete state), the user takes over.
                            auto_login = None;
                        }
                        Some(WatchEvent::Exhausted) => {
                            let attempts = auto_login
                                .as_ref()
                                .and_then(|runtime| runtime.watch.as_ref())
                                .map(DeclarativeWatch::attempts_used)
                                .unwrap_or(1);
                            closing_reason.get_or_insert_with(|| {
                                format!(
                                    "auto-login failed: host rejected the login after {attempts} attempt(s)"
                                )
                            });
                            break;
                        }
                        None => {}
                    }
                    if let Some(runtime) = auto_login.as_mut() {
                        let hit = chunk_text.as_deref().and_then(|text| {
                            runtime.engine.observe(text, unix_now_ms()).map(|decision| {
                                (decision, runtime.engine.placeholders().clone(), runtime.engine.pacing())
                            })
                        });
                        if let Some((decision, placeholders, pacing)) = hit {
                            match apply_trigger_decision(
                                &mut write_half,
                                &decision,
                                &placeholders,
                                pacing,
                            )
                            .await
                            {
                                // D6: the event never carries answer content.
                                Ok(true) => {
                                    answered_stages = true;
                                    let _ = emitter.event(
                                        "telnet/trigger",
                                        json!({
                                            "sessionId": session_id,
                                            "stage": decision.stage,
                                            "kind": decision.kind.name(),
                                        }),
                                    );
                                }
                                Ok(false) => {}
                                Err(_) => {
                                    closing_reason.get_or_insert_with(|| "write failed".to_string());
                                    break;
                                }
                            }
                        }
                    }
                }
                command = cmd_rx.recv() => match command {
                    Some(TelnetCommand::Input(data)) => {
                        let wire = transform_input(&data, enter_mode, backspace_mode);
                        if !wire.is_empty() && write_half.write_all(&wire).await.is_err() {
                            closing_reason.get_or_insert_with(|| "write failed".to_string());
                            break;
                        }
                    }
                    Some(TelnetCommand::Resize { cols, rows }) => {
                        let frame = naws_frame(cols, rows);
                        if write_half.write_all(&frame).await.is_err() {
                            closing_reason.get_or_insert_with(|| "write failed".to_string());
                            break;
                        }
                    }
                    Some(TelnetCommand::Close) | None => break,
                },
            }
        }
        // Terminal-lifecycle State frame so a drained workbench observes the
        // end of the stream, mirroring `local-terminal-exited`.
        publish_telnet_output(
            &output_channel,
            TerminalStream::State,
            b"telnet-session-closed".to_vec(),
            &replay,
            &emitter,
        )
        .await;
        let _ = emit_state("closed", closing_reason);
        sessions.write().await.remove(&session_id);
    });
}

/// Decodes the `telnet/write` JSON fallback payload and the binary
/// `telnet/terminal/in/{id}` frames.
pub fn decode_write_payload(data_base64: &str) -> Result<Vec<u8>, String> {
    let data = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|_| "telnet/write: dataBase64 is not valid base64".to_string())?;
    if data.len() > WRITE_PAYLOAD_LIMIT {
        return Err(format!(
            "telnet write payload of {} bytes exceeds the {} byte limit",
            data.len(),
            WRITE_PAYLOAD_LIMIT
        ));
    }
    Ok(data)
}
