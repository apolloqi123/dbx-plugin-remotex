//! Serial-port sessions (RS-232 consoles: routers, dev boards, embeddeds).
//!
//! Desktop-only by nature: the sidecar must sit on the machine that owns the
//! port. Mirrors [`crate::sessions::telnet`] — same 9-byte `TerminalFrame`
//! output channel (`serial/terminal/out/{id}`), same state-event shape
//! (`serial/session/state`), same session-table lifecycle. The blocking
//! `serialport` handle lives on a dedicated OS read thread; writes go through
//! a shared mutex (keystroke-sized, so the brief std lock on the async side
//! is acceptable for an MVP).
//!
//! `BackspaceMode` reuses the telnet mapping: `ctrl_h` rewrites DEL (0x7F)
//! into BS (0x08) for devices that expect a vt100-style erase.
//!
//! Line parameters are validated strictly (unknown values fail the start
//! with a readable error instead of silently degrading to 8N1), and port
//! enumeration labels are normalized so `path (description)` glue from some
//! backends never leaks into the device path the UI dials.

pub(crate) mod detect;
pub(crate) mod xmodem;

use crate::sessions::serial::xmodem as serial_xmodem;

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::RwLock;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

mod input;
mod labels;
mod params;
mod pump;
mod runtime;
mod types;
mod upload;
mod write_queue;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use input::*;
#[allow(unused_imports)]
pub use labels::*;
#[allow(unused_imports)]
pub use params::*;
#[allow(unused_imports)]
pub use pump::*;
#[allow(unused_imports)]
pub use runtime::*;
#[allow(unused_imports)]
pub use types::*;
#[allow(unused_imports)]
pub use upload::*;
#[allow(unused_imports)]
pub use write_queue::*;

use dbx_plugin_sdk::PluginEmitter;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::core::model::TerminalStream;
use crate::sessions::serial::xmodem::{
    Output, ProgressState, TransferProgress, UploadEngine, UploadProtocol,
};

const READ_BUFFER: usize = 4096;
/// The blocking read timeout also bounds how long a close can stall.
const READ_TIMEOUT: Duration = Duration::from_millis(10);
/// Serial replay budget (design doc §3): per-frame bounded ring kept for the
/// whole session lifetime, far smaller than the shared 2 MiB terminal limit —
/// serial output is console chatter, not full-screen repaints.
const SERIAL_REPLAY_BYTE_LIMIT: usize = 128 * 1024;

/// 写序列化常量（设计稿「写序列化与回压」节，草案参数实施定稿值）：
/// - 键入大包按 4 KiB 小块分帧入队，单块持锁写时间有上界（@9600 波特约
///   4 秒，键入与引擎 ACK 不再被 64 KiB 大块饿死 66 秒）；
/// - 队列按 256 KiB 字节预算有界，满时按来源拒绝而非无界堆积；
/// - 键入单包上限 16 KiB：覆盖 xterm 括号粘贴突发，更大的编程性写入整包
///   丢弃并向前端回报丢弃事件。
const SERIAL_WRITE_CHUNK_BYTES: usize = 4 * 1024;
const SERIAL_WRITE_QUEUE_BUDGET: usize = 256 * 1024;
const SERIAL_KEYSTROKE_MAX_BYTES: usize = 16 * 1024;
