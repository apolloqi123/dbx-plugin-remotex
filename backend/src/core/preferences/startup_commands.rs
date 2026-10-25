//! Connection-scoped startup commands (Tabby "Login scripts" parity, M7 P0-4).
//!
//! The workbench edits, per connection id, an ordered list of commands that the
//! sidecar types into the interactive PTY right after the shell comes up
//! (`ssh.rs` spawns the injector once `request_shell` succeeded). This is a
//! shell-level keyboard sequence — deliberately NOT a `RemoteCommand`: an exec
//! replaces the shell instead of typing into it, so sessions with a non-empty
//! `remote_command` skip startup commands entirely (see [`executes_for`]).
//!
//! Storage reuses the allowlisted `preferences.json` infra under one key,
//! `startup_commands`: `{ <connectionId>: { enabled, commands: [...] } }`.
//! Command contents can carry secrets, so they never reach logs, audit or
//! events — the `ssh/startup` event only reports the count and completion.
//!
//! Everything here is pure (or driven through an injected sender) so the
//! parsing, limits and injection sequence are unit-testable without SSH.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Map, Value};

/// Per-command pause before typing (default). Gives the shell time to show a
/// prompt after the session opens.
pub const DEFAULT_DELAY_MS: u64 = 300;
/// Per-command pause ceiling: a stuck delay must not stall the session for
/// minutes (Tabby's login-script delay has the same "short wait" semantics).
pub const MAX_DELAY_MS: u64 = 30_000;
/// Commands per connection.
pub const MAX_COMMANDS: usize = 20;
/// Single-command size ceiling (bytes, UTF-8 boundary safe). Startup commands
/// are typed keystrokes, not scripts.
pub const MAX_COMMAND_BYTES: usize = 4 * 1024;
/// Connections tracked in the shared preference map, so one file cannot grow
/// without bound.
pub const MAX_CONNECTIONS: usize = 512;

/// One typed command: the text plus the pause before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupCommand {
    pub command: String,
    pub delay_ms: u64,
}

/// Whether startup commands apply to a session: only real shell sessions.
/// A non-empty `remote_command` execs instead of spawning a shell, so there is
/// no prompt to type into — a semantic conflict, never a fallback.
pub fn executes_for(remote_command: &str) -> bool {
    remote_command.is_empty()
}

/// Strips the trailing CR/LF run (the injector appends its own Enter) and caps
/// the command at [`MAX_COMMAND_BYTES`] on a UTF-8 char boundary.
fn clean_command(raw: &str) -> Option<String> {
    let trimmed = raw.trim_end_matches(['\r', '\n']);
    // 纯空白（含全空格）没有可执行内容：直接丢弃而不是原样注入。
    if trimmed.trim().is_empty() {
        return None;
    }
    let owned = if trimmed.len() <= MAX_COMMAND_BYTES {
        trimmed.to_string()
    } else {
        let mut end = MAX_COMMAND_BYTES;
        while !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        trimmed[..end].to_string()
    };
    Some(owned)
}

/// Clamps the per-command pause: missing/non-numeric falls back to the
/// default, values above the ceiling truncate to the ceiling (never rejected —
/// a hand-edited or stale file must not break the session).
fn clamp_delay(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .unwrap_or(DEFAULT_DELAY_MS)
            .min(MAX_DELAY_MS),
        Some(Value::String(text)) => text
            .trim()
            .parse::<u64>()
            .unwrap_or(DEFAULT_DELAY_MS)
            .min(MAX_DELAY_MS),
        _ => DEFAULT_DELAY_MS,
    }
}

/// Runtime parse of one command entry. Disabled (`enabled: false`), empty or
/// malformed entries are dropped rather than failing the whole plan.
fn parse_entry(value: &Value) -> Option<StartupCommand> {
    let object = value.as_object()?;
    if object.get("enabled").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let raw = object.get("command")?.as_str()?;
    let command = clean_command(raw)?;
    Some(StartupCommand {
        command,
        delay_ms: clamp_delay(object.get("delayMs")),
    })
}

/// Persisted shape of one entry: normalized for the workbench round-trip.
/// Unlike [`parse_entry`] this keeps `enabled: false` rows so the editor can
/// show them.
pub(crate) fn sanitize_entry(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let raw = object.get("command").and_then(Value::as_str)?;
    let command = clean_command(raw)?;
    Some(json!({
        "command": command,
        "delayMs": clamp_delay(object.get("delayMs")),
        "enabled": object.get("enabled").and_then(Value::as_bool).unwrap_or(true),
    }))
}

/// Sanitizes the whole `startup_commands` preference value: an object keyed by
/// connection id, each entry `{ enabled: bool (default false), commands: [...] }`.
/// Malformed connections/entries are dropped, never rejected, so a hand-edited
/// file cannot wedge the workbench preferences panel.
pub(crate) fn sanitize_store(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let mut out = Map::new();
    for (connection_id, entry) in object {
        if out.len() >= MAX_CONNECTIONS {
            break;
        }
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let commands = entry
            .get("commands")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(sanitize_entry)
                    .take(MAX_COMMANDS)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        out.insert(
            connection_id.clone(),
            json!({
                "enabled": entry.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                "commands": commands,
            }),
        );
    }
    Some(Value::Object(out))
}

/// The active plan for one connection from a sanitized store value: nothing
/// unless the connection's master switch is on; disabled entries filtered;
/// order preserved.
pub(crate) fn active_commands(store: Option<&Value>, connection_id: &str) -> Vec<StartupCommand> {
    let Some(store) = store.and_then(Value::as_object) else {
        return Vec::new();
    };
    let Some(entry) = store.get(connection_id) else {
        return Vec::new();
    };
    if entry.get("enabled").and_then(Value::as_bool) != Some(true) {
        return Vec::new();
    }
    entry
        .get("commands")
        .and_then(Value::as_array)
        // Re-state the cap on the read path: the write path sanitizes, but a
        // hand-crafted preferences.json must not inject an unbounded sequence.
        .map(|items| {
            items
                .iter()
                .filter_map(parse_entry)
                .take(MAX_COMMANDS)
                .collect()
        })
        .unwrap_or_default()
}

/// Reads the plan straight from `preferences.json` on disk. Missing or
/// corrupted file degrades to an empty plan (same policy as the preferences
/// loader) — a broken store must never block opening a session.
pub(crate) fn load_plan(data_dir: &Path, connection_id: &str) -> Vec<StartupCommand> {
    let text =
        std::fs::read_to_string(crate::core::preferences::store_path(data_dir)).unwrap_or_default();
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    active_commands(
        value
            .get("prefs")
            .and_then(|prefs| prefs.get("startup_commands")),
        connection_id,
    )
}

/// The bytes typed into the PTY for one command: the text plus Enter.
pub(crate) fn input_payload(command: &str) -> Vec<u8> {
    let mut payload = command.as_bytes().to_vec();
    payload.push(b'\r');
    payload
}

/// Sequential injector (Tabby login-script semantics): waits `delay_ms` before
/// each command (the first wait also covers shell spin-up), then hands the
/// payload to `send`. `send` returning `false` — the session's input channel is
/// gone — stops the sequence and reports incomplete. `true` = every command
/// was delivered.
pub(crate) async fn inject_sequence<S, Fut>(commands: &[StartupCommand], mut send: S) -> bool
where
    S: FnMut(Vec<u8>) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for command in commands {
        tokio::time::sleep(Duration::from_millis(command.delay_ms)).await;
        if !send(input_payload(&command.command)).await {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests;
