//! Metrics support.

use super::*;

/// Reads and hashes `path`. `None` when the file is gone, unreadable, or not
/// a regular file — the caller treats `None` as "cannot confirm a change"
/// (and `watch/start` as "nothing to watch"). Files over [`MAX_HASH_BYTES`]
/// keep a size+mtime fingerprint (`sha256: None`) instead: the baseline must
/// exist for large downloads or external editing breaks above 64 MiB.
pub fn file_fingerprint(path: &Path) -> Option<FileFingerprint> {
    file_fingerprint_with_limit(path, MAX_HASH_BYTES)
}

/// [`file_fingerprint`] with an injectable hash cap — the seam the watcher
/// tests use to exercise the oversized branch without a 64 MiB fixture.
pub(crate) fn file_fingerprint_with_limit(
    path: &Path,
    max_hash_bytes: u64,
) -> Option<FileFingerprint> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let len = metadata.len();
    let modified_ms = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    if len > max_hash_bytes {
        return Some(FileFingerprint {
            len,
            modified_ms,
            sha256: None,
        });
    }
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut read_total = 0u64;
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        read_total += read as u64;
    }
    // The file may have been truncated between metadata() and the read loop;
    // a snapshot whose byte count no longer matches its own metadata is not a
    // trustworthy baseline.
    if read_total != len {
        return None;
    }
    let sha256: [u8; 32] = hasher.finalize().into();
    Some(FileFingerprint {
        len,
        modified_ms,
        sha256: Some(sha256),
    })
}

/// Pure comparison core of the emit decision (unit-tested without IO).
pub fn classify_change(
    baseline: &Option<FileFingerprint>,
    current: &Option<FileFingerprint>,
) -> ChangeVerdict {
    let (Some(baseline), Some(current)) = (baseline, current) else {
        return ChangeVerdict::Undeterminable;
    };
    if baseline.len != current.len {
        return ChangeVerdict::ContentChanged;
    }
    if baseline.modified_ms == current.modified_ms {
        return ChangeVerdict::Same;
    }
    // Oversized snapshots (sha256 None) have no content proof either way:
    // with len equal and mtime ticked, mtime IS the decision — a coarse but
    // working signal where refusing to decide would drop real saves.
    if let (Some(baseline_hash), Some(current_hash)) = (baseline.sha256, current.sha256) {
        if baseline_hash == current_hash {
            return ChangeVerdict::Same;
        }
    }
    ChangeVerdict::ContentChanged
}

/// Runs [`file_fingerprint`] on the blocking pool: hashing up to 64 MiB with
/// SHA-256 must not stall the async worker between editor saves. Dedup
/// semantics are unchanged — only the execution site moves; a failed join is
/// collapsed into `None`, which means "cannot confirm a change" exactly like
/// an unreadable file.
pub(crate) async fn spawn_fingerprint(path: PathBuf) -> Option<FileFingerprint> {
    tokio::task::spawn_blocking(move || file_fingerprint(&path))
        .await
        .ok()
        .flatten()
}
