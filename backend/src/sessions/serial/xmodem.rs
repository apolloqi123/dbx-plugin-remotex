//! Serial-port file upload engines: XMODEM / YMODEM / ZMODEM (NyaTerm parity).
//!
//! Pure state machines: the caller (the serial read loop in
//! [`crate::sessions::serial`]) feeds peer bytes in and drains [`Output`] items
//! (wire bytes to write to the port + progress events) back out. No engine
//! touches the port, the write channel, or the wall clock directly — public
//! entry points use `Instant::now()` while `*_at` variants take an injected
//! `Instant`, so every path is unit-testable without hardware.
//!
//! Wire semantics follow the NyaTerm reference (X/YMODEM per its
//! `serial/xymodem.rs`; ZMODEM per its vendored `zmodem2` sender, re-written
//! here by hand — the vendor crate is MIT/Apache-2.0 but stays out of the
//! dependency tree). Notable behaviors kept for interoperability with
//! `lrzsz`/`rz` receivers:
//!
//! * XMODEM 128-byte blocks, sequence from 1, CRC16 (`C`) or 8-bit checksum
//!   (`NAK`) per handshake, CPM-EOF (0x1A) tail padding, EOT + ACK close.
//! * YMODEM batch shape: block 0 header (`name\0size` zero-padded, CRC only),
//!   data blocks from sequence 1, EOT retry on NAK, then a final all-zero
//!   block 0 closes the batch. One file per upload session.
//! * ZMODEM sender: ZRQINIT(hex) → ZFILE(bin32 CRC + ZCRCW subpacket) →
//!   ZDATA(bin32, one ZCRCW subpacket per ZRPOS/ZACK) → ZEOF → ZFIN(hex) →
//!   `OO`. ZDLE escaping (CR/XON/XOFF/DEL/high-bit twins) matches the vendor
//!   sender so receivers that accept NyaTerm accept this engine.
//! * Upload data streams in from the frontend (`serial/upload/data`); reads
//!   that outrun the stream park the engine (`Pending`) instead of emitting a
//!   truncated frame, and idle-based timeouts pause while parked.

use std::time::{Duration, Instant};

mod engine;
mod types;
mod xy;
mod zmodem;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use engine::*;
#[allow(unused_imports)]
pub use types::*;
#[allow(unused_imports)]
pub use xy::*;
#[allow(unused_imports)]
pub use zmodem::*;

// —— 协议常量（X/YMODEM 与 NyaTerm 对齐）——————————————————————————
const SOH: u8 = 0x01;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const CRC_REQUEST: u8 = b'C';
const CPM_EOF: u8 = 0x1a;
const BLOCK_SIZE: usize = 128;

// —— ZMODEM 帧类型/编码（值与 zmodem2 语义一致）—————————————————————
const ZDLE: u8 = 0x18;
const XON: u8 = 0x11;
const ZPAD: u8 = b'*';
const FRAME_ZRQINIT: u8 = 0;
const FRAME_ZRINIT: u8 = 1;
const FRAME_ZACK: u8 = 3;
const FRAME_ZFILE: u8 = 4;
const FRAME_ZSKIP: u8 = 5;
const FRAME_ZNAK: u8 = 6;
const FRAME_ZABORT: u8 = 7;
const FRAME_ZFIN: u8 = 8;
const FRAME_ZRPOS: u8 = 9;
const FRAME_ZDATA: u8 = 10;
const FRAME_ZEOF: u8 = 11;
const FRAME_ZFERR: u8 = 12;
const FRAME_ZCAN: u8 = 16;
const SUBPACKET_ZCRCW: u8 = 0x6b;
/// ZFILE 标志取 zmodem2 默认：ZF0=ZCBIN（二进制直传）。
const ZFILE_ZCBIN_FLAGS: [u8; 4] = [1, 0, 0, 0];
/// ZFILE payload：`name\0{size} {mtime:o} {mode:o} 0 0 0\0`（mtime 未知传 0）。
const ZFILE_DEFAULT_MODE: u32 = 0o100644;

/// 静默窗口：超窗即重发最后一帧（握手态直接判失败）。
const IDLE_RETRY: Duration = Duration::from_secs(10);
/// 重发上限，超过即放弃（与 NyaTerm MAX_RETRIES 一致）。
const MAX_RETRIES: u8 = 10;
/// 单个 ZDATA 子包上限：对端 ZRINIT 未声明缓冲时按 zmodem2 的 8 KiB 兜底。
const ZMODEM_MAX_SUBPACKET: usize = 8 * 1024;
/// 对端连续取消字节阈值（ZMODEM 计 ZDLE，X/Y 计 CAN）。
const XY_CANCEL_COUNT: usize = 2;
const Z_CANCEL_COUNT: usize = 5;
/// 两个有效帧头之间连续坏帧（ZNAK）上限：恶意对端的持续垃圾流会让每个坏帧
/// 都回一个 ZNAK 且刷新 last_activity（永不超时 + 字节写放大）；与
/// MAX_RETRIES 同语义，收到任一有效帧头即复位。
const MAX_CORRUPT_FRAMES: usize = 32;

/// RPC 入口约束：前端 File API 分块 ≤64 KiB，单次上传总量 ≤256 MiB。
pub const UPLOAD_CHUNK_LIMIT: usize = 64 * 1024;
pub const MAX_UPLOAD_BYTES: u64 = 256 * 1024 * 1024;
/// 进度事件限流：事件走宿主桥 JSON，128B 块逐块上报过于密集。
pub const PROGRESS_DELTA_BYTES: u64 = 4096;
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// CRC-32/ISO-HDLC（zlib 兼容）。
fn crc32_iso_hdlc(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
