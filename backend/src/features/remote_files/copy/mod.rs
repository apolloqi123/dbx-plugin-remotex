//! Background local ↔ remote copy tasks for the file transfer window.
//!
//! Both directions reuse the plain SFTP pipelines: uploads stream through
//! `start_upload`/`append_upload`/`finish_upload`, downloads through
//! `start_download`/`download_chunk`/`complete_download`. Each outer task is
//! reported on `filetransfer/progress` so the panel can show one aggregate
//! bar for a multi-file or multi-folder copy.

mod download;
mod plan;
mod upload;

use super::RemoteFilesRuntime;
use crate::core::model::normalize_remote_path;
use dbx_plugin_sdk::PluginEmitter;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::runtime::Handle;
use uuid::Uuid;

use download::download_inner;
use upload::upload_inner;

struct FinishedTask {
    files_done: usize,
    files_total: usize,
    bytes_done: u64,
    bytes_total: u64,
    skipped: Option<usize>,
    failed: usize,
    local_paths: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    emitter: &PluginEmitter,
    task_id: &str,
    session_id: &str,
    direction: &str,
    status: &str,
    file: Option<&str>,
    files_done: usize,
    files_total: usize,
    bytes_done: u64,
    bytes_total: u64,
    error: Option<&str>,
    background: bool,
) {
    let percent = if bytes_total > 0 {
        ((bytes_done.min(bytes_total) as f64 / bytes_total as f64) * 100.0).round() as u64
    } else if status == "done" {
        100
    } else {
        0
    };
    let mut payload = json!({
        "taskId": task_id,
        "sessionId": session_id,
        "direction": direction,
        "status": status,
        "file": file,
        "filesDone": files_done,
        "filesTotal": files_total,
        "bytesDone": bytes_done,
        "bytesTotal": bytes_total,
        "percent": percent,
    });
    if background {
        payload["background"] = json!(true);
    }
    if let Some(error) = error {
        payload["error"] = json!(error);
    }
    let _ = emitter.event("filetransfer/progress", payload);
}

fn finish_task(
    runtime: &RemoteFilesRuntime,
    emitter: &PluginEmitter,
    task_id: &str,
    session_id: &str,
    cancel: &AtomicBool,
    direction: &str,
    outcome: Result<FinishedTask, String>,
) {
    runtime.finish_task(task_id);
    match outcome {
        Ok(done) => {
            emit_progress(
                emitter,
                task_id,
                session_id,
                direction,
                "done",
                None,
                done.files_done,
                done.files_total,
                done.bytes_done,
                done.bytes_total,
                None,
                false,
            );
            let mut payload = json!({
                "taskId": task_id,
                "sessionId": session_id,
                "direction": direction,
                "filesDone": done.files_done,
                "filesTotal": done.files_total,
            });
            if !done.local_paths.is_empty() {
                payload["localPaths"] = json!(done.local_paths);
            }
            if let Some(skipped) = done.skipped {
                payload["skipped"] = json!(skipped);
            }
            payload["failed"] = json!(done.failed);
            let _ = emitter.event("filetransfer/done", payload);
        }
        Err(error) => {
            let status = if cancel.load(Ordering::Acquire) || error == "canceled" {
                "canceled"
            } else {
                "error"
            };
            emit_progress(
                emitter,
                task_id,
                session_id,
                direction,
                status,
                None,
                0,
                0,
                0,
                0,
                Some(&error),
                false,
            );
        }
    }
}

pub(crate) fn start_upload(
    runtime: &Arc<RemoteFilesRuntime>,
    handle: &Handle,
    params: &Value,
    emitter: &PluginEmitter,
) -> Result<Value, String> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("Missing sessionId")?
        .to_string();
    let remote_dir = params
        .get("remoteDir")
        .and_then(Value::as_str)
        .ok_or("Missing remoteDir")?;
    let remote_dir = normalize_remote_path(remote_dir)?;
    let sources = params
        .get("localPaths")
        .and_then(Value::as_array)
        .ok_or("Missing localPaths")?;
    if sources.is_empty() {
        return Err("Select at least one local file or folder".to_string());
    }
    if sources.len() > 200 {
        return Err("Too many local sources selected".to_string());
    }
    let mut paths = Vec::with_capacity(sources.len());
    for source in sources {
        let text = source.as_str().ok_or("Local path is invalid")?;
        let path = PathBuf::from(text);
        if !path.is_absolute() {
            return Err("Local paths must be absolute".to_string());
        }
        if fs::symlink_metadata(&path).is_err() {
            return Err(format!("Local path '{}' does not exist", path.display()));
        }
        paths.push(path);
    }
    let overwrite = params.get("conflict").and_then(Value::as_str) == Some("overwrite");
    let task_id = Uuid::new_v4().to_string();
    let cancel = runtime.register_task(&task_id, &session_id);
    let ssh = runtime.ssh.clone();
    let runtime_clone = runtime.clone();
    let emitter_clone = emitter.clone();
    let task_id_clone = task_id.clone();
    let session_id_clone = session_id.clone();
    handle.spawn(async move {
        let outcome = upload_inner(
            ssh,
            &emitter_clone,
            &task_id_clone,
            &cancel,
            &paths,
            &remote_dir,
            overwrite,
            &session_id_clone,
        )
        .await;
        finish_task(
            &runtime_clone,
            &emitter_clone,
            &task_id_clone,
            &session_id_clone,
            &cancel,
            "upload",
            outcome,
        );
    });
    Ok(json!({ "taskId": task_id }))
}

pub(crate) fn start_download(
    runtime: &Arc<RemoteFilesRuntime>,
    handle: &Handle,
    params: &Value,
    emitter: &PluginEmitter,
) -> Result<Value, String> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("Missing sessionId")?
        .to_string();
    let local_dir = params
        .get("localDir")
        .and_then(Value::as_str)
        .ok_or("Missing localDir")?;
    let local_dir = super::local::validate_destination(local_dir)?
        .to_string_lossy()
        .to_string();
    let rows = params
        .get("remotePaths")
        .and_then(Value::as_array)
        .ok_or("Missing remotePaths")?;
    if rows.is_empty() {
        return Err("Select at least one remote file or folder".to_string());
    }
    if rows.len() > 200 {
        return Err("Too many remote sources selected".to_string());
    }
    let mut paths = Vec::with_capacity(rows.len());
    for row in rows {
        let text = row.as_str().ok_or("Remote path is invalid")?;
        paths.push(normalize_remote_path(text)?);
    }
    let conflict = match params.get("conflict").and_then(Value::as_str) {
        Some("overwrite") => "overwrite",
        _ => "rename",
    }
    .to_string();
    let task_id = Uuid::new_v4().to_string();
    let cancel = runtime.register_task(&task_id, &session_id);
    let ssh = runtime.ssh.clone();
    let runtime_clone = runtime.clone();
    let emitter_clone = emitter.clone();
    let task_id_clone = task_id.clone();
    let session_id_clone = session_id.clone();
    handle.spawn(async move {
        let outcome = download_inner(
            ssh,
            &emitter_clone,
            &task_id_clone,
            &cancel,
            &paths,
            &local_dir,
            &conflict,
            &session_id_clone,
            false,
        )
        .await;
        finish_task(
            &runtime_clone,
            &emitter_clone,
            &task_id_clone,
            &session_id_clone,
            &cancel,
            "download",
            outcome,
        );
    });
    Ok(json!({ "taskId": task_id }))
}

/// Fetches files the remote user copied inside a VNC desktop. The destination
/// is plugin-managed (not the window's current pane), and success writes the
/// downloaded files to the host clipboard so a Finder paste yields real files.
pub(crate) fn start_clipboard_download(
    runtime: &Arc<RemoteFilesRuntime>,
    handle: &Handle,
    params: &Value,
    emitter: &PluginEmitter,
) -> Result<Value, String> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("Missing sessionId")?
        .to_string();
    let request_id = params
        .get("requestId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("Missing requestId")?
        .to_string();
    if !runtime.touch_session(&session_id) {
        return Err("File transfer session is not active".to_string());
    }
    let rows = params
        .get("remotePaths")
        .and_then(Value::as_array)
        .ok_or("Missing remotePaths")?;
    if rows.is_empty() {
        return Err("No remote clipboard files were selected".to_string());
    }
    if rows.len() > 200 {
        return Err("Too many remote clipboard files".to_string());
    }
    let mut paths = Vec::with_capacity(rows.len());
    for row in rows {
        let text = row.as_str().ok_or("Remote path is invalid")?;
        paths.push(normalize_remote_path(text)?);
    }
    let local_dir = super::local::clipboard_dir()?.to_string_lossy().to_string();
    let task_id = Uuid::new_v4().to_string();
    let cancel = runtime.register_task(&task_id, &session_id);
    let ssh = runtime.ssh.clone();
    let runtime_clone = runtime.clone();
    let emitter_clone = emitter.clone();
    let task_id_clone = task_id.clone();
    let session_id_clone = session_id.clone();
    let local_dir_clone = local_dir.clone();
    handle.spawn(async move {
        let outcome = download_inner(
            ssh,
            &emitter_clone,
            &task_id_clone,
            &cancel,
            &paths,
            &local_dir_clone,
            "rename",
            &session_id_clone,
            true,
        )
        .await;
        runtime_clone.finish_task(&task_id_clone);
        match outcome {
            Ok(done) => {
                let clipboard_len = match super::clipboard::write_paths(&done.local_paths) {
                    Ok(count) => count,
                    Err(error) => {
                        let _ = emitter_clone.event(
                            "filetransfer/clipboard/ready",
                            json!({
                                "requestId": request_id,
                                "sessionId": session_id,
                                "taskId": task_id_clone,
                                "localPaths": done.local_paths,
                                "dir": local_dir,
                                "clipboard": false,
                                "clipboardError": error,
                                "filesDone": done.files_done,
                                "failed": done.failed,
                            }),
                        );
                        return;
                    }
                };
                let _ = emitter_clone.event(
                    "filetransfer/clipboard/ready",
                    json!({
                        "requestId": request_id,
                        "sessionId": session_id,
                        "taskId": task_id_clone,
                        "localPaths": done.local_paths,
                        "dir": local_dir,
                        "clipboard": true,
                        "count": clipboard_len,
                        "filesDone": done.files_done,
                        "failed": done.failed,
                    }),
                );
            }
            Err(error) => {
                let _ = emitter_clone.event(
                    "filetransfer/clipboard/error",
                    json!({
                        "requestId": request_id,
                        "sessionId": session_id,
                        "taskId": task_id_clone,
                        "error": error,
                    }),
                );
            }
        }
    });
    Ok(json!({ "taskId": task_id }))
}

pub(crate) fn cancel(runtime: &RemoteFilesRuntime, params: &Value) -> Result<Value, String> {
    let task_id = params
        .get("taskId")
        .and_then(Value::as_str)
        .ok_or("Missing taskId")?;
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or("Missing sessionId")?;
    Ok(json!({ "success": runtime.cancel_task(task_id, session_id) }))
}
