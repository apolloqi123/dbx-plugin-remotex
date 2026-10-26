//! Chunked local uploads (`docker/files/upload/*`): one temp file per
//! upload, strictly sequential offsets, 512 MiB cap, at most four active
//! uploads and a 30-minute TTL for abandoned ones.
//!
//! The registry only ever writes inside its own `dbx-sshx-upload-<uuid>`
//! directory; the finished file is a normal local staging path, so the
//! shared [`validate_staging_path`](super::validate_staging_path) gate
//! accepts it for `import`/`cleanup` without special cases.

use super::*;

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use uuid::Uuid;

/// Per-file upload ceiling.
pub const UPLOAD_MAX_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum decoded bytes accepted per `upload/chunk` call.
pub const UPLOAD_CHUNK_BYTES: usize = 256 * 1024;
/// Concurrent (unfinished) uploads per sidecar.
pub const UPLOAD_MAX_ACTIVE: usize = 4;
/// Abandoned uploads expire and their temp directory is removed.
pub const UPLOAD_TTL: Duration = Duration::from_secs(30 * 60);
/// Prefix of the generated per-upload directory.
pub const UPLOAD_DIR_PREFIX: &str = "dbx-sshx-upload-";

struct UploadEntry {
    path: PathBuf,
    dir: PathBuf,
    file: Option<File>,
    received: u64,
    finished: bool,
    last_used: Instant,
}

/// In-memory table of in-flight uploads (one per sidecar).
pub struct UploadRegistry {
    base: PathBuf,
    entries: HashMap<String, UploadEntry>,
}

impl UploadRegistry {
    /// `base` is the system temp directory in production; tests pass a
    /// tempdir so nothing touches the real temp tree.
    pub fn new(base: PathBuf) -> Self {
        Self {
            base,
            entries: HashMap::new(),
        }
    }

    /// Creates a temp file and returns the upload id.
    pub fn begin(&mut self, name: &str) -> Result<String, String> {
        self.sweep();
        let active = self.entries.values().filter(|entry| !entry.finished).count();
        if active >= UPLOAD_MAX_ACTIVE {
            return Err(format!(
                "Too many active uploads (max {UPLOAD_MAX_ACTIVE}); finish or cancel one first"
            ));
        }
        let id = Uuid::new_v4().to_string();
        let dir = self.base.join(format!("{UPLOAD_DIR_PREFIX}{id}"));
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("Failed to create upload directory: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        let path = dir.join(safe_staging_name(name));
        let file = File::create(&path)
            .map_err(|error| format!("Failed to create upload file: {error}"))?;
        self.entries.insert(
            id.clone(),
            UploadEntry {
                path,
                dir,
                file: Some(file),
                received: 0,
                finished: false,
                last_used: Instant::now(),
            },
        );
        Ok(id)
    }

    /// Checks that `bytes` more would stay under [`UPLOAD_MAX_BYTES`].
    pub fn reserve(&mut self, id: &str, bytes: u64) -> Result<(), String> {
        let entry = self.entry(id)?;
        if entry.received.saturating_add(bytes) > UPLOAD_MAX_BYTES {
            return Err(format!(
                "Upload exceeds the {UPLOAD_MAX_BYTES}-byte limit"
            ));
        }
        Ok(())
    }

    /// Appends one chunk; `offset` must equal the bytes received so far.
    /// Returns the new total.
    pub fn chunk(&mut self, id: &str, offset: u64, data: &[u8]) -> Result<u64, String> {
        self.reserve(id, data.len() as u64)?;
        let entry = self.entry(id)?;
        if entry.finished {
            return Err("Upload is already finished".to_string());
        }
        if offset != entry.received {
            return Err(format!(
                "Invalid upload offset {offset}: expected {}",
                entry.received
            ));
        }
        let file = entry
            .file
            .as_mut()
            .ok_or_else(|| "Upload has no open file".to_string())?;
        file.write_all(data)
            .map_err(|error| format!("Failed to write upload chunk: {error}"))?;
        entry.received += data.len() as u64;
        entry.last_used = Instant::now();
        Ok(entry.received)
    }

    /// Flushes and closes the file, returning its staging path and size.
    /// The entry stays registered so the caller can `import` then
    /// `cleanup`; it no longer occupies an active upload slot.
    pub fn finish(&mut self, id: &str) -> Result<(String, u64), String> {
        let entry = self.entry(id)?;
        if entry.finished {
            return Err("Upload is already finished".to_string());
        }
        let mut file = entry
            .file
            .take()
            .ok_or_else(|| "Upload has no open file".to_string())?;
        file.flush()
            .map_err(|error| format!("Failed to flush upload: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Failed to close upload: {error}"))?;
        drop(file);
        entry.finished = true;
        entry.last_used = Instant::now();
        Ok((entry.path.to_string_lossy().into_owned(), entry.received))
    }

    /// Drops the upload and removes its temp directory.
    pub fn cancel(&mut self, id: &str) -> Result<(), String> {
        let mut entry = self
            .entries
            .remove(id)
            .ok_or_else(|| format!("Unknown uploadId '{id}'"))?;
        drop(entry.file.take());
        let _ = std::fs::remove_dir_all(&entry.dir);
        Ok(())
    }

    fn entry(&mut self, id: &str) -> Result<&mut UploadEntry, String> {
        self.entries
            .get_mut(id)
            .ok_or_else(|| format!("Unknown uploadId '{id}'"))
    }

    /// Removes entries untouched for [`UPLOAD_TTL`], including their temp
    /// directories. Called on `begin`; an idle registry costs nothing.
    fn sweep(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, entry| {
            if now.duration_since(entry.last_used) < UPLOAD_TTL {
                return true;
            }
            drop(entry.file.take());
            let _ = std::fs::remove_dir_all(&entry.dir);
            false
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_out_of_order_offsets() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = UploadRegistry::new(dir.path().to_path_buf());
        let id = registry.begin("report.log").unwrap();
        assert!(registry.chunk(&id, 1, b"x").is_err());
        assert!(registry.chunk(&id, 0, b"abc").is_ok());
        assert!(registry.chunk(&id, 0, b"dup").is_err());
    }

    #[test]
    fn rejects_oversized_upload() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = UploadRegistry::new(dir.path().to_path_buf());
        let id = registry.begin("big.bin").unwrap();
        assert!(registry.reserve(&id, UPLOAD_MAX_BYTES + 1).is_err());
    }

    #[test]
    fn finishes_and_cancels_uploads() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = UploadRegistry::new(dir.path().to_path_buf());
        let id = registry.begin("report.log").unwrap();
        registry.chunk(&id, 0, b"hello").unwrap();
        let (staging, size) = registry.finish(&id).unwrap();
        assert_eq!(size, 5);
        assert_eq!(std::fs::read(&staging).unwrap(), b"hello");
        // A finished upload no longer occupies an active slot.
        for index in 0..UPLOAD_MAX_ACTIVE {
            registry.begin(&format!("extra-{index}")).unwrap();
        }
        assert!(registry.begin("overflow").is_err());
        let upload_dir = std::path::Path::new(&staging)
            .parent()
            .unwrap()
            .to_path_buf();
        registry.cancel(&id).unwrap();
        assert!(!upload_dir.exists());
    }

    #[test]
    fn unknown_upload_ids_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = UploadRegistry::new(dir.path().to_path_buf());
        assert!(registry.chunk("missing", 0, b"x").is_err());
        assert!(registry.finish("missing").is_err());
        assert!(registry.cancel("missing").is_err());
    }
}
