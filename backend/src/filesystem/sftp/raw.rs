//! 最小顺序式 SFTP v3 裸包客户端（M14-B）：为「非 UTF-8 文件名」保住原始字节。
//!
//! 上游 `russh-sftp` 在反序列化层对文件名/handle 做 `from_utf8_lossy`，合法
//! UTF-8 服务器下字节往返无损，但 latin-1 等旧编码的文件名在列表阶段就丢失
//! 原始字节。本模块只覆盖字节保真必需的最小操作面（INIT/OPENDIR/READDIR/
//! CLOSE/STAT/LSTAT/OPEN/READ/CLOSE + M15-B 的 REMOVE/MKDIR/RMDIR/RENAME +
//! M16 的 OPEN 写/WRITE/SETSTAT/READLINK/SYMLINK，覆盖上传族/直写/touch/
//! symlink 的写路径），全部请求严格串行（发一收一），不与高层
//! `SftpSession` 共享通道。文件名字节原样返回，编码解释交给 `sftp_name`。
//!
//! 已知边界：attrs 只按 v3 布局解析（INIT 显式请求版本 3，RFC 要求服务器
//! 回应版本不高于请求值）；本模块不发起任何 extended 请求（兼容模式友好）。
//!
//! 真机回归（CI ssh-smoke run 36146663371）：MKDIR 的 wire 布局必须携带
//! ATTRS 字段（draft-ietf-secsh-filexfer-02 §5.2）——OpenSSH sftp-server
//! 对缺失 attrs 的 MKDIR 直接 fatal 退出，客户端侧表现为通道 EOF
//! （"SFTP raw read failed: early eof"）。测试模块的严格一致性桩
//! （`test_support::validate_request`）把这一语义搬进了离线回归。

use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

mod client;
mod packet;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use client::*;
#[allow(unused_imports)]
pub use packet::*;

/// 单个 SFTP 包上限（readdir 批量 NAME 包可能较大；OpenSSH 上限 256 KiB，
/// 这里放宽到 4 MiB 防御非标准服务器）。
const MAX_PACKET_LEN: usize = 4 * 1024 * 1024;
/// READDIR 批次条目上限（防御恶意/异常服务器撑爆内存）。
const MAX_READDIR_ENTRIES: usize = 100_000;
/// 每个请求的等待上限，与高层会话的 10s 语义一致。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

// 包类型常量（draft-ietf-secsh-filexfer-02 §3）。
const FXP_INIT: u8 = 1;
const FXP_VERSION: u8 = 2;
const FXP_OPEN: u8 = 3;
const FXP_CLOSE: u8 = 4;
const FXP_READ: u8 = 5;
const FXP_WRITE: u8 = 6;
const FXP_LSTAT: u8 = 7;
const FXP_SETSTAT: u8 = 9;
const FXP_OPENDIR: u8 = 11;
// READDIR 是 12（draft-ietf-secsh-filexfer-02 §3）：16 是 REALPATH——曾误写成
// 16，发出的帧被 OpenSSH sftp-server 当 REALPATH 解析（handle 当路径，含
// NUL 字节），进程 fatal 退出即通道 EOF（真机表现为 "SFTP raw read failed:
// early eof"）。
const FXP_READDIR: u8 = 12;
const FXP_REMOVE: u8 = 13;
const FXP_MKDIR: u8 = 14;
const FXP_RMDIR: u8 = 15;
const FXP_STAT: u8 = 17;
const FXP_RENAME: u8 = 18;
const FXP_READLINK: u8 = 19;
const FXP_SYMLINK: u8 = 20;
const FXP_STATUS: u8 = 101;
const FXP_HANDLE: u8 = 102;
const FXP_DATA: u8 = 103;
const FXP_NAME: u8 = 104;
const FXP_ATTRS: u8 = 105;

// STATUS 错误码。
const SSH_FX_EOF: u32 = 1;
/// SSH_FX_NO_SUCH_FILE（draft-ietf-secsh-filexfer-02 §7）：`sftp_exists`
/// 等「存在性」语义只认这个码，其余错误必须如实上抛。
pub const SSH_FX_NO_SUCH_FILE: u32 = 2;

// ATTRS 标志位（v3）。
const ATTR_SIZE: u32 = 0x1;
const ATTR_UIDGID: u32 = 0x2;
const ATTR_PERMISSIONS: u32 = 0x4;
const ATTR_ACMODTIME: u32 = 0x8;
const ATTR_EXTENDED: u32 = 0x10;

// OPEN pflags。
const PFLAGS_READ: u32 = 0x1;
const PFLAGS_WRITE: u32 = 0x2;
const PFLAGS_CREAT: u32 = 0x8;
const PFLAGS_TRUNC: u32 = 0x10;

/// WRITE 单包数据上限：SFTPv3 规范建议 ≤32768 以保证最大兼容（OpenSSH 的
/// 包上限是 256 KiB，但旧服务器可能更小），调用方按此切分数据。
pub const MAX_WRITE_CHUNK: usize = 32 * 1024;

/// READ 单请求建议长度：与写侧同取 v3 规范建议的 32 KiB（`read_file` 的
/// 分块粒度；workbench 下载逐 chunk 的 256 KiB 是高层客户端语义，这里按
/// 裸包最大兼容口径）。
const READ_CHUNK: usize = 32 * 1024;

/// 内存桩的跨模块复用（`#[cfg(test)]`）：mcp.rs 的 M18 工具级闭环测试用
/// 同一套桩服务器与回包构造器做 latin-1 往返验证，不连真实 SSH。
#[cfg(test)]
pub(crate) mod test_support;
