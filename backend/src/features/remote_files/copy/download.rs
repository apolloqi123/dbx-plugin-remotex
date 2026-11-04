//! Remote → local download task.

use super::plan::ENCODING;
use super::{emit_progress, FinishedTask};
use crate::features::transfer::progress::ProgressThrottle;
use crate::filesystem::sftp::ext;
use crate::sessions::ssh::SshRuntime;
use dbx_plugin_sdk::PluginEmitter;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[allow(clippy::too_many_arguments)]
pub(super) async fn download_inner(
    ssh: Arc<SshRuntime>,
    emitter: &PluginEmitter,
    task_id: &str,
    cancel: &AtomicBool,
    remote_paths: &[String],
    local_dir: &str,
    conflict: &str,
    session_id: &str,
    background: bool,
) -> Result<FinishedTask, String> {
    let files_total = remote_paths.len();
    let mut throttle = ProgressThrottle::new();
    let mut files_done = 0usize;
    let mut bytes_done = 0u64;
    let mut bytes_total = 0u64;
    let mut failed = 0usize;
    let mut downloaded = Vec::new();
    for remote_path in remote_paths {
        if cancel.load(Ordering::Acquire) {
            return Err("canceled".to_string());
        }
        let stat = ext::stat(&ssh, session_id, remote_path, ENCODING).await?;
        let directory = stat.get("kind").and_then(Value::as_str) == Some("directory");
        let started = if directory {
            ssh.start_tree_download(
                session_id,
                remote_path,
                Some(local_dir),
                ENCODING,
                false,
                emitter,
            )
            .await?
        } else {
            ssh.start_download(
                session_id,
                remote_path,
                0,
                true,
                Some(local_dir),
                Some(conflict),
                ENCODING,
                false,
                emitter,
            )
            .await?
        };
        let download_task = started
            .get("taskId")
            .and_then(Value::as_str)
            .ok_or("Download start did not return a task id")?
            .to_string();
        wait_until_ready(&ssh, &download_task, cancel, emitter).await?;
        let file_name = started
            .get("fileName")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                remote_path
                    .rsplit('/')
                    .find(|part| !part.is_empty())
                    .unwrap_or(remote_path)
            })
            .to_string();
        let size = started.get("size").and_then(Value::as_u64).unwrap_or(0);
        bytes_total = bytes_total.saturating_add(size);
        emit_progress(
            emitter,
            task_id,
            session_id,
            "download",
            "running",
            Some(&file_name),
            files_done,
            files_total,
            bytes_done,
            bytes_total,
            None,
            background,
        );
        let mut offset = 0u64;
        loop {
            if cancel.load(Ordering::Acquire) {
                let _ = ssh
                    .cancel_transfer(&download_task, Some("canceled"), emitter)
                    .await;
                return Err("canceled".to_string());
            }
            let chunk = match ssh.download_chunk(&download_task, offset, emitter).await {
                Ok(chunk) => chunk,
                Err(error) => {
                    let _ = ssh
                        .cancel_transfer(&download_task, Some("error"), emitter)
                        .await;
                    return Err(error);
                }
            };
            let length = chunk.get("length").and_then(Value::as_u64).unwrap_or(0);
            offset = offset.saturating_add(length);
            bytes_done = bytes_done.saturating_add(length);
            let eof = chunk.get("eof").and_then(Value::as_bool).unwrap_or(false);
            if throttle.should_emit(eof) {
                emit_progress(
                    emitter,
                    task_id,
                    session_id,
                    "download",
                    "running",
                    Some(&file_name),
                    files_done,
                    files_total,
                    bytes_done,
                    bytes_total,
                    None,
                    background,
                );
            }
            if eof {
                break;
            }
        }
        let done = match ssh.complete_download(&download_task, emitter).await {
            Ok(done) => done,
            Err(error) => {
                let _ = ssh
                    .cancel_transfer(&download_task, Some("error"), emitter)
                    .await;
                return Err(error);
            }
        };
        failed += done
            .get("failedCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        if let Some(local) = done.get("localPath").and_then(Value::as_str) {
            downloaded.push(local.to_string());
        }
        files_done += 1;
        emit_progress(
            emitter,
            task_id,
            session_id,
            "download",
            "running",
            Some(&file_name),
            files_done,
            files_total,
            bytes_done,
            bytes_total,
            None,
            background,
        );
    }
    Ok(FinishedTask {
        files_done,
        files_total,
        bytes_done,
        bytes_total,
        skipped: None,
        failed,
        local_paths: downloaded,
    })
}

/// A gzip download prepares its stream asynchronously; `download_chunk`
/// rejects requests until it is ready. Wait for the ready/plain-fallback
/// transition before pumping (the SFTP workbench does the same through the
/// ready event, which this aggregate task does not consume).
async fn wait_until_ready(
    ssh: &SshRuntime,
    download_task: &str,
    cancel: &AtomicBool,
    emitter: &PluginEmitter,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2 * 60 * 60);
    loop {
        if cancel.load(Ordering::Acquire) {
            let _ = ssh
                .cancel_transfer(download_task, Some("canceled"), emitter)
                .await;
            return Err("canceled".to_string());
        }
        let status = match ssh.transfer_status(download_task).await {
            Ok(status) => status,
            // Removed by a failed prep or a completed/very fast task; the
            // first chunk will surface the real error/state.
            Err(error) if error.contains("not found") => return Ok(()),
            Err(error) => return Err(error),
        };
        let compressed = status.get("compression").and_then(Value::as_str) == Some("gzip");
        let ready = status.get("ready").and_then(Value::as_bool) == Some(true);
        if !compressed || ready {
            return Ok(());
        }
        if status.get("phase").and_then(Value::as_str) == Some("failed") {
            return Err("Compressed download preparation failed".to_string());
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = ssh
                .cancel_transfer(download_task, Some("error"), emitter)
                .await;
            return Err("Compressed download preparation timed out".to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }
}
