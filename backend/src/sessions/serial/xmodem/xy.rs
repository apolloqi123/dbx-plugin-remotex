//! Serial upload engine layer.

use super::*;

// —— X/YMODEM ——————————————————————————————————————————————————

/// X/YMODEM 传输状态（与 NyaTerm 的 TransferState 同形）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XyState {
    WaitingHandshake,
    WaitingHeaderAck,
    WaitingDataStart,
    WaitingDataAck,
    WaitingEotAck,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckMode {
    Checksum,
    Crc,
}

/// X/YMODEM 引擎。`acked` 为对端已确认的负载字节数（进度依据）。
#[derive(Debug)]
pub(crate) struct XyEngine {
    protocol: UploadProtocol,
    state: XyState,
    pub(crate) source: UploadSource,
    /// 下一读取偏移（= 已送上线路的负载字节数）。
    offset: u64,
    /// 对端确认的负载字节数。
    acked: u64,
    sequence: u8,
    check_mode: CheckMode,
    /// 等待确认期间可重发的最后一包（数据块或 EOT）。
    last_packet: Vec<u8>,
    /// 最后一包的真实负载字节数（不含填充；ACK 到达时累入 acked）。
    last_payload_len: u64,
    /// YMODEM：EOT 已确认，接下来握手要的是收尾全零头块。
    files_done: bool,
    retries: u8,
    remote_cancel_count: usize,
    pub(crate) last_activity: Instant,
    /// 握手/启动信号已被消费但数据分块未到齐：暂停时钟等 `append_data`。
    handshake_pending: bool,
    sent_reported: bool,
}

impl XyEngine {
    pub(super) fn new(protocol: UploadProtocol, source: UploadSource, now: Instant) -> Self {
        Self {
            protocol,
            state: XyState::WaitingHandshake,
            source,
            offset: 0,
            acked: 0,
            sequence: 1,
            check_mode: CheckMode::Crc,
            last_packet: Vec::new(),
            last_payload_len: 0,
            files_done: false,
            retries: 0,
            remote_cancel_count: 0,
            last_activity: now,
            handshake_pending: false,
            sent_reported: false,
        }
    }

    pub(super) fn is_done(&self) -> bool {
        self.state == XyState::Done
    }

    pub(super) fn progress(
        &self,
        state: ProgressState,
        reason: Option<String>,
    ) -> TransferProgress {
        TransferProgress {
            protocol: self.protocol,
            file_name: self.source.file_name().to_string(),
            file_index: 0,
            sent: self.acked.min(self.source.total()),
            total: self.source.total(),
            state,
            reason,
        }
    }

    pub(super) fn fail(&mut self, reason: &str) -> Vec<Output> {
        self.state = XyState::Done;
        self.handshake_pending = false;
        vec![
            Output::Write(vec![CAN; 8]),
            Output::Progress(self.progress(ProgressState::Failed, Some(reason.to_string()))),
        ]
    }

    pub(super) fn feed_at(&mut self, bytes: &[u8], now: Instant) -> Vec<Output> {
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
            let before = actions.len();
            if byte == CAN {
                self.remote_cancel_count = self.remote_cancel_count.saturating_add(1);
                if self.remote_cancel_count >= XY_CANCEL_COUNT {
                    actions.extend(self.fail("Receiver cancelled the transfer"));
                }
                continue;
            }
            self.remote_cancel_count = 0;
            match self.protocol {
                UploadProtocol::Xmodem => self.handle_xmodem_byte(byte, now, &mut actions),
                UploadProtocol::Ymodem => self.handle_ymodem_byte(byte, now, &mut actions),
                UploadProtocol::Zmodem => {}
            }
            // 一轮交互只发一包：重复 ACK 不得推进两个块（NyaTerm 语义）。
            if actions[before..]
                .iter()
                .any(|action| matches!(action, Output::Write(_)))
            {
                break;
            }
        }
        actions
    }

    pub(super) fn tick_at(&mut self, now: Instant) -> Vec<Output> {
        if self.is_done() || self.handshake_pending {
            return Vec::new();
        }
        if now.duration_since(self.last_activity) < IDLE_RETRY {
            return Vec::new();
        }
        match self.state {
            XyState::WaitingHandshake | XyState::WaitingDataStart => {
                self.fail("Timed out waiting for the receiver handshake")
            }
            XyState::WaitingHeaderAck | XyState::WaitingDataAck | XyState::WaitingEotAck => {
                self.retry_last(now)
            }
            XyState::Done => Vec::new(),
        }
    }

    pub(super) fn retry_last(&mut self, now: Instant) -> Vec<Output> {
        if self.last_packet.is_empty() {
            return self.fail("Receiver did not acknowledge the transfer");
        }
        self.retries += 1;
        if self.retries > MAX_RETRIES {
            return self.fail("Receiver did not acknowledge the transfer");
        }
        self.last_activity = now;
        vec![Output::Write(self.last_packet.clone())]
    }

    pub(super) fn handle_xmodem_byte(&mut self, byte: u8, now: Instant, actions: &mut Vec<Output>) {
        match self.state {
            XyState::WaitingHandshake => {
                let mode = match byte {
                    NAK => CheckMode::Checksum,
                    CRC_REQUEST => CheckMode::Crc,
                    _ => return,
                };
                self.check_mode = mode;
                self.sequence = 1;
                if let Err(reason) = self.send_next_data(now, actions) {
                    actions.extend(self.fail(&reason));
                }
            }
            XyState::WaitingDataAck if byte == ACK => {
                self.confirm_payload(now, actions);
                self.sequence = self.sequence.wrapping_add(1);
                self.retries = 0;
                if let Err(reason) = self.send_next_data(now, actions) {
                    actions.extend(self.fail(&reason));
                }
            }
            XyState::WaitingDataAck if byte == NAK => actions.extend(self.retry_last(now)),
            XyState::WaitingEotAck if byte == ACK => {
                self.state = XyState::Done;
                actions.push(Output::Progress(
                    self.progress(ProgressState::Complete, None),
                ));
            }
            XyState::WaitingEotAck if byte == NAK => actions.extend(self.retry_last(now)),
            _ => {}
        }
    }

    pub(super) fn handle_ymodem_byte(&mut self, byte: u8, now: Instant, actions: &mut Vec<Output>) {
        match self.state {
            XyState::WaitingHandshake if byte == CRC_REQUEST => {
                if let Err(reason) = self.send_header_block(now, actions) {
                    actions.extend(self.fail(&reason));
                }
            }
            XyState::WaitingHeaderAck if byte == ACK => {
                self.retries = 0;
                if self.files_done {
                    // 收尾空头块确认：整批结束。
                    self.state = XyState::Done;
                    actions.push(Output::Progress(
                        self.progress(ProgressState::Complete, None),
                    ));
                } else {
                    // 头块确认后等待接收方再发 'C' 才开始数据块。
                    self.state = XyState::WaitingDataStart;
                }
            }
            XyState::WaitingHeaderAck if byte == NAK => actions.extend(self.retry_last(now)),
            XyState::WaitingDataStart if byte == CRC_REQUEST => {
                self.sequence = 1;
                if let Err(reason) = self.send_next_data(now, actions) {
                    actions.extend(self.fail(&reason));
                }
            }
            XyState::WaitingDataAck if byte == ACK => {
                self.confirm_payload(now, actions);
                self.sequence = self.sequence.wrapping_add(1);
                self.retries = 0;
                if let Err(reason) = self.send_next_data(now, actions) {
                    actions.extend(self.fail(&reason));
                }
            }
            XyState::WaitingDataAck if byte == NAK => actions.extend(self.retry_last(now)),
            XyState::WaitingEotAck if byte == NAK => actions.extend(self.retry_last(now)),
            XyState::WaitingEotAck if byte == ACK => {
                // EOT 确认：本文件完成；接收方随后再发 'C' 索要收尾空头块。
                self.retries = 0;
                self.files_done = true;
                actions.push(Output::Progress(
                    self.progress(ProgressState::FileComplete, None),
                ));
                self.last_packet = Vec::new();
                self.last_payload_len = 0;
                self.state = XyState::WaitingHandshake;
            }
            _ => {}
        }
    }

    /// 数据块 ACK 到达：累计确认字节数并上报进度。
    pub(super) fn confirm_payload(&mut self, now: Instant, actions: &mut Vec<Output>) {
        self.acked = (self.acked + self.last_payload_len).min(self.source.total());
        self.last_activity = now;
        actions.push(Output::Progress(
            self.progress(ProgressState::Running, None),
        ));
    }

    /// YMODEM 头块：`name\0size` 零填充到 128B，块号 0，固定 CRC 校验；
    /// 收尾阶段发全零块。
    pub(super) fn send_header_block(
        &mut self,
        now: Instant,
        actions: &mut Vec<Output>,
    ) -> Result<(), String> {
        let mut data = [0u8; BLOCK_SIZE];
        if !self.files_done {
            let name = self.source.file_name().as_bytes();
            let size = self.source.total().to_string();
            if name.len() + 1 + size.len() > BLOCK_SIZE {
                return Err(format!(
                    "YMODEM file name is too long: {}",
                    self.source.file_name()
                ));
            }
            data[..name.len()].copy_from_slice(name);
            let size_start = name.len() + 1;
            data[size_start..size_start + size.len()].copy_from_slice(size.as_bytes());
        }
        let packet = build_xy_packet(0, &data, CheckMode::Crc);
        self.queue_packet(packet, 0, XyState::WaitingHeaderAck, now, actions);
        Ok(())
    }

    pub(super) fn send_next_data(
        &mut self,
        now: Instant,
        actions: &mut Vec<Output>,
    ) -> Result<(), String> {
        match self.source.read_upto(self.offset, BLOCK_SIZE)? {
            ReadOutcome::Pending => {
                // 分块仍在途：保持当前状态，时钟不推进，等 append_data 唤醒。
                self.handshake_pending = true;
                return Ok(());
            }
            ReadOutcome::Eof => {
                self.handshake_pending = false;
                self.queue_packet(vec![EOT], 0, XyState::WaitingEotAck, now, actions);
                return Ok(());
            }
            ReadOutcome::Ready(chunk) => {
                self.handshake_pending = false;
                let mut data = [CPM_EOF; BLOCK_SIZE];
                data[..chunk.len()].copy_from_slice(&chunk);
                let packet = build_xy_packet(self.sequence, &data, self.check_mode);
                self.offset = (self.offset + chunk.len() as u64).min(self.source.total());
                self.queue_packet(
                    packet,
                    chunk.len() as u64,
                    XyState::WaitingDataAck,
                    now,
                    actions,
                );
            }
        }
        Ok(())
    }

    pub(super) fn queue_packet(
        &mut self,
        packet: Vec<u8>,
        payload_len: u64,
        state: XyState,
        now: Instant,
        actions: &mut Vec<Output>,
    ) {
        self.last_packet = packet.clone();
        self.last_payload_len = payload_len;
        self.state = state;
        self.retries = 0;
        self.last_activity = now;
        actions.push(Output::Write(packet));
        if !self.sent_reported {
            self.sent_reported = true;
            actions.push(Output::Progress(
                self.progress(ProgressState::Running, None),
            ));
        }
    }

    pub(super) fn append_data_at(&mut self, now: Instant) -> Vec<Output> {
        if !self.handshake_pending || self.is_done() {
            return Vec::new();
        }
        let mut actions = Vec::new();
        match self.state {
            XyState::WaitingHandshake if self.files_done => {
                // 收尾头块不依赖数据，正常情况下不该挂起；防御性重发。
                if let Err(reason) = self.send_header_block(now, &mut actions) {
                    actions.extend(self.fail(&reason));
                }
            }
            XyState::WaitingHandshake | XyState::WaitingDataStart => {
                if let Err(reason) = self.send_next_data(now, &mut actions) {
                    actions.extend(self.fail(&reason));
                }
            }
            _ => {
                self.handshake_pending = false;
            }
        }
        actions
    }

    pub(super) fn cancel_at(&mut self) -> Vec<Output> {
        if self.is_done() {
            return Vec::new();
        }
        self.state = XyState::Done;
        self.source.finish();
        vec![
            Output::Write(vec![CAN; 8]),
            Output::Progress(
                self.progress(ProgressState::Failed, Some("cancelled by user".to_string())),
            ),
        ]
    }
}

/// X/YMODEM 分包：SOH + 块号 + 块号取反 + 128B 数据 + checksum/CRC16。
pub(crate) fn build_xy_packet(sequence: u8, data: &[u8; BLOCK_SIZE], mode: CheckMode) -> Vec<u8> {
    let mut packet = Vec::with_capacity(BLOCK_SIZE + 5);
    packet.extend_from_slice(&[SOH, sequence, !sequence]);
    packet.extend_from_slice(data);
    match mode {
        CheckMode::Checksum => {
            packet.push(data.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)))
        }
        CheckMode::Crc => packet.extend_from_slice(&crc16_xmodem(data).to_be_bytes()),
    }
    packet
}
