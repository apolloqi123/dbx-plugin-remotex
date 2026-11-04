//! Clipboard RPC surface: text (`rdp/set-clipboard`) and the CLIPRDR file
//! transfer endpoints (`rdp/clipboard-files/*`).

use crate::platform::diagnostics::log_rdp_debug;
use crate::sessions::rdp::session::*;

impl RdpSessionRuntime {
    /// Local clipboard → remote (`rdp/set-clipboard`): stage the text and
    /// advertise it as CF_UNICODETEXT.
    pub async fn set_clipboard(&self, session_id: &str, text: String) -> Result<(), String> {
        let entry = self.session(session_id).await?;
        if !entry.clipboard_enabled {
            return Err(
                "rdp/set-clipboard: clipboard redirection is disabled for this session".to_string(),
            );
        }
        // Finder file copies also expose their names as text. The sidecar owns
        // the native file advertisement; a text advert here would replace it.
        if crate::platform::pasteboard::supported() && crate::platform::pasteboard::has_file_urls() {
            log_rdp_debug(format!(
                "rdp clipboard session={session_id} text advert skipped: macOS pasteboard holds files"
            ));
            return Ok(());
        }
        validate_clipboard_text(&text)?;
        // Text wins: a local text copy replaces any staged/offered files.
        entry.clipboard_files.clear();
        log_rdp_debug(format!(
            "rdp clipboard session={session_id} set-clipboard len={}",
            text.len()
        ));
        entry.clipboard_stage.set_text(text);
        let sender = self.command_sender(session_id).await?;
        sender
            .send(RdpCommand::ClipboardAdvertise)
            .await
            .map_err(|_| "RDP session is closed".to_string())
    }

    // —— 文件/文件夹剪贴板（CLIPRDR 文件传输） ——

    /// `rdp/clipboard-files/begin`: validates the manifest and opens one
    /// upload slot (1 MiB chunks).
    pub async fn clipboard_files_begin(
        &self,
        session_id: &str,
        files: &[ClipboardFileEntry],
    ) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        self.require_file_clipboard(&entry)?;
        let (upload_id, chunk_bytes) = entry.clipboard_files.begin_upload(files)?;
        Ok(json!({
            "sessionId": session_id,
            "uploadId": upload_id,
            "chunkBytes": chunk_bytes,
        }))
    }

    /// `rdp/clipboard-files/chunk`: writes one base64 chunk at `offset`.
    pub async fn clipboard_files_chunk(
        &self,
        session_id: &str,
        upload_id: &str,
        index: usize,
        offset: u64,
        data_base64: &str,
    ) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        self.require_file_clipboard(&entry)?;
        // Bound the encoded size before decoding so an oversized payload never
        // materializes: 1 MiB decodes from at most ceil(1 MiB / 3) * 4 chars.
        let max_encoded = (CLIPBOARD_FILE_CHUNK_BYTES + 2) / 3 * 4;
        if data_base64.len() > max_encoded {
            return Err(format!(
                "rdp/clipboard-files/chunk: chunk exceeds the {} MiB limit",
                CLIPBOARD_FILE_CHUNK_BYTES / 1024 / 1024
            ));
        }
        let data = BASE64_STANDARD
            .decode(data_base64)
            .map_err(|_| "rdp/clipboard-files/chunk: invalid base64 payload".to_string())?;
        if data.len() > CLIPBOARD_FILE_CHUNK_BYTES {
            return Err(format!(
                "rdp/clipboard-files/chunk: chunk exceeds the {} MiB limit",
                CLIPBOARD_FILE_CHUNK_BYTES / 1024 / 1024
            ));
        }
        let received = entry
            .clipboard_files
            .write_chunk(upload_id, index, offset, &data)?;
        Ok(json!({
            "sessionId": session_id,
            "received": received,
        }))
    }

    /// `rdp/clipboard-files/commit`: verifies sizes, stages the descriptors and
    /// queues the file-copy advertisement on the current engine.
    pub async fn clipboard_files_commit(
        &self,
        session_id: &str,
        upload_id: &str,
    ) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        self.require_file_clipboard(&entry)?;
        // Refuse to queue when the server did not accept streamed file
        // clipboard: the engine would fail `initiate_file_copy` hard.
        if !entry.clipboard_files.supports_file_copy() {
            return Err(
                "rdp/clipboard-files/commit: the remote does not support file clipboard transfer"
                    .to_string(),
            );
        }
        let (descriptors, total_bytes) = entry.clipboard_files.commit_upload(upload_id)?;
        let file_count = descriptors.len();
        // File contents win over any previously staged text.
        entry.clipboard_stage.clear();
        let sender = self.command_sender(session_id).await?;
        sender
            .send(RdpCommand::ClipboardAdvertiseFiles(descriptors))
            .await
            .map_err(|_| "RDP session is closed".to_string())?;
        Ok(json!({
            "sessionId": session_id,
            "success": true,
            "fileCount": file_count,
            "totalBytes": total_bytes,
        }))
    }

    /// `rdp/clipboard-files/abort`: drops the in-flight upload and its temp dir.
    pub async fn clipboard_files_abort(
        &self,
        session_id: &str,
        upload_id: &str,
    ) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        self.require_file_clipboard(&entry)?;
        let removed = entry.clipboard_files.abort_upload(upload_id);
        Ok(json!({
            "sessionId": session_id,
            "success": removed,
        }))
    }

    /// `rdp/clipboard-files/archive`: builds the save-only zip on demand.
    pub async fn clipboard_files_archive(&self, session_id: &str) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        self.require_file_clipboard(&entry)?;
        entry.clipboard_files.build_archive()
    }

    /// `rdp/clipboard-files/read`: streams the downloaded archive in ≤1 MiB
    /// slices (`length` defaults to 1 MiB).
    pub async fn clipboard_files_read(
        &self,
        session_id: &str,
        offset: u64,
        length: Option<u64>,
    ) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        let (data_base64, eof, total, archive_name) = entry
            .clipboard_files
            .read_archive(offset, length.unwrap_or(CLIPBOARD_FILE_CHUNK_BYTES as u64))?;
        Ok(json!({
            "sessionId": session_id,
            "dataBase64": data_base64,
            "eof": eof,
            "total": total,
            "archiveName": archive_name,
        }))
    }

    /// `rdp/clipboard-files/clear`: drops archive, remote state and temp dirs.
    /// `rdp/clipboard-files/download`: explicit remote→host transfer. Metadata
    /// (and the available event) arrived earlier; this queues the first SIZE
    /// request on the current engine.
    pub async fn clipboard_files_download(&self, session_id: &str) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        self.require_file_clipboard(&entry)?;
        let request = entry.clipboard_files.start_download()?;
        let Some(request) = request else {
            return Ok(json!({
                "sessionId": session_id,
                "success": true,
                "downloading": false,
            }));
        };
        let sender = self.command_sender(session_id).await?;
        if sender
            .send(RdpCommand::ClipboardDownloadFiles(request))
            .await
            .is_err()
        {
            // Do not leave the stage marked "downloading" with no engine.
            entry.clipboard_files.cancel_download();
            return Err("RDP session is closed".to_string());
        }
        Ok(json!({
            "sessionId": session_id,
            "success": true,
            "downloading": true,
        }))
    }

    pub async fn clipboard_files_clear(&self, session_id: &str) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        entry.clipboard_files.clear();
        Ok(json!({ "sessionId": session_id, "success": true }))
    }

    /// `rdp/clipboard-files/status`: remote file list, archive readiness and
    /// download progress.
    pub async fn clipboard_files_status(&self, session_id: &str) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        Ok(entry.clipboard_files.status())
    }

    fn require_file_clipboard(&self, entry: &RdpSessionEntry) -> Result<(), String> {
        if !entry.clipboard_enabled {
            return Err("RDP clipboard redirection is disabled for this session".to_string());
        }
        Ok(())
    }
}
