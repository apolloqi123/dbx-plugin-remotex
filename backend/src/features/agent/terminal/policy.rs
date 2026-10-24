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

/// The §1 matrix: `auto` approves low-risk commands and prompts for
/// elevated ones; `strict` prompts for everything.
///
/// `Off` only reaches here when `runInTerminal: true` forced the terminal
/// path (otherwise the routing never leaves the hidden channel): low-risk
/// commands still run because they are plain visibility, and elevated ones
/// raise the in-app approval prompt — the agent explicitly opted into the
/// visible terminal, the human still approves each command, and an
/// unanswered prompt times out to a refusal. Without that explicit opt-in
/// an elevated command under `off` is denied outright.
pub fn decide(
    mode: AgentTerminalMode,
    risk: CommandRisk,
    explicit_run_in_terminal: bool,
) -> RoutingDecision {
    match (mode, risk) {
        (AgentTerminalMode::Off, CommandRisk::Low) => RoutingDecision::Run,
        (AgentTerminalMode::Off, CommandRisk::Elevated) if explicit_run_in_terminal => {
            RoutingDecision::Prompt
        }
        (AgentTerminalMode::Off, CommandRisk::Elevated) => RoutingDecision::Deny(
            "Agent terminal mode is off; retry with runInTerminal: true to ask the user to \
             approve this command in the terminal, or keep it on the hidden channel",
        ),
        (AgentTerminalMode::Auto, CommandRisk::Low) => RoutingDecision::Run,
        (AgentTerminalMode::Auto, CommandRisk::Elevated) => RoutingDecision::Prompt,
        (AgentTerminalMode::Strict, CommandRisk::Low) => RoutingDecision::Prompt,
        (AgentTerminalMode::Strict, CommandRisk::Elevated) => RoutingDecision::Prompt,
    }
}

/// §IMPL_PLAN D1: a remembered approval downgrades `Prompt` to `Run`;
/// `Deny` is never overridden (mode-level semantics stay intact).
///
/// The remember list is a single-command exemption: a command the user
/// already approved (possibly hand-generalized into a wildcard) skips the
/// approval prompt, but it cannot widen what a mode allows — `off` without
/// an explicit `runInTerminal` still refuses elevated commands outright, so
/// remembering can never turn a silent denial into a silent run. Callers
/// must additionally exclude `Destructive` commands before treating
/// `remembered` as true (decision D2: the disaster gate stays last).
pub fn decide_with_memory(
    mode: AgentTerminalMode,
    risk: CommandRisk,
    explicit_run_in_terminal: bool,
    remembered: bool,
) -> RoutingDecision {
    match decide(mode, risk, explicit_run_in_terminal) {
        RoutingDecision::Prompt if remembered => RoutingDecision::Run,
        other => other,
    }
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
