use super::*;

impl SshRuntime {
    /// Whether the task is still registered (cancel/finish remove it first).
    /// Checked between chunk I/O steps so an in-flight chunk aborts early.
    pub(super) fn tree_download_active(&self, task_id: &str) -> Result<bool, String> {
        self.downloads
            .lock()
            .map(|downloads| downloads.contains_key(task_id))
            .map_err(|_| "Download registry is poisoned".to_string())
    }

    /// Opens the high-level read handle for one tree file. Production opens an
    /// SFTP handle; tests open `<local_source_root>/<relative>` so the pump's
    /// reuse/seek/drop semantics are covered without a server.
    async fn open_tree_read_handle(
        &self,
        _tree: &TreeDownloadState,
        session_id: &str,
        file: &sftp_tree::TreeFile,
    ) -> Result<TreeReadHandle, String> {
        #[cfg(test)]
        if let Some(root) = _tree.local_source_root.as_ref() {
            _tree.opened_handles.fetch_add(1, Ordering::Relaxed);
            let source = tokio::fs::File::open(root.join(&file.relative))
                .await
                .map_err(|error| format!("SFTP download failed: {error}"))?;
            return Ok(Box::new(source));
        }
        let sftp = self.sftp(session_id).await?;
        let guard = sftp.lock().await;
        let opened = guard
            .open(file.remote_path.clone())
            .await
            .map_err(sftp_error)?;
        drop(guard);
        #[cfg(test)]
        _tree.opened_handles.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(opened))
    }

    /// High-level lane for one chunk: reuses the cached handle for the same
    /// file, opens a fresh one on file switch, and moves the handle out of
    /// `progress` across seek/read so cancel/finish never wait on the network.
    pub(super) async fn read_tree_high_level(
        &self,
        tree: &TreeDownloadState,
        session_id: &str,
        file: &sftp_tree::TreeFile,
        current_offset: u64,
        requested: usize,
    ) -> Result<(Vec<u8>, usize, TreeReadHandle), TreeReadFailure> {
        let cached = {
            let mut progress = tree.progress.lock().await;
            if progress.handle_path.as_deref() == Some(file.remote_path.as_str()) {
                progress.read_handle.take()
            } else {
                // 切换文件：旧句柄的 Drop 发 close_nowait，槽位置 None 即完成
                // 关闭；新文件在锁外重新 open。
                progress.read_handle = None;
                progress.handle_path = None;
                None
            }
        };
        let mut source = match cached {
            Some(source) => source,
            None => match self.open_tree_read_handle(tree, session_id, file).await {
                Ok(source) => source,
                Err(message) => {
                    let mut progress = tree.progress.lock().await;
                    progress.read_handle = None;
                    progress.handle_path = None;
                    return Err(TreeReadFailure {
                        channel_level: is_channel_level_sftp_error(&message),
                        message,
                    });
                }
            },
        };
        if let Err(error) = source.seek(std::io::SeekFrom::Start(current_offset)).await {
            let mut progress = tree.progress.lock().await;
            progress.read_handle = None;
            progress.handle_path = None;
            return Err(TreeReadFailure {
                message: format!("SFTP download seek failed: {error}"),
                channel_level: false,
            });
        }
        let mut chunk = vec![0_u8; requested];
        match source.read(&mut chunk).await {
            Ok(length) => Ok((chunk, length, source)),
            Err(error) => {
                let message = format!("SFTP download failed: {error}");
                let channel_level = is_channel_level_sftp_error(&message);
                let mut progress = tree.progress.lock().await;
                progress.read_handle = None;
                progress.handle_path = None;
                Err(TreeReadFailure {
                    message,
                    channel_level,
                })
            }
        }
    }
}

/// Failure of one tree-file read. `channel_level` errors invalidate the whole
/// SFTP client cache and abort the batch instead of failing every queued file.
pub(super) struct TreeReadFailure {
    pub(super) message: String,
    pub(super) channel_level: bool,
}
