//! Sidecar exit and input diagnostic logs.

use crate::platform::paths::plugin_data_dir;
use super::*;

/// Best-effort append to `<data-dir>/sidecar-exit.log`; never fails startup
/// or shutdown when the data dir is unusable.

pub(super) fn log_sidecar_exit(stage: String) {
    use std::io::Write;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let path = plugin_data_dir().join("sidecar-exit.log");
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(file, "{} pid={} {}", timestamp, std::process::id(), stage);
}

/// Terminal input frames received from the host bridge (#33/#71 rapid-input
/// loss diagnosis): the count dumped to `<data-dir>/terminal-input-count.log`
/// every few seconds is the sidecar-side ground truth. Compared against what
/// the user actually typed it tells input loss before the sidecar (webview or
/// host bridge dropped frames) apart from loss after it (display side).
pub(super) static TERMINAL_INPUT_FRAMES_RECEIVED: AtomicU64 = AtomicU64::new(0);

pub(super) fn spawn_terminal_input_counter() {
    std::thread::spawn(|| {
        let mut last_dumped = 0u64;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let total = TERMINAL_INPUT_FRAMES_RECEIVED.load(Ordering::Relaxed);
            if total == last_dumped {
                continue;
            }
            last_dumped = total;
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or(0);
            let path = plugin_data_dir().join("terminal-input-count.log");
            // 一次性排障插桩也会常驻生产进程：超限截断（丢弃最旧时序，
            // 继续追加），避免长期高频使用把日志累积到几十 MB。
            const DIAG_LOG_LIMIT: u64 = 256 * 1024;
            if std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0) > DIAG_LOG_LIMIT {
                let _ = std::fs::remove_file(&path);
            }
            let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            else {
                continue;
            };
            use std::io::Write;
            let _ = writeln!(file, "{timestamp} pid={} total={total}", std::process::id());
        }
    });
}
