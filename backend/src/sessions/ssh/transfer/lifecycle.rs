//! Transfer operations for [`SshRuntime`].

use super::*;
use crate::core::time::unix_now_ms;

impl SshRuntime {
    pub(crate) async fn record_transfer(&self, mut task: Value) {
        let Some(task_id) = task
            .get("taskId")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        let session_id = task
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let explicit = task
            .get("connectionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or_default();
        // Resolve the stable saved-connection owner before the live registry
        // entry disappears: terminal transitions are raised from in-memory
        // state that only carries the ephemeral session id.
        let session_connection = if explicit.is_empty() {
            self.session_connection_id(session_id).await
        } else {
            String::new()
        };
        let persisted_connection = if explicit.is_empty() && session_connection.is_empty() {
            transfer_history::load_history(&self.data_dir)
                .iter()
                .find(|row| row.get("taskId").and_then(Value::as_str) == Some(task_id.as_str()))
                .and_then(|row| row.get("connectionId").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string()
        } else {
            String::new()
        };
        let resolved = resolve_transfer_connection_id(
            &task,
            &session_connection,
            &persisted_connection,
        );
        if !resolved.is_empty() {
            task["connectionId"] = json!(resolved);
        }
        if let Ok(mut history) = self.transfer_history.lock() {
            history.retain(|entry| entry.get("taskId").and_then(Value::as_str) != Some(task_id.as_str()));
            history.push_back(task.clone());
            while history.len() > 64 {
                history.pop_front();
            }
        }
        // Terminal transitions also land in <data_dir>/transfer-history.json
        // so the workbench keeps them across sidecar restarts. Cross-process
        // semantics are last-writer-wins (see transfer_history.rs).
        self.persist_transfer_record(&task);
    }

    /// Persists the start transition of a transfer job. Live tasks stay in
    /// the in-memory registries; the persisted history only sees status
    /// transitions, never per-chunk progress.
    pub(crate) fn record_transfer_start(
        &self,
        task_id: &str,
        session_id: &str,
        connection_id: &str,
        direction: &str,
        file_name: &str,
        size: u64,
    ) {
        self.persist_transfer_record(&json!({
            "taskId": task_id,
            "sessionId": session_id,
            "connectionId": connection_id,
            "direction": direction,
            "fileName": file_name,
            "size": size,
            "transferred": 0,
            "status": "running",
            "startedAt": unix_now_ms(),
            "finishedAt": Value::Null,
        }));
    }

    /// Best-effort persistence of one status transition; a failed write is
    /// logged but never fails the transfer itself (history is UX data, not
    /// an audit ledger).
    pub(crate) fn persist_transfer_record(&self, task: &Value) {
        if let Err(error) = transfer_history::record_transition(&self.data_dir, task) {
            eprintln!("[ssh-trace] failed to persist transfer history: {error}");
        }
    }

    /// Connection owning a session, or empty when the session is no longer
    /// registered (sidecar restarted after the workbench attached).
    pub(crate) async fn session_connection_id(&self, session_id: &str) -> String {
        self.sessions
            .read()
            .await
            .get(session_id)
            .map(|session| session.connection_id.clone())
            .unwrap_or_default()
    }

    /// 会话级传输并发深度（M14-B）：偏好 `transfer_max_active` 可配
    /// （1..=8，缺省 3 = 历史硬编码值）；老旧服务器兼容模式下强制 1。
    /// 每次任务启动现读现用——改动即时生效，进行中的任务按原深度自然完成。
    pub(crate) fn transfer_depth_limit(&self) -> u64 {
        if crate::core::preferences::sftp_compat_mode(&self.data_dir) {
            return 1;
        }
        crate::core::preferences::transfer_max_active(&self.data_dir)
    }

    pub(crate) fn active_transfer_count(&self, session_id: &str) -> Result<usize, String> {
        let uploads = self
            .uploads
            .lock()
            .map_err(|_| "Upload registry is poisoned".to_string())?
            .values()
            .filter(|upload| upload.session_id == session_id)
            .count();
        let downloads = self
            .downloads
            .lock()
            .map_err(|_| "Download registry is poisoned".to_string())?
            .values()
            .filter(|download| download.session_id == session_id)
            .count();
        let finishing_uploads = self
            .finishing_uploads
            .lock()
            .map_err(|_| "Finishing upload registry is poisoned".to_string())?
            .values()
            .filter(|upload| upload.session_id == session_id)
            .count();
        Ok(uploads + finishing_uploads + downloads)
    }

    pub(crate) async fn cleanup_session_transfers(
        &self,
        session_id: &str,
        connection_id: &str,
    ) -> Result<(), String> {
        // Both teardown paths remove the session from the registry before
        // calling this method, so the caller must capture the stable owner
        // from the removed entry; a registry lookup here would always be
        // empty and strand the failed rows as unattributed.
        let connection_id = connection_id.to_string();
        let removed_uploads = {
            let mut uploads = self
                .uploads
                .lock()
                .map_err(|_| "Upload registry is poisoned".to_string())?;
            let task_ids = uploads
                .iter()
                .filter(|(_, upload)| upload.session_id == session_id)
                .map(|(task_id, _)| task_id.clone())
                .collect::<Vec<_>>();
            task_ids
                .into_iter()
                .filter_map(|task_id| uploads.remove_entry(&task_id))
                .collect::<Vec<_>>()
        };
        for (task_id, upload) in removed_uploads {
            let _ = std::fs::remove_file(&upload.local_path);
            // 压缩预压半成品与 sidecar meta 同属本任务（评审 L-2）：完成
            // 路径已清的这里重复删除无害。
            let _ = std::fs::remove_file(format!("{}.gz", upload.local_path.display()));
            remove_upload_meta(&self.transfer_dir, &task_id);
            // A session close aborts its unfinished uploads; record them as
            // failed so the persisted history keeps no silent ghosts.
            self.persist_transfer_record(&json!({
                "taskId": task_id,
                "sessionId": upload.session_id,
                "connectionId": connection_id.as_str(),
                "direction": "upload",
                "fileName": upload.remote_path.rsplit('/').next().unwrap_or("upload"),
                "size": upload.expected_size,
                "transferred": upload.received,
                "status": "failed",
                "error": "Transfer was aborted because the session closed",
            }));
        }
        let removed_downloads = {
            let mut downloads = self
                .downloads
                .lock()
                .map_err(|_| "Download registry is poisoned".to_string())?;
            let task_ids = downloads
                .iter()
                .filter(|(_, download)| download.session_id == session_id)
                .map(|(task_id, _)| task_id.clone())
                .collect::<Vec<_>>();
            task_ids
                .into_iter()
                .filter_map(|task_id| downloads.remove_entry(&task_id))
                .collect::<Vec<_>>()
        };
        for (task_id, download) in removed_downloads {
            // 会话关闭与取消是等价的清理事件（评审 H-2）：树半成品根、
            // 压缩 plain/.gz、远端临时件统一穿过 cleanup_download_assets，
            // 不再只删 sink .part。（registry guard 不跨 await——摘除在
            // 锁内完成，清理在锁外执行。）
            self.cleanup_download_assets(&download.session_id, &download)
                .await;
            self.persist_transfer_record(&json!({
                "taskId": task_id,
                "sessionId": download.session_id,
                "connectionId": connection_id.as_str(),
                "direction": "download",
                "fileName": download.file_name,
                "size": download.size,
                "transferred": download.next_offset,
                "status": "failed",
                "error": "Transfer was aborted because the session closed",
            }));
        }
        for upload in self
            .finishing_uploads
            .lock()
            .map_err(|_| "Finishing upload registry is poisoned".to_string())?
            .values()
            .filter(|upload| upload.session_id == session_id)
        {
            upload.cancelled.store(true, Ordering::Release);
        }
        if let Ok(mut history) = self.transfer_history.lock() {
            // Keep rows owned by other connections. Legacy rows without a
            // `connectionId` still fall back to the closing session id so
            // disconnect clears the ghosts it used to clear.
            history.retain(|task| {
                match task.get("connectionId").and_then(Value::as_str) {
                    Some(scope) if !scope.is_empty() && !connection_id.is_empty() => {
                        scope != connection_id
                    }
                    _ => task.get("sessionId").and_then(Value::as_str) != Some(session_id),
                }
            });
        }
        Ok(())
    }
}

/// Pure ownership resolution shared by terminal transitions: an explicit
/// payload scope wins, then the still-registered session's connection, then
/// the persisted start row. An empty result means legacy/unattributed.
pub(crate) fn resolve_transfer_connection_id(
    task: &Value,
    session_connection_id: &str,
    persisted_connection_id: &str,
) -> String {
    task.get("connectionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .or_else(|| Some(session_connection_id).filter(|value| !value.is_empty()))
        .or_else(|| Some(persisted_connection_id).filter(|value| !value.is_empty()))
        .unwrap_or_default()
        .to_string()
}
