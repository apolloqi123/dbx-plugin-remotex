//! Transfer operations for [`SshRuntime`].

use super::*;

impl SshRuntime {
    pub fn transfer_list(&self, session_id: &str) -> Result<Value, String> {
        let uploads = self
            .uploads
            .lock()
            .map_err(|_| "Upload registry is poisoned".to_string())?;
        let downloads = self
            .downloads
            .lock()
            .map_err(|_| "Download registry is poisoned".to_string())?;
        let finishing_uploads = self
            .finishing_uploads
            .lock()
            .map_err(|_| "Finishing upload registry is poisoned".to_string())?;
        let history = self
            .transfer_history
            .lock()
            .map_err(|_| "Transfer history is poisoned".to_string())?;
        let mut tasks = history
            .iter()
            .filter(|task| task.get("sessionId").and_then(Value::as_str) == Some(session_id))
            .cloned()
            .collect::<Vec<_>>();
        tasks.extend(
            uploads
                .iter()
                .filter(|(_, upload)| upload.session_id == session_id)
                .map(|(task_id, upload)| {
                    upload_progress_payload(
                        task_id,
                        session_id,
                        Some(upload.remote_path.rsplit('/').next().unwrap_or("upload")),
                        upload.received,
                        upload.expected_size,
                        UploadPhase::Staging,
                        "running",
                        upload.compression,
                    )
                })
                .collect::<Vec<_>>(),
        );
        tasks.extend(
            finishing_uploads
                .iter()
                .filter(|(_, upload)| upload.session_id == session_id)
                .map(|(task_id, upload)| {
                    upload_progress_payload(
                        task_id,
                        session_id,
                        Some(upload.remote_path.rsplit('/').next().unwrap_or("upload")),
                        upload.transferred.load(Ordering::Acquire),
                        upload.size,
                        UploadPhase::Uploading,
                        if upload.cancelled.load(Ordering::Acquire) {
                            "cancelled"
                        } else {
                            "running"
                        },
                        upload.compression,
                    )
                }),
        );
        tasks.extend(
            downloads
                .iter()
                .filter(|(_, download)| download.session_id == session_id)
                .map(|(task_id, download)| {
                    let mut row = json!({ "taskId": task_id, "sessionId": session_id, "direction": "download", "fileName": download.file_name, "size": download.size, "transferred": download.next_offset, "status": "running" });
                    if let Some(tree) = download.tree.as_ref() {
                        row["fileCount"] = json!(tree.file_count);
                    }
                    row
                }),
        );
        Ok(json!({ "tasks": tasks }))
    }

    pub async fn transfer_status(&self, task_id: &str) -> Result<Value, String> {
        if let Some(upload) = self
            .uploads
            .lock()
            .map_err(|_| "Upload registry is poisoned".to_string())?
            .get(task_id)
        {
            let status = upload_progress_payload(
                task_id,
                &upload.session_id,
                Some(upload.remote_path.rsplit('/').next().unwrap_or("upload")),
                upload.received,
                upload.expected_size,
                UploadPhase::Staging,
                "running",
                upload.compression,
            );
            return Ok(status);
        }
        if let Some(upload) = self
            .finishing_uploads
            .lock()
            .map_err(|_| "Finishing upload registry is poisoned".to_string())?
            .get(task_id)
        {
            let status = upload_progress_payload(
                task_id,
                &upload.session_id,
                Some(upload.remote_path.rsplit('/').next().unwrap_or("upload")),
                upload.transferred.load(Ordering::Acquire),
                upload.size,
                UploadPhase::Uploading,
                if upload.cancelled.load(Ordering::Acquire) {
                    "cancelled"
                } else {
                    "running"
                },
                upload.compression,
            );
            return Ok(status);
        }
        // Clone the lightweight snapshot before any await: the registry lock
        // must not be held across the tree progress lock. `DownloadState` is
        // cheap to clone now (the tree is shared via Arc).
        let download = self
            .downloads
            .lock()
            .map_err(|_| "Download registry is poisoned".to_string())?
            .get(task_id)
            .cloned();
        if let Some(download) = download {
            let mut status = json!({ "taskId": task_id, "sessionId": download.session_id, "direction": "download", "size": download.size, "transferred": download.next_offset, "status": "running" });
            if download.compression == CompressionMode::Gzip {
                // ready-wait 的兜底轮询面（M33）：事件丢失时前端据此判断
                // prep 是否完成；进行中的 prep 阶段从 prep 注册表取。
                status["compression"] = json!("gzip");
                status["ready"] = json!(download.ready);
                if !download.ready {
                    if let Ok(preps) = self.download_preps.lock() {
                        if let Some(prep) = preps.get(task_id) {
                            if let Ok(phase) = prep.phase.lock() {
                                status["phase"] = json!(*phase);
                            }
                        }
                    }
                }
            }
            if let Some(tree) = download.tree.as_ref() {
                // Static fields never need the progress lock. Dynamic fields
                // wait for the pump's short snapshot lock (all I/O happens
                // outside it), so the response shape is always complete: no
                // intermittently missing `filesRemaining`/`currentFile`.
                status["fileCount"] = json!(tree.file_count);
                let progress = tree.progress.lock().await;
                status["filesRemaining"] =
                    json!(progress.files.len() + usize::from(progress.current.is_some()));
                status["currentFile"] =
                    json!(progress.current.as_ref().map(|file| file.relative.clone()));
            }
            return Ok(status);
        }
        if let Some(task) = self
            .transfer_history
            .lock()
            .map_err(|_| "Transfer history is poisoned".to_string())?
            .iter()
            .find(|task| task.get("taskId").and_then(Value::as_str) == Some(task_id))
        {
            return Ok(task.clone());
        }
        Err("Transfer task was not found".to_string())
    }

    /// `sftp/transfer/history`: merges the persisted history with the
    /// in-memory live tasks, deduplicated by taskId (the live snapshot wins
    /// because it carries fresh progress), newest first. Pure local data, so
    /// it answers without any active connection.
    pub async fn transfer_history_query(
        &self,
        session_id: Option<&str>,
        connection_id: Option<&str>,
        limit: usize,
    ) -> Result<Value, String> {
        let sessions = self.sessions.read().await;
        let connection_id_for = |sid: &str| {
            sessions
                .get(sid)
                .map(|session| session.connection_id.clone())
                .unwrap_or_default()
        };
        self.build_transfer_history(session_id, connection_id, limit, &connection_id_for)
    }

    pub async fn clear_transfer_history(&self, connection_id: Option<&str>) -> Result<(), String> {
        let mut history = self
            .transfer_history
            .lock()
            .map_err(|_| "Transfer history is poisoned".to_string())?;
        match connection_id.filter(|scope| !scope.is_empty()) {
            Some(scope) => history
                .retain(|task| task.get("connectionId").and_then(Value::as_str) != Some(scope)),
            None => history.clear(),
        }
        drop(history);
        transfer_history::clear_history(&self.data_dir, connection_id)
    }

    /// Synchronous core of `transfer_history_query`; `connection_id_for`
    /// resolves the owning connection of a session (empty when unknown) so
    /// tests can inject the lookup without live sessions.
    pub(crate) fn build_transfer_history(
        &self,
        session_id: Option<&str>,
        connection_id: Option<&str>,
        limit: usize,
        connection_id_for: &impl Fn(&str) -> String,
    ) -> Result<Value, String> {
        // Stale running/queued rows from a dead sidecar surface as failed
        // here (they are never rewritten on disk, so another live process'
        // in-flight transfers stay untouched).
        let mut tasks = transfer_history::load_history(&self.data_dir);
        for row in self.live_transfer_rows(connection_id_for) {
            let task_id = row
                .get("taskId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            // The live snapshot has no start context of its own; keep the
            // persisted row's startedAt/connectionId when present.
            let mut merged = row;
            if let Some(persisted) = tasks
                .iter()
                .find(|task| task.get("taskId").and_then(Value::as_str) == Some(task_id.as_str()))
            {
                for key in ["startedAt", "connectionId"] {
                    if let Some(value) = persisted.get(key) {
                        merged[key] = value.clone();
                    }
                }
            }
            tasks.retain(|task| {
                task.get("taskId").and_then(Value::as_str) != Some(task_id.as_str())
            });
            tasks.push(merged);
        }
        if let Some(filter) = session_id {
            tasks.retain(|task| task.get("sessionId").and_then(Value::as_str) == Some(filter));
        }
        // Stable saved-connection ownership is a second, independent AND
        // filter. Legacy rows without the key never match a scoped view.
        if let Some(filter) = connection_id.filter(|value| !value.is_empty()) {
            tasks.retain(|task| task.get("connectionId").and_then(Value::as_str) == Some(filter));
        }
        // Issue #18: the previous sort keyed on `startedAt` alone and left
        // rows without one (live rows whose start record never reached the
        // disk) stacked at the bottom in whatever order the in-memory
        // registries happened to iterate — so the same task jumped between
        // the top, middle and bottom of the panel between refreshes. A total
        // order with a taskId tiebreak keeps every poll deterministic.
        tasks.sort_by(compare_history_rows);
        tasks.truncate(limit);
        Ok(json!({ "tasks": tasks }))
    }

    /// Live snapshot rows from the in-memory transfer registries. These rows
    /// carry fresh progress but no start context; `build_transfer_history`
    /// merges them over the persisted rows. A poisoned registry is skipped:
    /// the history query stays answerable from the persisted store.
    pub(crate) fn live_transfer_rows(
        &self,
        connection_id_for: &impl Fn(&str) -> String,
    ) -> Vec<Value> {
        let mut rows = Vec::new();
        if let Ok(uploads) = self.uploads.lock() {
            for (task_id, upload) in uploads.iter() {
                rows.push(json!({
                    "taskId": task_id,
                    "sessionId": upload.session_id,
                    "connectionId": connection_id_for(&upload.session_id),
                    "direction": "upload",
                    "fileName": upload.remote_path.rsplit('/').next().unwrap_or("upload"),
                    "size": upload.expected_size,
                    "transferred": upload.received,
                    "status": "running",
                }));
            }
        }
        if let Ok(finishing) = self.finishing_uploads.lock() {
            for (task_id, upload) in finishing.iter() {
                rows.push(json!({
                    "taskId": task_id,
                    "sessionId": upload.session_id,
                    "connectionId": connection_id_for(&upload.session_id),
                    "direction": "upload",
                    "fileName": upload.remote_path.rsplit('/').next().unwrap_or("upload"),
                    "size": upload.size,
                    "transferred": upload.transferred.load(Ordering::Acquire),
                    "status": if upload.cancelled.load(Ordering::Acquire) { "cancelled" } else { "running" },
                }));
            }
        }
        if let Ok(downloads) = self.downloads.lock() {
            for (task_id, download) in downloads.iter() {
                rows.push(json!({
                    "taskId": task_id,
                    "sessionId": download.session_id,
                    "connectionId": connection_id_for(&download.session_id),
                    "direction": "download",
                    "fileName": download.file_name,
                    "size": download.size,
                    "transferred": download.next_offset,
                    "status": "running",
                }));
            }
        }
        rows
    }
}
