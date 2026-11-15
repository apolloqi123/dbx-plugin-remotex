//! Transfer operations for [`SshRuntime`].

use super::*;

impl SshRuntime {
    pub async fn cancel_transfer(
        &self,
        task_id: &str,
        reason: Option<&str>,
        emitter: &PluginEmitter,
    ) -> Result<(), String> {
        let upload = self
            .uploads
            .lock()
            .map_err(|_| "Upload registry is poisoned".to_string())?
            .remove(task_id);
        let download = self
            .downloads
            .lock()
            .map_err(|_| "Download registry is poisoned".to_string())?
            .remove(task_id);
        let finishing = self
            .finishing_uploads
            .lock()
            .map_err(|_| "Finishing upload registry is poisoned".to_string())?
            .get(task_id)
            .map(|upload| {
                upload.cancelled.store(true, Ordering::Release);
                if let Ok(mut slot) = upload.cancel_reason.lock() {
                    if slot.is_none() {
                        *slot = reason.map(str::to_string);
                    }
                }
                let mut task = upload_progress_payload(
                    task_id,
                    &upload.session_id,
                    Some(upload.remote_path.rsplit('/').next().unwrap_or("upload")),
                    upload.transferred.load(Ordering::Acquire),
                    upload.size,
                    UploadPhase::Uploading,
                    "cancelled",
                    upload.compression,
                );
                task["error"] = json!(upload_cancel_error(reason));
                (
                    task,
                    upload.session_id.clone(),
                    upload.remote_path.clone(),
                    upload.sudo_staging.clone(),
                )
            });
        // 压缩下载 prep（M33）：置取消旗标让 prep 循环在块边界退出；prep
        // 独占（任务已不在 downloads registry 的竞窗）时由这里直接清理。
        let prep = self
            .download_preps
            .lock()
            .map_err(|_| "Download prep registry is poisoned".to_string())?
            .get(task_id)
            .map(|prep| {
                prep.cancelled.store(true, Ordering::Release);
                if let Ok(mut slot) = prep.cancel_reason.lock() {
                    if slot.is_none() {
                        *slot = reason.map(str::to_string);
                    }
                }
                (
                    prep.remote_temp.clone(),
                    prep.gz_staging.clone(),
                    prep.session_id.clone(),
                    prep.phase.clone(),
                )
            });
        if let Some(upload) = upload.as_ref() {
            let _ = std::fs::remove_file(&upload.local_path);
            remove_upload_meta(&self.transfer_dir, task_id);
        }
        if let Some(download) = download.as_ref() {
            self.cleanup_download_assets(&download.session_id, download)
                .await;
        }
        // prep 独占的取消（任务条目已被先前的取消摘除）：直接清压缩产物。
        if let (true, Some((remote_temp, gz_staging, session_id, _))) =
            (download.is_none(), prep.as_ref())
        {
            if let Some(tmp) = remote_temp {
                if let Ok(sftp) = self.sftp(session_id).await {
                    let _ = sftp.lock().await.remove_file(tmp.clone()).await;
                }
            }
            let _ = std::fs::remove_file(gz_staging);
        }
        if upload.is_none() && download.is_none() && finishing.is_none() && prep.is_none() {
            return Err("Transfer task was not found".to_string());
        }
        // 推送阶段取消的远端收尾提前：推送循环要到下一个分块边界才观察到
        // cancelled 标志并自行删除远端临时文件，取消 RPC 返回后立刻列目录会
        // 看见最长一个分块周期的 .part 残留（#60 回归记录的竞窗）。这里
        // best-effort 提前删掉推送暂存件。只删暂存件，绝不碰 .backup——取消
        // 可能与提交链的 target→backup→target 往返并发，删 backup 会破坏回滚；
        // 与推送循环自身的 remove 并发安全（重复删除只是一次无害的 NoSuchFile）。
        // sudo 车道（UploadSudo）的推送暂存件在登录用户 home 下，按注册时
        // 定死的 sudo_staging 删，不落目标目录。
        if let Some((_, session_id, remote_path, sudo_staging)) = finishing.as_ref() {
            let temporary = match sudo_staging.as_ref() {
                Some(staging) => staging.clone(),
                None => match remote_transfer_paths(remote_path, task_id) {
                    Ok((temporary, _backup)) => temporary,
                    Err(_) => String::new(),
                },
            };
            if !temporary.is_empty() {
                match self.sftp(session_id).await {
                    Ok(sftp) => {
                        if let Err(error) = sftp.lock().await.remove_file(temporary).await {
                            eprintln!("[sftp] cancel cleanup: remote temp not removed: {error}");
                        }
                    }
                    Err(error) => {
                        eprintln!("[sftp] cancel cleanup: session unavailable: {error}")
                    }
                }
            }
        }
        let finishing = finishing.map(|(task, _, _, _)| task);
        let task = upload
            .as_ref()
            .map(|upload| {
                let mut task = upload_progress_payload(
                    task_id,
                    &upload.session_id,
                    Some(upload.remote_path.rsplit('/').next().unwrap_or("upload")),
                    upload.received,
                    upload.expected_size,
                    UploadPhase::Staging,
                    "cancelled",
                    upload.compression,
                );
                task["error"] = json!(upload_cancel_error(reason));
                task
            })
            .or_else(|| download.as_ref().map(|download| json!({ "taskId": task_id, "sessionId": download.session_id, "direction": "download", "fileName": download.file_name, "size": download.size, "transferred": download.next_offset, "status": "cancelled" })))
            .or(finishing)
            // prep-only cancel: an earlier cancel already removed the task
            // entry while the compression prep loop still runs. Surface the
            // cancellation from the prep record instead of panicking on an
            // absent transfer (the prep itself is torn down by its loop on
            // the cancel flag we just set).
            .or_else(|| {
                prep.as_ref().map(|(_, _, session_id, phase)| {
                    let mut task = json!({
                        "taskId": task_id,
                        "sessionId": session_id,
                        "direction": "download",
                        "status": "cancelled",
                        "compression": CompressionMode::Gzip.as_str(),
                        "error": download_cancel_error(reason),
                    });
                    if let Ok(phase) = phase.lock() {
                        task["phase"] = json!(*phase);
                    }
                    task
                })
            });
        let Some(task) = task else {
            return Err("Transfer task was not found".to_string());
        };
        eprintln!(
            "[sftp] transfer {task_id} cancelled (reason={})",
            reason.unwrap_or("unspecified")
        );
        self.record_transfer(task.clone()).await;
        emitter
            .event("sftp/transfer/progress", task)
            .map_err(plugin_error)
    }
}
