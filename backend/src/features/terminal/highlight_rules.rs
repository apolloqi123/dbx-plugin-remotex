//! Terminal keyword highlight rules (Netcatty parity batch, IMPL_PLAN §1.1):
//! user-defined `{pattern, color, isRegex, caseSensitive, enabled}` entries
//! persisted in `<plugin_data_dir>/highlight-rules.json` and shared by every
//! workbench terminal. The module mirrors `quick_commands.rs` line by line
//! (same storage policy: versioned pretty JSON, tmp+rename atomic write,
//! 0600 on Unix, corrupted file degrades to an empty store).
//!
//! Regex validity is deliberately NOT checked here (no `regex` dependency;
//! the frontend compiles patterns before saving per §1.1) — the backend only
//! validates the shape: non-empty trimmed pattern up to 200 chars and a
//! `#rrggbb` color.

use std::path::Path;

use serde_json::{json, Value};

use crate::core::time::unix_now_secs;

const STORAGE_VERSION: u64 = 1;
const FILE_NAME: &str = "highlight-rules.json";
/// Cap shared with the workbench editor (`HIGHLIGHT_RULES_LIMIT`).
pub const MAX_RULES: usize = 30;
const MAX_PATTERN_CHARS: usize = 200;
/// Default color for new rules (amber-500), mirrored by the frontend swatch.
pub const DEFAULT_COLOR: &str = "#f59e0b";

/// First-run rules seeded into an empty data dir (deterministic ids so a
/// future "restore defaults" can recognize them). Every pattern is something
/// an SSH user actually reads in terminal output, colored from the workbench
/// palette by severity: red = hard error, amber = command failed / restricted,
/// yellow = warning, green = success / healthy, blue = extractable info.
///
/// Matching priority is pattern length (frontend sorts longest-first), so the
/// specific red phrases win over their generic amber stems inside the same
/// line: "Permission denied" renders red while "Access denied" stays amber.
/// `(id, pattern, color, caseSensitive, isRegex)` — plain + case-insensitive
/// everywhere except the exact-case `PASSED` (avoids prose "passed") and the
/// IPv4 regex.
const DEFAULT_RULE_SPECS: [(&str, &str, &str, bool, bool); 22] = [
    // ---- 红：硬错误 ----
    ("default-error", "ERROR", "#ef4444", false, false),
    ("default-fatal", "FATAL", "#ef4444", false, false),
    (
        "default-permission-denied",
        "Permission denied",
        "#ef4444",
        false,
        false,
    ),
    (
        "default-no-such-file",
        "No such file or directory",
        "#ef4444",
        false,
        false,
    ),
    (
        "default-command-not-found",
        "command not found",
        "#ef4444",
        false,
        false,
    ),
    (
        "default-connection-refused",
        "Connection refused",
        "#ef4444",
        false,
        false,
    ),
    (
        "default-no-space-left",
        "No space left on device",
        "#ef4444",
        false,
        false,
    ),
    ("default-failed-to", "Failed to", "#ef4444", false, false),
    ("default-cannot", "cannot", "#ef4444", false, false),
    ("default-exception", "Exception", "#ef4444", false, false),
    (
        "default-traceback",
        "Traceback (most recent call last)",
        "#ef4444",
        false,
        false,
    ),
    // ---- 琥珀：命令失败 / 受限 ----
    ("default-fail", "FAIL", "#f59e0b", false, false),
    ("default-denied", "denied", "#f59e0b", false, false),
    ("default-timed-out", "timed out", "#f59e0b", false, false),
    // ---- 黄：警告 ----
    ("default-warn", "WARN", "#facc15", false, false),
    ("default-deprecated", "deprecated", "#facc15", false, false),
    // ---- 绿：成功 / 健康 ----
    ("default-success", "SUCCESS", "#22c55e", false, false),
    (
        "default-active-running",
        "active (running)",
        "#22c55e",
        false,
        false,
    ),
    ("default-done", "done", "#22c55e", false, false),
    ("default-check", "✓", "#22c55e", false, false),
    ("default-passed", "PASSED", "#22c55e", true, false),
    // ---- 蓝：可提取信息 ----
    (
        "default-ipv4",
        r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b",
        "#3b82f6",
        false,
        true,
    ),
];

fn default_rules(now: u64) -> Vec<HighlightRuleEntry> {
    DEFAULT_RULE_SPECS
        .iter()
        .map(
            |(id, pattern, color, case_sensitive, is_regex)| HighlightRuleEntry {
                id: (*id).to_string(),
                pattern: (*pattern).to_string(),
                is_regex: *is_regex,
                color: (*color).to_string(),
                case_sensitive: *case_sensitive,
                enabled: true,
                created_at: now,
                updated_at: now,
            },
        )
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct HighlightRuleEntry {
    pub id: String,
    pub pattern: String,
    pub is_regex: bool,
    pub color: String,
    pub case_sensitive: bool,
    pub enabled: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HighlightRuleStore {
    pub rules: Vec<HighlightRuleEntry>,
}

pub fn store_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(FILE_NAME)
}

/// Loads the store; a missing or corrupted file yields an empty store so a
/// bad file can never break the workbench (same policy as quick-commands).
pub fn load_store(data_dir: &Path) -> HighlightRuleStore {
    let text = std::fs::read_to_string(store_path(data_dir)).unwrap_or_default();
    let Some(value) = serde_json::from_str::<Value>(&text).ok() else {
        return HighlightRuleStore::default();
    };
    let rules = value
        .get("rules")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(entry_from_json).collect())
        .unwrap_or_default();
    HighlightRuleStore { rules }
}

/// Loads the store, seeding the first-run defaults when the store file does
/// not exist yet. A corrupted or user-emptied store is never reseeded (the
/// file existing is the "user already has a store" signal); a failed seed
/// write still returns the defaults for this call and retries on the next one.
pub fn load_or_seed_store(data_dir: &Path) -> HighlightRuleStore {
    if !store_path(data_dir).exists() {
        let store = HighlightRuleStore {
            rules: default_rules(unix_now_secs()),
        };
        let _ = save_store(data_dir, &store);
        return store;
    }
    load_store(data_dir)
}

/// Persists atomically (tmp + rename) with 0600 permissions on Unix.
pub fn save_store(data_dir: &Path, store: &HighlightRuleStore) -> Result<(), String> {
    let path = store_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let value = json!({
        "version": STORAGE_VERSION,
        "rules": store.rules.iter().map(entry_json).collect::<Vec<_>>(),
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Failed to encode highlight rules: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)
        .map_err(|error| format!("Failed to write {}: {error}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}

fn entry_from_json(value: &Value) -> Option<HighlightRuleEntry> {
    let object = value.as_object()?;
    let string = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Some(HighlightRuleEntry {
        id: string("id"),
        pattern: string("pattern"),
        is_regex: object
            .get("isRegex")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        color: string("color"),
        case_sensitive: object
            .get("caseSensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        enabled: object
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        created_at: object.get("createdAt").and_then(Value::as_u64).unwrap_or(0),
        updated_at: object.get("updatedAt").and_then(Value::as_u64).unwrap_or(0),
    })
}

fn entry_json(entry: &HighlightRuleEntry) -> Value {
    json!({
        "id": entry.id,
        "pattern": entry.pattern,
        "isRegex": entry.is_regex,
        "color": entry.color,
        "caseSensitive": entry.case_sensitive,
        "enabled": entry.enabled,
        "createdAt": entry.created_at,
        "updatedAt": entry.updated_at,
    })
}

pub fn entry_view(entry: &HighlightRuleEntry) -> Value {
    entry_json(entry)
}

/// Rules in `createdAt` ascending order (stable, so equal timestamps keep
/// insertion order) — the contract order for `ssh/highlightRules/list`.
pub fn list_views(store: &HighlightRuleStore) -> Vec<Value> {
    let mut entries: Vec<&HighlightRuleEntry> = store.rules.iter().collect();
    entries.sort_by_key(|entry| entry.created_at);
    entries.into_iter().map(entry_view).collect()
}

/// `true` for `#rrggbb` in any ASCII hex case.
fn is_valid_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Creates or updates one rule from the shared camelCase parameter shape
/// (protocol `ssh/highlightRules/save`). A blank/missing `id` creates; a
/// non-empty `id` must reference an existing rule. The 30-rule cap only
/// constrains creation. Regex validity is the frontend's job (§1.1).
pub fn save_entry(
    store: &mut HighlightRuleStore,
    params: &Value,
) -> Result<(HighlightRuleEntry, bool), String> {
    let pattern = params
        .get("pattern")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("Missing pattern")?;
    if pattern.chars().count() > MAX_PATTERN_CHARS {
        return Err(format!(
            "Highlight pattern is limited to {MAX_PATTERN_CHARS} characters"
        ));
    }
    let is_regex = params
        .get("isRegex")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let color = match params.get("color") {
        Some(value) => {
            let color = value
                .as_str()
                .ok_or("color must be a #rrggbb hex string")?
                .trim()
                .to_string();
            if !is_valid_color(&color) {
                return Err(format!("color must match #rrggbb (hex); got '{color}'"));
            }
            color
        }
        None => DEFAULT_COLOR.to_string(),
    };
    let case_sensitive = params
        .get("caseSensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let enabled = params
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let existing_id = params
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let existing = existing_id.and_then(|id| store.rules.iter().position(|entry| entry.id == id));
    if existing_id.is_some() && existing.is_none() {
        return Err(
            "Highlight rule to update was not found; omit id to create a new rule".to_string(),
        );
    }
    if existing.is_none() && store.rules.len() >= MAX_RULES {
        return Err(format!("At most {MAX_RULES} highlight rules are supported"));
    }
    let now = unix_now_secs();
    let entry = match existing {
        Some(index) => {
            let entry = &mut store.rules[index];
            entry.pattern = pattern.to_string();
            entry.is_regex = is_regex;
            entry.color = color;
            entry.case_sensitive = case_sensitive;
            entry.enabled = enabled;
            entry.updated_at = now;
            entry.clone()
        }
        None => {
            let entry = HighlightRuleEntry {
                id: uuid::Uuid::new_v4().to_string(),
                pattern: pattern.to_string(),
                is_regex,
                color,
                case_sensitive,
                enabled,
                created_at: now,
                updated_at: now,
            };
            store.rules.push(entry.clone());
            entry
        }
    };
    Ok((entry, existing.is_none()))
}

/// Removes a rule by id; returns false when the id is unknown (idempotent,
/// mirroring `ssh/quickCommands/delete` semantics — the file is only
/// rewritten when something was actually removed).
pub fn delete_entry(store: &mut HighlightRuleStore, id: &str) -> bool {
    let before = store.rules.len();
    store.rules.retain(|entry| entry.id != id);
    store.rules.len() != before
}

#[cfg(test)]
mod tests;
