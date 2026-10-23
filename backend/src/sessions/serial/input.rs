//! Serial session layer.

use super::*;

/// Base64 解码复用 telnet 的实现（同一负载形状）。
pub fn decode_write_payload(data_base64: &str) -> Result<Vec<u8>, String> {
    crate::sessions::telnet::decode_write_payload(data_base64)
}

/// 解码一条 `serial/terminal/in/{id}` 入站写帧：与输出帧同构的
/// `TerminalFrame` 形状（首字节流标签 + 大端 `u64` 序号 + 数据）。
///
/// B1 契约（docs/architecture/SERIAL_ENHANCE_DESIGN.zh-CN.md §2）：本通道只接受
/// `Stdin = 3` 标签，其余标签（含未知值）一律返回参数错误；长度不足帧头的
/// 数据同样报参数错误，绝不 panic。
pub(crate) fn decode_input_frame(data: &[u8]) -> Result<(u64, Vec<u8>), String> {
    if data.len() < 9 {
        return Err(
            "serial/terminal/in: frame is shorter than the 9-byte TerminalFrame header".to_string(),
        );
    }
    let tag = data[0];
    if tag != TerminalStream::Stdin as u8 {
        return Err(format!(
            "serial/terminal/in: stream tag {tag} is rejected (only {} / Stdin is accepted)",
            TerminalStream::Stdin as u8
        ));
    }
    let sequence = u64::from_be_bytes(
        data[1..9]
            .try_into()
            .map_err(|_| "serial/terminal/in: invalid sequence".to_string())?,
    );
    Ok((sequence, data[9..].to_vec()))
}
