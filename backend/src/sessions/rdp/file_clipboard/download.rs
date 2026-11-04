//! 入站（远端 → 本地）文件剪贴板下载状态机：SIZE → 1 MiB RANGE 分块，
//! 完成后打包为 `remote-clipboard-<timestamp>.zip`。所有函数只返回待发送的
//! `ClipboardMessage` 或发事件，绝不 panic / 不终止会话。

use super::paths::{
    create_private_dir, create_private_file, ensure_dir, join_relative, open_private_file,
    path_components, reject_symlink, remove_dir, validate_file_path,
};
use super::{
    emit_progress, CLIPBOARD_FILE_CHUNK_BYTES, MAX_CLIPBOARD_FILE_BYTES, MAX_CLIPBOARD_TOTAL_BYTES,
    MAX_DEDUPE_ATTEMPTS, PROGRESS_BYTES_STEP,
};
use dbx_plugin_sdk::PluginEmitter;
use ironrdp::cliprdr::backend::ClipboardMessage;
use ironrdp::cliprdr::pdu::{
    ClipboardFileAttributes, FileContentsFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) struct RemoteFile {
    pub(super) name: String,
    pub(super) relative_path: Option<String>,
    pub(super) size: u64,
    size_known: bool,
    pub(super) directory: bool,
    pub(super) path: PathBuf,
    completed: bool,
}

#[derive(Clone, Copy)]
pub(super) enum RemotePhase {
    Idle,
    Size { index: usize },
    Data { index: usize, offset: u64 },
}

pub(super) struct Archive {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) size: u64,
}

pub(super) struct RemoteTransfer {
    pub(super) dir: PathBuf,
    clip_data_id: Option<u32>,
    pub(super) files: Vec<RemoteFile>,
    pub(super) files_total: usize,
    pub(super) files_done: usize,
    pub(super) bytes_total: u64,
    pub(super) bytes_done: u64,
    last_progress_bytes: u64,
    phase: RemotePhase,
    pending_stream: Option<u32>,
    pub(super) archive: Option<Archive>,
    pub(super) downloading: bool,
    /// Last completed transfer is expected on the macOS pasteboard (revalidated
    /// against the live pasteboard before it is reported).
    pub(super) completed_pasteboard: bool,
}

/// Session-crossing stream ids: the vendored client correlates the single
/// in-flight FileContentsRequest by stream id, so a new transfer must never
/// reuse an id that a previous one may still have pending.
static NEXT_STREAM: AtomicU32 = AtomicU32::new(1);

impl RemoteTransfer {
    pub(super) fn new(
        dir: PathBuf,
        clip_data_id: Option<u32>,
        files: Vec<RemoteFile>,
        files_total: usize,
        bytes_total: u64,
    ) -> Self {
        Self {
            dir,
            clip_data_id,
            files,
            files_total,
            files_done: 0,
            bytes_total,
            bytes_done: 0,
            last_progress_bytes: 0,
            phase: RemotePhase::Idle,
            pending_stream: None,
            archive: None,
            // Metadata arrives first; the transfer starts only on an explicit
            // download request (or a threshold policy chosen by the caller).
            downloading: false,
            completed_pasteboard: false,
        }
    }

    pub(super) fn is_waiting(&self, stream_id: u32) -> bool {
        self.pending_stream == Some(stream_id)
    }

    pub(super) fn clear_pending(&mut self) {
        self.pending_stream = None;
    }

    pub(super) fn files_payload(&self) -> Vec<Value> {
        self.files
            .iter()
            .map(|file| {
                json!({
                    "name": file.name,
                    "relativePath": file.relative_path,
                    "size": file.size,
                    "directory": file.directory,
                })
            })
            .collect()
    }
}

/// Collects the remote descriptors, validates the caps and creates the
/// directory layout under `dir` (relative paths only, no symlinks).
pub(super) fn build_remote_files(
    dir: &Path,
    files: &[FileDescriptor],
) -> Result<(Vec<RemoteFile>, usize, u64), String> {
    let mut used = HashSet::new();
    let mut entries = Vec::with_capacity(files.len());
    let mut files_total = 0usize;
    let mut bytes_total = 0u64;
    for descriptor in files {
        let directory = descriptor
            .attributes
            .map(|attributes| attributes.contains(ClipboardFileAttributes::DIRECTORY))
            .unwrap_or(false);
        let (relative_path, name) =
            validate_file_path(&descriptor.name, descriptor.relative_path.as_deref())?;
        let (relative_path, name) = unique_remote_path(&mut used, relative_path, name)?;
        let size = descriptor.file_size.unwrap_or(0);
        if !directory {
            if size > MAX_CLIPBOARD_FILE_BYTES {
                return Err(format!(
                    "remote clipboard file exceeds the {} MiB limit",
                    MAX_CLIPBOARD_FILE_BYTES / 1024 / 1024
                ));
            }
            bytes_total = bytes_total
                .checked_add(size)
                .ok_or_else(|| "remote clipboard size overflow".to_string())?;
            if bytes_total > MAX_CLIPBOARD_TOTAL_BYTES {
                return Err(format!(
                    "remote clipboard files exceed the {} MiB total limit",
                    MAX_CLIPBOARD_TOTAL_BYTES / 1024 / 1024
                ));
            }
            files_total += 1;
        }
        // Metadata-only: compute paths but do not touch the filesystem. The
        // directory tree is materialized when the user starts the download.
        let path = join_relative(dir, relative_path.as_deref(), &name);
        entries.push(RemoteFile {
            name,
            relative_path,
            size,
            size_known: descriptor.file_size.is_some(),
            directory,
            path,
            completed: directory,
        });
    }
    Ok((entries, files_total, bytes_total))
}

/// Creates the download tree and re-checks symlinks; runs on the RPC thread
/// when the user starts (or retries) a download, never during metadata.
pub(super) fn prepare_remote_layout(remote: &RemoteTransfer) -> Result<(), String> {
    create_private_dir(&remote.dir)?;
    for file in &remote.files {
        if file.directory {
            let mut components = path_components(file.relative_path.as_deref());
            components.push(&file.name);
            ensure_dir(&remote.dir, &components)?;
        } else {
            ensure_dir(&remote.dir, &path_components(file.relative_path.as_deref()))?;
            reject_symlink(&file.path)?;
        }
    }
    Ok(())
}

/// Resets a failed/interrupted transfer so the user can download again.
pub(super) fn cancel_download(remote: &mut RemoteTransfer) {
    remote.downloading = false;
    remote.phase = RemotePhase::Idle;
    remote.pending_stream = None;
}

pub(super) fn reset_for_retry(remote: &mut RemoteTransfer) -> Result<(), String> {
    remove_dir(&remote.dir);
    remote.files_done = 0;
    remote.bytes_done = 0;
    remote.last_progress_bytes = 0;
    for file in &mut remote.files {
        file.completed = file.directory;
    }
    remote.phase = RemotePhase::Idle;
    remote.pending_stream = None;
    remote.archive = None;
    remote.downloading = false;
    remote.completed_pasteboard = false;
    prepare_remote_layout(remote)
}

pub(super) fn start_next_file(
    remote: &mut RemoteTransfer,
) -> Result<Option<ClipboardMessage>, String> {
    let Some(index) = remote
        .files
        .iter()
        .position(|file| !file.directory && !file.completed)
    else {
        remote.phase = RemotePhase::Idle;
        remote.downloading = false;
        return Ok(None);
    };
    remote.downloading = true;
    remote.phase = RemotePhase::Size { index };
    let stream_id = alloc_stream(remote);
    remote.pending_stream = Some(stream_id);
    Ok(Some(ClipboardMessage::SendFileContentsRequest(
        FileContentsRequest {
            stream_id,
            index: index as i32,
            flags: FileContentsFlags::SIZE,
            position: 0,
            requested_size: 8,
            data_id: remote.clip_data_id,
        },
    )))
}

fn request_range(remote: &mut RemoteTransfer, index: usize, offset: u64) -> ClipboardMessage {
    let remaining = remote.files[index].size.saturating_sub(offset);
    let chunk = remaining.min(CLIPBOARD_FILE_CHUNK_BYTES as u64);
    let stream_id = alloc_stream(remote);
    remote.pending_stream = Some(stream_id);
    remote.phase = RemotePhase::Data { index, offset };
    ClipboardMessage::SendFileContentsRequest(FileContentsRequest {
        stream_id,
        index: index as i32,
        flags: FileContentsFlags::RANGE,
        position: offset,
        requested_size: chunk as u32,
        data_id: remote.clip_data_id,
    })
}

fn alloc_stream(_remote: &mut RemoteTransfer) -> u32 {
    NEXT_STREAM.fetch_add(1, Ordering::Relaxed).max(1)
}

pub(super) fn handle_contents_response(
    remote: &mut RemoteTransfer,
    session_id: &str,
    emitter: &PluginEmitter,
    response: FileContentsResponse<'_>,
) -> Result<Vec<ClipboardMessage>, String> {
    if response.is_error() {
        return Err("remote clipboard file transfer was rejected by the server".to_string());
    }
    match remote.phase {
        RemotePhase::Idle => Ok(Vec::new()),
        RemotePhase::Size { index } => {
            let size = response
                .data_as_size()
                .map_err(|_| "remote clipboard file size response is malformed".to_string())?;
            let (known, advertised) = {
                let file = &remote.files[index];
                (file.size_known, file.size)
            };
            if known && size != advertised {
                return Err(
                    "remote clipboard file size does not match the advertised size".to_string(),
                );
            }
            if size > MAX_CLIPBOARD_FILE_BYTES {
                return Err(format!(
                    "remote clipboard file exceeds the {} MiB limit",
                    MAX_CLIPBOARD_FILE_BYTES / 1024 / 1024
                ));
            }
            if !known {
                remote.bytes_total = remote.bytes_total.saturating_add(size);
            }
            if remote.bytes_total > MAX_CLIPBOARD_TOTAL_BYTES {
                return Err(format!(
                    "remote clipboard files exceed the {} MiB total limit",
                    MAX_CLIPBOARD_TOTAL_BYTES / 1024 / 1024
                ));
            }
            {
                let file = &mut remote.files[index];
                file.size = size;
                file.size_known = true;
            }
            let path = remote.files[index].path.clone();
            reject_symlink(&path)?;
            create_private_file(&path)?;
            if size == 0 {
                complete_file(remote, index, session_id, emitter);
                return advance_after_file(remote);
            }
            Ok(vec![request_range(remote, index, 0)])
        }
        RemotePhase::Data { index, offset } => {
            let data = response.data();
            if data.is_empty() || data.len() > CLIPBOARD_FILE_CHUNK_BYTES {
                return Err("remote clipboard file chunk is malformed".to_string());
            }
            let size = remote.files[index].size;
            let end = offset.saturating_add(data.len() as u64);
            if end > size {
                return Err("remote clipboard file chunk exceeds the file size".to_string());
            }
            let path = remote.files[index].path.clone();
            reject_symlink(&path)?;
            let mut handle = open_private_file(&path)?;
            handle
                .seek(SeekFrom::Start(offset))
                .and_then(|_| handle.write_all(data))
                .map_err(|_| "unable to write the clipboard download chunk".to_string())?;
            remote.bytes_done = remote.bytes_done.saturating_add(data.len() as u64);
            if remote.bytes_done > MAX_CLIPBOARD_TOTAL_BYTES {
                return Err(format!(
                    "remote clipboard files exceed the {} MiB total limit",
                    MAX_CLIPBOARD_TOTAL_BYTES / 1024 / 1024
                ));
            }
            if remote.bytes_done.saturating_sub(remote.last_progress_bytes) >= PROGRESS_BYTES_STEP {
                remote.last_progress_bytes = remote.bytes_done;
                emit_progress(emitter, session_id, remote);
            }
            if end == size {
                complete_file(remote, index, session_id, emitter);
                advance_after_file(remote)
            } else {
                Ok(vec![request_range(remote, index, end)])
            }
        }
    }
}

fn complete_file(
    remote: &mut RemoteTransfer,
    index: usize,
    session_id: &str,
    emitter: &PluginEmitter,
) {
    remote.files[index].completed = true;
    remote.files_done += 1;
    remote.last_progress_bytes = remote.bytes_done;
    emit_progress(emitter, session_id, remote);
}

fn advance_after_file(remote: &mut RemoteTransfer) -> Result<Vec<ClipboardMessage>, String> {
    match start_next_file(remote)? {
        Some(message) => Ok(vec![message]),
        None => Ok(Vec::new()),
    }
}

/// Marks an inbound transfer complete and snapshots the `ready` payload.
/// No IO here: the caller drops the state mutex and runs the macOS pasteboard
/// write off the CLIPRDR engine thread.
pub(super) struct FinishedTransfer {
    pub(super) entry_count: usize,
    pub(super) files: Vec<Value>,
}

pub(super) fn finish_transfer(remote: &mut RemoteTransfer) -> Result<FinishedTransfer, String> {
    remote.phase = RemotePhase::Idle;
    remote.downloading = false;
    remote.completed_pasteboard = crate::platform::pasteboard::supported();
    let entry_count = remote.files.iter().filter(|file| !file.directory).count();
    if entry_count == 0 {
        return Err("remote clipboard contains no files".to_string());
    }
    let files: Vec<Value> = remote
        .files
        .iter()
        .filter(|file| !file.directory)
        .map(|file| json!({ "name": file.name, "relativePath": file.relative_path }))
        .collect();
    Ok(FinishedTransfer { entry_count, files })
}

/// Puts the downloaded top-level files/folders on the macOS pasteboard so the
/// user can Cmd+V in Finder. Only plugin-owned temp files are ever written.
pub(super) fn write_downloaded_to_pasteboard(dir: &Path) -> (bool, Option<String>) {
    if !crate::platform::pasteboard::supported() {
        return (false, None);
    }
    let Ok(read) = fs::read_dir(dir) else {
        return (
            false,
            Some("remote clipboard files are unavailable".to_string()),
        );
    };
    let mut top: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            fs::symlink_metadata(path)
                .map(|metadata| !metadata.file_type().is_symlink())
                .unwrap_or(false)
        })
        .collect();
    top.sort();
    if top.is_empty() {
        return (
            false,
            Some("remote clipboard contains no files".to_string()),
        );
    }
    match crate::platform::pasteboard::write_file_urls(&top) {
        Ok(()) => (true, None),
        Err(error) => (false, Some(error)),
    }
}

/// Builds the save-only zip on demand. Raw files stay in place (they may be
/// referenced by the macOS pasteboard or a second save).
pub(super) fn build_archive(
    dir: &Path,
    files: &[(String, Option<String>, PathBuf)],
) -> Result<(String, u64), String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let archive_name = format!("remote-clipboard-{timestamp}.zip");
    let archive_path = dir.join(&archive_name);
    let file = create_private_file(&archive_path)?;
    let mut writer = zip::ZipWriter::new(file);
    // Level 1 keeps the (user-initiated) archive preparation fast; the zip is a
    // transport container, not a long-term compressed artifact.
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(1));
    for (name, relative_path, path) in files {
        let mut entry_name = String::new();
        if let Some(path) = relative_path.as_deref().filter(|path| !path.is_empty()) {
            entry_name.push_str(&path.replace('\\', "/"));
            entry_name.push('/');
        }
        entry_name.push_str(name);
        writer
            .start_file(entry_name, options)
            .map_err(|_| "unable to add a file to the remote clipboard archive".to_string())?;
        let mut source =
            File::open(path).map_err(|_| "remote clipboard download is incomplete".to_string())?;
        std::io::copy(&mut source, &mut writer)
            .map_err(|_| "unable to store a file in the remote clipboard archive".to_string())?;
    }
    if files.is_empty() {
        drop(writer);
        let _ = fs::remove_file(&archive_path);
        return Err("remote clipboard contains no files".to_string());
    }
    writer
        .finish()
        .map_err(|_| "unable to finalize the remote clipboard archive".to_string())?;
    let size = fs::metadata(&archive_path)
        .map_err(|_| "unable to stat the remote clipboard archive".to_string())?
        .len();
    Ok((archive_name, size))
}

/// Stops the transfer in place (the caller removes the temp dir and emits).
pub(super) fn mark_failed(remote: &mut RemoteTransfer) {
    remote.downloading = false;
    remote.phase = RemotePhase::Idle;
    remote.pending_stream = None;
    remote.archive = None;
}

fn unique_remote_path(
    used: &mut HashSet<String>,
    relative_path: Option<String>,
    name: String,
) -> Result<(Option<String>, String), String> {
    let (stem, extension) = match name.rfind('.').filter(|position| *position > 0) {
        Some(position) => (&name[..position], &name[position..]),
        None => (name.as_str(), ""),
    };
    for attempt in 0..MAX_DEDUPE_ATTEMPTS {
        let candidate = if attempt == 0 {
            name.clone()
        } else {
            format!("{stem} ({attempt}){extension}")
        };
        let wire = match relative_path.as_deref() {
            Some(path) if !path.is_empty() => format!("{path}\\{candidate}"),
            _ => candidate.clone(),
        };
        if used.insert(wire.to_lowercase()) {
            return Ok((relative_path, candidate));
        }
    }
    Err("remote clipboard file list contains too many duplicate paths".to_string())
}
