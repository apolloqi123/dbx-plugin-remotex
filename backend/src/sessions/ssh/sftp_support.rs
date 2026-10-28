//! Sftp Support helpers for the SSH runtime.

use super::*;

/// SFTP 客户端每请求超时（秒）。russh-sftp 3.0.1 的 `request_timeout_secs`
/// 默认 10 秒且按单个请求计：慢链路/经跳板时单个写请求确认超 10 秒即整体
/// 失败，大文件上传被误杀（#206），统一放宽到 60 秒。
pub(super) const SFTP_REQUEST_TIMEOUT_SECS: u64 = 60;

/// 下载读流水线深度（crate 默认 16）。每个 READ 上限 256KiB（crate 默认
/// `max_packet_len`，并按服务端 `limits@openssh.com` 的 read 上限收窄），
/// 32 路在途 ≈ 8MiB 预读；配合 16MiB 接收窗口，40ms RTT 下单文件下载
/// 实测 43–83 MB/s（2MiB 窗口时约 17 MB/s）。再深只会让断点/取消后要
/// 丢弃的在途数据更多。兼容模式强制 1（老旧服务器不并发请求）。
///
/// 流水线也是每请求超时的“排队深度”：russh-sftp 在**创建**请求时就开始
/// 计 60s 超时，排在队尾的请求要等前面的确认落地才被轮询。8MiB 预读在
/// ≥0.55 Mbps 链路上都能在 60s 内轮到；再提高深度会把更慢的链路拖进
/// 超时（见 [`SFTP_WRITE_PIPELINE`] 的同样取舍）。
const SFTP_READ_PIPELINE: usize = 32;

/// 上传写流水线深度（crate 默认 16）。写包 32KiB 时 64 路 ≈ 2MiB 在途：
/// 服务端的接收窗口（实测约 0.8MiB/RTT）成为上限前，先把客户端侧的
/// “逐包等确认”彻底消除；40ms RTT 下单文件上传 14.6 → 22.6 MB/s（128 路
/// 实测同为 22.2–22.8，没有额外收益）。60s 请求超时从创建时开始计，
/// 2MiB 队列把可用链路的门槛放在约 0.27 Mbps——更深的队列会误杀超低速
/// 上传（#206 同类问题），所以不取 128。
const SFTP_WRITE_PIPELINE: usize = 64;

/// SFTP 客户端配置（纯函数，供单测）。`compat` 为老旧服务器兼容模式
/// （M14-B）：请求/响应不做流水线并发（读写各 1 路）；非 compat 使用
/// [`SFTP_READ_PIPELINE`]/[`SFTP_WRITE_PIPELINE`]。两种模式都放宽每请求
/// 超时（见 [`SFTP_REQUEST_TIMEOUT_SECS`]）。写包保持 crate 默认 32KiB——
/// 实测 256KiB 写包在高 RTT 下并不更快，且部分老旧 sftp-server 只接受小包。
pub(crate) fn sftp_client_config(compat: bool) -> SftpConfig {
    SftpConfig {
        max_concurrent_reads: if compat { 1 } else { SFTP_READ_PIPELINE },
        max_concurrent_writes: if compat { 1 } else { SFTP_WRITE_PIPELINE },
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
