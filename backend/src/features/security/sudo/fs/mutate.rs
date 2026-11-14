use super::*;

use crate::core::delete_plan::{build_rm_rf_command, chunk_delete_paths, normalize_delete_paths};

/// 批量 `rm -rf` 单片的预算（与 `sftp/deleteMany` 的 300 秒对齐；per-path
/// `remove_all` 归因仍用 TIMEOUT_WRITE_SECS）。
const TIMEOUT_DELETE_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// mkdir / remove / remove_all / chmod / rename
// ---------------------------------------------------------------------------

/// Creates a directory (and parents) via sudo `mkdir -p` (tiny-rdm MkdirSudo).
pub async fn mkdir(runtime: &SshRuntime, session_id: &str, path: &str) -> Result<(), String> {
    let path = normalize_remote_path(path)?;
    let command = format!("mkdir -p -- {}", shell_quote(&path));
    sudo_exec(runtime, session_id, &command, TIMEOUT_QUICK_SECS)
        .await
        .map(|_| ())
}

/// Removes a single file via sudo `rm -f` (tiny-rdm RemoveSudo).
pub async fn remove(runtime: &SshRuntime, session_id: &str, path: &str) -> Result<(), String> {
    let path = normalize_remote_path(path)?;
    let command = format!("rm -f -- {}", shell_quote(&path));
    sudo_exec(runtime, session_id, &command, TIMEOUT_QUICK_SECS)
        .await
        .map(|_| ())
}

/// Recursively removes a directory tree via sudo `rm -rf` (tiny-rdm
/// RemoveAllSudo). `rm` never follows symlinks: a symlink operand is removed
/// as a link, and symlinks inside a tree are unlinked, not descended into.
pub async fn remove_all(runtime: &SshRuntime, session_id: &str, path: &str) -> Result<(), String> {
    let path = normalize_remote_path(path)?;
    if path == "/" {
        return Err("Refusing to recursively delete the filesystem root".to_string());
    }
    let command = format!("rm -rf -- {}", shell_quote(&path));
    sudo_exec(runtime, session_id, &command, TIMEOUT_WRITE_SECS)
        .await
        .map(|_| ())
}

/// 批量递归删除（`sudo/removeMany`）：去重/祖先折叠后按 128 项分片，每片一次
/// `rm -rf -- ...`（Quick Sudo 认证只发生一次）。某片失败时逐项 `remove_all`
/// 归因——`rm -rf -f` 对已不存在的路径仍成功，无需存在性预检；单项失败不
/// 中断其余项。返回 `{ deleted, failed }`。
pub async fn remove_many(
    runtime: &SshRuntime,
    session_id: &str,
    paths: &[String],
) -> Result<Value, String> {
    let paths = normalize_delete_paths(paths)?;
    let mut deleted: Vec<String> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    for chunk in chunk_delete_paths(&paths) {
        let succeeded = sudo_exec(
            runtime,
            session_id,
            &build_rm_rf_command(&chunk),
            TIMEOUT_DELETE_SECS,
        )
        .await
        .is_ok();
        if succeeded {
            deleted.extend(chunk);
            continue;
        }
        for path in chunk {
            match remove_all(runtime, session_id, &path).await {
                Ok(()) => deleted.push(path),
                Err(error) => failed.push(json!({ "path": path, "error": error })),
            }
        }
    }
    Ok(json!({ "deleted": deleted, "failed": failed }))
}

/// Validates that a mode is a 3-4 digit octal string ("755", "4755").
pub(super) fn validate_octal_mode(mode: &str) -> Result<(), String> {
    let valid =
        (3..=4).contains(&mode.len()) && mode.bytes().all(|byte| (b'0'..=b'7').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(format!("Mode must be 3 or 4 octal digits, got '{mode}'"))
    }
}

/// Changes permission bits via sudo `chmod` (tiny-rdm ChmodSudo).
pub async fn chmod(
    runtime: &SshRuntime,
    session_id: &str,
    path: &str,
    mode: &str,
) -> Result<(), String> {
    let path = normalize_remote_path(path)?;
    validate_octal_mode(mode)?;
    let command = format!("chmod {} -- {}", mode, shell_quote(&path));
    sudo_exec(runtime, session_id, &command, TIMEOUT_QUICK_SECS)
        .await
        .map(|_| ())
}

/// Renames a path via sudo `mv` (tiny-rdm RenameSudo).
pub async fn rename(
    runtime: &SshRuntime,
    session_id: &str,
    old_path: &str,
    new_path: &str,
) -> Result<(), String> {
    let source = normalize_remote_path(old_path)?;
    let target = normalize_remote_path(new_path)?;
    let command = format!("mv -- {} {}", shell_quote(&source), shell_quote(&target));
    sudo_exec(runtime, session_id, &command, TIMEOUT_QUICK_SECS)
        .await
        .map(|_| ())
}
