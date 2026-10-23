//! Transfer Util helpers for the SSH runtime.

use super::*;

/// Reads an upload spool meta file (`upload-<taskId>.json`). Corrupt or
/// missing files yield `None` — a resume request against them is rejected.
pub(super) fn read_upload_meta(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&text).ok()
}

/// Writes an upload spool meta file atomically (tmp + rename, 0600).
pub(super) fn write_upload_meta(path: &Path, meta: &Value) -> Result<(), String> {
    let text = serde_json::to_string(meta)
        .map_err(|error| format!("Failed to encode upload meta: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)
        .map_err(|error| format!("Failed to write {}: {error}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

/// Removes the upload spool meta file when it is no longer resumable.
pub(super) fn remove_upload_meta(transfer_dir: &Path, task_id: &str) {
    let _ = std::fs::remove_file(transfer_dir.join(format!("upload-{task_id}.json")));
}

/// 可续传上传 spool 的龄期上限（评审 H-2 后续，2026-09-30）：断点续传按
/// 设计跨重启保留，但没有龄期约束时 16GiB 级残留可无限累积。7 天覆盖
/// 「当天断传隔天续」的现实窗口，超龄 spool 连同 meta 在启动清扫中回收。
pub(super) const UPLOAD_SPOOL_RETENTION: std::time::Duration =
    std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// 启动清扫（评审 H-2/L-2，2026-09-30）：sidecar 上个生命周期遗留的孤儿
/// 临时件。下载侧临时件（`downloads/` 下 .part/.part.gz/.plain/.tgz）全部
/// 不可续传，进程不在即成孤儿，整目录清空；上传 spool 支持断点续传
/// （by design 跨重启保留），只清三类——「meta 在而 spool 不在」的孤儿
/// meta、永远可再生的 `.part.gz` 压缩半成品（有效续传态只由
/// `.part`+`.json` 构成，finish 推送会重新生成 .gz）、超过
/// `UPLOAD_SPOOL_RETENTION` 的超龄 spool（连同 meta）。全部 best-effort，
/// 清扫失败不阻塞启动。
pub(super) fn sweep_stale_transfer_temps(transfer_dir: &Path) {
    let downloads = transfer_dir.join("downloads");
    if let Ok(entries) = std::fs::read_dir(&downloads) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(transfer_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let spool_id = name
                .strip_prefix("upload-")
                .and_then(|rest| rest.strip_suffix(".json"));
            if let Some(id) = spool_id {
                if !transfer_dir.join(format!("upload-{id}.part")).exists() {
                    let _ = std::fs::remove_file(entry.path());
                }
            } else if name.starts_with("upload-") && name.ends_with(".part.gz") {
                let _ = std::fs::remove_file(entry.path());
            } else if let Some(id) = name
                .strip_prefix("upload-")
                .and_then(|rest| rest.strip_suffix(".part"))
            {
                // 超龄 spool（连同 meta）回收：mtime 在未来的钟偏场景
                // elapsed() 失败按未超龄保留（保守侧）。
                let stale = std::fs::metadata(entry.path())
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > UPLOAD_SPOOL_RETENTION);
                if stale {
                    let _ = std::fs::remove_file(entry.path());
                    let _ = std::fs::remove_file(transfer_dir.join(format!("upload-{id}.json")));
                }
            }
        }
    }
}

/// Scans the transfer directory for interrupted uploads that can still be
/// resumed: a spool file plus its sidecar meta, whose task is not live and
/// whose persisted history row is not terminal-success/cancelled. Pure over
/// (transfer_dir, live task ids) so it is unit-testable without a runtime.
pub(super) fn resumable_uploads_from(transfer_dir: &Path, live_task_ids: &[String]) -> Vec<Value> {
    let mut tasks = Vec::new();
    let Ok(entries) = std::fs::read_dir(transfer_dir) else {
        return tasks;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(task_id) = name
            .strip_prefix("upload-")
            .and_then(|rest| rest.strip_suffix(".json"))
        else {
            continue;
        };
        if live_task_ids.iter().any(|live| live == task_id) {
            continue;
        }
        let Some(meta) = read_upload_meta(&path) else {
            continue;
        };
        let (Some(remote_path), Some(size)) = (
            meta.get("remotePath").and_then(Value::as_str),
            meta.get("size").and_then(Value::as_u64),
        ) else {
            continue;
        };
        let Ok(spool_len) = std::fs::metadata(transfer_dir.join(format!("upload-{task_id}.part")))
            .map(|meta| meta.len())
        else {
            continue;
        };
        if spool_len == 0 || spool_len > size {
            continue;
        }
        tasks.push(json!({
            "taskId": task_id,
            "direction": "upload",
            "remotePath": remote_path,
            "fileName": remote_path.rsplit('/').next().unwrap_or("upload"),
            "size": size,
            "resumableBytes": spool_len,
        }));
    }
    // Newest meta file first so the picker surfaces the freshest attempt.
    tasks.sort_by(|a, b| {
        b["taskId"]
            .as_str()
            .unwrap_or_default()
            .cmp(a["taskId"].as_str().unwrap_or_default())
    });
    tasks
}

/// 跨卷目录拷贝兜底（M33 finalize_tree_staging）：普通树落点本来就是最终
/// 目录用不到；压缩树 staging 同卷 rename 失败时的罕见回退。符号链接与
/// 特殊文件不拷贝（树语义不跟随 symlink）。
pub(super) fn copy_dir_recursive(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let dest = target.join(entry.file_name());
        if entry_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else if entry_type.is_file() {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

pub(super) fn remote_transfer_paths(
    target: &str,
    task_id: &str,
) -> Result<(String, String), String> {
    let (raw_parent, _) = target
        .rsplit_once('/')
        .ok_or("Remote upload path has no parent")?;
    let raw_parent = if raw_parent.is_empty() {
        "/"
    } else {
        raw_parent
    };
    // Segment-level `..` convergence (same shape as `normalize_remote_path`):
    // the `.part`/`.backup` staging files must land in the *resolved* target
    // directory, or the same-directory atomic rename premise breaks for
    // traversal-shaped targets (`/data/a/../f.bin` must stage in `/data/`,
    // not `/data/a/../`). A `..` arriving at the root is dropped, never
    // escaped; a relative parent that collapses entirely has no anchor left
    // and is rejected like a bare name (relative staging would depend on the
    // remote CWD).
    let absolute = raw_parent.starts_with('/');
    let mut segments: Vec<&str> = Vec::new();
    for segment in raw_parent.split('/') {
        if segment == ".." {
            // Only a real name segment can be popped; the leading "" of an
            // absolute path is the root marker and `..` there is discarded.
            let poppable = !(segments.len() == 1 && segments[0].is_empty());
            if poppable {
                segments.pop();
            }
        } else {
            segments.push(segment);
        }
    }
    if !absolute && segments.is_empty() {
        return Err("Remote upload path has no parent".to_string());
    }
    let joined = segments.join("/");
    let parent: &str = if joined.is_empty() { "/" } else { &joined };
    Ok((
        format!(
            "{}/.dbx-upload-{task_id}.part",
            parent.trim_end_matches('/')
        ),
        format!(
            "{}/.dbx-upload-{task_id}.backup",
            parent.trim_end_matches('/')
        ),
    ))
}

/// Permission attributes carried over from an existing target to the staged
/// replacement file: the mode's permission bits (`0o7777`, so an executable
/// script keeps its `+x` across saves) and nothing else. Ownership (uid/gid)
/// is deliberately not preserved — SETSTAT on uid/gid needs elevated
/// privileges, so the replacement keeps the writing user's own ownership.
pub(super) fn preserved_target_permissions(
    target: &russh_sftp::protocol::FileAttributes,
) -> Option<russh_sftp::protocol::FileAttributes> {
    let permissions = target.permissions?;
    Some(russh_sftp::protocol::FileAttributes {
        permissions: Some(permissions & 0o7777),
        ..Default::default()
    })
}

/// Applies [`preserved_target_permissions`] to the staged file before the
/// rename. A SETSTAT failure aborts the commit with the original target
/// untouched instead of silently saving a permission-downgraded copy (issue
/// #37: an executable script must not lose its `+x` on every save).
pub(crate) async fn apply_preserved_permissions(
    sftp: &Arc<AsyncMutex<SftpSession>>,
    temporary: &str,
    target: Option<&russh_sftp::protocol::FileAttributes>,
) -> Result<(), String> {
    if let Some(attributes) = target.and_then(preserved_target_permissions) {
        let mode = attributes.permissions.unwrap_or_default();
        if let Err(error) = sftp
            .lock()
            .await
            .set_metadata(temporary.to_string(), attributes)
            .await
        {
            let _ = sftp.lock().await.remove_file(temporary.to_string()).await;
            return Err(format!(
                "SFTP save failed while preserving permissions {}: {error}; original file left unchanged",
                format_permissions(mode),
            ));
        }
    }
    Ok(())
}

pub(super) async fn commit_remote_file(
    sftp: &Arc<AsyncMutex<SftpSession>>,
    temporary: &str,
    target: &str,
    backup: &str,
) -> Result<(), String> {
    let target_attributes = sftp.lock().await.metadata(target.to_string()).await.ok();
    apply_preserved_permissions(sftp, temporary, target_attributes.as_ref()).await?;
    let target_exists = target_attributes.is_some();
    if target_exists {
        sftp.lock()
            .await
            .rename(target.to_string(), backup.to_string())
            .await
            .map_err(sftp_error)?;
    }
    if let Err(error) = sftp
        .lock()
        .await
        .rename(temporary.to_string(), target.to_string())
        .await
    {
        if target_exists {
            let _ = sftp
                .lock()
                .await
                .rename(backup.to_string(), target.to_string())
                .await;
        }
        let _ = sftp.lock().await.remove_file(temporary.to_string()).await;
        return Err(sftp_error(error));
    }
    if target_exists {
        let _ = sftp.lock().await.remove_file(backup.to_string()).await;
    }
    Ok(())
}
