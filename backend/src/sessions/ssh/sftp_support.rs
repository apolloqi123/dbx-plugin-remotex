//! Sftp Support helpers for the SSH runtime.

use super::*;

/// SFTP 客户端每请求超时（秒）。russh-sftp 3.0.1 的 `request_timeout_secs`
/// 默认 10 秒且按单个请求计：慢链路/经跳板时单个写请求确认超 10 秒即整体
/// 失败，大文件上传被误杀（#206），统一放宽到 60 秒。
pub(super) const SFTP_REQUEST_TIMEOUT_SECS: u64 = 60;

/// SFTP 客户端配置（纯函数，供单测）。`compat` 为老旧服务器兼容模式
/// （M14-B）：请求/响应不做流水线并发（读写各 1 路）；非 compat 沿用
/// crate 默认并发。两种模式都放宽每请求超时（见 [`SFTP_REQUEST_TIMEOUT_SECS`]）。
pub(crate) fn sftp_client_config(compat: bool) -> SftpConfig {
    SftpConfig {
        max_concurrent_reads: if compat {
            1
        } else {
            SftpConfig::default().max_concurrent_reads
        },
        max_concurrent_writes: if compat {
            1
        } else {
            SftpConfig::default().max_concurrent_writes
        },
        request_timeout_secs: SFTP_REQUEST_TIMEOUT_SECS,
        ..SftpConfig::default()
    }
}

pub(super) fn sftp_error(error: impl std::fmt::Display) -> String {
    format!("SFTP operation failed: {error}")
}

/// 通道级（非文件级）SFTP 错误的保守判定。错误串已过 `Display`（变体信息
/// 丢失）：russh-sftp 的 `Status` 变体是服务器正常应答的文件级状态（通道
/// 健康），`I/O: …`/`Timeout`/`Unexpected …` 等形态通常意味着底层通道已死。
/// 宁可误判——误失效的代价只是下次调用多做一次握手重建；漏判的代价是该
/// 会话 SFTP 永久报错。
pub(super) fn is_channel_level_sftp_error(error: &str) -> bool {
    let lowered = error.to_ascii_lowercase();
    lowered.contains("i/o: ")
        || lowered.contains("timeout")
        || lowered.contains("unexpected packet")
        || lowered.contains("unexpected behavior")
        || lowered.contains("channel closed")
        || lowered.contains("channel open failed")
}

pub(super) fn plugin_error(error: PluginError) -> String {
    error.message
}
