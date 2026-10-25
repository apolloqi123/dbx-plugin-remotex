//! Task provider config layer.

use super::*;
use crate::sessions::ssh::exec::shell_quote as shell_single_quote;

pub(super) fn task_error(code: &str, message: impl std::fmt::Display) -> String {
    format!("{code}: {message}")
}

// ---------------------------------------------------------------------------
// Task config
// ---------------------------------------------------------------------------

/// Provider-side snapshot of one task invocation's config. Keys follow the
/// manifest form-field keys (`snake_case`, `binding: "config"`); the host
/// forwards the form values verbatim inside `task.config`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskConfig {
    /// Inline command text. Empty when the run uses `script_path` instead —
    /// the two are mutually exclusive and at least one must be set.
    pub command: String,
    /// Local script file (the plugin's own machine) executed instead of
    /// `command`. SSH targets upload it to a remote `mktemp` file first;
    /// local-shell targets execute the file directly. Empty = command mode.
    pub script_path: String,
    pub working_directory: String,
    pub environment: Vec<(String, String)>,
    /// Run-mode only: plugin-side wall-clock deadline. Absent means the host
    /// execution policy owns the timeout (`timeout` is still reported when
    /// the host cancels for its own deadline).
    pub timeout_seconds: Option<u64>,
    /// Request a PTY: output becomes a single merged stream (PTY merges
    /// stderr into stdout by SSH protocol semantics), documented trade-off
    /// for commands that need a TTY.
    pub pty: bool,
    /// Resident-mode only: bounded restart policy (ADR §2.5 execution-side
    /// mirror). Clamped by [`RestartPolicy::parse`] into the hard bounds.
    pub restart: RestartPolicy,
}

/// Bounded restart policy for resident sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RestartPolicy {
    pub max_restarts: u32,
    pub backoff_seconds: u64,
    /// Sliding window the restart count applies to; `None` counts over the
    /// sidecar process lifetime.
    pub window_seconds: Option<u64>,
}

/// Parses one `KEY=VALUE` environment entry. Keys must be valid POSIX
/// variable names; values must not carry raw newlines (they would break the
/// `export` script framing).
pub(super) fn parse_env_entry(entry: &str) -> Result<(String, String), String> {
    let Some((key, value)) = entry.split_once('=') else {
        return Err(format!("environment entry {entry:?} must be KEY=VALUE"));
    };
    let key = key.trim();
    let valid_key = !key.is_empty()
        && key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid_key {
        return Err(format!(
            "environment key {key:?} must match [A-Za-z_][A-Za-z0-9_]*"
        ));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(format!(
            "environment value for {key:?} must not contain line breaks"
        ));
    }
    Ok((key.to_string(), value.to_string()))
}

/// Collects (not first-fail) validation errors so the host form can show
/// every problem at once; `warnings` carries non-fatal observations.
pub(super) fn validate_task_config(trigger_id: &str, config: &Value) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if !TRIGGERS.contains(&trigger_id) {
        errors.push(format!(
            "unknown trigger {trigger_id:?}; expected one of {TRIGGERS:?}"
        ));
    }
    let command = config
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    // `script_path` is the second (mutually exclusive) way to name what runs.
    // Type check first so a mistyped value surfaces as its own error instead
    // of silently counting as "empty".
    if let Some(script_path) = config.get("script_path") {
        if !script_path.is_null() && script_path.as_str().is_none() {
            errors.push("script_path must be a string".to_string());
        }
    }
    let script_path = config
        .get("script_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if command.is_empty() && script_path.is_empty() {
        errors.push("either command or script_path is required".to_string());
    } else if !command.is_empty() && !script_path.is_empty() {
        // Deliberately a hard error, not a precedence rule: silently picking
        // one side would run something the author did not review.
        errors.push("command and script_path are mutually exclusive; set exactly one".to_string());
    }
    if let Some(environment) = config.get("environment").and_then(Value::as_str) {
        for line in environment.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Err(error) = parse_env_entry(line) {
                errors.push(error);
            }
        }
    } else if config.get("environment").is_some() {
        errors.push("environment must be a multi-line KEY=VALUE string".to_string());
    }
    if let Some(raw) = config.get("timeout_seconds") {
        match raw.as_u64() {
            Some(n) if (1..=86_400).contains(&n) => {}
            _ => errors.push("timeout_seconds must be an integer between 1 and 86400".to_string()),
        }
    }
    if let Some(raw) = config.get("pty") {
        if raw.as_bool().is_none() {
            errors.push("pty must be a boolean".to_string());
        } else if raw.as_bool() == Some(true) {
            warnings.push(
                "with pty enabled stdout and stderr arrive merged on the stdout stream".to_string(),
            );
        }
    }
    // Restart knobs are only meaningful for the resident trigger; unknown
    // keys elsewhere are a typo the form would silently swallow.
    if trigger_id != "resident" {
        for key in [
            "max_restarts",
            "restart_backoff_seconds",
            "restart_window_seconds",
        ] {
            if config.get(key).is_some() {
                warnings.push(format!("{key} only applies to the resident trigger"));
            }
        }
    }
    if trigger_id == "resident" {
        RestartPolicy::parse(config, &mut errors);
    }
    (errors, warnings)
}

impl TaskConfig {
    /// Strict parse for execution paths: any validation error fails the run
    /// up front (non-retryable `invalid_config`).
    pub(super) fn parse(trigger_id: &str, config: &Value) -> Result<Self, String> {
        let (errors, _warnings) = validate_task_config(trigger_id, config);
        if let Some(error) = errors.first() {
            return Err(task_error(CODE_INVALID_CONFIG, error.clone()));
        }
        let command = config
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let environment = config
            .get("environment")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                (!line.is_empty())
                    .then(|| parse_env_entry(line).ok())
                    .flatten()
            })
            .collect();
        Ok(Self {
            command,
            script_path: config
                .get("script_path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            working_directory: config
                .get("working_directory")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            environment,
            timeout_seconds: config
                .get("timeout_seconds")
                .and_then(Value::as_u64)
                .filter(|n| (1..=86_400).contains(n)),
            pty: config.get("pty").and_then(Value::as_bool).unwrap_or(false),
            restart: {
                let mut errors = Vec::new();
                RestartPolicy::parse(config, &mut errors)
            },
        })
    }
}

/// Builds the remote script: exports first, then an optional guarded
/// `cd`, then the user command as the last line so the channel's exit
/// status is the command's own. Multi-line on purpose — `cd X && cmd`
/// would leak later statements out of the directory.
pub(super) fn build_script(config: &TaskConfig) -> String {
    build_script_with(config, &config.command)
}

/// [`build_script`] with the trailing statement supplied by the caller —
/// the script-file path replaces the inline command with an interpreter
/// invocation while keeping the env/cd framing identical.
pub(super) fn build_script_with(config: &TaskConfig, final_statement: &str) -> String {
    let mut lines = Vec::new();
    for (key, value) in &config.environment {
        lines.push(format!("export {}={}", key, shell_single_quote(value)));
    }
    if !config.working_directory.is_empty() {
        // 125: distinct from typical command failures so a bad directory is
        // recognizable in exit codes; the following lines never run.
        lines.push(format!(
            "cd {} || exit 125",
            shell_single_quote(&config.working_directory)
        ));
    }
    lines.push(final_statement.to_string());
    lines.join("\n")
}

/// Interpreter invocation for an uploaded script file. `bash` when the
/// server has one (most scripts assume bashisms), `sh` as the POSIX
/// fallback; the temp path is single-quoted verbatim.
pub(super) fn script_command_line(has_bash: bool, temp_path: &str) -> String {
    let interpreter = if has_bash { "bash" } else { "sh" };
    format!("{interpreter} {}", shell_single_quote(temp_path))
}

// ---------------------------------------------------------------------------
// Local shell targets (io.dbx.sshx.local)
// ---------------------------------------------------------------------------

/// Command-flag spelling for the connection's shell: `cmd.exe` takes `/C`,
/// every POSIX shell (and PowerShell's `-c` alias) takes `-c`. Compares the
/// basename case-insensitively (Windows paths are case-insensitive) so
/// `C:\WINDOWS\system32\cmd.EXE` matches too.
pub(super) fn local_command_flag(shell: &str) -> &'static str {
    let basename = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    let basename = basename.strip_suffix(".exe").unwrap_or(&basename);
    if basename == "cmd" {
        "/C"
    } else {
        "-c"
    }
}

/// What a local spawn executes: in command mode the raw command text after
/// the shell flag; in script mode the file path itself as the shell's single
/// argument — `shell <script>` runs a non-executable file without a content
/// round-trip through argv and without chmod. Unlike the SSH path there is
/// no in-script `export`/`cd` framing: environment and working directory
/// apply at the process level (`Command::envs`/`current_dir`), so values
/// never need shell quoting.
pub(super) fn local_spawn_args(shell: &str, config: &TaskConfig) -> Vec<String> {
    if config.script_path.is_empty() {
        vec![
            local_command_flag(shell).to_string(),
            config.command.clone(),
        ]
    } else {
        vec![config.script_path.clone()]
    }
}

/// Cancellation/timeout teardown for a local child: TERM the whole process
/// group (the spawn created one, so `shell -c` compound commands cannot
/// leave grandchildren behind), give it a short grace window, then escalate
/// to SIGKILL — both the group (direct `killpg` syscall) and the leader
/// unconditionally through `start_kill`. Reaping stays with the caller.
#[cfg(unix)]
pub(super) async fn terminate_local_process(child: &mut tokio::process::Child) {
    let Some(pid) = child.id() else {
        return; // already exited and reaped by a previous pass
    };
    // Direct killpg syscall, not the external `kill` binary: the binary's
    // argv handling is environment-sensitive (a GitHub runner image change
    // made the group TERM there take down the runner agent mid-job, red CI
    // across main and PRs from 2026-10-12 on), and the fork/exec per
    // teardown is avoidable overhead.
    let group = pid as libc::pid_t;
    // SAFETY: killpg only sends a signal to the process group `group`; a
    // vanished group fails with ESRCH, which we deliberately ignore.
    unsafe {
        libc::killpg(group, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_millis(1_500);
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // SAFETY: as above.
    unsafe {
        libc::killpg(group, libc::SIGKILL);
    }
    let _ = child.start_kill();
}

/// Windows has no POSIX process groups; `kill_on_drop` plus the direct
/// `TerminateProcess` behind `start_kill` is the best available teardown
/// for the spawned shell itself.
#[cfg(not(unix))]
pub(super) async fn terminate_local_process(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
}

// ---------------------------------------------------------------------------
// Request shape (ADR §6.2)
// ---------------------------------------------------------------------------

/// Parsed `task` envelope shared by execute/start.
#[derive(Debug, Clone)]
pub(crate) struct TaskRequest {
    pub task_id: String,
    pub run_id: String,
    pub trigger_id: String,
    pub connection_id: String,
    pub config: Value,
}

impl TaskRequest {
    pub(super) fn parse(params: &Value) -> Result<Self, String> {
        let task = params
            .get("task")
            .ok_or_else(|| task_error(CODE_INVALID_CONFIG, "Missing task envelope"))?;
        let run_id = task
            .get("runId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                params
                    .get("run")
                    .and_then(|run| run.get("runId"))
                    .and_then(Value::as_str)
            })
            .map(str::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        let connection_id = task
            .get("connectionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| task_error(CODE_INVALID_CONFIG, "Missing task.connectionId"))?
            .to_string();
        Ok(Self {
            task_id: task
                .get("taskId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            run_id,
            trigger_id: task
                .get("triggerId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            connection_id,
            config: task.get("config").cloned().unwrap_or_else(|| json!({})),
        })
    }
}
