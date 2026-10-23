//! Serial upload engine layer.

use super::*;

/// 上传协议；wire 拼写为 `zmodem`（不是历史笔误 `zodem`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadProtocol {
    Xmodem,
    Ymodem,
    Zmodem,
}

impl UploadProtocol {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "xmodem" => Ok(Self::Xmodem),
            "ymodem" => Ok(Self::Ymodem),
            "zmodem" => Ok(Self::Zmodem),
            other => Err(format!(
                "serial/upload/start: invalid protocol \"{other}\" (expected xmodem, ymodem or zmodem)"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Xmodem => "xmodem",
            Self::Ymodem => "ymodem",
            Self::Zmodem => "zmodem",
        }
    }
}

/// 进度事件状态（`serial/upload/progress` 的 `state` 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    Running,
    FileComplete,
    Complete,
    Failed,
}

impl ProgressState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::FileComplete => "file_complete",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }
}

/// 进度事件负载；由 serial_session 拼成 JSON（不含文件内容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferProgress {
    pub protocol: UploadProtocol,
    pub file_name: String,
    pub file_index: u32,
    pub sent: u64,
    pub total: u64,
    pub state: ProgressState,
    pub reason: Option<String>,
}

/// 状态机输出：写往串口的字节，或要上报的进度事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    Write(Vec<u8>),
    Progress(TransferProgress),
}

/// 上传数据源：前端经 `serial/upload/data` 分块送入的字节缓冲。引擎按需
/// 读取（X/Y 顺序 128B，Z 按 ZRPOS 偏移），请求段未到齐且未收尾时返回
/// `Pending`，引擎暂停等待而不误判文件尾。
#[derive(Debug, Default)]
pub struct UploadSource {
    file_name: String,
    total: u64,
    data: Vec<u8>,
    complete: bool,
}

/// [`UploadSource::read_upto`] 的结果。
#[derive(Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    Ready(Vec<u8>),
    Pending,
    Eof,
}

impl UploadSource {
    pub fn new(file_name: String, total: u64) -> Self {
        Self {
            file_name,
            total,
            data: Vec::new(),
            complete: false,
        }
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    /// 已从 RPC 分块收到的字节数（进度回显用）。
    pub fn received(&self) -> u64 {
        self.data.len() as u64
    }

    /// 追加分块；超过 256 MiB 总量上限即拒绝。
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), String> {
        let next = self.data.len() + bytes.len();
        if next as u64 > MAX_UPLOAD_BYTES {
            return Err(format!(
                "serial/upload/data: upload exceeds the {MAX_UPLOAD_BYTES} byte limit"
            ));
        }
        self.data.extend_from_slice(bytes);
        Ok(())
    }

    pub fn finish(&mut self) {
        self.complete = true;
    }

    /// 读取 `offset` 起至多 `max` 字节。只有当请求段完整可得（或源已收尾）
    /// 才返回 `Ready`，避免把在途分块当成文件尾。
    pub fn read_upto(&self, offset: u64, max: usize) -> Result<ReadOutcome, String> {
        let offset = offset as usize;
        if offset > self.data.len() {
            return Ok(if self.complete {
                ReadOutcome::Eof
            } else {
                ReadOutcome::Pending
            });
        }
        let available = self.data.len() - offset;
        if available == 0 {
            return Ok(if self.complete {
                ReadOutcome::Eof
            } else {
                ReadOutcome::Pending
            });
        }
        if available < max && !self.complete {
            return Ok(ReadOutcome::Pending);
        }
        let end = offset + max.min(available);
        Ok(ReadOutcome::Ready(self.data[offset..end].to_vec()))
    }
}
