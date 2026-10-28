//! Transfer operations for [`SshRuntime`].

use super::*;

impl SshRuntime {
    pub async fn complete_download(
        &self,
        task_id: &str,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        // Folder finishes serialize with the chunk pump *before* the registry
        // entry is removed: removal is the pump's cancel signal, so a chunk
        // committing its final file would abort after staging the bytes while
        // finish (snapshotting the offset) would delete that `.part` and still
        // report completed. Waiting here lets the chunk rename the last file
        // first; the guard is then handed to `complete_tree_download` so the
        // completeness check cannot interleave with a commit either.
        let tree = self
            .downloads
            .lock()
            .map_err(|_| "Download registry is poisoned".to_string())?
            .get(task_id)
            .and_then(|download| download.tree.clone());
        if let Some(tree) = tree {
            let pump = tree.pump.lock().await;
            let download = self
                .downloads
                .lock()
                .map_err(|_| "Download registry is poisoned".to_string())?
                .remove(task_id)
                .ok_or("Download task was not found".to_string())?;
            return self
                .complete_tree_download(download, task_id, emitter, pump)
                .await;
        }
        let download = self
            .downloads
            .lock()
            .map_err(|_| "Download registry is poisoned".to_string())?
            .remove(task_id)
            .ok_or("Download task was not found".to_string())?;
        let sudo_tmp = download.sudo_tmp.clone();
        let session_id = download.session_id.clone();
        let compressed_cleanup = (download.compression == CompressionMode::Gzip).then(|| {
            (
                download.remote_temp.clone(),
                download.gz_staging.clone(),
                download.staged_plain.clone(),
                download.final_dir.is_none(),
            )
        });
        let result = self
            .complete_single_file_download(download, task_id, emitter)
            .await;
        // 压缩任务终态（M33，finally 语义）：远端 .gz 与本地压缩流暂存在
        // 完成/失败两条路径都清；plain 解压暂存只在「浏览器模式且成功」时
        // 清（字节已在前端）——saveToLocal 失败保留，与普通 .part「字节不
        // 丢」语义一致。
        if let Some((remote_temp, gz_staging, staged_plain, browser_mode)) = compressed_cleanup {
            self.cleanup_compressed_download(
                &session_id,
                remote_temp.as_deref(),
                gz_staging.as_deref(),
                staged_plain
                    .as_deref()
                    .filter(|_| browser_mode && result.is_ok()),
            )
            .await;
        }
        // finally 语义：sudo 下载的远端临时件在完成/失败两条路径上都要清理；
        // 清理失败不吞掉原结果，只在成功响应上附加 warning 兜底提示。
        if let Some(tmp) = sudo_tmp {
            match sudo_download::discard_tmp(self, &session_id, &tmp).await {
                Ok(()) => {}
                Err(error) => {
                    eprintln!("[sudo-download] temp cleanup failed ({tmp}): {error}");
                    let mut result = result;
                    if let Ok(response) = result.as_mut() {
                        response["warning"] = json!(format!(
                            "Remote sudo temp file cleanup failed: {tmp} ({error})"
                        ));
                    }
                    return result;
                }
            }
        }
        result
    }

    /// Single-file half of `sftp/download/finish`, extracted so the sudo
    /// variant can run its remote temp cleanup around it.
    pub(crate) async fn complete_single_file_download(
        &self,
        download: DownloadState,
        task_id: &str,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let record_failed = |error: &str| {
            self.record_transfer(json!({ "taskId": task_id, "sessionId": download.session_id, "direction": "download", "fileName": download.file_name, "size": download.size, "transferred": download.next_offset, "status": "failed", "error": error }));
        };
        if download.next_offset < download.size {
            let error = format!(
                "Download is incomplete: received {} of {} bytes",
                download.next_offset, download.size
            );
            // The task dies here without reaching the completed path, so
            // the failure is recorded explicitly.
            record_failed(&error);
            return Err(error);
        }
        let local_path = if let Some(staged) = download.staged_plain.as_ref() {
            // 压缩任务（M33）：saveToLocal 由解压暂存 rename 落位（浏览器
            // 模式字节在前端，暂存由 complete_download 包装层清理）。
            match download.final_dir.as_ref() {
                Some(dir) => {
                    match Self::finalize_staged_plain(
                        staged,
                        dir,
                        download.overwrite,
                        &download.file_name,
                    ) {
                        Ok(path) => Some(path),
                        Err(error) => {
                            record_failed(&error);
                            return Err(error);
                        }
                    }
                }
                None => None,
            }
        } else {
            match download.sink.as_ref() {
                None => None,
                Some(sink) => {
                    match Self::finalize_download_sink(self, sink, &download.file_name).await {
                        Ok(path) => Some(path),
                        Err(error) => {
                            record_failed(&error);
                            return Err(error);
                        }
                    }
                }
            }
        };
        let mut task = json!({ "taskId": task_id, "sessionId": download.session_id, "direction": "download", "fileName": download.file_name, "size": download.size, "transferred": download.size, "status": "completed" });
        if let Some(path) = local_path.as_ref() {
            task["localPath"] = json!(path.to_string_lossy());
        }
        self.record_transfer(task.clone());
        emitter
            .event("sftp/transfer/progress", task)
            .map_err(plugin_error)?;
        Ok(json!({
            "success": true,
            "taskId": task_id,
            "localPath": local_path.as_ref().map(|path| path.to_string_lossy()),
        }))
    }

    /// Tree flavor of `sftp/download/finish`: every queued file must be
    /// drained (eof), otherwise the whole folder is torn down — there is no
    /// tree resume, so a half-downloaded folder never lingers on disk. A
    /// drained tree completes even when individual files failed: the summary
    /// (`failedCount` + inline samples) is the contract for the workbench
    /// notice.
    pub(crate) async fn complete_tree_download(
        &self,
        download: DownloadState,
        task_id: &str,
        emitter: &PluginEmitter,
        _pump: tokio::sync::MutexGuard<'_, ()>,
    ) -> Result<Value, String> {
        let Some(tree) = download.tree.as_ref() else {
            return Err("Download task is not a folder download".to_string());
        };
        // 压缩树（M33）终态清理：远端 .tgz 与本地压缩流暂存在此入口即清
        // （解包已在 prep 完成，完成/未完成两态都不再需要；finally 语义与
        // sudo 临时件同向，取消路径由 cancel_transfer 负责）。清理失败不
        // 吞掉原结果，只留 stderr 诊断。
        if download.compression == CompressionMode::Gzip {
            self.cleanup_compressed_download(
                &download.session_id,
                download.remote_temp.as_deref(),
                download.gz_staging.as_deref(),
                None,
            )
            .await;
        }
        // The caller acquired `tree.pump` before removing the task from the
        // registry, so no chunk can be mid-commit here: a fully transferred
        // current file has already been renamed and cleared from `progress`.
        // The incomplete path can therefore tear the root down without racing
        // a writer, and the summary below sees the pump's final state.
        let remaining = {
            let progress = tree.progress.lock().await;
            progress.files.len()
                + usize::from(
                    progress
                        .current
                        .as_ref()
                        .is_some_and(|file| progress.current_offset < file.size),
                )
        };
        if remaining > 0 {
            // 未传完：整树拆除（无目录续传语义），失败入账。
            let _ = std::fs::remove_dir_all(&tree.root_local);
            let error =
                format!("Folder download is incomplete: {remaining} file(s) not transferred");
            self.record_transfer(json!({ "taskId": task_id, "sessionId": download.session_id, "direction": "download", "fileName": download.file_name, "size": download.size, "transferred": download.next_offset, "status": "failed", "error": error }));
            return Err(error);
        }
        let (failed_count, failed_files, leftover_sink) = {
            let progress = tree.progress.lock().await;
            let (failed_count, failed_files) = sftp_tree::failure_report(&progress.failures);
            (failed_count, failed_files, progress.sink.clone())
        };
        // 正常路径不会到达（current 已全部收尾）；防御性清理残留 .part。
        if let Some(sink) = leftover_sink {
            let _ = std::fs::remove_file(&sink.part_path);
        }
        {
            let mut progress = tree.progress.lock().await;
            progress.read_handle = None;
            progress.handle_path = None;
            progress.sink = None;
        }
        // 压缩树（M33）：staging 树 rename 落到最终名（staging 是落点目录
        // 内的隐藏目录，同卷 rename 零拷贝；跨卷失败递归拷贝兜底）。落位
        // 失败保留 staging 并按失败入账（与 sink 的「字节不丢」同向）。
        let final_root = if download.compression == CompressionMode::Gzip {
            match download.final_dir.as_ref() {
                Some(dir) => {
                    match Self::finalize_tree_staging(&tree.root_local, dir, &download.file_name) {
                        Ok(path) => path,
                        Err(error) => {
                            let error =
                                format!("Failed to move the downloaded folder into place: {error}");
                            self.record_transfer(json!({ "taskId": task_id, "sessionId": download.session_id, "direction": "download", "fileName": download.file_name, "size": download.size, "transferred": download.next_offset, "status": "failed", "error": error }));
                            return Err(error);
                        }
                    }
                }
                None => tree.root_local.clone(),
            }
        } else {
            tree.root_local.clone()
        };
        let mut task = json!({
            "taskId": task_id, "sessionId": download.session_id, "direction": "download",
            "fileName": download.file_name, "size": download.size, "transferred": download.next_offset,
            "status": "completed",
            "fileCount": tree.file_count, "failedCount": failed_count, "skippedCount": tree.skipped,
        });
        task["localPath"] = json!(final_root.to_string_lossy());
        self.record_transfer(task.clone());
        emitter
            .event("sftp/transfer/progress", task)
            .map_err(plugin_error)?;
        Ok(json!({
            "success": true,
            "taskId": task_id,
            "localPath": final_root.to_string_lossy(),
            "fileCount": tree.file_count,
            "failedCount": failed_count,
            "skippedCount": tree.skipped,
            "failedFiles": failed_files,
        }))
    }

    /// Flushes the staging file and moves it to its final non-colliding name
    /// in the user's Downloads folder. Rename first (same volume = cheap);
    /// staging and Downloads can live on different volumes, so a failed rename
    /// falls back to copy+delete. The staging file survives failures so the
    /// bytes are never lost silently.
    pub(crate) async fn finalize_download_sink(
        &self,
        sink: &DownloadSink,
        file_name: &str,
    ) -> Result<PathBuf, String> {
        {
            let file = sink.file.lock().await;
            file.sync_all()
                .await
                .map_err(|error| format!("Failed to flush local download file: {error}"))?;
        }
        let final_path = if sink.overwrite {
            local_downloads::final_download_path(&sink.final_dir, file_name, true)
        } else {
            local_downloads::reserve_download_path(&sink.final_dir, file_name)
                .map_err(|error| format!("Failed to reserve a download path: {error}"))?
        };
        if std::fs::rename(&sink.part_path, &final_path).is_ok() {
            return Ok(final_path);
        }
        let result = std::fs::copy(&sink.part_path, &final_path)
            .and_then(|_| std::fs::remove_file(&sink.part_path));
        if let Err(error) = result {
            if !sink.overwrite {
                let _ = std::fs::remove_file(&final_path);
            }
            return Err(format!(
                "Failed to move the download into '{}': {error}",
                final_path.display()
            ));
        }
        Ok(final_path)
    }

    /// 压缩单文件任务 saveToLocal 落位（M33）：解压暂存 rename（同卷）→
    /// 跨卷回退 copy+delete，语义与 [`Self::finalize_download_sink`] 对齐。
    pub(crate) fn finalize_staged_plain(
        staged: &Path,
        final_dir: &Path,
        overwrite: bool,
        file_name: &str,
    ) -> Result<PathBuf, String> {
        let final_path = if overwrite {
            local_downloads::final_download_path(final_dir, file_name, true)
        } else {
            local_downloads::reserve_download_path(final_dir, file_name)
                .map_err(|error| format!("Failed to reserve a download path: {error}"))?
        };
        if std::fs::rename(staged, &final_path).is_ok() {
            return Ok(final_path);
        }
        let result = std::fs::copy(staged, &final_path)
            .and_then(|_| std::fs::remove_file(staged));
        if let Err(error) = result {
            if !overwrite {
                let _ = std::fs::remove_file(&final_path);
            }
            return Err(format!(
                "Failed to move the download into '{}': {error}",
                final_path.display()
            ));
        }
        Ok(final_path)
    }

    /// 压缩树任务落位（M33）：staging 树 rename 成最终根名（staging 是落
    /// 点目录内的隐藏目录，同卷 rename 几乎必成）；跨卷失败递归拷贝兜底。
    pub(crate) fn finalize_tree_staging(
        staging: &Path,
        final_dir: &Path,
        root_name: &str,
    ) -> Result<PathBuf, String> {
        // Directory names cannot be reserved with create_new; serialize the
        // pick→rename window so two folder downloads with the same root name
        // cannot both choose the same target. Rename is the common path;
        // cross-volume copy under this lock is rare.
        let _guard = finalize_dir_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let final_path = local_downloads::final_download_path(final_dir, root_name, false);
        if std::fs::rename(staging, &final_path).is_ok() {
            return Ok(final_path);
        }
        copy_dir_recursive(staging, &final_path).map_err(|error| {
            format!(
                "Failed to move the download into '{}': {error}",
                final_path.display()
            )
        })?;
        let _ = std::fs::remove_dir_all(staging);
        Ok(final_path)
    }
}

fn finalize_dir_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}
