use super::*;

// —— 校验算法金标 ————————————————————————————————————————

#[test]
fn crc_vectors_match_known_values() {
    assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
    assert_eq!(crc32_iso_hdlc(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc16_xmodem(b""), 0);
}

// —— 测试工具 ——————————————————————————————————————————————

/// hex 帧（带 `\r\n` 收尾），模拟 lrzsz 接收器发来的 ZRINIT 等。
fn peer_hex_header(frame: u8, flags: [u8; 4]) -> Vec<u8> {
    hex_header(frame, flags)
}

/// bin32 帧，模拟 ZRPOS/ZACK/ZSKIP。
fn peer_bin32_header(frame: u8, count: u32) -> Vec<u8> {
    bin32_header(frame, count.to_le_bytes())
}

fn feed(engine: &mut UploadEngine, bytes: &[u8]) -> Vec<Output> {
    engine.feed(bytes)
}

fn writes(outputs: &[Output]) -> Vec<&Vec<u8>> {
    outputs
        .iter()
        .filter_map(|output| match output {
            Output::Write(data) => Some(data),
            Output::Progress(_) => None,
        })
        .collect()
}

fn last_progress(outputs: &[Output]) -> &TransferProgress {
    outputs
        .iter()
        .rev()
        .find_map(|output| match output {
            Output::Progress(progress) => Some(progress),
            Output::Write(_) => None,
        })
        .expect("progress output expected")
}

fn has_state(outputs: &[Output], state: ProgressState) -> bool {
    outputs
        .iter()
        .any(|output| matches!(output, Output::Progress(p) if p.state == state))
}

fn contains_pair(frame: &[u8], first: u8, second: u8) -> bool {
    frame.windows(2).any(|w| w == [first, second])
}

/// 建好引擎并灌满数据（final 分块），模拟文件已完整送入 sidecar。
fn engine_with_data(protocol: UploadProtocol, data: &[u8]) -> UploadEngine {
    let (mut engine, initial) = UploadEngine::new(
        protocol,
        "firmware.bin".to_string(),
        data.len() as u64,
        Instant::now(),
    );
    match protocol {
        UploadProtocol::Zmodem => assert_eq!(
            writes(&initial)[0][..4],
            [ZPAD, ZPAD, ZDLE, b'B'],
            "ZMODEM starts with a hex ZRQINIT"
        ),
        _ => assert!(initial.is_empty(), "X/Y engines start silent"),
    }
    let outputs = engine.append_data(data, true).unwrap();
    assert!(
        outputs.is_empty(),
        "no unlock outputs expected: {outputs:?}"
    );
    engine
}

// —— 上传数据源 ————————————————————————————————————————————

#[test]
fn upload_source_enforces_total_cap() {
    let mut source = UploadSource::new("f".to_string(), 10);
    source.append(&[0u8; 16]).unwrap();
    let error = source
        .append(&[0u8; MAX_UPLOAD_BYTES as usize])
        .unwrap_err();
    assert!(error.contains("limit"), "{error}");
}

#[test]
fn upload_source_read_gates_inflight_tail() {
    let mut source = UploadSource::new("f".to_string(), 10);
    source.append(&[0u8; 4]).unwrap();
    assert_eq!(source.read_upto(0, 10).unwrap(), ReadOutcome::Pending);
    source.finish();
    match source.read_upto(0, 10).unwrap() {
        ReadOutcome::Ready(bytes) => assert_eq!(bytes.len(), 4),
        other => panic!("expected ready, got {other:?}"),
    }
    assert_eq!(source.read_upto(4, 10).unwrap(), ReadOutcome::Eof);
}

// —— XMODEM ————————————————————————————————————————————————

#[test]
fn xmodem_crc_handshake_sends_block_one_and_pads_tail() {
    let mut engine = engine_with_data(UploadProtocol::Xmodem, b"hello");
    let outputs = feed(&mut engine, &[CRC_REQUEST]);
    let packet = writes(&outputs)[0].clone();
    assert_eq!(&packet[..3], &[SOH, 1, !1u8]);
    assert_eq!(
        packet.len(),
        BLOCK_SIZE + 5,
        "128B payload + SOH/seq/!seq + CRC16"
    );
    assert_eq!(
        &packet[3 + BLOCK_SIZE..],
        &crc16_xmodem(&packet[3..3 + BLOCK_SIZE]).to_be_bytes()
    );
    assert_eq!(&packet[3..8], b"hello");
    assert!(packet[8..3 + BLOCK_SIZE].iter().all(|&b| b == CPM_EOF));
    assert!(!engine.is_done());
}

#[test]
fn xmodem_nak_handshake_uses_checksum_trailer() {
    let mut engine = engine_with_data(UploadProtocol::Xmodem, b"hi");
    let outputs = feed(&mut engine, &[NAK]);
    let packet = writes(&outputs)[0].clone();
    assert_eq!(packet.len(), BLOCK_SIZE + 4, "checksum trailer is 1 byte");
    let sum = packet[3..3 + BLOCK_SIZE]
        .iter()
        .fold(0u8, |acc, b| acc.wrapping_add(*b));
    assert_eq!(packet[3 + BLOCK_SIZE], sum);
}

#[test]
fn xmodem_nak_retransmits_same_block_then_ack_advances() {
    let data = [0x5au8; BLOCK_SIZE * 2 + 10];
    let mut engine = engine_with_data(UploadProtocol::Xmodem, &data);
    let first = writes(&feed(&mut engine, &[CRC_REQUEST]))[0].clone();

    let retry = feed(&mut engine, &[NAK]);
    assert_eq!(writes(&retry)[0], &first, "NAK resends the same block");

    let second = feed(&mut engine, &[ACK]);
    let packet = writes(&second)[0].clone();
    assert_eq!(packet[1], 2, "ACK advances to the next block");

    let retry2 = feed(&mut engine, &[NAK]);
    assert_eq!(writes(&retry2)[0], &packet);
}

#[test]
fn xmodem_repeated_ack_confirms_only_one_block() {
    let data = [0x33u8; BLOCK_SIZE * 3];
    let mut engine = engine_with_data(UploadProtocol::Xmodem, &data);
    let _ = feed(&mut engine, &[CRC_REQUEST]);
    let outputs = feed(&mut engine, &[ACK, ACK]);
    let queued = writes(&outputs);
    assert_eq!(queued.len(), 1, "duplicate ACK must not queue two blocks");
    assert_eq!(queued[0][1], 2);
    // 进度按确认字节数报（块 1 的 128 字节），而非发送偏移。
    assert_eq!(last_progress(&outputs).sent, BLOCK_SIZE as u64);
}

#[test]
fn xmodem_eof_and_final_ack_completes() {
    let mut engine = engine_with_data(UploadProtocol::Xmodem, b"data");
    let _ = feed(&mut engine, &[CRC_REQUEST]);
    // 唯一数据块 ACK 后即 EOT；先被 NAK（lrzsz 风格）再 ACK 收尾。
    let eot = feed(&mut engine, &[ACK]);
    assert_eq!(writes(&eot)[0], &[EOT]);
    let retry = feed(&mut engine, &[NAK]);
    assert_eq!(writes(&retry)[0], &[EOT]);
    let done = feed(&mut engine, &[ACK]);
    assert!(has_state(&done, ProgressState::Complete));
    assert_eq!(last_progress(&done).sent, 4);
    assert!(engine.is_done());
    // 完成后的输入被忽略且不再输出。
    assert!(feed(&mut engine, &[ACK]).is_empty());
}

#[test]
fn xmodem_empty_file_sends_eot_directly() {
    let mut engine = engine_with_data(UploadProtocol::Xmodem, b"");
    let outputs = feed(&mut engine, &[CRC_REQUEST]);
    assert_eq!(writes(&outputs)[0], &[EOT]);
}

#[test]
fn xmodem_remote_cancel_fails_transfer() {
    let mut engine = engine_with_data(UploadProtocol::Xmodem, b"data");
    let outputs = feed(&mut engine, &[CAN, CAN]);
    assert!(has_state(&outputs, ProgressState::Failed));
    assert_eq!(writes(&outputs)[0], &vec![CAN; 8]);
    assert!(engine.is_done());
}

#[test]
fn xmodem_streaming_data_gates_first_block() {
    let (mut engine, initial) = UploadEngine::new(
        UploadProtocol::Xmodem,
        "fw.bin".to_string(),
        300,
        Instant::now(),
    );
    assert!(initial.is_empty());
    // 数据未到齐时握手信号被消费但不发包。
    assert!(feed(&mut engine, &[CRC_REQUEST]).is_empty());
    let _ = engine.append_data(&[0u8; 64], false).unwrap();
    assert!(
        feed(&mut engine, &[CRC_REQUEST]).is_empty(),
        "partial chunk must not be treated as tail"
    );
    // 补齐一块后从 append_data 直接解锁发包。
    let unlocked = engine.append_data(&[0u8; 64], false).unwrap();
    assert_eq!(writes(&unlocked).len(), 1);
    assert_eq!(writes(&unlocked)[0][1], 1);
}

#[test]
fn xmodem_idle_timeout_retries_then_fails() {
    let start = Instant::now();
    let (mut engine, _) = UploadEngine::new(
        UploadProtocol::Xmodem,
        "fw.bin".to_string(),
        3,
        start - Duration::from_secs(3600),
    );
    let _ = engine.append_data(b"abc", true).unwrap();
    // 先握手发出块 1，再回到过去制造静默窗口。
    let _ = feed(&mut engine, &[CRC_REQUEST]);
    let UploadEngine::Xy(inner) = &mut engine else {
        unreachable!()
    };
    let mut expected_data = [CPM_EOF; BLOCK_SIZE];
    expected_data[..3].copy_from_slice(b"abc");
    inner.last_activity = start - IDLE_RETRY - Duration::from_millis(1);
    let retry = inner.tick_at(start);
    assert_eq!(
        writes(&retry)[0],
        &build_xy_packet(1, &expected_data, CheckMode::Crc)
    );
    assert!(!inner.is_done());
    // 已重发 1 次；再重发 MAX_RETRIES-1 次后下一次 tick 判失败。
    for _ in 0..MAX_RETRIES - 1 {
        inner.last_activity = start - IDLE_RETRY - Duration::from_millis(1);
        let _ = inner.tick_at(start);
    }
    inner.last_activity = start - IDLE_RETRY - Duration::from_millis(1);
    let failed = inner.tick_at(start);
    assert!(has_state(&failed, ProgressState::Failed));
    assert!(inner.is_done());
}

#[test]
fn xmodem_handshake_idle_fails_when_no_receiver() {
    let start = Instant::now();
    let (mut engine, _) = UploadEngine::new(
        UploadProtocol::Xmodem,
        "fw.bin".to_string(),
        0,
        start - Duration::from_secs(3600),
    );
    let UploadEngine::Xy(inner) = &mut engine else {
        unreachable!()
    };
    let failed = inner.tick_at(start);
    assert!(has_state(&failed, ProgressState::Failed));
    assert!(inner.is_done());
}

// —— YMODEM ————————————————————————————————————————————————

#[test]
fn ymodem_header_block_carries_name_and_size_zero_padded() {
    let mut engine = engine_with_data(UploadProtocol::Ymodem, b"abc");
    let outputs = feed(&mut engine, &[CRC_REQUEST]);
    let header = writes(&outputs)[0].clone();
    assert_eq!(header[0], SOH);
    assert_eq!(header[1], 0, "header block uses sequence 0");
    assert_eq!(&header[3..15], b"firmware.bin");
    assert_eq!(header[15], 0, "name is NUL-terminated");
    assert_eq!(&header[16..17], b"3");
    assert!(header[17..3 + BLOCK_SIZE].iter().all(|&b| b == 0));
    assert_eq!(
        &header[3 + BLOCK_SIZE..],
        &crc16_xmodem(&header[3..3 + BLOCK_SIZE]).to_be_bytes()
    );
}

#[test]
fn ymodem_full_batch_flow_with_final_empty_header() {
    let mut engine = engine_with_data(UploadProtocol::Ymodem, b"abc");
    let _ = feed(&mut engine, &[CRC_REQUEST]);
    // 头块 ACK 后等待 'C' 才发数据块。
    assert!(feed(&mut engine, &[ACK]).is_empty());
    let data = writes(&feed(&mut engine, &[CRC_REQUEST]))[0].clone();
    assert_eq!(data[1], 1);
    assert_eq!(&data[3..6], b"abc");
    assert!(data[6..3 + BLOCK_SIZE].iter().all(|&b| b == CPM_EOF));

    // 数据 ACK → EOT；NAK 重发，ACK 后文件完成。
    let eot = writes(&feed(&mut engine, &[ACK]))[0].clone();
    assert_eq!(eot, &[EOT]);
    assert_eq!(writes(&feed(&mut engine, &[NAK]))[0], &[EOT]);
    let after_eot = feed(&mut engine, &[ACK]);
    assert!(has_state(&after_eot, ProgressState::FileComplete));
    // 收尾空头块也要等 'C'。
    assert!(feed(&mut engine, &[ACK]).is_empty());
    let closing = writes(&feed(&mut engine, &[CRC_REQUEST]))[0].clone();
    assert_eq!(closing[1], 0);
    assert!(closing[3..3 + BLOCK_SIZE].iter().all(|&b| b == 0));
    let done = feed(&mut engine, &[ACK]);
    assert!(has_state(&done, ProgressState::Complete));
    assert_eq!(last_progress(&done).sent, 3);
    assert!(engine.is_done());
}

#[test]
fn ymodem_nak_on_header_retransmits() {
    let mut engine = engine_with_data(UploadProtocol::Ymodem, b"abc");
    let header = writes(&feed(&mut engine, &[CRC_REQUEST]))[0].clone();
    let retry = feed(&mut engine, &[NAK]);
    assert_eq!(writes(&retry)[0], &header);
}

#[test]
fn ymodem_handshake_only_accepts_c() {
    let mut engine = engine_with_data(UploadProtocol::Ymodem, b"abc");
    assert!(feed(&mut engine, &[NAK]).is_empty());
    assert!(feed(&mut engine, &[ACK]).is_empty());
    assert!(!writes(&feed(&mut engine, &[CRC_REQUEST])).is_empty());
}

#[test]
fn ymodem_remote_cancel_fails() {
    let mut engine = engine_with_data(UploadProtocol::Ymodem, b"abc");
    let outputs = feed(&mut engine, &[CAN, CAN]);
    assert!(has_state(&outputs, ProgressState::Failed));
    assert!(engine.is_done());
}

#[test]
fn ymodem_user_cancel_sends_can_and_marks_failed() {
    let mut engine = engine_with_data(UploadProtocol::Ymodem, b"abc");
    let outputs = engine.cancel();
    assert_eq!(writes(&outputs)[0], &vec![CAN; 8]);
    assert!(has_state(&outputs, ProgressState::Failed));
    assert!(engine.is_done());
    assert!(engine.cancel().is_empty(), "cancel is idempotent once done");
}

// —— ZMODEM ————————————————————————————————————————————————

#[test]
fn zmodem_happy_path_from_zrinit_to_oo() {
    // 数据刻意包含 0x18/0x7f/0xff 等需转义字节。
    let data: Vec<u8> = (0..2500u32).map(|i| (i % 251) as u8).collect();
    let mut engine = engine_with_data(UploadProtocol::Zmodem, &data);

    // ZRINIT（缓冲未声明 → 8 KiB 子包）→ ZFILE。
    let outputs = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0, 0, 0, 0x23]));
    let zfile = writes(&outputs)[0].clone();
    assert_eq!(&zfile[..3], &[ZPAD, ZDLE, b'C'], "ZFILE is bin32");
    assert_eq!(zfile[3], FRAME_ZFILE);
    assert!(contains_pair(&zfile, ZDLE, SUBPACKET_ZCRCW));

    // ZRPOS(0) → ZDATA + 数据子包（≤8KiB，这里整文件一段）。
    let outputs = feed(&mut engine, &peer_bin32_header(FRAME_ZRPOS, 0));
    let zdata = writes(&outputs)[0].clone();
    assert_eq!(zdata[3], FRAME_ZDATA);
    assert_eq!(&zdata[4..8], &0u32.to_le_bytes());
    assert!(contains_pair(&zdata, ZDLE, SUBPACKET_ZCRCW));

    // ZACK(2048) → 从 2048 续传。
    let outputs = feed(&mut engine, &peer_bin32_header(FRAME_ZACK, 2048));
    let zdata2 = writes(&outputs)[0].clone();
    assert_eq!(&zdata2[4..8], &2048u32.to_le_bytes());

    // ZACK(2500 ≥ size) → ZEOF（文件完成确认在下一个 ZRINIT 上报）。
    let outputs = feed(&mut engine, &peer_bin32_header(FRAME_ZACK, 2500));
    let zeof = writes(&outputs)[0].clone();
    assert_eq!(zeof[3], FRAME_ZEOF);
    assert_eq!(&zeof[4..8], &2500u32.to_le_bytes());

    // ZRINIT 确认 EOF → FileComplete 事件 + ZFIN(hex)。
    let outputs = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0; 4]));
    assert!(has_state(&outputs, ProgressState::FileComplete));
    let zfin = writes(&outputs)[0].clone();
    assert_eq!(&zfin[..4], &[ZPAD, ZPAD, ZDLE, b'B']);
    assert!(!zfin.ends_with(&[XON]), "ZFIN hex header has no XON tail");

    // 对端 ZFIN → "OO" + Complete。
    let outputs = feed(&mut engine, &peer_hex_header(FRAME_ZFIN, [0; 4]));
    assert_eq!(writes(&outputs)[0], b"OO");
    assert!(has_state(&outputs, ProgressState::Complete));
    assert!(engine.is_done());
    assert!(feed(&mut engine, &peer_hex_header(FRAME_ZFIN, [0; 4])).is_empty());
    let _ = zdata2;
}

#[test]
fn zmodem_zfile_metadata_carries_name_size_and_mode() {
    let mut engine = engine_with_data(UploadProtocol::Zmodem, b"abc");
    let outputs = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0; 4]));
    let zfile = writes(&outputs)[0].clone();
    let text = String::from_utf8_lossy(&zfile);
    assert!(
        text.contains("firmware.bin\0"),
        "name in ZFILE payload: {text:?}"
    );
    assert!(
        text.contains("3 0 100644 0 0 0\0"),
        "size/mtime/mode fields: {text:?}"
    );
}

#[test]
fn zmodem_zrinit_buffer_size_caps_subpacket() {
    let data = [0x11u8; 4096];
    let mut engine = engine_with_data(UploadProtocol::Zmodem, &data);
    // 声明 0x0040 = 64 字节接收缓冲。
    let _ = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0x40, 0, 0, 0]));
    let outputs = feed(&mut engine, &peer_bin32_header(FRAME_ZRPOS, 0));
    let zdata = writes(&outputs)[0].clone();
    assert_eq!(zdata_payload_len(&zdata), 64);
}

/// 从 ZDATA 帧取数据子包负载长度（转义对按 1 字节计）。
fn zdata_payload_len(frame: &[u8]) -> usize {
    let mut len = 0;
    let mut i = 3; // 跳过 ZPAD ZDLE 'C'
    let mut in_payload = false;
    while i < frame.len() {
        if in_payload {
            if frame[i] == ZDLE {
                if frame.get(i + 1) == Some(&SUBPACKET_ZCRCW) {
                    break;
                }
                i += 2;
            } else {
                i += 1;
            }
            len += 1;
            continue;
        }
        // bin32 头负载 5 字节 + CRC32 4 字节（含可能的转义对）。
        if frame[i] == ZDLE {
            i += 2;
        } else {
            i += 1;
        }
        len += 1;
        if len == 9 {
            in_payload = true;
            len = 0;
        }
    }
    len
}

#[test]
fn zmodem_zskip_and_zcan_fail_transfer() {
    let mut engine = engine_with_data(UploadProtocol::Zmodem, b"abc");
    let _ = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0; 4]));
    let outputs = feed(&mut engine, &peer_bin32_header(FRAME_ZSKIP, 0));
    assert!(has_state(&outputs, ProgressState::Failed));
    assert!(engine.is_done());

    let mut engine = engine_with_data(UploadProtocol::Zmodem, b"abc");
    let outputs = feed(&mut engine, &peer_hex_header(FRAME_ZCAN, [0; 4]));
    assert!(has_state(&outputs, ProgressState::Failed));
    assert!(engine.is_done());
}

#[test]
fn zmodem_remote_cancel_via_five_can_bytes() {
    let mut engine = engine_with_data(UploadProtocol::Zmodem, b"abc");
    let outputs = feed(&mut engine, &[ZDLE; 5]);
    assert!(has_state(&outputs, ProgressState::Failed));
    assert!(engine.is_done());
    assert!(
        writes(&outputs).is_empty(),
        "remote cancel does not echo a cancel sequence"
    );
}

#[test]
fn zmodem_corrupt_header_sends_znak_and_recovers() {
    let mut engine = engine_with_data(UploadProtocol::Zmodem, b"abc");
    let garbage = vec![ZPAD, ZDLE, b'Z'];
    let outputs = feed(&mut engine, &garbage);
    let znak = writes(&outputs)[0].clone();
    assert_eq!(&znak[..4], &[ZPAD, ZPAD, ZDLE, b'B']);
    assert_eq!(znak[4], b'0', "hex ZNAK frame byte starts with '0'");
    assert_eq!(znak[5], b'6', "hex ZNAK frame byte ends with '6'");
    // 之后正常 ZRINIT 仍然被接受。
    let outputs = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0; 4]));
    assert_eq!(writes(&outputs)[0][3], FRAME_ZFILE);
}

#[test]
fn zmodem_crc_mismatch_is_reported_as_znak() {
    let mut engine = engine_with_data(UploadProtocol::Zmodem, b"abc");
    let mut frame = peer_bin32_header(FRAME_ZACK, 0);
    let last = frame.len() - 1;
    frame[last] ^= 0xff;
    let outputs = feed(&mut engine, &frame);
    assert!(
        has_state(&outputs, ProgressState::Failed) || !writes(&outputs).is_empty(),
        "corrupt frame must produce output"
    );
    assert!(
        writes(&outputs)
            .iter()
            .any(|f| f[..4] == [ZPAD, ZPAD, ZDLE, b'B'] && f[4] == b'0' && f[5] == b'6'),
        "expected a hex ZNAK"
    );
    // 帧流仍然可以恢复。
    let outputs = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0; 4]));
    assert_eq!(writes(&outputs)[0][3], FRAME_ZFILE);
}

#[test]
fn zmodem_garbage_stream_fails_after_the_corrupt_frame_budget() {
    // D2 回归：恶意对端的持续垃圾流曾让每个坏帧都回 ZNAK 并刷新
    // last_activity（永不超时 + 写放大）；连续坏帧达到预算即判失败。
    let mut engine = engine_with_data(UploadProtocol::Zmodem, b"abc");
    let garbage = vec![ZPAD, ZDLE, b'Z'];
    let mut frames = 0usize;
    while !engine.is_done() && frames < MAX_CORRUPT_FRAMES * 3 {
        let outputs = feed(&mut engine, &garbage);
        frames += 1;
        if frames < MAX_CORRUPT_FRAMES {
            assert!(
                !has_state(&outputs, ProgressState::Failed),
                "budget not exhausted yet at frame {frames}"
            );
        }
    }
    assert!(engine.is_done(), "garbage stream must fail at the budget");
    assert!(frames < MAX_CORRUPT_FRAMES * 3, "must not loop forever");
}

#[test]
fn zmodem_valid_header_resets_the_corrupt_frame_budget() {
    // 预算是「两个有效帧头之间」的：线路抖动下的零星坏帧 + 恢复后
    // 重新计数，不得累积成永久失败。
    let mut engine = engine_with_data(UploadProtocol::Zmodem, b"abc");
    let garbage = vec![ZPAD, ZDLE, b'Z'];
    for _ in 0..10 {
        let _ = feed(&mut engine, &garbage);
    }
    let _ = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0; 4]));
    for _ in 0..10 {
        let outputs = feed(&mut engine, &garbage);
        assert!(
            !has_state(&outputs, ProgressState::Failed),
            "a valid header must reset the corrupt-frame budget"
        );
    }
    assert!(!engine.is_done());
}

#[test]
fn xymodem_zero_byte_file_completes_after_the_final_chunk() {
    // D1 回归：0 字节文件以前端循环不发分块 → 源永不收尾 → 引擎停在
    // 握手挂起态且时钟被抑制（永不超时）。final 空分块收尾后，握手
    // 消费即直接走 EOT 收束。
    let mut engine = engine_with_data(UploadProtocol::Xmodem, b"");
    let outputs = feed(&mut engine, &[NAK]);
    assert_eq!(writes(&outputs).len(), 1);
    assert_eq!(
        writes(&outputs)[0][0],
        EOT,
        "empty source answers the handshake with EOT"
    );
    let outputs = feed(&mut engine, &[ACK]);
    assert!(has_state(&outputs, ProgressState::Complete));
    assert!(engine.is_done());
}

#[test]
fn ymodem_zero_byte_file_walks_header_eot_and_closing_block() {
    // D1 的 YMODEM 面：空文件同样要走出 头块(0) → EOT → 收尾全零块 的
    // 完整批次，不得挂在数据挂起态。
    let mut engine = engine_with_data(UploadProtocol::Ymodem, b"");
    // 'C' → 头块（name + size 0）。
    let outputs = feed(&mut engine, &[CRC_REQUEST]);
    assert_eq!(
        writes(&outputs).len(),
        1,
        "header block 0 for the empty file"
    );
    // ACK + 'C' → 数据读直接 EOF → EOT。
    let _ = feed(&mut engine, &[ACK]);
    let outputs = feed(&mut engine, &[CRC_REQUEST]);
    assert_eq!(writes(&outputs).len(), 1);
    assert_eq!(writes(&outputs)[0][0], EOT);
    // EOT ACK → 本文件完成（FileComplete 在 ACK 上回报）；'C' → 收尾
    // 全零头块；ACK → 批次结束。
    let outputs = feed(&mut engine, &[ACK]);
    assert!(has_state(&outputs, ProgressState::FileComplete));
    let outputs = feed(&mut engine, &[CRC_REQUEST]);
    assert_eq!(writes(&outputs).len(), 1, "closing all-zero header block");
    let outputs = feed(&mut engine, &[ACK]);
    assert!(has_state(&outputs, ProgressState::Complete));
    assert!(engine.is_done());
}

#[test]
fn zmodem_streaming_source_gates_zdata_until_chunks_arrive() {
    let (mut engine, initial) = UploadEngine::new(
        UploadProtocol::Zmodem,
        "fw.bin".to_string(),
        1000,
        Instant::now(),
    );
    assert_eq!(writes(&initial)[0][..4], [ZPAD, ZPAD, ZDLE, b'B']);
    let _ = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0; 4]));
    let outputs = feed(&mut engine, &peer_bin32_header(FRAME_ZRPOS, 0));
    assert!(
        writes(&outputs).is_empty(),
        "no data frame without source bytes"
    );
    // 半截分块且未收尾：继续等待。
    let _ = engine.append_data(&[0u8; 500], false).unwrap();
    assert!(feed(&mut engine, &[]).is_empty());
    // final 分块补齐 → ZDATA 立即从 append_data 解锁出现。
    let unlocked = engine.append_data(&[0u8; 500], true).unwrap();
    assert_eq!(writes(&unlocked).len(), 1);
    assert_eq!(writes(&unlocked)[0][3], FRAME_ZDATA);
}

#[test]
fn zmodem_resume_from_zrpos_offset_retransmits_from_there() {
    let data: Vec<u8> = (0..600u32).map(|i| i as u8).collect();
    let mut engine = engine_with_data(UploadProtocol::Zmodem, &data);
    let _ = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0; 4]));
    let _ = feed(&mut engine, &peer_bin32_header(FRAME_ZRPOS, 0));
    // 对端落盘到 512 后掉线重连，ZRPOS(512) 要求续传。
    let outputs = feed(&mut engine, &peer_bin32_header(FRAME_ZRPOS, 512));
    let zdata = writes(&outputs)[0].clone();
    assert_eq!(&zdata[4..8], &512u32.to_le_bytes());
    assert_eq!(zdata_payload_len(&zdata), 88);
}

#[test]
fn zmodem_user_cancel_sends_cancel_sequence() {
    let mut engine = engine_with_data(UploadProtocol::Zmodem, b"abc");
    let outputs = engine.cancel();
    assert_eq!(writes(&outputs)[0], &zmodem_cancel_sequence());
    assert_eq!(
        zmodem_cancel_sequence(),
        vec![ZDLE, ZDLE, ZDLE, ZDLE, ZDLE, 0x08, 0x08, 0x08, 0x08, 0x08]
    );
    assert!(has_state(&outputs, ProgressState::Failed));
    assert!(engine.is_done());
    assert!(engine.cancel().is_empty());
}

#[test]
fn zmodem_idle_timeout_resends_zrqinit_then_fails() {
    let start = Instant::now();
    let (engine, initial) =
        UploadEngine::new(UploadProtocol::Zmodem, "fw.bin".to_string(), 0, start);
    assert!(!initial.is_empty());
    let mut inner = match engine {
        UploadEngine::Z(inner) => inner,
        UploadEngine::Xy(_) => unreachable!(),
    };
    inner.last_activity = start - IDLE_RETRY - Duration::from_millis(1);
    let retry = inner.tick_at(start);
    assert_eq!(writes(&retry)[0], &hex_header(FRAME_ZRQINIT, [0; 4]));
    // 已重发 1 次；再重发 MAX_RETRIES-1 次后下一次 tick 判失败。
    for _ in 0..MAX_RETRIES - 1 {
        inner.last_activity = start - IDLE_RETRY - Duration::from_millis(1);
        let _ = inner.tick_at(start);
    }
    inner.last_activity = start - IDLE_RETRY - Duration::from_millis(1);
    let failed = inner.tick_at(start);
    assert!(has_state(&failed, ProgressState::Failed));
    assert!(inner.is_done());
}

#[test]
fn zmodem_progress_tracks_acked_bytes() {
    let data = [0x42u8; 300];
    let mut engine = engine_with_data(UploadProtocol::Zmodem, &data);
    let _ = feed(&mut engine, &peer_hex_header(FRAME_ZRINIT, [0; 4]));
    let outputs = feed(&mut engine, &peer_bin32_header(FRAME_ZRPOS, 0));
    assert_eq!(last_progress(&outputs).sent, 0, "nothing acked yet");
    let outputs = feed(&mut engine, &peer_bin32_header(FRAME_ZACK, 128));
    assert_eq!(last_progress(&outputs).sent, 128);
}

// —— 协议拼写与工具 ————————————————————————————————————————

#[test]
fn upload_protocol_parse_rejects_typo_and_junk() {
    assert_eq!(
        UploadProtocol::parse("xmodem").unwrap(),
        UploadProtocol::Xmodem
    );
    assert_eq!(
        UploadProtocol::parse("ymodem").unwrap(),
        UploadProtocol::Ymodem
    );
    assert_eq!(
        UploadProtocol::parse("zmodem").unwrap(),
        UploadProtocol::Zmodem
    );
    for junk in ["zodem", "XMODEM", "modem", ""] {
        assert!(
            UploadProtocol::parse(junk).is_err(),
            "{junk:?} must be rejected"
        );
    }
}

#[test]
fn zdle_escape_roundtrip_covers_special_bytes() {
    let raw: Vec<u8> = (0..=255u8).collect();
    let mut escaped = Vec::new();
    push_escaped(&mut escaped, &raw);
    let mut restored = Vec::new();
    let mut i = 0;
    while i < escaped.len() {
        if escaped[i] == ZDLE {
            restored.push(unescape_zdle(escaped[i + 1]));
            i += 2;
        } else {
            restored.push(escaped[i]);
            i += 1;
        }
    }
    assert_eq!(restored, raw);
}

#[test]
fn header_scanner_roundtrips_own_frames() {
    // 三种编码的帧都能被扫描器解回（逐字节喂入，含 hex 收尾 \r\n/XON）。
    let cases = [
        (
            hex_header(FRAME_ZRINIT, [0x40, 0, 0, 0x23]),
            FRAME_ZRINIT,
            0x2300_0040u32,
        ),
        (peer_bin32_header(FRAME_ZRPOS, 1234), FRAME_ZRPOS, 1234),
        (
            peer_bin32_header(FRAME_ZACK, 0xdead_beef),
            FRAME_ZACK,
            0xdead_beef,
        ),
        (hex_header(FRAME_ZFIN, [0; 4]), FRAME_ZFIN, 0),
    ];
    for (frame, expected_frame, expected_count) in cases {
        let mut scanner = HeaderScanner::default();
        let mut last = None;
        for &byte in &frame {
            if let Some(event) = scanner.feed_byte(byte) {
                last = Some(event);
            }
        }
        match last {
            Some(ScanEvent::Header(header)) => {
                assert_eq!(header.frame, expected_frame);
                assert_eq!(header.count(), expected_count);
            }
            other => panic!("expected header, got {other:?}"),
        }
    }
}

#[test]
fn header_scanner_decodes_known_zack_count() {
    let mut scanner = HeaderScanner::default();
    let mut header = None;
    for &byte in &peer_bin32_header(FRAME_ZACK, 0x0102_0304) {
        if let Some(event) = scanner.feed_byte(byte) {
            header = Some(event);
        }
    }
    match header {
        Some(ScanEvent::Header(parsed)) => {
            assert_eq!(parsed.frame, FRAME_ZACK);
            assert_eq!(parsed.count(), 0x0102_0304);
        }
        other => panic!("expected header, got {other:?}"),
    }
}
