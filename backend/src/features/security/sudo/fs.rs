//! Sudo-backed filesystem operations, mirroring tiny-rdm's Sudo SFTP surface.
//!
//! Every function shells out through [`SshRuntime::exec`] with `sudo = true`,
//! so Quick Sudo orchestration (password injection, PTY/MFA) and the
//! read-only connection guard apply uniformly without any duplication here.
//! Remote paths are always POSIX single-quoted (embedded quotes become
//! `'\''`), matching tiny-rdm's `shQuoted` helper.

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::core::model::{normalize_remote_path, sftp_uri, SftpEntry};
use crate::sessions::ssh::SshRuntime;

/// Per-command timeouts, sized per operation family (exec clamps to 5..300).
const TIMEOUT_QUICK_SECS: u64 = 30;
const TIMEOUT_LIST_SECS: u64 = 60;
const TIMEOUT_READ_SECS: u64 = 60;
const TIMEOUT_WRITE_SECS: u64 = 120;
/// Raw payload embedded per remote command in [`write_file`]; base64 expands
/// it by ~4/3, keeping the exec request well below channel packet limits.
const WRITE_CHUNK_BYTES: usize = 96 * 1024;

/// Wraps a path in single quotes for safe shell interpolation, replacing any
/// embedded quote with the POSIX `'\''` escape (tiny-rdm's shQuoted).
///
/// Canonical implementation lives in [`crate::sessions::ssh::exec::shell_quote`]; re-exported
/// here so the sudo family keeps its local name and `sudo_download`'s import
/// path is unchanged.
#[doc(hidden)]
pub(crate) use crate::sessions::ssh::exec::shell_quote;

/// Runs one command with Quick Sudo and returns its output, failing on a
/// non-zero exit code (mirrors tiny-rdm's execSudoResultMap handling).
pub(crate) async fn sudo_exec(
    runtime: &SshRuntime,
    session_id: &str,
    command: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let response = runtime
        .exec(session_id, None, command, true, Some(timeout_secs))
        .await?;
    let output = response
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let exit_code = response
        .get("exitCode")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if exit_code != 0 {
        return Err(format!("sudo command exited {exit_code}: {output}"));
    }
    Ok(output)
}

mod io;
mod mutate;
mod parse;

#[cfg(test)]
mod tests;

pub use io::*;
pub use mutate::*;
pub use parse::*;
