//! Docker feature layer.

use super::*;

// —— Embedded container terminal (`docker/exec`) ——————————————

/// Wait cap for one container command. The UI terminal tab runs one
/// independent `docker exec` per submitted command (no persistent shell, so
/// no PTY session to keep alive); a hung command is killed at this bound.
pub const EXEC_TIMEOUT: Duration = Duration::from_secs(60);

/// Command text cap in characters, measured after trimming. Keeps the
/// audit row and the shell-quoted command line bounded.
pub const EXEC_MAX_COMMAND_CHARS: usize = 8192;

/// Output cap (1 MiB). Anything larger is cut on a UTF-8 boundary and
/// followed by [`EXEC_TRUNCATED_SUFFIX`].
pub const EXEC_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// Trailer appended when [`exec_output`] had to cut the command output.
pub const EXEC_TRUNCATED_SUFFIX: &str = "\n[output truncated]";

/// Validates the terminal command text. The returned slice is the trimmed
/// command that is actually executed. Rejects empty/whitespace-only input,
/// over-long text, NUL/CR and every other control character except newline
/// and tab (multi-line scripts stay meaningful; `\r` would corrupt the
/// remote shell's line handling).
pub fn validate_exec_command(command: &str) -> Result<&str, String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(
            "Missing command: enter a non-empty command to run inside the container".to_string(),
        );
    }
    let length = trimmed.chars().count();
    if length > EXEC_MAX_COMMAND_CHARS {
        return Err(format!(
            "Command is too long ({length} characters); the limit is \
             {EXEC_MAX_COMMAND_CHARS} characters"
        ));
    }
    if let Some(character) = command
        .chars()
        .find(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(format!(
            "Invalid command: control character {character:?} is not allowed \
             (only newlines and tabs are accepted)"
        ));
    }
    Ok(trimmed)
}

/// Renders the SSH command text for one terminal call:
/// `{env prefix}{cli} exec {id} sh -lc {quoted command}`. The endpoint
/// prefix rides the engine env vars (never argv), and the user command is
/// single-quoted so the remote outer shell passes it verbatim to the
/// container's `sh -lc`.
pub fn exec_ssh_command(
    engine: &EngineConfig,
    container_id: &str,
    command: &str,
) -> Result<String, String> {
    let command = validate_exec_command(command)?;
    validate_container_id(container_id)?;
    Ok(format!(
        "{}{} exec {container_id} sh -lc {}",
        engine.env_prefix(),
        engine.cli,
        exec::shell_quote(command),
    ))
}

/// Audit/approval text for one terminal call: the short form without the
/// endpoint export (`{cli} exec {id} sh -lc '<command>'`), matching the
/// action convention where only the executed command carries the prefix.
pub fn exec_audit_text(
    engine: &EngineConfig,
    container_id: &str,
    command: &str,
) -> Result<String, String> {
    let command = validate_exec_command(command)?;
    validate_container_id(container_id)?;
    Ok(format!(
        "{} exec {container_id} sh -lc {}",
        engine.cli,
        exec::shell_quote(command),
    ))
}

/// Local argv for one terminal call: direct process exec, no shell of our
/// own (`exec`, id, `sh`, `-lc`, command). The daemon endpoint is injected
/// as an env var by [`run_local`], exactly like the other local helpers.
pub fn exec_argv(container_id: &str, command: &str) -> Result<Vec<String>, String> {
    let command = validate_exec_command(command)?;
    validate_container_id(container_id)?;
    Ok(vec![
        "exec".to_string(),
        container_id.to_string(),
        "sh".to_string(),
        "-lc".to_string(),
        command.to_string(),
    ])
}

/// Actionable hint for the failure signatures the terminal must explain
/// instead of dumping raw docker stderr only. Pure and content-free: the
/// hint never embeds command output, so it is safe for audit rows too.
pub fn exec_failure_hint(output: &str) -> Option<&'static str> {
    let lower = output.to_ascii_lowercase();
    if lower.contains("no such container") {
        Some("the container no longer exists; refresh the container list and reopen the terminal")
    } else if lower.contains("is not running") || lower.contains("container is paused") {
        Some("the container is not running; start it before running commands")
    } else if lower.contains("executable file not found") || lower.contains("sh: not found") {
        Some("the container has no /bin/sh (distroless/scratch image); the terminal requires sh")
    } else {
        None
    }
}

/// Final response output: on failure append the actionable hint (if any),
/// then cap at [`EXEC_MAX_OUTPUT_BYTES`] on a UTF-8 boundary.
pub fn exec_output(output: &str, exit_code: i32) -> String {
    let decorated = match (exit_code, exec_failure_hint(output)) {
        (0, _) | (_, None) => output.to_string(),
        (_, Some(hint)) => format!(
            "{}\n[hint: {hint}]\n",
            output.trim_end_matches(|character| character == '\n' || character == '\r')
        ),
    };
    clip_exec_output(&decorated)
}

/// Cuts `output` to [`EXEC_MAX_OUTPUT_BYTES`] without splitting a UTF-8
/// character, appending [`EXEC_TRUNCATED_SUFFIX`].
pub fn clip_exec_output(output: &str) -> String {
    if output.len() <= EXEC_MAX_OUTPUT_BYTES {
        return output.to_string();
    }
    let mut end = EXEC_MAX_OUTPUT_BYTES;
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    let mut clipped = output[..end].to_string();
    clipped.push_str(EXEC_TRUNCATED_SUFFIX);
    clipped
}

/// Audit reason for a completed-but-failed terminal call. Deliberately
/// excludes the command output: audit rows record the command and the exit
/// code, never what the container printed.
pub fn exec_failure_reason(engine: &EngineConfig, exit_code: i32, output: &str) -> String {
    match exec_failure_hint(output) {
        Some(hint) => format!("{} exec failed (exit {exit_code}): {hint}", engine.cli),
        None => format!("{} exec failed (exit {exit_code})", engine.cli),
    }
}

/// Audit reason for a transport-level failure. Daemon-permission errors
/// embed the CLI output, so they collapse to a fixed label; spawn/timeout
/// errors are already content-free and pass through (audit truncates).
pub fn exec_error_reason(error: &str) -> String {
    if error.contains("daemon permission") {
        "daemon permission denied".to_string()
    } else {
        error.chars().take(256).collect()
    }
}

/// `docker/exec` for `target: "local"`: argv-only direct exec with a 60s
/// cap and the engine endpoint as env vars. A non-zero container exit is a
/// normal response (`success: true` plus `exitCode`); only transport and
/// daemon-permission failures become RPC errors, the latter with the same
/// actionable guidance as the other local docker actions.
pub async fn perform_exec_local(
    engine: &EngineConfig,
    container_id: &str,
    command: &str,
) -> Result<Value, String> {
    let argv = exec_argv(container_id, command)?;
    let cli = find_local_cli(&engine.cli)
        .ok_or_else(|| format!("{} CLI not found on this machine", engine.cli))?;
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (exit_code, output) =
        run_local(&cli, &args, EXEC_TIMEOUT, engine.endpoint.as_deref()).await?;
    if exit_code != 0 && is_daemon_permission_failure(exit_code, &output) {
        return Err(format!(
            "{} exec: {output}. Local container access needs daemon permission: \
             start Docker Desktop/OrbStack/Podman Desktop, or add your user to \
             the docker group.",
            engine.cli
        ));
    }
    Ok(json!({
        "success": true,
        "output": exec_output(&output, exit_code),
        "exitCode": exit_code,
    }))
}
