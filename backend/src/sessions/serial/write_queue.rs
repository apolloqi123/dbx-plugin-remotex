//! Serial session layer.

use super::*;

/// 写入来源：准入策略不同（键入可丢并回报；上传引擎输出全有或全无）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteSource {
    /// 键盘/粘贴（B1 二进制帧与 `serial/write` JSON 同源）。
    Keystroke,
    /// 上传引擎的协议块输出：保持块级原样（不切分），队列满时报错让上传
    /// 以可读错误终止，绝不阻塞调用线程。
    Bulk,
}

/// 准入拒绝原因（`message()` 面向前端/日志，绝不含写入内容）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteReject {
    /// 键入单包超过 [`SERIAL_KEYSTROKE_MAX_BYTES`]。
    Oversize,
    /// 队列字节预算（[`SERIAL_WRITE_QUEUE_BUDGET`]）容不下整包。
    QueueFull,
    /// 队列已随会话关闭。
    Closed,
}

impl WriteReject {
    pub(crate) fn message(self) -> String {
        match self {
            Self::Oversize => {
                "serial write queue: keystroke payload exceeds the per-write limit".to_string()
            }
            Self::QueueFull => "serial write queue is full".to_string(),
            Self::Closed => "serial session is closed".to_string(),
        }
    }
}

/// 有界写队列：预分块 FIFO + 字节计数 + Condvar 唤醒专用写线程。
/// 生产者只做无阻塞 try 入队——回压以「拒绝」的形式回到来源方（键入丢弃
/// 并回报、引擎输出报错终止）；消费者（写线程）`pop` 阻塞等待。准入是
/// 全有或全无：半个包入队会让线上字节流停在任意中断点，拒绝必须整体。
pub(crate) struct WriteQueue {
    inner: Mutex<WriteQueueInner>,
    available: Condvar,
}

pub(crate) struct WriteQueueInner {
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
    closed: bool,
}

impl WriteQueue {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(WriteQueueInner {
                chunks: VecDeque::new(),
                bytes: 0,
                closed: false,
            }),
            available: Condvar::new(),
        }
    }

    /// 准入（纯内存，无 IO，永不动端口锁）：键入按小块分帧，引擎输出整块
    /// 入队；预算不足/超限/已关闭时整体拒绝。
    pub(crate) fn push(&self, data: &[u8], source: WriteSource) -> Result<(), WriteReject> {
        if data.is_empty() {
            return Ok(());
        }
        if source == WriteSource::Keystroke && data.len() > SERIAL_KEYSTROKE_MAX_BYTES {
            return Err(WriteReject::Oversize);
        }
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.closed {
            return Err(WriteReject::Closed);
        }
        if inner.bytes + data.len() > SERIAL_WRITE_QUEUE_BUDGET {
            return Err(WriteReject::QueueFull);
        }
        if source == WriteSource::Bulk {
            // 引擎输出保持块级原样（协议块语义），不受切分影响；尺寸上界
            // 由引擎设计保证（≤ 约 8 KiB 的 ZMODEM 块），远小于队列预算。
            inner.chunks.push_back(data.to_vec());
            inner.bytes += data.len();
        } else {
            // 键入大包按小块分帧：单块持锁写时间有上界。
            let mut offset = 0;
            while offset < data.len() {
                let end = (offset + SERIAL_WRITE_CHUNK_BYTES).min(data.len());
                inner.chunks.push_back(data[offset..end].to_vec());
                inner.bytes += end - offset;
                offset = end;
            }
        }
        drop(inner);
        self.available.notify_one();
        Ok(())
    }

    /// 写线程取块：阻塞直到有块或队列关闭排空（None = 退出）。
    pub(crate) fn pop(&self) -> Option<Vec<u8>> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if let Some(chunk) = inner.chunks.pop_front() {
                inner.bytes = inner.bytes.saturating_sub(chunk.len());
                return Some(chunk);
            }
            if inner.closed {
                return None;
            }
            inner = self
                .available
                .wait(inner)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// 取消路径：清空尚未写出的块，返回丢弃字节数（诊断用）。
    pub(crate) fn clear(&self) -> usize {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dropped = inner.bytes;
        inner.chunks.clear();
        inner.bytes = 0;
        dropped
    }

    /// 会话关闭：清空 + 置关闭 + 唤醒，写线程排空检查后随即退出。
    pub(crate) fn shutdown(&self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.chunks.clear();
        inner.bytes = 0;
        inner.closed = true;
        drop(inner);
        self.available.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn queued_bytes(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .bytes
    }
}
