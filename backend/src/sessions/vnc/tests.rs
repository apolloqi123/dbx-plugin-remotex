use super::*;

fn decode_patch_header(frame: &[u8]) -> (u64, u32, u32, u32, u32, u32, u32, u32, u32, u32) {
    assert!(frame.len() >= FRAME_HEADER_BYTES);
    let le32 = |offset: usize| u32::from_le_bytes(frame[offset..offset + 4].try_into().unwrap());
    (
        u64::from_le_bytes(frame[0..8].try_into().unwrap()),
        le32(8),
        le32(12),
        le32(16),
        le32(20),
        le32(24),
        le32(28),
        le32(32),
        le32(36),
        le32(40),
    )
}

// —— patch 头编码/解码往返 ————————————————————————————————————

#[test]
fn patch_header_roundtrips() {
    let payload = [1u8, 2, 3, 255, 4, 5, 6, 7];
    let frame = encode_frame_patch(&FramePatch {
        sequence: 7,
        desktop_width: 10,
        desktop_height: 10,
        x: 2,
        y: 3,
        width: 1,
        height: 2,
        stride: 4,
        payload: &payload,
    })
    .expect("frame should encode");
    assert_eq!(frame.len(), FRAME_HEADER_BYTES + payload.len());
    let (sequence, dw, dh, x, y, w, h, stride, format, payload_len) = decode_patch_header(&frame);
    assert_eq!(sequence, 7);
    assert_eq!((dw, dh, x, y, w, h, stride), (10, 10, 2, 3, 1, 2, 4));
    assert_eq!(format, PIXEL_FORMAT_RGBA8888);
    assert_eq!(PIXEL_FORMAT_RGBA8888, 2);
    assert_eq!(payload_len, 8);
    assert_eq!(&frame[FRAME_HEADER_BYTES..], &payload);
}

// —— 跨端 golden 向量 ————————————————————————————————————————
// 与 frontend/src/features/remote-desktop/lib/vncFrame.spec.ts 的 "cross-side golden vector"
// 断言硬编码同一 hex 字符串（互指）；字段序/字节序/stride 语义的权威
// 契约见 docs/architecture/PROTOCOL.zh-CN.md「VNC 帧补丁」小节。
//
// 向量定义：desktop 8x8，patch 位于 (2,1) 尺寸 4x4，sequence=42，
// stride=16（紧排 width*4），payload = 0x00..=0x3F（64 字节 RGBA）。
// 改协议字段序/字节序时两侧同步更新，任何一侧单独变化即测试失败。

const GOLDEN_FRAME_HEX: &str = "2a00000000000000080000000800000002000000010000000400000004000000100000000200000040000000000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f";

fn hex_decode(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0, "golden hex must be byte aligned");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("golden hex digit"))
        .collect()
}

#[test]
fn patch_frame_golden_vector_matches_frontend() {
    let payload: Vec<u8> = (0u8..=0x3f).collect();
    let frame = encode_frame_patch(&FramePatch {
        sequence: 42,
        desktop_width: 8,
        desktop_height: 8,
        x: 2,
        y: 1,
        width: 4,
        height: 4,
        stride: 16,
        payload: &payload,
    })
    .expect("golden frame should encode");
    assert_eq!(frame.len(), FRAME_HEADER_BYTES + 64);
    // 逐字节等于跨端 hex：字段序或字节序任一侧漂移立即失败。
    assert_eq!(frame, hex_decode(GOLDEN_FRAME_HEX));
    // 字段级复读，把契约钉在断言上（sequence u64 LE + 9 个 u32 LE）。
    let (sequence, dw, dh, x, y, w, h, stride, format, payload_len) = decode_patch_header(&frame);
    assert_eq!(sequence, 42);
    assert_eq!((dw, dh, x, y, w, h, stride), (8, 8, 2, 1, 4, 4, 16));
    assert_eq!(format, PIXEL_FORMAT_RGBA8888);
    assert_eq!(payload_len, 64);
    assert_eq!(&frame[FRAME_HEADER_BYTES..], &payload);
}

#[test]
fn patch_encoding_rejects_bad_geometry() {
    let payload = [0u8; 4];
    let base = |x: u32, y: u32, width: u32, height: u32, stride: u32| FramePatch {
        sequence: 1,
        desktop_width: 10,
        desktop_height: 10,
        x,
        y,
        width,
        height,
        stride,
        payload: &payload,
    };
    // 矩形出界
    let error = encode_frame_patch(&base(10, 0, 1, 1, 4)).unwrap_err();
    assert!(error.contains("exceeds framebuffer bounds"), "{error}");
    // 零尺寸
    assert!(encode_frame_patch(&base(0, 0, 0, 1, 4)).is_err());
    // stride 过小
    let error = encode_frame_patch(&base(0, 0, 2, 1, 4)).unwrap_err();
    assert!(error.contains("stride is too small"), "{error}");
    // payload 不足
    let error = encode_frame_patch(&base(0, 0, 1, 2, 4)).unwrap_err();
    assert!(error.contains("payload is too small"), "{error}");
    // 坐标回绕
    assert!(encode_frame_patch(&base(u32::MAX, 0, 1, 1, 4)).is_err());
    // 空桌面
    let error = encode_frame_patch(&FramePatch {
        sequence: 1,
        desktop_width: 0,
        desktop_height: 10,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        stride: 4,
        payload: &payload,
    })
    .unwrap_err();
    assert!(error.contains("non-zero"), "{error}");
}

// —— 有界保护 ————————————————————————————————————————————————

#[test]
fn framebuffer_is_bounded() {
    // 超宽/超高被拒（契约上限 3840x2160）。
    assert!(VncFramebuffer::new(3841, 1).is_err());
    assert!(VncFramebuffer::new(1, 2161).is_err());
    assert!(VncFramebuffer::new(0, 10).is_err());
    // 恰好在上限内可建（33MB 上界 < 64MB patch 线制上限）。
    assert!(VncFramebuffer::new(MAX_FRAMEBUFFER_WIDTH, MAX_FRAMEBUFFER_HEIGHT).is_ok());
}

#[test]
fn framebuffer_applies_patches_and_rebuilds_full_frames() {
    let mut framebuffer = VncFramebuffer::new(2, 2).expect("framebuffer");
    framebuffer
        .apply_rgba(
            Rect {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
            },
            &[1, 2, 3, 255],
        )
        .expect("patch");
    // 出界与长度不符都被拒。
    assert!(framebuffer
        .apply_rgba(
            Rect {
                x: 2,
                y: 0,
                width: 1,
                height: 1
            },
            &[0; 4]
        )
        .is_err());
    assert!(framebuffer
        .apply_rgba(
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            },
            &[0; 3]
        )
        .is_err());
    let frame = framebuffer.full_frame_bytes(9).expect("full frame");
    let (sequence, dw, dh, x, y, w, h, stride, _, payload_len) = decode_patch_header(&frame);
    assert_eq!(sequence, 9);
    assert_eq!(
        (dw, dh, x, y, w, h, stride, payload_len),
        (2, 2, 0, 0, 2, 2, 8, 16)
    );
    // 全帧 payload 即整幅 RGBA，补丁后的像素在 (1,1)。
    assert_eq!(
        &frame[FRAME_HEADER_BYTES + 12..FRAME_HEADER_BYTES + 16],
        &[1, 2, 3, 255]
    );
}

// —— 认证参数校验（classic VNC-Auth ≤8 字节）——————————————————

#[test]
fn password_validation_enforces_eight_bytes() {
    assert!(validate_password(None).is_ok());
    assert!(validate_password(Some("")).is_ok());
    assert!(validate_password(Some("12345678")).is_ok());
    let error = validate_password(Some("123456789")).unwrap_err();
    assert!(error.contains("at most 8 bytes"), "{error}");
    // 按字节计：4 个多字节字符 = 12 字节 > 8。
    assert!(validate_password(Some("密码密码")).is_err());
}

#[test]
fn vnc_auth_key_bit_reverses_the_first_eight_bytes() {
    // RFC 6143 7.2.2：每个字节位反转，空位补 0。独立参照 u8::reverse_bits。
    let key = vnc_auth_key("password");
    let expected: [u8; 8] = "password"
        .as_bytes()
        .iter()
        .map(|byte| byte.reverse_bits())
        .collect::<Vec<u8>>()
        .try_into()
        .unwrap();
    assert_eq!(key, expected);
    // 已知向量：'p' = 0x70 → 0x0E；短密码尾部补零；超长截断到 8。
    assert_eq!(key[0], 0x0e);
    assert_eq!(vnc_auth_key("a")[0], 0x86);
    assert_eq!(vnc_auth_key("a"), [0x86, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        vnc_auth_key("123456789")[..8],
        vnc_auth_key("12345678")[..8]
    );
}

// —— 输入/剪贴板/缩放参数 ————————————————————————————————————

#[test]
fn input_events_decode_from_the_documented_shapes() {
    let key: VncInputEvent =
        serde_json::from_value(json!({ "kind": "key", "keysym": 0xff1b, "pressed": true }))
            .expect("key");
    assert!(matches!(
        key,
        VncInputEvent::Key {
            keysym: 0xff1b,
            pressed: true
        }
    ));
    let pointer: VncInputEvent = serde_json::from_value(json!(
        { "kind": "pointer", "x": 10, "y": 20, "buttonMask": 1 }
    ))
    .expect("pointer");
    assert!(matches!(
        pointer,
        VncInputEvent::Pointer {
            x: 10,
            y: 20,
            button_mask: 1
        }
    ));
    let release: VncInputEvent =
        serde_json::from_value(json!({ "kind": "release-all" })).expect("release-all");
    assert!(matches!(release, VncInputEvent::ReleaseAll));
    // 未知 kind / 缺 keysym 必须拒绝（不静默降级）。
    assert!(serde_json::from_value::<VncInputEvent>(json!({ "kind": "drag" })).is_err());
    assert!(serde_json::from_value::<VncInputEvent>(json!({ "kind": "key" })).is_err());
    // 完整请求（camelCase sessionId + flatten event）。
    let request: VncInputRequest = serde_json::from_value(json!({
        "sessionId": "s1", "kind": "pointer", "x": 1, "y": 2, "buttonMask": 4
    }))
    .expect("request");
    assert_eq!(request.session_id, "s1");
}

#[test]
fn clipboard_guard_is_utf8_and_byte_bounded() {
    assert!(validate_clipboard_text("hello").is_ok());
    assert!(validate_clipboard_text("").is_ok());
    // vnc-rs exchanges UTF-8 cut text; Chinese host/remote text must pass.
    assert!(validate_clipboard_text("中文剪贴板").is_ok());
    let error = validate_clipboard_text(&"x".repeat(1024 * 1024 + 1)).unwrap_err();
    assert!(error.contains("1 MiB"), "{error}");
}

#[test]
fn scale_mode_accepts_wire_names_only() {
    assert_eq!(ScaleMode::parse("fit"), Some(ScaleMode::Fit));
    assert_eq!(ScaleMode::parse("stretch"), Some(ScaleMode::Stretch));
    assert_eq!(ScaleMode::parse("actual"), Some(ScaleMode::Actual));
    assert_eq!(ScaleMode::parse("auto"), None);
    assert_eq!(ScaleMode::default(), ScaleMode::Fit);
    assert_eq!(ScaleMode::Stretch.as_str(), "stretch");
}

#[test]
fn reconnect_delay_is_bounded() {
    assert_eq!(reconnect_delay(1), Duration::from_secs(1));
    assert_eq!(reconnect_delay(2), Duration::from_secs(2));
    assert_eq!(reconnect_delay(3), Duration::from_secs(4));
    assert_eq!(reconnect_delay(100), Duration::from_secs(4));
}

#[test]
fn classify_routes_transport_as_retryable_only() {
    let io = VncError::IoError(std::io::Error::other("reset"));
    assert!(classify_vnc_error(io).1);
    for protocol in [
        VncError::NoPassword,
        VncError::WrongPassword,
        VncError::ConnectError,
        VncError::InvalidSecurityTyep(19),
        VncError::InvalidImageData,
        VncError::General("denied".to_string()),
    ] {
        assert!(!classify_vnc_error(protocol).1);
    }
}

// —— 2026-10-31 protocol depth: CopyRect / Tight JPEG / CursorPseudo ——

fn rgba_rect(width: u16, height: u16, seed: u8) -> Vec<u8> {
    (0..usize::from(width) * usize::from(height) * 4)
        .map(|index| seed.wrapping_add(index as u8))
        .collect()
}

#[test]
fn copy_rect_composites_overlapping_regions() {
    let mut framebuffer = VncFramebuffer::new(4, 2).expect("framebuffer");
    let source = rgba_rect(2, 1, 10);
    framebuffer
        .apply_rgba(
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 1,
            },
            &source,
        )
        .expect("apply source");
    let copied = framebuffer
        .copy_rect(
            Rect {
                x: 2,
                y: 1,
                width: 2,
                height: 1,
            },
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 1,
            },
        )
        .expect("copy");
    assert_eq!(copied, source);
    // The destination pixels, not just the temporary copy result, must land in
    // the retained desktop used by reconnect repaints.
    let frame = framebuffer.full_frame_bytes(0).expect("full frame");
    let destination = FRAME_HEADER_BYTES + ((1 * 4) + 2) * 4;
    assert_eq!(
        &frame[destination..destination + source.len()],
        source.as_slice()
    );
    // Out-of-bounds and mismatched sizes are rejected.
    assert!(framebuffer
        .copy_rect(
            Rect {
                x: 3,
                y: 0,
                width: 2,
                height: 1
            },
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 1
            },
        )
        .is_err());
    assert!(framebuffer
        .copy_rect(
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            },
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 1
            },
        )
        .is_err());
}

#[test]
fn tight_jpeg_rect_decodes_to_rgba_and_checks_dimensions() {
    let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        2,
        2,
        image::Rgba([255, 0, 0, 255]),
    ));
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
        .encode_image(&image)
        .expect("encode jpeg");
    let pixels = decode_jpeg_rect(
        Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        },
        &jpeg,
    )
    .expect("decode jpeg");
    assert_eq!(pixels.len(), 2 * 2 * 4);
    assert!(pixels[0] > 200, "red channel should survive JPEG");
    assert_eq!(pixels[3], 255, "alpha is opaque");
    assert!(decode_jpeg_rect(
        Rect {
            x: 0,
            y: 0,
            width: 3,
            height: 2
        },
        &jpeg
    )
    .is_err());
}

#[test]
fn cursor_pseudo_payload_matches_the_rdp_pointer_shape() {
    let bitmap = cursor_event_payload(
        "s1",
        Rect {
            x: 2,
            y: 3,
            width: 1,
            height: 1,
        },
        &[10, 20, 30, 255],
    );
    assert_eq!(bitmap["type"], "bitmap");
    assert_eq!(bitmap["width"], 1);
    assert_eq!(bitmap["hotspotX"], 2);
    assert_eq!(bitmap["hotspotY"], 3);
    assert!(bitmap["rgbaBase64"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    let empty = cursor_event_payload(
        "s1",
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        },
        &[],
    );
    assert_eq!(empty["type"], "hidden");
    let oversized = cursor_event_payload(
        "s1",
        Rect {
            x: 0,
            y: 0,
            width: 300,
            height: 1,
        },
        &vec![0; 300 * 4],
    );
    assert_eq!(oversized["type"], "default");
    let malformed = cursor_event_payload(
        "s1",
        Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        },
        &[1, 2, 3],
    );
    assert_eq!(malformed["type"], "default");
}

// —— 会话内动态分辨率（ExtendedDesktopSize）——————————————

fn test_session_entry() -> VncSessionEntry {
    VncSessionEntry {
        connection_id: None,
        workbench_id: "w-resize".to_string(),
        host: "10.255.255.1".to_string(),
        port: 5900,
        runtime_host: "10.255.255.1".to_string(),
        runtime_port: 5900,
        password: None,
        scale_mode: ScaleMode::Fit,
        reconnect_attempts: DEFAULT_RECONNECT_ATTEMPTS,
        created_at_secs: 0,
        generation: AtomicU64::new(7),
        frame_sequence: AtomicU64::new(0),
        close_requested: AtomicBool::new(false),
        command_sender: tokio::sync::Mutex::new(None),
        framebuffer: tokio::sync::Mutex::new(None),
    }
}

#[test]
fn resize_size_gate_matches_the_framebuffer_contract() {
    assert!(validate_resize_size(200, 200).is_ok());
    assert!(validate_resize_size(2560, 1600).is_ok());
    assert!(validate_resize_size(MAX_FRAMEBUFFER_WIDTH, MAX_FRAMEBUFFER_HEIGHT).is_ok());
    assert!(validate_resize_size(199, 480).is_err());
    assert!(validate_resize_size(640, 199).is_err());
    assert!(validate_resize_size(MAX_FRAMEBUFFER_WIDTH + 1, 480).is_err());
    assert!(validate_resize_size(640, MAX_FRAMEBUFFER_HEIGHT + 1).is_err());
}

#[test]
fn desktop_layout_replaces_only_on_a_size_change() {
    let mut framebuffer = Some(VncFramebuffer::new(4, 3).expect("framebuffer"));
    assert!(apply_desktop_layout(&mut framebuffer, 8, 6).expect("layout"));
    assert!(framebuffer.as_ref().expect("desktop").matches_size(8, 6));
    // Same geometry keeps the retained pixels (no rebuild).
    assert!(!apply_desktop_layout(&mut framebuffer, 8, 6).expect("layout"));
    // Out-of-contract layouts fail the session rather than keeping bad state.
    assert!(apply_desktop_layout(&mut framebuffer, 4000, 100).is_err());
}

#[tokio::test]
async fn resize_queues_set_desktop_size_without_restarting_the_generation() {
    let runtime = VncSessionRuntime::new();
    let session_id = "vnc-live-resize".to_string();
    let entry = Arc::new(test_session_entry());
    entry.generation.store(4, Ordering::Release);
    let (sender, mut receiver) = mpsc::channel(4);
    *entry.command_sender.lock().await = Some(sender);
    runtime
        .sessions
        .write()
        .await
        .insert(session_id.clone(), entry.clone());

    let response = runtime
        .resize(&session_id, 1600, 900)
        .await
        .expect("resize");
    assert_eq!(response["live"], true);
    match receiver.recv().await.expect("resize command") {
        VncCommand::Resize { width, height } => assert_eq!((width, height), (1600, 900)),
        _ => panic!("expected a live resize command"),
    }
    // No generation restart: the same engine applies SetDesktopSize.
    assert_eq!(entry.generation.load(Ordering::Acquire), 4);
}

#[tokio::test]
async fn resize_without_an_engine_reports_not_live() {
    let runtime = VncSessionRuntime::new();
    let session_id = "vnc-idle-resize".to_string();
    let entry = Arc::new(test_session_entry());
    runtime
        .sessions
        .write()
        .await
        .insert(session_id.clone(), entry);

    let response = runtime
        .resize(&session_id, 1440, 900)
        .await
        .expect("deferred resize");
    assert_eq!(response["live"], false);
}
