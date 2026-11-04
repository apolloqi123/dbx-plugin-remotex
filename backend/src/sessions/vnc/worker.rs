//! VNC session support.

use super::*;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;

/// Dial timeout for the initial TCP connect (same contract as Telnet).
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// RFB handshake + security negotiation timeout.
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Output-event poll cadence: the engine queues at most a couple of decoded
/// events, so the pump must drain frequently (NyaTerm uses the same 8ms).
pub(crate) const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(8);

/// Incremental framebuffer-update request cadence after observed traffic.
pub(crate) const UPDATE_REQUEST_INTERVAL: Duration = Duration::from_millis(16);

/// Placeholder delay that keeps the pinned `Refresh` timer dormant until the
/// first decoded event re-arms it.
pub(crate) const REFRESH_ARM_DELAY: Duration = Duration::from_secs(86_400);

/// Bounded backpressure for input/clipboard commands.
pub(crate) const COMMAND_CHANNEL_CAPACITY: usize = 256;

pub(crate) fn spawn_worker(
    session_id: String,
    entry: Arc<VncSessionEntry>,
    emitter: PluginEmitter,
    sessions: Arc<RwLock<HashMap<String, Arc<VncSessionEntry>>>>,
) {
    tokio::spawn(async move {
        let emit_state = |state: &str, error: Option<String>| {
            let mut payload = json!({
                "sessionId": session_id,
                "workbenchId": entry.workbench_id,
                "state": state,
            });
            if let Some(error) = error {
                payload["error"] = Value::String(error);
            }
            emitter.event("vnc/session/state", payload)
        };
        let mut attempt: u32 = 0;
        loop {
            if entry.close_requested.load(Ordering::Acquire) {
                return;
            }
            let generation = entry.generation.fetch_add(1, Ordering::AcqRel) + 1;
            let _ = emit_state("connecting", None);
            match run_generation(&session_id, &entry, generation, &emitter).await {
                GenerationEnd::Closed | GenerationEnd::Superseded => {
                    // Only the current generation may publish the terminal
                    // state (close() already emitted it; reconnect() has a
                    // fresh worker that owns the session now).
                    if entry.generation.load(Ordering::Acquire) == generation
                        && !entry.close_requested.load(Ordering::Acquire)
                    {
                        let _ = emit_state("closed", None);
                        sessions.write().await.remove(&session_id);
                    }
                    return;
                }
                GenerationEnd::Failed {
                    error,
                    retryable,
                    was_active,
                } => {
                    let _ = emit_state("error", Some(error));
                    if !retryable
                        || attempt >= entry.reconnect_attempts
                        || entry.close_requested.load(Ordering::Acquire)
                        || entry.generation.load(Ordering::Acquire) != generation
                    {
                        let _ = emit_state("closed", None);
                        sessions.write().await.remove(&session_id);
                        return;
                    }
                    // 活跃代际（完成过握手、进入过泵循环）的失败是"服务过一
                    // 段时间再断"：退避梯子复位重新计步——attempt 贯穿 worker
                    // 生命周期不复位的话，长会话经历几次偶发断线后就会因
                    // 梯子耗尽被永久关闭。总预算由 reconnect_attempts 每次
                    // 重新给满（与 rdp ReconnectBudget.on_active_generation
                    // 的语义一致：attempt 复位、预算另计）。
                    if was_active {
                        attempt = 0;
                    }
                    attempt += 1;
                    tokio::time::sleep(reconnect_delay(attempt)).await;
                }
            }
        }
    });
}

/// One dial → handshake → pump cycle. Returns why it ended; all teardown of
/// shared state happens in [`spawn_worker`].
pub(crate) async fn run_generation(
    session_id: &str,
    entry: &Arc<VncSessionEntry>,
    generation: u64,
    emitter: &PluginEmitter,
) -> GenerationEnd {
    if entry.generation.load(Ordering::Acquire) != generation {
        return GenerationEnd::Superseded;
    }
    // Password is validated at start; re-check before the DES key so a
    // future reconnect path can never truncate silently.
    if let Err(error) = validate_password(entry.password.as_ref().map(|p| p.as_str())) {
        return GenerationEnd::Failed {
            error,
            retryable: false,
            was_active: false,
        };
    }
    let password = entry.password.clone().unwrap_or_default();
    let stream = match tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((entry.runtime_host.as_str(), entry.runtime_port)),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            return GenerationEnd::Failed {
                error: format!(
                    "connect {}:{}: {error}",
                    entry.runtime_host, entry.runtime_port
                ),
                retryable: true,
                was_active: false,
            };
        }
        Err(_) => {
            return GenerationEnd::Failed {
                error: format!(
                    "connect {}:{} timed out after {}s",
                    entry.runtime_host,
                    entry.runtime_port,
                    CONNECT_TIMEOUT.as_secs()
                ),
                retryable: true,
                was_active: false,
            };
        }
    };
    // The auth closure always exists so the future keeps one concrete type;
    // with no password it answers `NoPassword` (upstream behaviour: a server
    // that insists on VNC-Auth fails the handshake with a readable error).
    let state = match VncConnector::new(stream)
        .set_auth_method(async move {
            if password.is_empty() {
                Err(VncError::NoPassword)
            } else {
                Ok((*password).clone())
            }
        })
        .set_pixel_format(PixelFormat::rgba())
        .add_encoding(VncEncoding::ExtendedDesktopSizePseudo)
        .add_encoding(VncEncoding::DesktopSizePseudo)
        .add_encoding(VncEncoding::CursorPseudo)
        .add_encoding(VncEncoding::CopyRect)
        .add_encoding(VncEncoding::Zrle)
        .add_encoding(VncEncoding::Trle)
        .add_encoding(VncEncoding::Tight)
        .add_encoding(VncEncoding::Raw)
        .allow_shared(true)
        .build()
    {
        Ok(state) => state,
        Err(error) => return GenerationEnd::failed(error),
    };
    let client = match tokio::time::timeout(HANDSHAKE_TIMEOUT, state.try_start()).await {
        Ok(Ok(state)) => match state.finish() {
            Ok(client) => client,
            Err(error) => {
                return GenerationEnd::failed(error);
            }
        },
        Ok(Err(error)) => {
            return GenerationEnd::failed(error);
        }
        Err(_) => {
            return GenerationEnd::Failed {
                error: format!(
                    "VNC protocol negotiation timed out after {}s",
                    HANDSHAKE_TIMEOUT.as_secs()
                ),
                retryable: true,
                was_active: false,
            };
        }
    };
    if entry.generation.load(Ordering::Acquire) != generation {
        let _ = client.close().await;
        return GenerationEnd::Superseded;
    }
    let _ = emitter.event(
        "vnc/session/state",
        json!({
            "sessionId": session_id,
            "workbenchId": entry.workbench_id,
            "state": "connected",
        }),
    );

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<VncCommand>(COMMAND_CHANNEL_CAPACITY);
    *entry.command_sender.lock().await = Some(cmd_tx);

    // Reconnect repaint: push the retained desktop as one full-frame patch,
    // then ask the server for a full refresh so new damage keeps flowing.
    {
        let framebuffer = entry.framebuffer.lock().await;
        if let Some(desktop) = framebuffer.as_ref() {
            let sequence = entry.frame_sequence.fetch_add(1, Ordering::AcqRel);
            match desktop.full_frame_bytes(sequence) {
                Ok(frame) => publish_frame(session_id, &frame, emitter),
                Err(error) => eprintln!("[dbx-plugin-sshx] vnc full-frame repaint failed: {error}"),
            }
            let _ = client.input(X11Event::FullRefresh).await;
        }
    }

    let mut pressed_keys = Vec::<u32>::new();
    let mut poll = tokio::time::interval(EVENT_POLL_INTERVAL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut refresh_due = false;
    let refresh_delay = tokio::time::sleep(REFRESH_ARM_DELAY);
    tokio::pin!(refresh_delay);
    let end = loop {
        if entry.generation.load(Ordering::Acquire) != generation {
            break GenerationEnd::Superseded;
        }
        tokio::select! {
            _ = poll.tick() => {
                let mut fatal: Option<GenerationEnd> = None;
                loop {
                    match client.poll_event().await {
                        Ok(Some(event)) => {
                            if let Some(end) =
                                handle_vnc_event(session_id, entry, generation, event, emitter)
                                    .await
                            {
                                fatal = Some(end);
                                break;
                            }
                            refresh_due = true;
                            refresh_delay
                                .as_mut()
                                .reset(tokio::time::Instant::now() + UPDATE_REQUEST_INTERVAL);
                        }
                        Ok(None) => break,
                        Err(error) => {
                            fatal = Some(GenerationEnd::failed(error));
                            break;
                        }
                    }
                }
                if let Some(end) = fatal {
                    break end;
                }
            }
            _ = &mut refresh_delay, if refresh_due => {
                if let Err(error) = client.input(X11Event::Refresh).await {
                    break GenerationEnd::failed(error);
                }
                refresh_due = false;
            }
            command = cmd_rx.recv() => match command {
                Some(VncCommand::Input(input)) => {
                    if let Err(error) =
                        apply_input(&client, input, &mut pressed_keys).await
                    {
                        break GenerationEnd::failed(error);
                    }
                }
                Some(VncCommand::Resize { width, height }) => {
                    spawn_resize(client.clone(), session_id.to_string(), width, height);
                }
                Some(VncCommand::Clipboard(text)) => {
                    if let Err(error) = client.input(X11Event::CopyText(text)).await {
                        break GenerationEnd::failed(error);
                    }
                }
                Some(VncCommand::Close) | None => {
                    break GenerationEnd::Closed;
                }
            },
        }
    };
    // Clear only when this generation still owns the channel: a superseded
    // worker must not drop the successor's sender.
    if entry.generation.load(Ordering::Acquire) == generation {
        *entry.command_sender.lock().await = None;
    }
    let _ = client.close().await;
    end
}

pub(crate) async fn apply_input(
    client: &VncClient,
    input: VncInputEvent,
    pressed_keys: &mut Vec<u32>,
) -> Result<(), VncError> {
    match input {
        VncInputEvent::Key { keysym, pressed } => {
            if pressed {
                if !pressed_keys.contains(&keysym) {
                    pressed_keys.push(keysym);
                }
            } else {
                pressed_keys.retain(|key| *key != keysym);
            }
            client
                .input(X11Event::KeyEvent(ClientKeyEvent {
                    keycode: keysym,
                    down: pressed,
                }))
                .await
        }
        VncInputEvent::Pointer { x, y, button_mask } => {
            client
                .input(X11Event::PointerEvent(ClientMouseEvent {
                    position_x: x,
                    position_y: y,
                    bottons: button_mask,
                }))
                .await
        }
        VncInputEvent::ReleaseAll => {
            for keysym in pressed_keys.drain(..) {
                let _ = client
                    .input(X11Event::KeyEvent(ClientKeyEvent {
                        keycode: keysym,
                        down: false,
                    }))
                    .await;
            }
            Ok(())
        }
    }
}

/// Applies one server event. `Some(end)` aborts the pump (readable protocol
/// violations — the server sent something this client never advertised).
pub(crate) async fn handle_vnc_event(
    session_id: &str,
    entry: &Arc<VncSessionEntry>,
    generation: u64,
    event: VncEvent,
    emitter: &PluginEmitter,
) -> Option<GenerationEnd> {
    if entry.generation.load(Ordering::Acquire) != generation {
        return Some(GenerationEnd::Superseded);
    }
    match event {
        VncEvent::SetResolution(Screen { width, height }) => {
            let framebuffer = match VncFramebuffer::new(width, height) {
                Ok(framebuffer) => framebuffer,
                Err(error) => {
                    return Some(GenerationEnd::Failed {
                        error,
                        retryable: false,
                        was_active: true,
                    });
                }
            };
            *entry.framebuffer.lock().await = Some(framebuffer);
            None
        }
        VncEvent::RawImage(rect, pixels) => {
            let mut framebuffer = match entry.framebuffer_for(rect).await {
                Ok(framebuffer) => framebuffer,
                Err(error) => {
                    return Some(GenerationEnd::Failed {
                        error,
                        retryable: false,
                        was_active: true,
                    });
                }
            };
            let desktop = framebuffer.as_mut().expect("framebuffer initialized");
            if let Err(error) = desktop.apply_rgba(rect, &pixels) {
                return Some(GenerationEnd::Failed {
                    error,
                    retryable: false,
                    was_active: true,
                });
            }
            let sequence = entry.frame_sequence.fetch_add(1, Ordering::AcqRel);
            match desktop.patch_bytes(sequence, rect, &pixels) {
                Ok(frame) => publish_frame(session_id, &frame, emitter),
                Err(error) => {
                    return Some(GenerationEnd::Failed {
                        error,
                        retryable: false,
                        was_active: true,
                    });
                }
            }
            None
        }
        VncEvent::JpegImage(rect, jpeg) => {
            let pixels = match decode_jpeg_rect(rect, &jpeg) {
                Ok(pixels) => pixels,
                Err(error) => {
                    return Some(GenerationEnd::Failed {
                        error,
                        retryable: false,
                        was_active: true,
                    });
                }
            };
            let mut framebuffer = match entry.framebuffer_for(rect).await {
                Ok(framebuffer) => framebuffer,
                Err(error) => {
                    return Some(GenerationEnd::Failed {
                        error,
                        retryable: false,
                        was_active: true,
                    });
                }
            };
            let desktop = framebuffer.as_mut().expect("framebuffer initialized");
            if let Err(error) = desktop.apply_rgba(rect, &pixels) {
                return Some(GenerationEnd::Failed {
                    error,
                    retryable: false,
                    was_active: true,
                });
            }
            let sequence = entry.frame_sequence.fetch_add(1, Ordering::AcqRel);
            match desktop.patch_bytes(sequence, rect, &pixels) {
                Ok(frame) => publish_frame(session_id, &frame, emitter),
                Err(error) => {
                    return Some(GenerationEnd::Failed {
                        error,
                        retryable: false,
                        was_active: true,
                    });
                }
            }
            None
        }
        VncEvent::Copy(dst, src) => {
            let mut framebuffer = entry.framebuffer.lock().await;
            let Some(desktop) = framebuffer.as_mut() else {
                // Servers without DesktopSizePseudo may copy before the first
                // raw rect. Visual-only: ignore instead of killing the session.
                eprintln!(
                    "[dbx-plugin-sshx] vnc CopyRect before desktop size; ignoring this update"
                );
                return None;
            };
            let pixels = match desktop.copy_rect(dst, src) {
                Ok(pixels) => pixels,
                Err(error) => {
                    return Some(GenerationEnd::Failed {
                        error,
                        retryable: false,
                        was_active: true,
                    });
                }
            };
            let sequence = entry.frame_sequence.fetch_add(1, Ordering::AcqRel);
            match desktop.patch_bytes(sequence, dst, &pixels) {
                Ok(frame) => publish_frame(session_id, &frame, emitter),
                Err(error) => {
                    return Some(GenerationEnd::Failed {
                        error,
                        retryable: false,
                        was_active: true,
                    });
                }
            }
            None
        }
        VncEvent::Text(text) => {
            // Inbound clipboard; the same UTF-8/1 MiB guard as outbound.
            if validate_clipboard_text(&text).is_ok() {
                super::clipboard::forward_text(session_id, text, emitter);
            }
            None
        }
        VncEvent::SetCursor(rect, pixels) => {
            let payload = cursor_event_payload(session_id, rect, &pixels);
            let _ = emitter.event("vnc/pointer", payload);
            None
        }
        VncEvent::DesktopUpdate(update) => {
            // ExtendedDesktopSize reply (our resize or another client's):
            // the retained desktop must adopt the new geometry before the
            // following image updates arrive, otherwise their bounds check
            // fails against the old framebuffer.
            let Some(layout) = update.layout else {
                return None;
            };
            let mut framebuffer = entry.framebuffer.lock().await;
            match apply_desktop_layout(&mut framebuffer, layout.width, layout.height) {
                Ok(_) => None,
                Err(error) => Some(GenerationEnd::Failed {
                    error,
                    retryable: false,
                    was_active: true,
                }),
            }
        }
        VncEvent::Error(message) => Some(GenerationEnd::Failed {
            error: message,
            retryable: false,
            was_active: true,
        }),
        // SetPixelFormat (we pin the format), Bell and future variants carry
        // no pixels or layout.
        _ => None,
    }
}

/// Decodes a Tight JPEG rect to tightly packed RGBA. The JPEG dimensions are
/// read from the header and must match the rect before any full decode, so a
/// tiny malicious JPEG cannot force a huge allocation; decode limits repeat
/// the bound as a second line of defence.
pub(crate) fn decode_jpeg_rect(rect: Rect, jpeg: &[u8]) -> Result<Vec<u8>, String> {
    if rect.width == 0
        || rect.height == 0
        || rect.width > MAX_FRAMEBUFFER_WIDTH
        || rect.height > MAX_FRAMEBUFFER_HEIGHT
    {
        return Err("VNC Tight JPEG rect is outside the supported bounds".to_string());
    }
    let expected_width = u32::from(rect.width);
    let expected_height = u32::from(rect.height);
    let dimensions = image::ImageReader::with_format(
        std::io::Cursor::new(jpeg),
        image::ImageFormat::Jpeg,
    )
    .into_dimensions()
    .map_err(|error| format!("VNC Tight JPEG header is invalid: {error}"))?;
    if dimensions != (expected_width, expected_height) {
        return Err(format!(
            "VNC Tight JPEG dimensions {}x{} do not match rect {}x{}",
            dimensions.0, dimensions.1, rect.width, rect.height
        ));
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(expected_width);
    limits.max_image_height = Some(expected_height);
    limits.max_alloc = Some(u64::from(expected_width) * u64::from(expected_height) * 8);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("VNC Tight JPEG decode failed: {error}"))?;
    Ok(image.to_rgba8().into_raw())
}

/// Cursor pseudo-encoding → same `vnc/pointer` shape as RDP so the frontend
/// can reuse one CSS-cursor path. 0×0 means the server asked for no cursor
/// (RFC 6143 7.8.1) → `hidden`; oversized/malformed payloads fall back to the
/// default cursor instead of tearing the session down (visual-only data).
pub(crate) fn cursor_event_payload(session_id: &str, rect: Rect, pixels: &[u8]) -> Value {
    if rect.width == 0 || rect.height == 0 {
        return json!({ "sessionId": session_id, "type": "hidden" });
    }
    let expected = usize::from(rect.width) * usize::from(rect.height) * BYTES_PER_PIXEL;
    if rect.width > 256 || rect.height > 256 || expected == 0 || pixels.len() != expected {
        return json!({ "sessionId": session_id, "type": "default" });
    }
    json!({
        "sessionId": session_id,
        "type": "bitmap",
        "width": rect.width,
        "height": rect.height,
        "hotspotX": rect.x,
        "hotspotY": rect.y,
        "rgbaBase64": BASE64_STANDARD.encode(pixels),
    })
}

pub(crate) fn publish_frame(session_id: &str, frame: &[u8], emitter: &PluginEmitter) {
    if let Err(error) = emitter.binary(&format!("vnc/frame/{session_id}"), frame) {
        // Frame loss is visual-only; the engine's own I/O errors tear the
        // session down, so a failing emit only needs a log line.
        eprintln!(
            "[dbx-plugin-sshx] vnc frame publish failed: {}",
            error.message
        );
    }
}
