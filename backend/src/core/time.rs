//! Shared wall-clock helpers.

/// Seconds since the Unix epoch; a clock before the epoch collapses to `0`
/// so callers never see a panic or a negative timestamp.
pub(crate) fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Milliseconds since the Unix epoch; same pre-epoch fallback as [`unix_now_secs`].
pub(crate) fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
