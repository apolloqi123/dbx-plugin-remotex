//! Extended SFTP operation layer.

// ---------------------------------------------------------------------------
// Helpers (pure, unit-testable)
// ---------------------------------------------------------------------------

/// Octal permission string in `0755` style (type bits masked away).
pub(crate) fn format_mode(value: u32) -> String {
    format!("{:04o}", value & 0o7777)
}

pub(crate) fn current_unix_secs() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as u32)
        .unwrap_or(0)
}

pub(crate) fn sftp_error(error: impl std::fmt::Display) -> String {
    format!("SFTP operation failed: {error}")
}

/// Joins a normalized directory with a bare file name (`/` root included).
pub(crate) fn join_remote_name(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}
