//! CLIPRDR 文件/文件夹剪贴板状态机（MS-RDPECLIP 文件剪贴板，双向）。
//!
//! 本模块只持有状态与磁盘暂存，所有远端交互都通过返回
//! `Vec<ClipboardMessage>` 交给 `clipboard.rs` 的 backend 适配层发送：
//! 独立于引擎通道，纯状态机可单测（tests/file_clipboard.rs）。
//! - `paths.rs`：名字/相对路径校验 + 临时目录磁盘助手
//! - `upload.rs`：出站暂存（begin/chunk/commit/abort）与 SIZE/RANGE 应答
//! - `download.rs`：入站下载（SIZE → 1 MiB RANGE）与 zip 打包
//!
//! 安全边界（评审红线延伸）：
//! - 出站：前端只投递常规文件；路径逐段校验（穿越、绝对路径、保留设备名、
//!   控制字符、259 字符线制名上限），临时目录带 session+upload UUID，拒绝
//!   跟随符号链接；远端只能按已申报大小读，单块 ≤1 MiB。
//! - 入站：Cliprdr 已做路径清洗，此处再校验一次；文件数/单文件/总量上限
//!   2000 / 512 MiB / 512 MiB；任何错误只发 `rdp/clipboard-files/error`
//!   事件并清理半成品，绝不让异常穿回引擎泵。
//! - 日志只出通用错误文本，绝不记录文件内容与本地路径。

mod download;
mod local;
mod paths;
mod status;
mod upload;

use crate::platform::diagnostics::log_rdp_debug;
use dbx_plugin_sdk::PluginEmitter;
use download::{
    build_archive as build_download_archive, build_remote_files, cancel_download, finish_transfer,
    handle_contents_response, mark_failed, prepare_remote_layout, reset_for_retry, start_next_file,
    write_downloaded_to_pasteboard, Archive, RemoteTransfer,
};
use ironrdp::cliprdr::backend::ClipboardMessage;
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    FileContentsRequest, FileContentsResponse, FileDescriptor,
};
#[cfg(test)]
pub(crate) use paths::validate_file_path;
use paths::{remove_dir, remove_dir_async_quarantined, temp_root};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex as StdMutex;

/// 1 MiB：RPC chunk 上限、文件内容单次读写上限、进度事件粒度基准。
pub(crate) const CLIPBOARD_FILE_CHUNK_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CLIPBOARD_FILES: usize = 2000;
pub(crate) const MAX_CLIPBOARD_FILE_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_CLIPBOARD_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
/// MS-RDPECLIP 2.2.5.2.3.1：cFileName 为 520 字节 UTF-16，线制名 ≤259 字符。
pub(crate) const MAX_CLIPBOARD_WIRE_NAME_CHARS: usize = 259;
pub(super) const PROGRESS_BYTES_STEP: u64 = 4 * 1024 * 1024;
pub(super) const TEMP_PREFIX: &str = "dbx-sshx-rdp-clipboard";
pub(super) const MAX_DEDUPE_ATTEMPTS: u32 = 1000;
/// Native Finder→remote auto-download policy for the reverse direction:
/// small remote copies are fetched and placed on the macOS pasteboard
/// automatically; larger sets wait for the overlay's explicit action.
pub(super) const AUTO_PASTEBOARD_FILES: usize = 8;
pub(super) const AUTO_PASTEBOARD_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Default)]
struct Inner {
    upload: Option<upload::Upload>,
    outgoing: Option<upload::StagedFiles>,
    remote: Option<RemoteTransfer>,
    remote_file_list_format: Option<ClipboardFormatId>,
    last_error: Option<String>,
}

/// Session-scoped file clipboard state shared by the RPC runtime and the
/// CLIPRDR backend of every generation.
pub(crate) struct FileClipboardStage {
    session_id: String,
    inner: StdMutex<Inner>,
    /// Capabilities negotiated by the current CLIPRDR channel.
    negotiated: AtomicU32,
    /// Channel reached Ready (Commit refuses to queue before this).
    ready: AtomicBool,
}

impl std::fmt::Debug for FileClipboardStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileClipboardStage")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl Drop for FileClipboardStage {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(upload) = inner.upload.as_ref() {
                remove_dir(&upload.dir);
            }
            if let Some(outgoing) = inner.outgoing.take() {
                outgoing.cleanup();
            }
            if let Some(remote) = inner.remote.as_ref() {
                remove_remote_dir_unless_pasted(&remote.dir);
            }
        }
    }
}

impl FileClipboardStage {
    pub(crate) fn new(session_id: String) -> Self {
        Self {
            session_id,
            inner: StdMutex::new(Inner::default()),
            negotiated: AtomicU32::new(0),
            ready: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, String> {
        self.inner
            .lock()
            .map_err(|_| "RDP file clipboard state is unavailable".to_string())
    }

    // —— 能力协商 ——

    pub(crate) fn set_negotiated_capabilities(&self, flags: ClipboardGeneralCapabilityFlags) {
        self.negotiated.store(flags.bits(), Ordering::Release);
    }

    /// A fresh CLIPRDR channel starts un-negotiated.
    pub(crate) fn reset_channel(&self) {
        self.negotiated.store(0, Ordering::Release);
        self.ready.store(false, Ordering::Release);
        // A dead CLIPRDR channel cannot finish an inbound transfer: drop its
        // partial files so the overlay cannot freeze on stale progress.
        if let Ok(mut inner) = self.lock() {
            if let Some(remote) = inner.remote.take() {
                remove_remote_dir_unless_pasted(&remote.dir);
            }
            inner.last_error = None;
        }
    }

    pub(crate) fn set_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// `rdp/clipboard-files/commit` gate: the channel must be Ready and the
    /// server must have accepted `CB_STREAM_FILECLIP_ENABLED`.
    pub(crate) fn supports_file_copy(&self) -> bool {
        ClipboardGeneralCapabilityFlags::from_bits_retain(self.negotiated.load(Ordering::Acquire))
            .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
            && self.ready.load(Ordering::Acquire)
    }

    // —— 入站：远端 → 本地 ——

    /// Records the remote FileGroupDescriptorW format id from its FormatList.
    pub(crate) fn note_remote_copy(
        &self,
        formats: &[ClipboardFormat],
    ) -> Option<ClipboardFormatId> {
        let id = formats
            .iter()
            .find(|format| {
                format
                    .name
                    .as_ref()
                    .map(|name| name.value() == ClipboardFormatName::FILE_LIST.value())
                    .unwrap_or(false)
            })
            .map(|format| format.id)?;
        if let Ok(mut inner) = self.lock() {
            inner.remote_file_list_format = Some(id);
        }
        Some(id)
    }

    /// Handles the remote file metadata and starts downloading every regular
    /// file (SIZE first, then 1 MiB RANGE chunks).
    pub(crate) fn on_remote_file_list(
        &self,
        files: &[FileDescriptor],
        clip_data_id: Option<u32>,
        emitter: &PluginEmitter,
    ) -> Vec<ClipboardMessage> {
        if !self.supports_file_copy() {
            return Vec::new();
        }
        match self.start_remote_download(files, clip_data_id, emitter) {
            Ok(messages) => messages,
            Err(message) => {
                if let Ok(mut inner) = self.lock() {
                    fail_remote(&mut inner, &self.session_id, emitter, message);
                }
                Vec::new()
            }
        }
    }

    fn start_remote_download(
        &self,
        files: &[FileDescriptor],
        clip_data_id: Option<u32>,
        emitter: &PluginEmitter,
    ) -> Result<Vec<ClipboardMessage>, String> {
        if files.len() > MAX_CLIPBOARD_FILES {
            return Err(format!(
                "remote clipboard file list exceeds the {MAX_CLIPBOARD_FILES} file limit"
            ));
        }
        let mut inner = self.lock()?;
        if let Some(previous) = inner.remote.take() {
            remove_remote_dir_unless_pasted(&previous.dir);
        }
        inner.last_error = None;
        let dir = temp_root().join(format!("{}-recv-{}", self.session_id, uuid::Uuid::new_v4()));
        let (remote_files, files_total, bytes_total) = match build_remote_files(&dir, files) {
            Ok(built) => built,
            Err(message) => {
                remove_dir(&dir);
                return Err(message);
            }
        };
        let remote = RemoteTransfer::new(dir, clip_data_id, remote_files, files_total, bytes_total);
        let session_id = self.session_id.clone();
        // Native file paste: small remote copies are fetched automatically and
        // (after completion) placed on the macOS pasteboard so Finder can
        // Cmd+V. Larger sets stay metadata-only until the user acts.
        let auto = crate::platform::pasteboard::supported()
            && remote.files_total > 0
            && remote.files_total <= AUTO_PASTEBOARD_FILES
            && remote.bytes_total <= AUTO_PASTEBOARD_BYTES;
        let available = json!({
            "sessionId": session_id,
            "files": remote.files_payload(),
            "totalBytes": remote.bytes_total,
            "downloading": auto,
        });
        inner.remote = Some(remote);
        emit_event(emitter, "rdp/clipboard-files/available", available);
        if !auto {
            return Ok(Vec::new());
        }
        let remote = inner.remote.as_mut().expect("remote stored above");
        prepare_remote_layout(remote)?;
        match start_next_file(remote)? {
            Some(ClipboardMessage::SendFileContentsRequest(request)) => {
                Ok(vec![ClipboardMessage::SendFileContentsRequest(request)])
            }
            Some(_) => Err("unexpected remote clipboard request".to_string()),
            None => Ok(Vec::new()),
        }
    }

    /// Starts (or resumes) the inbound download after an explicit user action.
    /// Returns the first FileContentsRequest for the worker to send.
    pub(crate) fn start_download(&self) -> Result<Option<FileContentsRequest>, String> {
        let mut inner = self.lock()?;
        let retry = inner.last_error.take().is_some();
        let remote = inner
            .remote
            .as_mut()
            .ok_or_else(|| "remote clipboard has no files".to_string())?;
        if remote.archive.is_some() {
            return Ok(None);
        }
        if remote.downloading {
            return Ok(None);
        }
        if remote.files_total == 0 {
            return Err("remote clipboard contains no files".to_string());
        }
        if retry || !remote.dir.exists() {
            reset_for_retry(remote)?;
        } else {
            prepare_remote_layout(remote)?;
        }
        match start_next_file(remote)? {
            Some(ClipboardMessage::SendFileContentsRequest(request)) => Ok(Some(request)),
            Some(_) => Err("unexpected remote clipboard request".to_string()),
            None => Ok(None),
        }
    }

    /// Advances the inbound download from a `FileContentsResponse`. Completion
    /// snapshots the `ready` payload under the lock, then writes the macOS
    /// pasteboard off the CLIPRDR engine thread.
    pub(crate) fn on_file_contents_response(
        &self,
        response: FileContentsResponse<'_>,
        emitter: &PluginEmitter,
    ) -> Vec<ClipboardMessage> {
        let Ok(mut inner) = self.lock() else {
            return Vec::new();
        };
        let stale = inner
            .remote
            .as_ref()
            .map(|remote| !remote.is_waiting(response.stream_id()))
            .unwrap_or(true);
        if stale {
            return Vec::new();
        }
        let session_id = self.session_id.clone();
        let (action, finished, dir) = {
            let remote = inner.remote.as_mut().expect("checked above");
            remote.clear_pending();
            let action = handle_contents_response(remote, &session_id, emitter, response);
            let done = action.is_ok()
                && !remote.downloading
                && remote.files_total > 0
                && remote.files_done >= remote.files_total;
            let finished = if done {
                finish_transfer(remote).ok()
            } else {
                None
            };
            (action, finished, remote.dir.clone())
        };
        let messages = match action {
            Ok(messages) => messages,
            Err(message) => {
                fail_remote(&mut inner, &session_id, emitter, message);
                Vec::new()
            }
        };
        drop(inner);
        if let Some(finished) = finished {
            let emitter = emitter.clone();
            let run = move || {
                let (pasteboard, error) = write_downloaded_to_pasteboard(&dir);
                let mut payload = json!({
                    "sessionId": session_id,
                    "fileCount": finished.entry_count,
                    "files": finished.files,
                    "pasteboard": pasteboard,
                });
                if let Some(error) = error {
                    payload["pasteboardError"] = json!(error);
                }
                emit_event(&emitter, "rdp/clipboard-files/ready", payload);
            };
            if cfg!(test) {
                run();
            } else {
                std::thread::spawn(run);
            }
        }
        messages
    }

    /// Builds the remote clipboard zip on demand (outside the state mutex; the
    /// raw files are kept for the macOS pasteboard and repeat saves).
    pub(crate) fn build_archive(&self) -> Result<Value, String> {
        let (dir, files) = {
            let inner = self.lock()?;
            let remote = inner
                .remote
                .as_ref()
                .ok_or_else(|| "remote clipboard has no files".to_string())?;
            if remote.downloading {
                return Err("the remote clipboard download is still running".to_string());
            }
            if remote.files_total == 0 {
                return Err("remote clipboard contains no files".to_string());
            }
            if let Some(archive) = remote.archive.as_ref() {
                return Ok(json!({
                    "sessionId": self.session_id,
                    "archiveName": archive.name,
                    "archiveSize": archive.size,
                }));
            }
            let files: Vec<(String, Option<String>, std::path::PathBuf)> = remote
                .files
                .iter()
                .filter(|file| !file.directory)
                .map(|file| {
                    (
                        file.name.clone(),
                        file.relative_path.clone(),
                        file.path.clone(),
                    )
                })
                .collect();
            (remote.dir.clone(), files)
        };
        let (name, size) = build_download_archive(&dir, &files)?;
        let mut inner = self.lock()?;
        if let Some(remote) = inner.remote.as_mut().filter(|remote| remote.dir == dir) {
            remote.archive = Some(Archive {
                name: name.clone(),
                path: dir.join(&name),
                size,
            });
        }
        Ok(json!({ "sessionId": self.session_id, "archiveName": name, "archiveSize": size }))
    }

    pub(crate) fn read_archive(
        &self,
        offset: u64,
        length: u64,
    ) -> Result<(String, bool, u64, String), String> {
        use base64::Engine as _;
        // Copy the path/size/name under the lock, then stream the file outside
        // it: archive reads can be a few milliseconds and must not block the
        // CLIPRDR pump or status/clear callbacks.
        let (path, size, name) = {
            let inner = self.lock()?;
            let archive = inner
                .remote
                .as_ref()
                .and_then(|remote| remote.archive.as_ref())
                .ok_or_else(|| "no remote clipboard archive is available".to_string())?;
            (archive.path.clone(), archive.size, archive.name.clone())
        };
        let length = length.clamp(1, CLIPBOARD_FILE_CHUNK_BYTES as u64) as usize;
        let mut buffer = vec![0u8; length];
        let read = File::open(&path)
            .and_then(|mut handle| {
                handle.seek(SeekFrom::Start(offset))?;
                handle.read(&mut buffer)
            })
            .map_err(|_| "unable to read the remote clipboard archive".to_string())?;
        buffer.truncate(read);
        let eof = offset.saturating_add(read as u64) >= size;
        Ok((
            base64::engine::general_purpose::STANDARD.encode(&buffer),
            eof,
            size,
            name,
        ))
    }

    #[cfg(test)]
    pub(crate) fn remote_dir_for_tests(&self) -> Option<std::path::PathBuf> {
        self.lock()
            .ok()
            .and_then(|inner| inner.remote.as_ref().map(|remote| remote.dir.clone()))
    }

    pub(crate) fn status(&self) -> Value {
        status::status(self)
    }

    /// Drops every staged and downloaded file (text wins, close, reconnect).
    pub(crate) fn cancel_download(&self) {
        if let Ok(mut inner) = self.lock() {
            if let Some(remote) = inner.remote.as_mut() {
                cancel_download(remote);
            }
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut inner) = self.lock() {
            if let Some(upload) = inner.upload.take() {
                remove_dir(&upload.dir);
            }
            if let Some(outgoing) = inner.outgoing.take() {
                outgoing.cleanup();
            }
            if let Some(remote) = inner.remote.take() {
                remove_remote_dir_unless_pasted(&remote.dir);
            }
            inner.last_error = None;
            inner.remote_file_list_format = None;
        }
    }
}

/// True when the macOS pasteboard currently references files under `dir`.
pub(crate) fn pasteboard_references_dir(dir: &std::path::Path) -> bool {
    if !crate::platform::pasteboard::supported() {
        return false;
    }
    let Ok(canonical_dir) = std::fs::canonicalize(dir) else {
        return false;
    };
    crate::platform::pasteboard::file_urls().iter().any(|path| {
        std::fs::canonicalize(path)
            .map(|path| path.starts_with(&canonical_dir))
            .unwrap_or(false)
    })
}

impl FileClipboardStage {
    /// The macOS pasteboard holds files we downloaded for the remote; an
    /// inbound text event would replace them before Finder can paste.
    pub(crate) fn pasteboard_holds_download(&self) -> bool {
        self.lock()
            .ok()
            .and_then(|inner| {
                inner.remote.as_ref().map(|remote| {
                    remote.completed_pasteboard && pasteboard_references_dir(&remote.dir)
                })
            })
            .unwrap_or(false)
    }
}

/// Keeps a completed download on disk while the macOS pasteboard references it,
/// so Finder can still paste after the overlay was saved/dismissed; otherwise
/// delete it immediately.
fn remove_remote_dir_unless_pasted(dir: &std::path::Path) {
    if pasteboard_references_dir(dir) {
        log_rdp_debug(
            "rdp clipboard download dir kept: referenced by the macOS pasteboard".to_string(),
        );
        return;
    }
    remove_dir(dir);
}

/// True when `path` lives under the plugin's clipboard temp root (our own
/// downloaded files written to the pasteboard must never be advertised back).
pub(crate) fn is_plugin_temp_path(path: &std::path::Path) -> bool {
    // NSPasteboard reports canonical macOS paths (/private/var/...) while
    // std::env::temp_dir() may keep the /var form; compare canonical roots.
    let Ok(root) = std::fs::canonicalize(temp_root()) else {
        return false;
    };
    let Ok(path) = std::fs::canonicalize(path) else {
        return false;
    };
    path.starts_with(root)
}

fn fail_remote(inner: &mut Inner, session_id: &str, emitter: &PluginEmitter, message: String) {
    if let Some(remote) = inner.remote.as_mut() {
        mark_failed(remote);
        remove_dir_async_quarantined(remote.dir.clone());
    }
    inner.last_error = Some(message.clone());
    emit_error(emitter, session_id, &message);
}

// —— 事件（失败只记日志，绝不影响会话） ——

fn emit_event(emitter: &PluginEmitter, method: &str, payload: Value) {
    if let Err(error) = emitter.event(method, payload) {
        eprintln!(
            "[dbx-plugin-sshx] rdp clipboard files event dropped: {}",
            error.message
        );
    }
}

fn emit_progress(emitter: &PluginEmitter, session_id: &str, remote: &RemoteTransfer) {
    emit_event(
        emitter,
        "rdp/clipboard-files/progress",
        json!({
            "sessionId": session_id,
            "filesDone": remote.files_done,
            "filesTotal": remote.files_total,
            "bytesDone": remote.bytes_done,
            "bytesTotal": remote.bytes_total.max(remote.bytes_done),
        }),
    );
}

fn emit_error(emitter: &PluginEmitter, session_id: &str, message: &str) {
    emit_event(
        emitter,
        "rdp/clipboard-files/error",
        json!({ "sessionId": session_id, "message": message }),
    );
}
