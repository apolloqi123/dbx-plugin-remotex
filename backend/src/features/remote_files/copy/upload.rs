//! Local → remote upload task.

use super::plan::{
    join_remote, plan_source, read_filled_sync, remote_names, unique_name, ENCODING,
};
use super::{emit_progress, FinishedTask};
use crate::core::model::TRANSFER_CHUNK_SIZE;
use crate::features::transfer::progress::ProgressThrottle;
use crate::sessions::ssh::SshRuntime;
use dbx_plugin_sdk::PluginEmitter;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[allow(clippy::too_many_arguments)]
pub(super) async fn upload_inner(
    ssh: Arc<SshRuntime>,
    emitter: &PluginEmitter,
    task_id: &str,
    cancel: &AtomicBool,
    sources: &[PathBuf],
    remote_dir: &str,
    overwrite: bool,
    session_id: &str,
) -> Result<FinishedTask, String> {
    let mut plans = Vec::with_capacity(sources.len());
    for source in sources {
        plans.push(plan_source(source)?);
    }
    let files_total = plans.iter().map(|plan| plan.files.len()).sum();
    let bytes_total = plans.iter().map(|plan| plan.total).sum();
    let skipped = plans.iter().map(|plan| plan.skipped).sum();
    let mut existing = remote_names(&ssh, session_id, remote_dir).await?;
    let mut files_done = 0usize;
    let mut bytes_done = 0u64;
    emit_progress(
        emitter,
        task_id,
        session_id,
        "upload",
        "running",
        None,
        0,
        files_total,
        0,
        bytes_total,
        None,
        false,
    );
    for plan in plans {
        let root = if plan.directory {
            let directory_name = unique_name(&mut existing, &plan.name);
            let root = join_remote(remote_dir, &directory_name);
            ssh.sftp_create_directory(session_id, &root, ENCODING)
                .await?;
            for relative in &plan.dirs {
                let path = join_remote(&root, relative);
                ssh.sftp_create_directory(session_id, &path, ENCODING)
                    .await?;
            }
            root
        } else {
            let name = if overwrite {
                plan.name.clone()
            } else {
                unique_name(&mut existing, &plan.name)
            };
            join_remote(remote_dir, &name)
        };
        for file in &plan.files {
            if cancel.load(Ordering::Acquire) {
                return Err("canceled".to_string());
            }
            let target = if plan.directory {
                join_remote(&root, &file.relative)
            } else {
                root.clone()
            };
            emit_progress(
                emitter,
                task_id,
                session_id,
                "upload",
                "running",
                Some(&file.relative),
                files_done,
                files_total,
                bytes_done,
                bytes_total,
                None,
                false,
            );
            let started = ssh
                .start_upload(
                    session_id.to_string(),
                    target,
                    file.size,
                    None,
                    ENCODING,
                    false,
                    emitter,
                )
                .await?;
            let upload_task = started
                .get("taskId")
                .and_then(Value::as_str)
                .ok_or("Upload start did not return a task id")?
                .to_string();
            let mut handle = fs::File::open(&file.path)
                .map_err(|error| format!("Unable to open '{}': {error}", file.path.display()))?;
            let mut buffer = vec![0u8; TRANSFER_CHUNK_SIZE];
            let mut offset = 0u64;
            loop {
                if cancel.load(Ordering::Acquire) {
                    let _ = ssh
                        .cancel_transfer(&upload_task, Some("canceled"), emitter)
                        .await;
                    return Err("canceled".to_string());
                }
                let read = match read_filled_sync(&mut handle, &mut buffer) {
                    Ok(read) => read,
                    Err(error) => {
                        let _ = ssh
                            .cancel_transfer(&upload_task, Some("error"), emitter)
                            .await;
                        return Err(error);
                    }
                };
                if read == 0 {
                    break;
                }
                let mut payload = Vec::with_capacity(8 + read);
                payload.extend_from_slice(&offset.to_be_bytes());
                payload.extend_from_slice(&buffer[..read]);
                if let Err(error) = ssh.append_upload(&upload_task, &payload, emitter) {
                    let _ = ssh
                        .cancel_transfer(&upload_task, Some("error"), emitter)
                        .await;
                    return Err(error);
                }
                offset = offset.saturating_add(read as u64);
            }
            if let Err(error) = ssh.finish_upload(&upload_task, ENCODING, emitter).await {
                let _ = ssh
                    .cancel_transfer(&upload_task, Some("error"), emitter)
                    .await;
                return Err(error);
            }
            // `finish_upload` only hands the spool to a background push; wait
            // for that push before counting the file so the aggregate result
            // (and the transfer-depth budget) reflects reality.
            wait_for_upload(
                &ssh,
                &upload_task,
                cancel,
                emitter,
                task_id,
                session_id,
                &file.relative,
                files_done,
                files_total,
                bytes_done,
                file.size,
                bytes_total,
            )
            .await?;
            bytes_done = bytes_done.saturating_add(file.size);
            files_done += 1;
            emit_progress(
                emitter,
                task_id,
                session_id,
                "upload",
                "running",
                Some(&file.relative),
                files_done,
                files_total,
                bytes_done,
                bytes_total,
                None,
                false,
            );
        }
    }
    Ok(FinishedTask {
        files_done,
        files_total,
        bytes_done,
        bytes_total,
        skipped: Some(skipped),
        failed: 0,
        local_paths: Vec::new(),
    })
}

/// `finish_upload` hands the spool to a background push and returns at once
/// (issue #60: holding the RPC open would expose multi-GB pushes to bridge
/// deadlines). The aggregate panel task must still wait for that push before
/// reporting the file as done, otherwise it reports success early and starts
/// the next file while the previous one occupies the transfer-depth budget.
#[allow(clippy::too_many_arguments)]
async fn wait_for_upload(
    ssh: &SshRuntime,
    upload_task: &str,
    cancel: &AtomicBool,
    emitter: &PluginEmitter,
    aggregate_task: &str,
    session_id: &str,
    file: &str,
    files_done: usize,
    files_total: usize,
    base_bytes: u64,
    file_size: u64,
    bytes_total: u64,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(6 * 60 * 60);
    let mut missing_polls = 0u32;
    let mut throttle = ProgressThrottle::new();
    loop {
        if cancel.load(Ordering::Acquire) {
            let _ = ssh
                .cancel_transfer(upload_task, Some("canceled"), emitter)
                .await;
            return Err("canceled".to_string());
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = ssh
                .cancel_transfer(upload_task, Some("error"), emitter)
                .await;
            return Err("Upload push timed out".to_string());
        }
        match ssh.transfer_status(upload_task).await {
            Ok(status) => {
                missing_polls = 0;
                match status.get("status").and_then(Value::as_str) {
                    Some("completed") => return Ok(()),
                    Some("failed") => {
                        return Err(status
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Upload push failed")
                            .to_string())
                    }
                    Some("cancelled") => return Err("canceled".to_string()),
                    _ => {}
                }
                if let Some(transferred) = status.get("transferred").and_then(Value::as_u64) {
                    if throttle.should_emit(false) {
                        emit_progress(
                            emitter,
                            aggregate_task,
                            session_id,
                            "upload",
                            "running",
                            Some(file),
                            files_done,
                            files_total,
                            base_bytes.saturating_add(transferred.min(file_size)),
                            bytes_total,
                            None,
                            false,
                        );
                    }
                }
            }
            // The finishing task removes itself just before writing its
            // terminal history entry; retry briefly instead of failing.
            Err(error) if error.contains("not found") => {
                missing_polls += 1;
                if missing_polls >= 20 {
                    return Err("Upload task vanished before the remote push completed".to_string());
                }
            }
            Err(error) => return Err(error),
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
