//! Serial session layer.

use super::*;

/// A candidate port after label normalization: the device path to open plus
/// an optional human-readable description for pickers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedPortLabel {
    pub(crate) path: String,
    pub(crate) description: Option<String>,
}

/// Splits a glued `path (description)` label. Some serialport backends fold
/// the OS description into the port name, which breaks both connect (the
/// glue is not a device path) and display. The split is conservative: only
/// labels ending in ")" with a " (" separator are split, and nested
/// parentheses stay with the description (`COM3 (USB Serial Port (COM3))`).
/// Anything else passes through untouched.
pub(crate) fn split_glued_description(raw: &str) -> (String, Option<String>) {
    let raw = raw.trim();
    if raw.ends_with(')') {
        if let Some(sep) = raw.find(" (") {
            let path = raw[..sep].trim();
            let description = raw[sep + 2..raw.len() - 1].trim();
            if !path.is_empty() {
                let description = (!description.is_empty()).then(|| description.to_string());
                return (path.to_string(), description);
            }
        }
    }
    (raw.to_string(), None)
}

/// Normalizes one enumeration entry; the structured description from the
/// typed port info wins over anything glued into the raw label.
pub(crate) fn normalize_port_label(raw: &str, known: Option<&str>) -> NormalizedPortLabel {
    let (path, glued) = split_glued_description(raw);
    let description = known
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .or(glued);
    NormalizedPortLabel { path, description }
}

/// Human label for a USB port: product name, then manufacturer, then a
/// VID:PID tag so pickers never show a bare path for known adapters.
pub(crate) fn usb_port_description(info: &serialport::UsbPortInfo) -> String {
    let named = info
        .product
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .or_else(|| {
            info.manufacturer
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
        });
    named
        .map(str::to_string)
        .unwrap_or_else(|| format!("USB {:04x}:{:04x}", info.vid, info.pid))
}

/// `serial/start` 响应（camelCase）。`binaryInput` 是 B1 能力探测字段
/// （设计稿 §2 兼容策略）：声明 true 表示支持 `serial/terminal/in` 二进制
/// 写通道；未声明该字段的旧 sidecar 由前端降级 JSON `serial/write`。
pub(crate) fn start_response(session_id: &str, port: &str, baud_rate: u32) -> Value {
    json!({
        "sessionId": session_id,
        "port": port,
        "baudRate": baud_rate,
        "binaryInput": true,
    })
}
