use super::*;

// —— 端口标签规范化 ——————————————————————————————————————

#[test]
fn port_label_split_covers_platform_shapes() {
    // (原始标签, 期望路径, 期望描述)：Linux ttyUSB/ttyACM、macOS cu.、Windows COM。
    let cases: &[(&str, &str, Option<&str>)] = &[
        (
            "/dev/ttyUSB0 (FTDI FT232R USB UART)",
            "/dev/ttyUSB0",
            Some("FTDI FT232R USB UART"),
        ),
        (
            "/dev/ttyACM0 (Arduino (www.arduino.cc))",
            "/dev/ttyACM0",
            Some("Arduino (www.arduino.cc)"),
        ),
        (
            "/dev/cu.usbserial-1420 (Silicon Labs CP210x USB to UART Bridge)",
            "/dev/cu.usbserial-1420",
            Some("Silicon Labs CP210x USB to UART Bridge"),
        ),
        ("/dev/cu.usbmodem14101", "/dev/cu.usbmodem14101", None),
        (
            "COM3 (USB Serial Port (COM3))",
            "COM3",
            Some("USB Serial Port (COM3)"),
        ),
        ("COM3", "COM3", None),
        ("/dev/ttyS0", "/dev/ttyS0", None),
        ("/dev/ttyUSB0 ()", "/dev/ttyUSB0", None),
        // 未闭合括号与空路径不拆分，原样保留。
        ("/dev/ttyUSB0 (FTDI", "/dev/ttyUSB0 (FTDI", None),
        ("(junk)", "(junk)", None),
    ];
    for (raw, path, description) in cases {
        let (got_path, got_description) = split_glued_description(raw);
        assert_eq!(&got_path, path, "path for {raw:?}");
        assert_eq!(
            got_description.as_deref(),
            *description,
            "description for {raw:?}"
        );
    }
}

#[test]
fn port_label_prefers_structured_description_over_glue() {
    let structured = normalize_port_label("/dev/cu.usbserial-1420 (glued)", Some("CP210x"));
    assert_eq!(structured.path, "/dev/cu.usbserial-1420");
    assert_eq!(structured.description.as_deref(), Some("CP210x"));
    // 结构化描述为空时退回粘合后缀。
    let fallback = normalize_port_label("/dev/ttyUSB0 (FTDI FT232R)", Some("  "));
    assert_eq!(fallback.path, "/dev/ttyUSB0");
    assert_eq!(fallback.description.as_deref(), Some("FTDI FT232R"));
    let plain = normalize_port_label("/dev/ttyS0", None);
    assert_eq!(plain.path, "/dev/ttyS0");
    assert!(plain.description.is_none());
}

#[test]
fn usb_description_prefers_product_then_manufacturer_then_vid_pid() {
    let base = serialport::UsbPortInfo {
        vid: 0x0403,
        pid: 0x6001,
        serial_number: None,
        manufacturer: None,
        product: None,
    };
    assert_eq!(usb_port_description(&base), "USB 0403:6001");
    let with_manufacturer = serialport::UsbPortInfo {
        manufacturer: Some("FTDI".to_string()),
        ..base.clone()
    };
    assert_eq!(usb_port_description(&with_manufacturer), "FTDI");
    let with_product = serialport::UsbPortInfo {
        product: Some("FT232R USB UART".to_string()),
        ..with_manufacturer
    };
    assert_eq!(usb_port_description(&with_product), "FT232R USB UART");
    // 空白字符串视为缺失。
    let blank_product = serialport::UsbPortInfo {
        product: Some("   ".to_string()),
        manufacturer: None,
        ..base
    };
    assert_eq!(usb_port_description(&blank_product), "USB 0403:6001");
}

// —— 串口参数校验 ——————————————————————————————————————

#[test]
fn data_bits_accepts_the_full_range_and_rejects_junk() {
    let cases = [
        ("5", serialport::DataBits::Five),
        ("6", serialport::DataBits::Six),
        ("7", serialport::DataBits::Seven),
        ("8", serialport::DataBits::Eight),
    ];
    for (raw, expected) in cases {
        assert_eq!(parse_data_bits(Some(&raw.to_string())).unwrap(), expected);
    }
    assert_eq!(parse_data_bits(None).unwrap(), serialport::DataBits::Eight);
    for junk in ["9", "eight", "", "8 "] {
        assert!(
            parse_data_bits(Some(&junk.to_string())).is_err(),
            "{junk:?} must be rejected"
        );
    }
}

#[test]
fn parity_accepts_the_wire_vocabulary_and_rejects_junk() {
    let cases = [
        ("none", serialport::Parity::None),
        ("even", serialport::Parity::Even),
        ("odd", serialport::Parity::Odd),
    ];
    for (raw, expected) in cases {
        assert_eq!(parse_parity(Some(&raw.to_string())).unwrap(), expected);
    }
    assert_eq!(parse_parity(None).unwrap(), serialport::Parity::None);
    for junk in ["mark", "space", "EVEN", ""] {
        assert!(
            parse_parity(Some(&junk.to_string())).is_err(),
            "{junk:?} must be rejected"
        );
    }
}

#[test]
fn stop_bits_accepts_one_or_two_and_rejects_junk() {
    assert_eq!(
        parse_stop_bits(Some(&"1".to_string())).unwrap(),
        serialport::StopBits::One
    );
    assert_eq!(
        parse_stop_bits(Some(&"2".to_string())).unwrap(),
        serialport::StopBits::Two
    );
    assert_eq!(parse_stop_bits(None).unwrap(), serialport::StopBits::One);
    for junk in ["1.5", "0", ""] {
        assert!(
            parse_stop_bits(Some(&junk.to_string())).is_err(),
            "{junk:?} must be rejected"
        );
    }
}

#[test]
fn flow_control_accepts_the_wire_vocabulary_and_rejects_junk() {
    let cases = [
        ("none", serialport::FlowControl::None),
        ("rts_cts", serialport::FlowControl::Hardware),
        ("xon_xoff", serialport::FlowControl::Software),
    ];
    for (raw, expected) in cases {
        assert_eq!(
            parse_flow_control(Some(&raw.to_string())).unwrap(),
            expected
        );
    }
    assert_eq!(
        parse_flow_control(None).unwrap(),
        serialport::FlowControl::None
    );
    for junk in ["hardware", "rtscts", "software", ""] {
        assert!(
            parse_flow_control(Some(&junk.to_string())).is_err(),
            "{junk:?} must be rejected"
        );
    }
}

#[test]
fn resolve_reports_the_offending_parameter() {
    let request = SerialStartRequest {
        data_bits: Some("8".to_string()),
        parity: Some("mark".to_string()),
        stop_bits: Some("1".to_string()),
        ..SerialStartRequest::default()
    };
    let error = resolve_line_params(&request).unwrap_err();
    assert!(error.starts_with("serial/start:"), "got {error}");
    assert!(error.contains("invalid parity"), "got {error}");
    assert!(
        !error.contains("dataBits"),
        "only the offender is named: {error}"
    );
}

#[test]
fn default_request_resolves_to_8n1_without_flow_control() {
    let line = resolve_line_params(&SerialStartRequest::default()).unwrap();
    assert_eq!(line.data_bits, serialport::DataBits::Eight);
    assert_eq!(line.parity, serialport::Parity::None);
    assert_eq!(line.stop_bits, serialport::StopBits::One);
    assert_eq!(line.flow_control, serialport::FlowControl::None);
}

#[test]
fn baud_rate_bounds_match_the_documented_range() {
    for good in [50u32, 9_600, 115_200, 230_400, 3_000_000, 4_000_000] {
        assert_eq!(checked_baud_rate(good).unwrap(), good);
    }
    for bad in [0, 49, 4_000_001, u32::MAX] {
        let error = checked_baud_rate(bad).unwrap_err();
        assert!(
            error.starts_with("serial/start:") && error.contains("baudRate"),
            "{bad} error names the field: {error}"
        );
    }
}

// —— B1 二进制写通道（serial/terminal/in）—————————————————

fn stdin_frame(sequence: u64, data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(9 + data.len());
    frame.push(TerminalStream::Stdin as u8);
    frame.extend_from_slice(&sequence.to_be_bytes());
    frame.extend_from_slice(data);
    frame
}

#[test]
fn start_response_declares_the_binary_input_capability() {
    // 能力探测（设计稿 §2 兼容策略）：响应含 binaryInput=true；旧字段
    // （sessionId/port/baudRate）形状不变，老前端不受影响。
    let response = start_response("sid-1", "/dev/ttyUSB0", 115_200);
    assert_eq!(response["sessionId"], "sid-1");
    assert_eq!(response["port"], "/dev/ttyUSB0");
    assert_eq!(response["baudRate"], 115_200);
    assert_eq!(response["binaryInput"], serde_json::Value::Bool(true));
}

#[test]
fn input_frame_roundtrips_stream_tag_sequence_and_payload() {
    // 编解码对称性：Stdin 标签 + 大端序号 + 原始键序字节。
    let (sequence, payload) = decode_input_frame(&stdin_frame(42, b"ls\r")).unwrap();
    assert_eq!(sequence, 42);
    assert_eq!(payload, b"ls\r");
    // 空负载（纯确认帧）同样合法。
    let (sequence, payload) = decode_input_frame(&stdin_frame(0, b"")).unwrap();
    assert_eq!(sequence, 0);
    assert!(payload.is_empty());
    // 大序号不截断。
    let (sequence, _) = decode_input_frame(&stdin_frame(u64::MAX, b"x")).unwrap();
    assert_eq!(sequence, u64::MAX);
}

#[test]
fn input_frame_rejects_non_stdin_tags_as_parameter_errors() {
    // 已知标签但方向不对（Stdout/Stderr/State）→ 参数错误，绝不当作键入。
    for tag in [0u8, 1, 2] {
        let mut frame = stdin_frame(1, b"x");
        frame[0] = tag;
        let error = decode_input_frame(&frame).unwrap_err();
        assert!(error.contains("serial/terminal/in"), "tag {tag}: {error}");
        assert!(error.contains("rejected"), "tag {tag}: {error}");
    }
    // 未知标签同样参数错误（不静默、不断连——错误由宿主桥按参数错误回）。
    let mut frame = stdin_frame(1, b"x");
    frame[0] = 99;
    assert!(decode_input_frame(&frame).is_err());
}

#[test]
fn input_frame_rejects_truncated_headers_without_panicking() {
    for len in 0..9usize {
        let frame = stdin_frame(1, b"payload");
        assert!(
            decode_input_frame(&frame[..len]).is_err(),
            "length {len} must be rejected"
        );
    }
    // 恰好 9 字节（帧头无负载）合法。
    assert_eq!(
        decode_input_frame(&stdin_frame(3, b"")).unwrap(),
        (3, Vec::<u8>::new())
    );
}

// —— serial/replay 序号制回放（设计稿 §3）———————————————————

#[test]
fn replay_buffer_semantics_match_the_serial_summary_contract() {
    // 读线程经 ReplayBuffer 分配序号后，replay 摘要的字段语义：
    // after_sequence → 重发帧 + first/tail/complete，与 telnet/local 同构。
    let mut replay = crate::sessions::ssh::ReplayBuffer::with_byte_limit(SERIAL_REPLAY_BYTE_LIMIT);
    let f1 = replay.push(TerminalStream::Stdout, b"a".to_vec());
    let f2 = replay.push(TerminalStream::Stdout, b"b".to_vec());
    assert_eq!((f1.sequence, f2.sequence), (1, 2));
    assert_eq!(replay.first_sequence(), 1);
    assert_eq!(replay.tail_sequence(), 2);
    // 完整回放：afterSequence=0 → 两帧 + complete。
    let frames = replay.after(0);
    assert_eq!(frames.len(), 2);
    assert!(1 >= replay.first_sequence(), "complete = after+1 >= first");
    // 增量回放：afterSequence=1 → 仅第 2 帧。
    assert_eq!(replay.after(1).len(), 1);
    // 空缓冲（会话刚开、尚无输出）：first = tail+1，complete 按同式成立。
    let empty = crate::sessions::ssh::ReplayBuffer::with_byte_limit(SERIAL_REPLAY_BYTE_LIMIT);
    assert_eq!(empty.first_sequence(), 1);
    assert_eq!(empty.tail_sequence(), 0);
    // afterSequence=0 → 0+1 >= first(1) = complete。
    let after_sequence = 0u64;
    assert!(after_sequence + 1 >= empty.first_sequence());
}

#[test]
fn replay_buffer_wraps_at_the_serial_byte_budget_and_reports_incomplete() {
    // 128 KiB 预算绕回：预算装不下的旧帧被逐出后 first_sequence 前移，
    // 从被逐出序号之后的请求得到不完整回放（complete: false 的判定来源）。
    let mut replay = crate::sessions::ssh::ReplayBuffer::with_byte_limit(4);
    replay.push(TerminalStream::Stdout, vec![b'x'; 3]);
    replay.push(TerminalStream::Stdout, vec![b'y'; 3]);
    replay.push(TerminalStream::Stdout, vec![b'z'; 3]);
    // 预算 4 字节只容得下最后一帧（前两帧先后被逐出）。
    assert_eq!(
        replay.first_sequence(),
        3,
        "frames 1-2 evicted by the budget"
    );
    // 请求 afterSequence=0（第 1 帧之后）：第 1、2 帧已不在缓冲 → 不完整。
    let after_sequence = 0u64;
    assert!(
        !(after_sequence + 1 >= replay.first_sequence()),
        "afterSequence 0 below first_available must be incomplete"
    );
    assert_eq!(replay.after(0).len(), 1);
    // 从可得帧之后回放（afterSequence=2）→ 完整。
    assert!(2u64 + 1 >= replay.first_sequence());
    // 串口实际预算常量为 128 KiB。
    assert_eq!(SERIAL_REPLAY_BYTE_LIMIT, 128 * 1024);
}

// —— 写序列化与回压（设计稿「写序列化与回压」节）———————————————

#[test]
fn write_queue_chunks_keystrokes_and_keeps_fifo_byte_order() {
    // 键入大包按 4 KiB 小块分帧入队；FIFO 出队拼回字节流逐字节一致。
    // （pop 在未关闭的队列上会阻塞，按预期块数出队。）
    let queue = WriteQueue::new();
    let payload: Vec<u8> = (0..(SERIAL_WRITE_CHUNK_BYTES * 2 + 3))
        .map(|i| i as u8)
        .collect();
    queue.push(&payload, WriteSource::Keystroke).unwrap();
    assert_eq!(queue.queued_bytes(), payload.len());
    let expected_chunks = payload.len().div_ceil(SERIAL_WRITE_CHUNK_BYTES);
    let mut reassembled = Vec::new();
    for _ in 0..expected_chunks {
        let chunk = queue.pop().expect("queued chunk must be present");
        assert!(chunk.len() <= SERIAL_WRITE_CHUNK_BYTES, "chunk is bounded");
        reassembled.extend_from_slice(&chunk);
    }
    assert_eq!(reassembled, payload);
    assert_eq!(queue.queued_bytes(), 0);
}

#[test]
fn write_queue_keeps_upload_blocks_undivided() {
    // 引擎输出（Bulk）保持块级原样，不受切分影响（哪怕超过分帧上限）。
    let queue = WriteQueue::new();
    let block = vec![0xAA; SERIAL_WRITE_CHUNK_BYTES + 512];
    queue.push(&block, WriteSource::Bulk).unwrap();
    let first = queue.pop().unwrap();
    assert_eq!(first.len(), block.len(), "bulk stays one chunk");
    // 排空检查前置关闭，pop 的 None 退出语义由 shutdown 用例覆盖。
    assert_eq!(queue.queued_bytes(), 0);
}

#[test]
fn write_queue_rejects_whole_packets_and_reports_the_reason() {
    // 键入超单包上限 → Oversize；Bulk 无单包上限但受预算约束（超预算的
    // 引擎块 → QueueFull）。两种拒绝都全有或全无（队列原样未动）。
    let queue = WriteQueue::new();
    let oversize = vec![0u8; SERIAL_KEYSTROKE_MAX_BYTES + 1];
    assert_eq!(
        queue.push(&oversize, WriteSource::Keystroke),
        Err(WriteReject::Oversize)
    );
    assert_eq!(queue.queued_bytes(), 0, "all-or-nothing admission");

    let beyond_budget = vec![0u8; SERIAL_WRITE_QUEUE_BUDGET + 1];
    assert_eq!(
        queue.push(&beyond_budget, WriteSource::Bulk),
        Err(WriteReject::QueueFull)
    );
    assert_eq!(queue.queued_bytes(), 0, "all-or-nothing admission");

    let filler = vec![0u8; SERIAL_WRITE_QUEUE_BUDGET - 8];
    queue.push(&filler, WriteSource::Bulk).unwrap();
    let budget = WriteQueue::new();
    budget.push(&filler, WriteSource::Bulk).unwrap();
    assert_eq!(
        budget.push(b"late keystroke", WriteSource::Keystroke),
        Err(WriteReject::QueueFull)
    );
    assert_eq!(budget.queued_bytes(), filler.len(), "queue untouched");
    assert!(!WriteReject::QueueFull.message().contains("keystroke"));
}

#[test]
fn write_queue_clear_and_shutdown_bound_the_cancel_and_close_paths() {
    // 取消：clear 丢掉未写出的块并返回字节数，队列随后可继续入队
    // （取消序列本身在清空后入队）。关闭：shutdown 后 push 报 Closed，
    // pop 排空即 None（写线程退出条件）。
    let queue = WriteQueue::new();
    queue
        .push(b"stale upload block", WriteSource::Bulk)
        .unwrap();
    assert_eq!(queue.clear(), "stale upload block".len());
    assert_eq!(queue.queued_bytes(), 0);
    queue.push(b"cancel sequence", WriteSource::Bulk).unwrap();

    let closed = WriteQueue::new();
    closed.push(b"pending", WriteSource::Keystroke).unwrap();
    closed.shutdown();
    assert_eq!(
        closed.push(b"late", WriteSource::Keystroke),
        Err(WriteReject::Closed)
    );
    assert_eq!(closed.pop(), None);
}

#[test]
fn write_queue_pop_blocks_until_a_producer_pushes() {
    // 消费者阻塞等待 + Condvar 唤醒：写线程在无块时休眠，生产者入队即醒。
    let queue = Arc::new(WriteQueue::new());
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let consumer_queue = Arc::clone(&queue);
    std::thread::spawn(move || {
        done_tx.send(consumer_queue.pop()).expect("report pop");
    });
    // 生产者尚未入队：短暂窗口内不得拿到块（未唤醒即返回就是 bug）。
    assert!(done_rx.recv_timeout(Duration::from_millis(40)).is_err());
    queue.push(b"wake", WriteSource::Keystroke).unwrap();
    let chunk = done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("pop wakes on push")
        .expect("closed queues yield None, open ones yield the chunk");
    assert_eq!(chunk, b"wake".to_vec());
}

#[test]
fn workbench_cleanup_selects_only_sessions_owned_by_the_closing_tab() {
    let sessions = [
        ("serial-a", "workbench-a"),
        ("serial-b", "workbench-b"),
        ("serial-c", "workbench-a"),
    ];
    assert_eq!(
        session_ids_for_workbench(sessions, "workbench-a"),
        vec!["serial-a".to_string(), "serial-c".to_string()]
    );
}
