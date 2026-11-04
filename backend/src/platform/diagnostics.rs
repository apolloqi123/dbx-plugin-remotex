//! Diagnostic log sinks shared by sessions and the application shell.

use crate::platform::paths::plugin_data_dir;

/// Best-effort append to `<data-dir>/rdp-debug.log`; never fails startup
/// or shutdown when the data dir is unusable.
/// 临时排障通道（rdp 路由诊断）：追加到 `<data-dir>/rdp-debug.log`。
/// 仅记录连接 id/协议/错误分类等非敏感字段，凭据（Zeroizing）永不入文件。
pub(crate) fn log_rdp_debug(message: String) {
    append_log("rdp-debug.log", message);
}

/// Lifecycle teardown/re-push decisions (ids only, never credentials).
/// `lifecycle-debug.log` is intentionally separate so SSH reconnect reports
/// do not require dragging in RDP diagnostics.
pub(crate) fn log_lifecycle_debug(message: String) {
    append_log("lifecycle-debug.log", message);
}

fn append_log(file_name: &str, message: String) {
    use std::io::Write;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let path = plugin_data_dir().join(file_name);
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(file, "{} pid={} {}", timestamp, std::process::id(), message);
}
