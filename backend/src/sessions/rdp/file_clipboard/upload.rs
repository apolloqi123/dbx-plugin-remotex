//! 出站（本地 → 远端）上传暂存：begin/chunk/commit/abort 与远端
//! `FileContentsRequest` 的服务逻辑（SIZE / 1 MiB 上限的 RANGE）。

use super::paths::{
    create_private_dir, create_private_file, local_path, open_private_file, remove_dir, temp_root,
    validate_file_path,
};
use super::{
    FileClipboardStage, CLIPBOARD_FILE_CHUNK_BYTES, MAX_CLIPBOARD_FILES, MAX_CLIPBOARD_FILE_BYTES,
    MAX_CLIPBOARD_TOTAL_BYTES,
};
use crate::sessions::rdp::wire::ClipboardFileEntry;
use ironrdp::cliprdr::pdu::{
    ClipboardFileAttributes, FileContentsFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor,
};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

pub(super) struct UploadFile {
    pub(super) name: String,
    pub(super) relative_path: Option<String>,
    pub(super) size: u64,
    /// Strictly sequential bytes accepted so far; gaps/overlaps are rejected.
    pub(super) written: u64,
}

pub(super) struct Upload {
    pub(super) id: String,
    pub(super) dir: PathBuf,
    pub(super) files: Vec<UploadFile>,
}

pub(super) struct StagedFile {
    pub(super) path: PathBuf,
    pub(super) size: u64,
    pub(super) directory: bool,
}

/// Where the currently advertised outgoing file list came from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OutgoingSource {
    /// Frontend upload staged into a private temp dir.
    Upload,
    /// Finder file URLs served directly from their original paths.
    Pasteboard,
}

pub(super) struct StagedFiles {
    pub(super) id: String,
    /// Owned temp dir (uploads only); pasteboard sources point at user files.
    dir: Option<PathBuf>,
    files: Vec<StagedFile>,
    /// Kept for a bounded re-advertisement when CB_RESPONSE_FAIL arrives.
    descriptors: Vec<FileDescriptor>,
    pub(super) source: OutgoingSource,
}

impl StagedFiles {
    pub(super) fn pasteboard(files: Vec<StagedFile>, descriptors: Vec<FileDescriptor>) -> Self {
        Self {
            id: format!("pasteboard-{}", uuid::Uuid::new_v4()),
            dir: None,
            files,
            descriptors,
            source: OutgoingSource::Pasteboard,
        }
    }

    pub(super) fn cleanup(self) {
        if let Some(dir) = self.dir {
            remove_dir(&dir);
        }
    }
}

impl FileClipboardStage {
    /// Starts a new upload. Replaces any in-flight upload (and its staged
    /// files): at most one upload is alive per session.
    pub(crate) fn begin_upload(
        &self,
        files: &[ClipboardFileEntry],
    ) -> Result<(String, usize), String> {
        if files.is_empty() {
            return Err("clipboard file list is empty".to_string());
        }
        if files.len() > MAX_CLIPBOARD_FILES {
            return Err(format!(
                "clipboard file list exceeds the {MAX_CLIPBOARD_FILES} file limit"
            ));
        }
        let mut validated = Vec::with_capacity(files.len());
        let mut seen = HashSet::new();
        let mut total = 0u64;
        for entry in files {
            if entry.size > MAX_CLIPBOARD_FILE_BYTES {
                return Err(format!(
                    "clipboard file exceeds the {} MiB limit",
                    MAX_CLIPBOARD_FILE_BYTES / 1024 / 1024
                ));
            }
            let (relative_path, name) =
                validate_file_path(&entry.name, entry.relative_path.as_deref())?;
            let wire = match relative_path.as_deref() {
                Some(path) => format!("{path}\\{name}").to_lowercase(),
                None => name.to_lowercase(),
            };
            if !seen.insert(wire) {
                return Err("clipboard file list contains duplicate paths".to_string());
            }
            total = total
                .checked_add(entry.size)
                .ok_or_else(|| "clipboard file list size overflow".to_string())?;
            validated.push(UploadFile {
                name,
                relative_path,
                size: entry.size,
                written: 0,
            });
        }
        if total > MAX_CLIPBOARD_TOTAL_BYTES {
            return Err(format!(
                "clipboard file list exceeds the {} MiB total limit",
                MAX_CLIPBOARD_TOTAL_BYTES / 1024 / 1024
            ));
        }
        let upload_id = uuid::Uuid::new_v4().to_string();
        let dir = temp_root().join(format!("{}-{upload_id}", self.session_id));
        let mut inner = self.lock()?;
        if let Some(previous) = inner.upload.take() {
            remove_dir(&previous.dir);
        }
        // Keep the currently advertised outgoing files until this upload
        // commits: an aborted send must not break a remote paste in flight.
        inner.last_error = None;
        create_private_dir(&dir)?;
        inner.upload = Some(Upload {
            id: upload_id.clone(),
            dir,
            files: validated,
        });
        Ok((upload_id, CLIPBOARD_FILE_CHUNK_BYTES))
    }

    pub(crate) fn write_chunk(
        &self,
        upload_id: &str,
        index: usize,
        offset: u64,
        data: &[u8],
    ) -> Result<u64, String> {
        if data.len() > CLIPBOARD_FILE_CHUNK_BYTES {
            return Err(format!(
                "clipboard file chunk exceeds the {} MiB limit",
                CLIPBOARD_FILE_CHUNK_BYTES / 1024 / 1024
            ));
        }
        let mut inner = self.lock()?;
        let upload = inner
            .upload
            .as_mut()
            .filter(|upload| upload.id == upload_id)
            .ok_or_else(|| "clipboard upload was not found".to_string())?;
        let dir = upload.dir.clone();
        let file = upload
            .files
            .get_mut(index)
            .ok_or_else(|| "clipboard file index is out of range".to_string())?;
        if offset != file.written {
            return Err("clipboard file chunks must be written sequentially".to_string());
        }
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or_else(|| "clipboard file chunk offset overflow".to_string())?;
        if end > file.size {
            return Err("clipboard file chunk exceeds the declared file size".to_string());
        }
        let path = local_path(&dir, file.relative_path.as_deref(), &file.name)?;
        let mut handle = open_private_file(&path)?;
        handle
            .seek(SeekFrom::Start(offset))
            .and_then(|_| handle.write_all(data))
            .map_err(|_| "unable to write the clipboard upload chunk".to_string())?;
        file.written = end;
        Ok(end)
    }

    /// Verifies the declared sizes and stages the descriptors for the remote.
    pub(crate) fn commit_upload(
        &self,
        upload_id: &str,
    ) -> Result<(Vec<FileDescriptor>, u64), String> {
        let mut inner = self.lock()?;
        if !inner
            .upload
            .as_ref()
            .is_some_and(|upload| upload.id == upload_id)
        {
            return Err("clipboard upload was not found".to_string());
        }
        let upload = inner.upload.take().expect("upload id just checked");
        let mut descriptors = Vec::with_capacity(upload.files.len());
        let mut staged = Vec::with_capacity(upload.files.len());
        let mut total = 0u64;
        // Any failure after the upload was claimed must drop its temp dir too.
        let built = (|| -> Result<(), String> {
            for file in &upload.files {
                let path = local_path(&upload.dir, file.relative_path.as_deref(), &file.name)?;
                let written = match fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err("clipboard temp path is a symlink".to_string());
                    }
                    Ok(metadata) => metadata.len(),
                    // A zero-byte file never needs a chunk: materialize it now.
                    Err(_) if file.size == 0 => {
                        create_private_file(&path)?;
                        0
                    }
                    Err(_) => return Err("clipboard upload is missing a file".to_string()),
                };
                if written != file.size {
                    return Err(
                        "clipboard upload size does not match the declared size".to_string()
                    );
                }
                let descriptor = FileDescriptor::new(file.name.clone())
                    .with_attributes(ClipboardFileAttributes::ARCHIVE)
                    .with_file_size(file.size);
                let descriptor = match file.relative_path.as_deref() {
                    Some(path) if !path.is_empty() => descriptor.with_relative_path(path),
                    _ => descriptor,
                };
                descriptors.push(descriptor);
                staged.push(StagedFile {
                    path,
                    size: file.size,
                    directory: false,
                });
                total += file.size;
            }
            Ok(())
        })();
        if let Err(error) = built {
            remove_dir(&upload.dir);
            return Err(error);
        }
        if let Some(previous) = inner.outgoing.replace(StagedFiles {
            id: upload_id.to_string(),
            dir: Some(upload.dir),
            files: staged,
            descriptors: descriptors.clone(),
            source: OutgoingSource::Upload,
        }) {
            previous.cleanup();
        }
        Ok((descriptors, total))
    }

    /// Descriptors of the currently staged outgoing file copy, if any. Used for
    /// one bounded re-advertisement after `CB_RESPONSE_FAIL`.
    pub(crate) fn outgoing_descriptors(&self) -> Option<Vec<FileDescriptor>> {
        let inner = self.lock().ok()?;
        inner
            .outgoing
            .as_ref()
            .filter(|outgoing| !outgoing.descriptors.is_empty())
            .map(|outgoing| outgoing.descriptors.clone())
    }

    pub(crate) fn clear_outgoing(&self) {
        if let Ok(mut inner) = self.lock() {
            if let Some(outgoing) = inner.outgoing.take() {
                outgoing.cleanup();
            }
        }
    }

    /// Clears only a Finder-pasteboard file list; explicit "Send files" staging
    /// is left alone (copying text later must not cancel a pending send).
    pub(crate) fn clear_pasteboard_outgoing(&self) -> bool {
        let Ok(mut inner) = self.lock() else {
            return false;
        };
        if inner
            .outgoing
            .as_ref()
            .is_some_and(|outgoing| outgoing.source == OutgoingSource::Pasteboard)
        {
            if let Some(outgoing) = inner.outgoing.take() {
                outgoing.cleanup();
            }
            return true;
        }
        false
    }

    pub(crate) fn abort_upload(&self, upload_id: &str) -> bool {
        let Ok(mut inner) = self.lock() else {
            return false;
        };
        let mut removed = false;
        if inner
            .upload
            .as_ref()
            .is_some_and(|upload| upload.id == upload_id)
        {
            if let Some(upload) = inner.upload.take() {
                remove_dir(&upload.dir);
                removed = true;
            }
        }
        if inner
            .outgoing
            .as_ref()
            .is_some_and(|outgoing| outgoing.id == upload_id)
        {
            if let Some(outgoing) = inner.outgoing.take() {
                outgoing.cleanup();
                removed = true;
            }
        }
        removed
    }

    /// Test-only: stages descriptors directly so SIZE/RANGE serving (including
    /// directory rejection) can be exercised without a live channel.
    #[cfg(test)]
    pub(crate) fn stage_for_tests(&self, dir: PathBuf, entries: Vec<(PathBuf, u64, bool)>) {
        if let Ok(mut inner) = self.lock() {
            inner.outgoing = Some(StagedFiles {
                id: "test-upload".to_string(),
                dir: Some(dir),
                files: entries
                    .into_iter()
                    .map(|(path, size, directory)| StagedFile {
                        path,
                        size,
                        directory,
                    })
                    .collect(),
                descriptors: Vec::new(),
                source: OutgoingSource::Upload,
            });
        }
    }

    /// Serves an outbound `FileContentsRequest` from the staged files.
    pub(crate) fn file_contents_response(
        &self,
        request: &FileContentsRequest,
    ) -> FileContentsResponse<'static> {
        let Ok(inner) = self.lock() else {
            return FileContentsResponse::new_error(request.stream_id);
        };
        let Some(outgoing) = inner.outgoing.as_ref() else {
            return FileContentsResponse::new_error(request.stream_id);
        };
        let Some(file) = outgoing.files.get(request.index as usize) else {
            return FileContentsResponse::new_error(request.stream_id);
        };
        if file.directory {
            return FileContentsResponse::new_error(request.stream_id);
        }
        if request.flags.contains(FileContentsFlags::SIZE) {
            return FileContentsResponse::new_size_response(request.stream_id, file.size);
        }
        let remaining = file.size.saturating_sub(request.position);
        if remaining == 0 {
            return FileContentsResponse::new_error(request.stream_id);
        }
        let wanted = u64::from(request.requested_size)
            .min(CLIPBOARD_FILE_CHUNK_BYTES as u64)
            .min(remaining) as usize;
        if wanted == 0 {
            return FileContentsResponse::new_error(request.stream_id);
        }
        let mut buffer = vec![0u8; wanted];
        let content = File::open(&file.path).and_then(|mut handle| {
            handle.seek(SeekFrom::Start(request.position))?;
            handle.read_exact(&mut buffer)?;
            Ok(buffer)
        });
        match content {
            Ok(buffer) => FileContentsResponse::new_data_response(request.stream_id, buffer),
            Err(_) => FileContentsResponse::new_error(request.stream_id),
        }
    }
}
