use super::*;

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
