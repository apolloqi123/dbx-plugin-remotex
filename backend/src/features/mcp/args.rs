//! MCP args helpers.

use super::*;

pub(super) fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("Missing or invalid parameter: {key} (expected a non-empty string)"))
}

/// Enumerates EVERY absent required string parameter in one error
/// (`Missing required parameters: a, b`): an LLM caller fixes all gaps in a
/// single turn instead of discovering them one fail-fast round at a time.
/// Present-but-invalid values (wrong type, empty string) are NOT listed here
/// — the per-parameter `required_str`/type checks that follow name them
/// precisely with `Missing or invalid parameter: <key>`.
pub(super) fn missing_required(arguments: &Value, keys: &[&str]) -> Result<(), String> {
    let missing: Vec<&str> = keys
        .iter()
        .copied()
        .filter(|key| matches!(arguments.get(key), None | Some(Value::Null)))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Missing required parameters: {}",
            missing.join(", ")
        ))
    }
}

/// Reads an integer argument that may arrive as a JSON number or a numeric
/// string (LLMs frequently quote numbers: `"port": "2222"`). Returns
/// `Ok(None)` when absent/null; a non-numeric value is a clear parameter
/// error instead of silently falling back to the argument's default.
pub(crate) fn arg_u64(arguments: &Value, key: &str) -> Result<Option<u64>, String> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if let Some(number) = value.as_u64() {
        return Ok(Some(number));
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if let Ok(number) = trimmed.parse::<u64>() {
            return Ok(Some(number));
        }
        return Err(format!("{key} must be an integer, got '{trimmed}'"));
    }
    Err(format!("{key} must be an integer, got {value}"))
}

/// Strict port validation: absent → 22, a number or numeric string within
/// `1..=65535` is accepted, anything else is a range/type error. Runs in
/// `call_tool` before every gate so a malformed port can never silently
/// dial the default port 22 instead of the requested one.
pub(super) fn arg_port(arguments: &Value) -> Result<u16, String> {
    match arg_u64(arguments, "port")? {
        None => Ok(22),
        Some(port) if (1..=u16::MAX as u64).contains(&port) => Ok(port as u16),
        Some(port) => Err(format!("port must be between 1 and 65535, got {port}")),
    }
}

/// Lossy port read for gate/pool lookups (absent or malformed → 22).
/// `call_tool` rejects malformed ports before any gate runs, so the
/// fallback is only reachable through direct test entry points.
pub(super) fn arg_port_lossy(arguments: &Value) -> u16 {
    arg_port(arguments).unwrap_or(22)
}

/// Reads a boolean argument, tolerating the `"true"`/`"false"` string
/// variants LLMs emit (`confirmDestructive: "true"` must not silently read
/// as `false`). Absent/null → `Ok(None)`.
pub(crate) fn arg_bool(arguments: &Value, key: &str) -> Result<Option<bool>, String> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if let Some(flag) = value.as_bool() {
        return Ok(Some(flag));
    }
    if let Some(text) = value.as_str() {
        return match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Some(true)),
            "false" | "0" | "no" | "off" => Ok(Some(false)),
            _ => Err(format!("{key} must be a boolean, got '{text}'")),
        };
    }
    Err(format!("{key} must be a boolean, got {value}"))
}

pub(super) fn non_empty_argument<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Copy)]
pub(super) struct EndpointSelector<'a> {
    pub(super) host: &'a str,
    pub(super) port: u16,
    pub(super) username: &'a str,
}

/// A host selector is intentionally complete: host plus username, with port
/// defaulting to 22. Matching only on host would silently choose the wrong
/// saved account when a machine has multiple SSH users.
pub(super) fn endpoint_selector(arguments: &Value) -> Option<EndpointSelector<'_>> {
    let host = non_empty_argument(arguments, "host")?;
    let username = non_empty_argument(arguments, "username")?;
    let port = arg_port_lossy(arguments);
    Some(EndpointSelector {
        host,
        port,
        username,
    })
}

pub(super) fn connection_matches_selectors(
    connection: &StoredConnection,
    name: Option<&str>,
    endpoint: Option<&EndpointSelector<'_>>,
) -> bool {
    name.is_none_or(|wanted| connection.name.as_deref() == Some(wanted))
        && endpoint.is_none_or(|selector| {
            connection.host.trim().eq_ignore_ascii_case(selector.host)
                && connection.port == selector.port
                && connection.username == selector.username
        })
}

pub(super) fn endpoint_selector_label(selector: &EndpointSelector<'_>) -> String {
    format!(
        "endpoint {}@{}:{}",
        selector.username, selector.host, selector.port
    )
}

pub(super) fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "ssh_exec_sudo" | "ssh_run_bg" | "sftp_write_file" | "sftp_upload"
    )
}

/// Local write targets that must never receive downloaded content through
/// the MCP channel: shell/SSH bootstrap files plus cron / systemd / launchd
/// drop locations turn a remote file into local code execution. Applied on
/// every connection (read-only or not) — this guard protects the operator
/// machine, not the remote side. Defense in depth, not a sandbox.
pub(super) fn is_sensitive_local_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    // Match on a collapsed form so `/etc//cron.d/x`, `/etc/./profile` and
    // friends cannot dodge the component / prefix rules (reliability
    // round 5 adversarial pass).
    let normalized = mcp_safety::normalized_path(&lower);
    const COMPONENTS: &[&str] = &[".ssh", ".gnupg"];
    if normalized
        .split('/')
        .any(|component| COMPONENTS.contains(&component))
    {
        return true;
    }
    const PREFIXES: &[&str] = &[
        "/etc/cron",
        "/var/spool/cron",
        "/etc/sudoers",
        "/etc/ssh",
        "/etc/ld.so",
        "/etc/pam.d",
        "/etc/profile",
        "/etc/bash",
        "/etc/rc",
        "/etc/systemd/system",
        "/library/launchdaemons",
        "/library/launchagents",
    ];
    if PREFIXES.iter().any(|prefix| normalized.starts_with(prefix)) {
        return true;
    }
    let file_name = Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    matches!(
        file_name,
        ".bashrc"
            | ".zshrc"
            | ".profile"
            | ".bash_profile"
            | ".zprofile"
            | ".zshenv"
            | ".zlogin"
            | ".cshrc"
            | ".tcshrc"
            | "authorized_keys"
            | ".netrc"
            | ".git-credentials"
            | ".npmrc"
            | ".htpasswd"
    )
}

/// Remote-path arguments of read tools that must respect the sensitive-path
/// denylist on read-only connections. These tools return file content or
/// listings straight to the LLM, a channel the exec command whitelist cannot
/// classify; the denylist itself lives in `mcp_safety::is_sensitive_path`.
pub(super) fn sensitive_read_path(name: &str, arguments: &Value) -> Option<String> {
    if !matches!(
        name,
        "sftp_list_dir" | "sftp_read_file" | "sftp_stat" | "sftp_download" | "ssh_task_status"
    ) {
        return None;
    }
    ["path", "remotePath", "logPath"].iter().find_map(|key| {
        arguments
            .get(*key)
            .and_then(Value::as_str)
            .filter(|value| mcp_safety::is_sensitive_path(value))
            .map(str::to_string)
    })
}

pub(super) fn is_known_tool(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

/// Actionable error for an unregistered tool name: a separator/case
/// variant (`sftp-listdir`, `SSH_EXEC`) suggests the exact registered
/// name, and every miss points at the discovery surface.
pub(super) fn unknown_tool_message(name: &str) -> String {
    let compact = |text: &str| text.to_ascii_lowercase().replace(['-', '_', ' '], "");
    let query = compact(name);
    let mut suggestion = None;
    for tool in TOOL_NAMES {
        let candidate = compact(tool);
        if candidate == query {
            suggestion = Some(*tool);
            break;
        }
        if query.len() >= 4 && (candidate.contains(&query) || query.contains(&candidate)) {
            suggestion = Some(*tool);
        }
    }
    format!(
        "Unknown tool: '{name}'. {}Use tools/list to list the {} available tools \
         (ssh_exec, ssh_exec_sudo, ssh_run_bg, ssh_task_status, \
         ssh_terminal_input, ssh_multi_exec, sftp_*, ...).",
        suggestion
            .map(|tool| format!("Did you mean '{tool}'? "))
            .unwrap_or_default(),
        TOOL_NAMES.len(),
    )
}

/// Runs a dynamically sized batch of futures concurrently and returns all
/// outputs in input order. `tokio::join!` arities are static, so this small
/// divide-and-conquer helper covers `ssh_multi_exec`'s dynamic 1-10 fan-out
/// without pulling in the `futures` crate: halves recurse (depth ≤4 for 10
/// targets) until each leaf is a static-arity `join!`. The previous single
/// sweep awaited each slot to completion inside the loop — sequential
/// execution wearing a concurrent signature; 10 targets × a 300s timeout
/// serialized to up to 3000s.
pub(super) async fn join_all<F, T>(mut futures: Vec<F>) -> Vec<T>
where
    F: Future<Output = T> + Send,
    T: Send,
{
    match futures.len() {
        0 => Vec::new(),
        1 => {
            let mut iter = futures;
            vec![iter.pop().expect("len 1").await]
        }
        2 => {
            let mut iter = futures;
            let second = iter.pop().expect("len 2");
            let first = iter.pop().expect("len 2");
            let (first_output, second_output) = tokio::join!(first, second);
            vec![first_output, second_output]
        }
        len => {
            let second = futures.split_off(len / 2);
            // 递归分支经 Box::pin 间接化：async fn 直接自递归的 future 大小
            // 无界，编译期即拒绝。
            let (first_outputs, second_outputs) =
                tokio::join!(Box::pin(join_all(futures)), Box::pin(join_all(second)));
            first_outputs.into_iter().chain(second_outputs).collect()
        }
    }
}
