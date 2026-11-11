//! MCP execution helpers.

use super::*;

impl McpState {
    /// `ssh_run_bg`: stages a long-running command detached on the remote
    /// host (nohup, output appended to `~/.dbx-ssh-tasks/<taskId>.log`)
    /// and returns immediately. The server-side log file is the task's
    /// durable record: status polling reattaches over a fresh connection, so
    /// flapping networks, disconnects, and MCP-host wait caps never lose
    /// output or kill the job.
    pub(crate) async fn ssh_run_bg_tool(&self, arguments: &Value) -> Result<Value, String> {
        let command = required_str(arguments, "command")?;
        // Sudo allowlist rebuild, identical to `ssh_exec_tool`. This path used
        // to ship the raw text: `enforce_command_text_gates` only *matched* the
        // allowlist with `is_allowed`, then the original characters went into
        // the remote `sh -c '...'` wrapper. A trailing-`*` entry such as
        // `docker restart *` accepts `x;` as an ordinary token, so
        // `sudo docker restart x; cp /bin/sh /tmp/s` passed the gate and the
        // second statement ran as root. The rebuild token-quotes every
        // argument, which turns `;` back into a literal character.
        let allowlist = self.sudo_allowlist_for(arguments).await?;
        let command: String = if !allowlist.is_empty() && mcp_safety::runs_under_sudo(command) {
            sudo_allowlist::allowed_rebuilt_command(&allowlist, command).ok_or_else(|| {
                format!(
                    "sudo command is not allowed by this connection's whitelist. \
                     Allowed patterns: {}",
                    sudo_allowlist::render_entries(&allowlist)
                )
            })?
        } else {
            command.to_string()
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        // Task id must be collision-free *within one millisecond*: the remote
        // staging line truncates the log with `: > "$f"`, so two calls sharing
        // an id silently destroy each other's output and pid file. The pid is
        // constant inside this process, so the old
        // `bg-{ms}-{pid%100000}` had zero entropy per millisecond; the uuid
        // suffix is the discriminator.
        let task_id = background_task_id(stamp);
        // Single-quote escape for embedding inside the remote `sh -c '...'`.
        let escaped = command.replace('\'', "'\\''");
        // The task dir lives under $HOME (never a shared /tmp path another
        // local user could pre-create or symlink-bait) and is created 0700 so
        // task logs/pids stay private to the remote account.
        let remote = format!(
            "d=\"$HOME/.dbx-ssh-tasks\"; mkdir -p -m 700 \"$d\" || exit 3; f=\"$d/{task_id}.log\"; : > \"$f\"; \
             nohup sh -c '{escaped}; s=$?; echo EXIT_$s' >> \"$f\" 2>&1 & p=$!; \
             echo \"$p\" > \"$f.pid\"; echo \"PID=$p\"; echo \"LOG=$f\""
        );
        let connection = self.connection(arguments).await?;
        // Internal staging wrapper: env-free so the wrapper's own output
        // stays parseable; the user command inherits the server defaults.
        let outcome = exec::exec_plain(&connection, &remote, Duration::from_secs(15), &[]).await?;
        if outcome.exit_code != 0 {
            return Err(format!(
                "Failed to stage background task (exit {}): {}",
                outcome.exit_code, outcome.output
            ));
        }
        let default_log = format!("~/.dbx-ssh-tasks/{task_id}.log");
        let (pid, log_path) = parse_bg_start_output(&outcome.output, default_log);
        Ok(json!({
            "taskId": task_id,
            "pid": pid,
            "logPath": log_path,
            "pollWith": "ssh_task_status",
            "note": "Running detached (nohup). Poll ssh_task_status(logPath); output survives disconnects and session restarts.",
        }))
    }

    /// `ssh_task_status`: polls a task started by `ssh_run_bg` through its
    /// server-side log file. Works across disconnects and from later
    /// sessions because the log lives on the remote host, not in this MCP
    /// session.
    pub(crate) async fn ssh_task_status_tool(&self, arguments: &Value) -> Result<Value, String> {
        let log_path = required_str(arguments, "logPath")?;
        validate_task_log_path(log_path)?;
        let tail_bytes = arg_u64(arguments, "tailBytes")?
            .unwrap_or(4_000)
            .clamp(200, 16_000);
        let quoted = exec::shell_quote(log_path);
        let remote = format!(
            "f={quoted}; if [ ! -f \"$f\" ]; then echo STATE=MISSING; exit 0; fi; \
             if grep -q '^EXIT_[0-9][0-9]*$' \"$f\" 2>/dev/null; then \
               echo STATE=DONE; echo CODE=$(grep -o '^EXIT_[0-9][0-9]*$' \"$f\" | tail -1); \
             else echo STATE=RUNNING; fi; \
             pf=\"$f.pid\"; if [ -f \"$pf\" ]; then p=$(cat \"$pf\"); \
               kill -0 \"$p\" 2>/dev/null && echo PID_ALIVE=yes || echo PID_ALIVE=no; fi; \
             echo ===TAIL===; tail -c {tail_bytes} \"$f\""
        );
        let connection = self.connection(arguments).await?;
        let outcome = exec::exec_plain(&connection, &remote, Duration::from_secs(15), &[]).await?;
        if outcome.exit_code != 0 {
            return Err(format!(
                "Failed to read task status (exit {}): {}",
                outcome.exit_code, outcome.output
            ));
        }
        let (state, exit_code, pid_alive, tail) = parse_task_status_output(&outcome.output);
        Ok(json!({
            "state": state,
            "exitCode": exit_code,
            "pidAlive": pid_alive,
            "output": tail.trim_end(),
            "logPath": log_path,
            "done": state == "done",
        }))
    }
}

/// Parses the `PID=` / `LOG=` lines emitted by the `ssh_run_bg` staging
/// command. Falls back to the derived default log path when the remote
/// never echoed `LOG=`.
pub(crate) fn parse_bg_start_output(output: &str, default_log: String) -> (String, String) {
    let mut pid = String::new();
    let mut log_path = default_log;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("PID=") {
            pid = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("LOG=") {
            log_path = value.trim().to_string();
        }
    }
    (pid, log_path)
}

/// Parses the STATE= / CODE= / PID_ALIVE= markers and the post-`===TAIL===`
/// section emitted by the `ssh_task_status` probe command.
pub(crate) type TaskStatusParse = (String, Option<i64>, Option<bool>, String);

pub(crate) fn parse_task_status_output(output: &str) -> TaskStatusParse {
    let mut state = "unknown".to_string();
    let mut exit_code: Option<i64> = None;
    let mut pid_alive: Option<bool> = None;
    let mut tail = String::new();
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("STATE=") {
            state = value.trim().to_lowercase();
        } else if let Some(value) = line.strip_prefix("CODE=EXIT_") {
            exit_code = value.trim().parse().ok();
        } else if let Some(value) = line.strip_prefix("PID_ALIVE=") {
            pid_alive = Some(value.trim() == "yes");
        } else if line == "===TAIL===" {
            tail.clear();
        } else if !line.is_empty() {
            tail.push_str(line);
            tail.push('\n');
        }
    }
    (state, exit_code, pid_alive, tail)
}

/// Confines `ssh_task_status` to the log files `ssh_run_bg` itself creates.
///
/// The tool used to accept any path, shell-quote it, and return the trailing
/// 16 KiB — a remote arbitrary-file-read primitive on every writable
/// connection (`logPath: /etc/shadow`, a private key, another user's task
/// log). The only guard was `is_sensitive_path`, which applies to read-only
/// connections exclusively. Task logs always live in `~/.dbx-ssh-tasks/` with
/// a `bg-` prefix, so anything else is a caller mistake or an attempt to
/// repurpose the tool.
pub(crate) fn validate_task_log_path(log_path: &str) -> Result<(), String> {
    let trimmed = log_path.trim();
    if trimmed.is_empty() {
        return Err("logPath is required".to_string());
    }
    // `..` would let `~/.dbx-ssh-tasks/../../etc/shadow` pass a naive prefix
    // test; reject it outright rather than trying to normalize remotely.
    if trimmed.split(['/', '\\']).any(|segment| segment == "..") {
        return Err(format!("logPath must not contain '..': {trimmed}"));
    }
    let in_task_dir = trimmed.starts_with("~/.dbx-ssh-tasks/")
        || trimmed.starts_with("$HOME/.dbx-ssh-tasks/")
        || trimmed.contains("/.dbx-ssh-tasks/");
    let file_name = trimmed.rsplit(['/', '\\']).next().unwrap_or_default();
    if !in_task_dir || !file_name.starts_with("bg-") {
        return Err(format!(
            "logPath must point at a task log under ~/.dbx-ssh-tasks/ (bg-*.log); \
             use the logPath returned by ssh_run_bg instead of: {trimmed}"
        ));
    }
    Ok(())
}

/// Server-side task id for `ssh_run_bg`.
///
/// `stamp` is the unix millisecond, which by itself is not a discriminator:
/// two calls in the same millisecond inside one process produced the same id
/// (`pid % 100_000` is constant), and the remote staging line truncates the
/// log file with `: > "$f"` — so the second task wiped the first one's output
/// and pid. The uuid suffix makes same-millisecond ids distinct.
pub(crate) fn background_task_id(stamp: u128) -> String {
    format!(
        "bg-{stamp}-{}-{}",
        std::process::id() % 100_000,
        uuid::Uuid::new_v4().simple()
    )
}
