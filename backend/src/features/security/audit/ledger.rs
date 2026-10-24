use super::*;

/// Audit file path inside the plugin data directory.
pub fn audit_path(data_dir: &Path) -> PathBuf {
    data_dir.join("audit-log.jsonl")
}

/// Rotated generation (single retained predecessor).
pub(super) fn rotated_path(data_dir: &Path) -> PathBuf {
    data_dir.join("audit-log.jsonl.1")
}

/// Process-local serialization of the size-check + rotate + append sequence.
/// Cross-process safety comes from the O_APPEND single-write discipline, not
/// from this lock.
fn append_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Appends one entry as a single JSONL line. Rotates a full file first. The
/// in-process mutex serializes rotation + append; the open-append-close per
/// call keeps inter-process (embedded sidecar vs stdio `--mcp`) writes
/// line-atomic via O_APPEND (decision D5).
pub fn append(data_dir: &Path, entry: &AuditEntry) -> Result<(), String> {
    if let Some(parent) = data_dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let path = audit_path(data_dir);
    let _guard = append_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() >= ROTATE_BYTES {
            // Overwrite the previous generation; ignore failures — a stuck
            // rotation must never block auditing.
            let _ = std::fs::rename(&path, rotated_path(data_dir));
        }
    }
    // Clamp before writing so one bad string cannot blow up the line.
    let mut clamped = entry.clone();
    if let Some(error) = clamped.error.take() {
        clamped.error = Some(error.chars().take(MAX_ERROR_CHARS).collect());
    }
    if let Some(command) = clamped.command.take() {
        clamped.command = Some(command.chars().take(MAX_COMMAND_CHARS).collect());
    }
    if let Some(output) = clamped.output.take() {
        clamped.output = Some(output.chars().take(MAX_OUTPUT_CHARS).collect());
    }
    let mut line = serde_json::to_string(&clamped)
        .map_err(|error| format!("Failed to encode audit entry: {error}"))?;
    line.push('\n');
    // Fresh open-append-close per entry (decision D5). The ledger records
    // executed command text and output tails, so create it owner-only (0600)
    // like otp-entries/recordings/sudo-profiles; .mode() only applies at
    // creation, pre-existing files keep their permissions.
    use std::io::Write;
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path)
    };
    #[cfg(not(unix))]
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path);
    let mut file = file.map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
    file.write_all(line.as_bytes())
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))?;
    Ok(())
}

/// Reads the newest entries in file order (oldest → newest). Malformed lines
/// are skipped; a missing file yields an empty list. `before_ts_ms` keeps
/// strictly older entries (`ts_ms < before_ts_ms`); `limit` is clamped to
/// [`TAIL_LIMIT_CAP`] (the caller owns the protocol [1, 500] validation).
pub fn tail(
    data_dir: &Path,
    limit: usize,
    before_ts_ms: Option<u64>,
) -> Result<Vec<AuditEntry>, String> {
    let path = audit_path(data_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Failed to read {}: {error}", path.display())),
    };
    let limit = limit.min(TAIL_LIMIT_CAP);
    let mut entries: Vec<AuditEntry> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<AuditEntry>(line) {
            Ok(entry) => {
                if before_ts_ms.is_some_and(|before| entry.ts_ms >= before) {
                    continue;
                }
                entries.push(entry);
            }
            // Bad lines (torn writes, older shapes) never break replay.
            Err(_) => continue,
        }
    }
    if entries.len() > limit {
        let drop = entries.len() - limit;
        entries.drain(..drop);
    }
    Ok(entries)
}

/// Clears the audit ledger: truncates the active JSONL to empty and drops
/// the rotated generation. The file itself is kept (mode/mask intact) so a
/// concurrent O_APPEND appender never sees a missing file; entries appended
/// after the clear naturally start the new generation. Used by the
/// workbench settings「审计日志」清空按钮 via `ssh/audit/clear`.
pub fn clear(data_dir: &Path) -> Result<(), String> {
    let _guard = append_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = audit_path(data_dir);
    std::fs::write(&path, b"")
        .map_err(|error| format!("Failed to clear {}: {error}", path.display()))?;
    let _ = std::fs::remove_file(rotated_path(data_dir));
    Ok(())
}
