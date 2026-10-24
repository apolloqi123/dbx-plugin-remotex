use super::*;

use std::io::Write;
use std::time::Instant;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

/// Live recorder for one session. Fed synchronously from the session read
/// loop; `finish` flushes and returns the summary the stop RPC reports.
pub struct SessionRecorder {
    recording_id: String,
    path: PathBuf,
    file: std::io::BufWriter<std::fs::File>,
    started_at: Instant,
    started_at_ms: u64,
    connection_id: String,
    session_id: String,
    host: String,
    events: u64,
    bytes: u64,
}

impl SessionRecorder {
    /// Starts a recording, writing the v2 header. Fails when the recordings
    /// directory cannot be created or the file cannot be opened — callers
    /// surface the error instead of silently dropping output.
    pub fn start(
        data_dir: &Path,
        recording_id: &str,
        connection_id: &str,
        host: &str,
        session_id: &str,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let dir = recordings_dir(data_dir);
        // 录像是完整终端输出流（可能含回显的 token / 内联密码），目录与文件
        // 权限对齐 preferences / otp_store 的 0600 纪律，只留属主可读。
        #[cfg(unix)]
        std::fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&dir)
            .map_err(|error| format!("Failed to create recordings directory: {error}"))?;
        #[cfg(not(unix))]
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("Failed to create recordings directory: {error}"))?;
        let path = dir.join(format!("{recording_id}.cast"));
        #[cfg(unix)]
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| format!("Failed to create recording file: {error}"))?;
        #[cfg(not(unix))]
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| format!("Failed to create recording file: {error}"))?;
        let mut file = std::io::BufWriter::new(file);
        writeln!(
            file,
            "{}",
            header(connection_id, host, session_id, width, height)
        )
        .map_err(|error| format!("Failed to write recording header: {error}"))?;
        Ok(Self {
            recording_id: recording_id.to_string(),
            path,
            file,
            started_at: Instant::now(),
            started_at_ms: unix_now_secs() * 1000,
            connection_id: connection_id.to_string(),
            session_id: session_id.to_string(),
            host: host.to_string(),
            events: 0,
            bytes: 0,
        })
    }

    /// Appends one output burst as an asciicast `o` event. Write errors are
    /// swallowed (the recorder degrades to a truncated recording) — a
    /// failing disk must not kill the terminal stream.
    pub fn observe(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let data = if data.len() > MAX_EVENT_BYTES {
            &data[..MAX_EVENT_BYTES]
        } else {
            data
        };
        let text = String::from_utf8_lossy(data);
        let event = json!({
            "time": self.started_at.elapsed().as_secs_f64(),
            "eventtype": "o",
            "eventdata": text,
        });
        let _ = writeln!(self.file, "{event}");
        self.events += 1;
        self.bytes += data.len() as u64;
    }

    /// Flushes and closes the recording, returning its summary.
    pub fn finish(mut self) -> Result<Value, String> {
        let _ = self.file.flush();
        drop(self.file);
        Ok(json!({
            "recordingId": self.recording_id,
            "sessionId": self.session_id,
            "connectionId": self.connection_id,
            "host": self.host,
            "path": self.path.display().to_string(),
            "startedAt": self.started_at_ms,
            "durationSecs": self.started_at.elapsed().as_secs_f64(),
            "events": self.events,
            "bytes": self.bytes,
        }))
    }
}
