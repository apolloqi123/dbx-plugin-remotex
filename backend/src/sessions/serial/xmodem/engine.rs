//! Serial upload engine layer.

use super::*;

// —— 统一引擎门面 ————————————————————————————————————————————————

/// serial_session 只与它交互；三协议共用同一组入口。
#[derive(Debug)]
pub enum UploadEngine {
    Xy(Box<XyEngine>),
    Z(Box<ZEngine>),
}

impl UploadEngine {
    /// 创建引擎并返回初始输出（ZMODEM 立即发 ZRQINIT；X/Y 静默等握手）。
    pub fn new(
        protocol: UploadProtocol,
        file_name: String,
        total: u64,
        now: Instant,
    ) -> (Self, Vec<Output>) {
        let source = UploadSource::new(file_name, total);
        match protocol {
            UploadProtocol::Zmodem => {
                let (engine, outputs) = ZEngine::new(source, now);
                (Self::Z(Box::new(engine)), outputs)
            }
            _ => (
                Self::Xy(Box::new(XyEngine::new(protocol, source, now))),
                Vec::new(),
            ),
        }
    }

    /// 对端字节流（终端照常上屏，引擎按状态选择性消费）。
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Output> {
        let now = Instant::now();
        match self {
            Self::Xy(engine) => engine.feed_at(bytes, now),
            Self::Z(engine) => engine.feed_at(bytes, now),
        }
    }

    /// 空闲滴答：驱动静默重发/失败判定。
    pub fn tick(&mut self) -> Vec<Output> {
        let now = Instant::now();
        match self {
            Self::Xy(engine) => engine.tick_at(now),
            Self::Z(engine) => engine.tick_at(now),
        }
    }

    /// 前端分块到达；可能解锁挂起的数据读请求。
    pub fn append_data(&mut self, bytes: &[u8], final_chunk: bool) -> Result<Vec<Output>, String> {
        let now = Instant::now();
        {
            let source = match self {
                Self::Xy(engine) => &mut engine.source,
                Self::Z(engine) => &mut engine.source,
            };
            source.append(bytes)?;
            if final_chunk {
                source.finish();
            }
        }
        match self {
            Self::Xy(engine) => Ok(engine.append_data_at(now)),
            Self::Z(engine) => Ok(engine.append_data_at(now)),
        }
    }

    /// 用户取消：发取消序列并落 Failed 事件（已结束则幂等）。
    pub fn cancel(&mut self) -> Vec<Output> {
        match self {
            Self::Xy(engine) => engine.cancel_at(),
            Self::Z(engine) => engine.cancel_at(),
        }
    }

    pub fn is_done(&self) -> bool {
        match self {
            Self::Xy(engine) => engine.is_done(),
            Self::Z(engine) => engine.is_done(),
        }
    }

    pub fn source(&self) -> &UploadSource {
        match self {
            Self::Xy(engine) => &engine.source,
            Self::Z(engine) => &engine.source,
        }
    }
}
