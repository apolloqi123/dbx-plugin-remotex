//! Terminal routing policy and persisted agent-mode settings.

use super::*;

/// Verdict of the §1 policy matrix for a terminal-routed command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingDecision {
    /// Inject into the terminal directly (a notice event is still emitted).
    Run,
    /// Raise an approval challenge and wait for `ssh/agent/resolve`.
    Prompt,
    /// Refuse the terminal path outright (the AI keeps the hidden channel).
    Deny(&'static str),
}

/// Whether one exec call enters the visible terminal at all.
///
/// The connection-level setting is authoritative: `off` means "AI commands
/// stay on the hidden channel" and `auto`/`strict` route through the terminal.
/// The per-call `runInTerminal` flag is only a hint kept for schema
/// compatibility — it can no longer force the hidden channel to dodge the
/// approval matrix, nor reopen a terminal the user disabled.
pub fn routes_to_terminal(mode: AgentTerminalMode) -> bool {
    mode != AgentTerminalMode::Off
}

/// The approval matrix for a terminal-routed command.
///
/// `auto` (graded approval): provably read-only views run directly; any
/// command that can change remote state (mutating) or escalate (sudo /
/// catastrophic) raises the approval prompt. `strict` prompts for every
/// command, including read-only ones. `off` never reaches here because
/// [`routes_to_terminal`] keeps those commands on the hidden channel (the
/// branch stays as a fail-closed fallback).
pub fn decide(mode: AgentTerminalMode, risk: CommandRisk) -> RoutingDecision {
    match (mode, risk) {
        (AgentTerminalMode::Off, _) => RoutingDecision::Deny(
            "Agent terminal mode is off; AI commands stay on the hidden channel",
        ),
        (AgentTerminalMode::Auto, CommandRisk::ReadOnly) => RoutingDecision::Run,
        (AgentTerminalMode::Auto, CommandRisk::Mutating | CommandRisk::Elevated) => {
            RoutingDecision::Prompt
        }
        (AgentTerminalMode::Strict, _) => RoutingDecision::Prompt,
    }
}

/// §IMPL_PLAN D1: a remembered approval downgrades `Prompt` to `Run`; `Deny`
/// is never overridden. The disaster gate stays last: callers must pass
/// `remembered = false` for a catastrophic command (see
/// [`remembered_bypasses_prompt`]), so a wildcard remembered line can never
/// bless `rm -rf /` or `mkfs`.
pub fn decide_with_memory(
    mode: AgentTerminalMode,
    risk: CommandRisk,
    remembered: bool,
) -> RoutingDecision {
    match decide(mode, risk) {
        RoutingDecision::Prompt if remembered => RoutingDecision::Run,
        other => other,
    }
}

/// Whether a remembered approval may skip the prompt for this exact command.
/// Re-checks the disaster classifier (D2) so a stored wildcard cannot widen
/// into a catastrophic command.
pub fn remembered_bypasses_prompt(command: &str, remembered: bool) -> bool {
    remembered
        && !matches!(
            crate::features::mcp::safety::assess_command(command),
            crate::features::mcp::safety::CommandRisk::Destructive(_)
        )
}

/// File the per-connection mode map persists to, relative to the plugin data
/// directory. Plain `{ "version": 1, "modes": { "<connectionId>": "auto" } }`
/// JSON — no credential material, so the sudo-profile 0600 dance is skipped.
const MODES_FILE_NAME: &str = "agent-modes.json";

/// Per-connection persisted store path.
pub fn modes_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(MODES_FILE_NAME)
}

/// Loads the persisted mode map; a missing or corrupted file yields an empty
/// map so a bad file can never break connecting (same policy as
/// mcp-settings.json and the Quick Sudo store).
pub fn load_modes(data_dir: &std::path::Path) -> HashMap<String, AgentTerminalMode> {
    let text = std::fs::read_to_string(modes_path(data_dir)).unwrap_or_default();
    let Some(value) = serde_json::from_str::<serde_json::Value>(&text).ok() else {
        return HashMap::new();
    };
    value
        .get("modes")
        .and_then(serde_json::Value::as_object)
        .map(|map| {
            map.iter()
                .filter(|(_, mode)| mode.is_string())
                .map(|(connection, mode)| {
                    (
                        connection.clone(),
                        AgentTerminalMode::parse(mode.as_str().unwrap_or("")),
                    )
                })
                .filter(|(_, mode)| *mode != AgentTerminalMode::Off)
                .collect()
        })
        .unwrap_or_default()
}

/// Persists the mode map atomically (tmp + rename). Failures surface to the
/// caller: the persistence is the feature (the in-memory toggle already
/// worked), so a silent loss would quietly resurrect the restart bug.
pub fn save_modes(
    data_dir: &std::path::Path,
    modes: &HashMap<String, AgentTerminalMode>,
) -> Result<(), String> {
    let path = modes_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let entries: serde_json::Map<String, serde_json::Value> = modes
        .iter()
        .map(|(connection, mode)| (connection.clone(), serde_json::Value::from(mode.name())))
        .collect();
    let value = serde_json::json!({ "version": 1, "modes": entries });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Failed to encode agent terminal modes: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)
        .map_err(|error| format!("Failed to write {}: {error}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}
