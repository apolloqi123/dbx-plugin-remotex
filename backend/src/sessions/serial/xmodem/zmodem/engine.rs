//! Serial upload engine layer.

use super::*;

// —— ZMODEM 发送端 ——————————————————————————————————————————————

/// ZMODEM 发送端状态（对应 zmodem2 SendState 的单文件子集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ZState {
    WaitReceiverInit,
    WaitFilePos,
    /// ZDATA+数据已发，等 ZRPOS/ZACK。
    WaitFileAck,
    /// ZEOF 已发，等 ZRINIT。
    WaitFileDone,
    /// ZFIN 已发，等 ZFIN/ZRINIT。
    WaitFinish,
    Done,
}

/// ZMODEM 上传引擎（单文件）。子包策略取保守模式：每帧一个 ZCRCW 子包
/// （强制对端落盘确认），子包大小按对端 ZRINIT 声明的接收缓冲截断。
#[derive(Debug)]
pub(crate) struct ZEngine {
    state: ZState,
    pub(crate) source: UploadSource,
    /// 对端确认的偏移（进度与续传依据）。
    acked: u64,
    chunk: usize,
    scanner: HeaderScanner,
    cancel_count: usize,
    /// 自上一个有效帧头以来的连续坏帧数（ZNAK 预算，见 MAX_CORRUPT_FRAMES）。
    corrupt_count: usize,
    retries: u8,
    /// 可重发的最后一帧（ZRQINIT/ZFILE/ZDATA+数据/ZEOF/ZFIN）。
    last_frame: Vec<u8>,
    pub(crate) last_activity: Instant,
    /// 数据分块未到齐：时钟暂停，等 `append_data` 唤醒。
    waiting_data: bool,
    /// ZRPOS/ZACK 之后待履行的读请求。
    pending_read: bool,
    sent_reported: bool,
}

impl ZEngine {
    pub(crate) fn new(source: UploadSource, now: Instant) -> (Self, Vec<Output>) {
        let zrqinit = hex_header(FRAME_ZRQINIT, [0; 4]);
        let engine = Self {
            state: ZState::WaitReceiverInit,
            source,
            acked: 0,
            chunk: ZMODEM_MAX_SUBPACKET,
            scanner: HeaderScanner::default(),
            cancel_count: 0,
            corrupt_count: 0,
            retries: 0,
            last_frame: zrqinit.clone(),
            last_activity: now,
            waiting_data: false,
            pending_read: false,
            sent_reported: false,
        };
        (engine, vec![Output::Write(zrqinit)])
    }

    pub(crate) fn is_done(&self) -> bool {
        self.state == ZState::Done
    }

    pub(crate) fn progress(
        &self,
        state: ProgressState,
        reason: Option<String>,
    ) -> TransferProgress {
        TransferProgress {
            protocol: UploadProtocol::Zmodem,
            file_name: self.source.file_name().to_string(),
            file_index: 0,
            sent: self.acked.min(self.source.total()),
            total: self.source.total(),
            state,
            reason,
        }
    }

    pub(crate) fn fail(&mut self, reason: &str) -> Vec<Output> {
        self.state = ZState::Done;
        self.waiting_data = false;
        self.pending_read = false;
        vec![
            Output::Write(zmodem_cancel_sequence()),
            Output::Progress(self.progress(ProgressState::Failed, Some(reason.to_string()))),
        ]
    }

    pub(crate) fn feed_at(&mut self, bytes: &[u8], now: Instant) -> Vec<Output> {
        if self.is_done() {
            return Vec::new();
        }
        if !bytes.is_empty() {
            self.last_activity = now;
        }
        let mut actions = Vec::new();
        for &byte in bytes {
            if self.is_done() {
                break;
            }
            // 连续 ZDLE 是取消副信道；字节本身仍要喂给扫描器（合法帧头
            // 也以 ZDLE 起始），与 NyaTerm 的处理一致。
            if byte == ZDLE {
                self.cancel_count = self.cancel_count.saturating_add(1);
                if self.cancel_count >= Z_CANCEL_COUNT {
                    self.state = ZState::Done;
                    self.waiting_data = false;
                    self.pending_read = false;
                    actions.push(Output::Progress(self.progress(
                        ProgressState::Failed,
                        Some("Remote cancelled the transfer".to_string()),
                    )));
                    break;
                }
            } else {
                self.cancel_count = 0;
            }
            match self.scanner.feed_byte(byte) {
                Some(ScanEvent::Header(header)) => {
                    // 有效帧头恢复 ZNAK 预算：线路抖动下的零星坏帧可长期
                    // 重试，持续垃圾流则在上限处判失败。
                    self.corrupt_count = 0;
                    actions.extend(self.handle_header(header, now));
                }
                Some(ScanEvent::Corrupt) => {
                    // 坏帧：ZNAK 重扫（与 zmodem2 CRC 错误降级一致）。
                    self.last_activity = now;
                    actions.push(Output::Write(hex_header(FRAME_ZNAK, [0; 4])));
                    self.corrupt_count += 1;
                    if self.corrupt_count >= MAX_CORRUPT_FRAMES {
                        actions.extend(self.fail("Too many corrupt frames from the receiver"));
                    }
                }
                None => {}
            }
        }
        actions
    }

    pub(crate) fn tick_at(&mut self, now: Instant) -> Vec<Output> {
        if self.is_done() || self.waiting_data {
            return Vec::new();
        }
        if now.duration_since(self.last_activity) < IDLE_RETRY {
            return Vec::new();
        }
        if self.last_frame.is_empty() {
            return self.fail("Receiver did not respond");
        }
        self.retries += 1;
        if self.retries > MAX_RETRIES {
            return self.fail("Receiver did not respond");
        }
        self.last_activity = now;
        vec![Output::Write(self.last_frame.clone())]
    }

    pub(crate) fn handle_header(&mut self, header: ZHeader, now: Instant) -> Vec<Output> {
        match header.frame {
            FRAME_ZRINIT => self.on_zrinit(header, now),
            FRAME_ZRPOS | FRAME_ZACK => self.on_position(header.count(), now),
            FRAME_ZSKIP => self.fail("Remote refused the file"),
            FRAME_ZFIN => self.on_zfin(),
            FRAME_ZCAN | FRAME_ZFERR | FRAME_ZABORT => self.fail("Remote aborted the transfer"),
            _ => {
                // 未知帧：WaitReceiverInit 下重发 ZRQINIT（zmodem2 语义）。
                if self.state == ZState::WaitReceiverInit {
                    self.retries = 0;
                    let frame = hex_header(FRAME_ZRQINIT, [0; 4]);
                    self.last_activity = now;
                    self.last_frame = frame.clone();
                    return vec![Output::Write(frame)];
                }
                Vec::new()
            }
        }
    }

    pub(crate) fn on_zrinit(&mut self, header: ZHeader, now: Instant) -> Vec<Output> {
        // ZR0/ZR1 = 接收缓冲 LE16；0 视为未声明 → 8 KiB；下限 64 上限 8 KiB。
        let declared = u16::from_le_bytes([header.flags[0], header.flags[1]]);
        self.chunk = if declared == 0 {
            ZMODEM_MAX_SUBPACKET
        } else {
            (declared as usize).clamp(64, ZMODEM_MAX_SUBPACKET)
        };
        match self.state {
            ZState::WaitReceiverInit => {
                self.retries = 0;
                let frame = self.build_zfile();
                self.last_frame = frame.clone();
                self.last_activity = now;
                self.state = ZState::WaitFilePos;
                vec![Output::Write(frame)]
            }
            ZState::WaitFileDone => {
                self.retries = 0;
                let mut actions = vec![Output::Progress(
                    self.progress(ProgressState::FileComplete, None),
                )];
                let zfin = hex_header(FRAME_ZFIN, [0; 4]);
                self.last_frame = zfin.clone();
                self.last_activity = now;
                self.state = ZState::WaitFinish;
                actions.push(Output::Write(zfin));
                actions
            }
            ZState::WaitFinish => self.on_zfin(),
            _ => Vec::new(),
        }
    }

    /// ZRPOS/ZACK：携带的对端确认偏移决定续传点。
    pub(crate) fn on_position(&mut self, count: u32, now: Instant) -> Vec<Output> {
        match self.state {
            ZState::WaitFilePos | ZState::WaitFileAck => {
                self.acked = u64::from(count).min(self.source.total());
                let mut actions = vec![Output::Progress(
                    self.progress(ProgressState::Running, None),
                )];
                if self.acked >= self.source.total() {
                    let frame = bin32_header(FRAME_ZEOF, (self.acked as u32).to_le_bytes());
                    self.last_frame = frame.clone();
                    self.retries = 0;
                    self.last_activity = now;
                    self.state = ZState::WaitFileDone;
                    actions.push(Output::Write(frame));
                    return actions;
                }
                self.retries = 0;
                self.pending_read = true;
                actions.extend(self.fulfill_read(now));
                actions
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn on_zfin(&mut self) -> Vec<Output> {
        if self.state == ZState::WaitFinish {
            self.state = ZState::Done;
            vec![
                Output::Write(b"OO".to_vec()),
                Output::Progress(self.progress(ProgressState::Complete, None)),
            ]
        } else {
            Vec::new()
        }
    }

    /// ZFILE 帧头 + ZCRCW 元数据子包：`name\0{size} {mtime:o} {mode:o} 0 0 0\0`。
    pub(crate) fn build_zfile(&self) -> Vec<u8> {
        let mut meta = Vec::new();
        meta.extend_from_slice(self.source.file_name().as_bytes());
        meta.push(0);
        meta.extend_from_slice(
            format!(
                "{} {:o} {:o} 0 0 0\0",
                self.source.total(),
                0u32,
                ZFILE_DEFAULT_MODE
            )
            .as_bytes(),
        );
        let mut frame = bin32_header(FRAME_ZFILE, ZFILE_ZCBIN_FLAGS);
        frame.extend_from_slice(&zcrcw_subpacket(&meta));
        frame
    }

    /// 挂起读请求到达后的续传：组 ZDATA + 单个 ZCRCW 数据子包。
    pub(crate) fn fulfill_read(&mut self, now: Instant) -> Vec<Output> {
        if !self.pending_read || self.is_done() {
            return Vec::new();
        }
        let want = ((self.source.total() - self.acked) as usize).min(self.chunk);
        match self.source.read_upto(self.acked, want) {
            Ok(ReadOutcome::Ready(chunk)) => {
                self.waiting_data = false;
                self.pending_read = false;
                let mut frame = bin32_header(FRAME_ZDATA, (self.acked as u32).to_le_bytes());
                frame.extend_from_slice(&zcrcw_subpacket(&chunk));
                self.last_frame = frame.clone();
                self.last_activity = now;
                self.state = ZState::WaitFileAck;
                let mut actions = Vec::new();
                if !self.sent_reported {
                    self.sent_reported = true;
                    actions.push(Output::Progress(
                        self.progress(ProgressState::Running, None),
                    ));
                }
                actions.push(Output::Write(frame));
                actions
            }
            Ok(ReadOutcome::Pending) => {
                // 分块在途：暂停时钟等待 append_data 唤醒。
                self.waiting_data = true;
                Vec::new()
            }
            Ok(ReadOutcome::Eof) => {
                // 对端确认越过已收尾的文件尾：按当前位置收束为 ZEOF。
                let frame = bin32_header(FRAME_ZEOF, (self.acked as u32).to_le_bytes());
                self.waiting_data = false;
                self.pending_read = false;
                self.last_frame = frame.clone();
                self.last_activity = now;
                self.state = ZState::WaitFileDone;
                vec![Output::Write(frame)]
            }
            Err(reason) => self.fail(&reason),
        }
    }

    pub(crate) fn append_data_at(&mut self, now: Instant) -> Vec<Output> {
        if self.waiting_data {
            self.waiting_data = false;
            self.last_activity = now;
            return self.fulfill_read(now);
        }
        Vec::new()
    }

    pub(crate) fn cancel_at(&mut self) -> Vec<Output> {
        if self.is_done() {
            return Vec::new();
        }
        self.state = ZState::Done;
        self.waiting_data = false;
        self.pending_read = false;
        vec![
            Output::Write(zmodem_cancel_sequence()),
            Output::Progress(
                self.progress(ProgressState::Failed, Some("cancelled by user".to_string())),
            ),
        ]
    }
}
