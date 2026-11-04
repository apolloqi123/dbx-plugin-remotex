//! Docker feature layer.

use super::*;

/// Lifecycle verbs the panel may run. Anything outside this whitelist is
/// rejected before any remote I/O (`parse_action`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerAction {
    Start,
    Stop,
    Restart,
    Kill,
    Remove,
}

impl DockerAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Kill => "kill",
            Self::Remove => "rm",
        }
    }
}

/// Parses the requested action name. Unknown verbs fail with the accepted
/// list in the error (typo-guidance style, like metrics sections).
pub fn parse_action(name: &str) -> Result<DockerAction, String> {
    match name {
        "start" => Ok(DockerAction::Start),
        "stop" => Ok(DockerAction::Stop),
        "restart" => Ok(DockerAction::Restart),
        "kill" => Ok(DockerAction::Kill),
        "rm" | "remove" => Ok(DockerAction::Remove),
        _ => Err(format!(
            "Unsupported docker action '{}'. Supported: start, stop, restart, kill, rm",
            name
        )),
    }
}

/// Renders the remote command for one action as the approval/audit text:
/// `{cli} {action} {id}` — no endpoint env prefix (the gate text must stay
/// short and stable). The id is inserted verbatim; callers must have run
/// [`validate_container_id`] (both protocol entry points do, and the
/// format! below only ever receives validated input).
pub fn action_command(engine: &EngineConfig, action: DockerAction, container_id: &str) -> String {
    format!("{} {} {}", engine.cli, action.as_str(), container_id)
}

/// The command actually executed: the endpoint export (if any) prepended
/// to [`action_command`]. POSIX shells only — Windows hosts reach actions
/// through the same text (`set` prefixes never gate actions because the
/// daemon-permission fallback does not apply there).
pub fn exec_command(engine: &EngineConfig, action: DockerAction, container_id: &str) -> String {
    format!(
        "{}{}",
        engine.env_prefix(),
        action_command(engine, action, container_id)
    )
}

/// Container id gate: `^[0-9a-f]{12,64}$`. Hand-rolled (no regex needed):
/// lowercase hex only, 12 (short id) to 64 (full sha256) chars. Anything
/// else — names, ids with shells metacharacters, uppercase — is refused
/// before a command is ever rendered.
pub fn validate_container_id(container_id: &str) -> Result<(), String> {
    let valid_len = (12..=64).contains(&container_id.len());
    let hex_only = container_id
        .bytes()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if valid_len && hex_only {
        return Ok(());
    }
    Err(format!(
        "Invalid containerId '{container_id}': expected 12-64 lowercase hex characters"
    ))
}

/// Clamps the requested tail line count into [10, 1000].
pub fn clamp_tail(tail: u64) -> u64 {
    tail.clamp(TAIL_MIN, TAIL_MAX)
}
