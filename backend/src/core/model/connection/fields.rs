//! Connection string and numeric field helpers.

use super::*;

/// First present string among `keys` in the config object.
pub(in crate::core::model) fn config_text<'a>(
    config: Option<&'a serde_json::Map<String, Value>>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| config.and_then(|config| config.get(*key)))
        .and_then(Value::as_str)
}

/// Terminal activity keepalive: 0 disables, anything else is bounded so a
/// typo can neither hammer the PTY (5s floor) nor idle for days (1h ceiling).
pub(in crate::core::model) fn clamp_terminal_keepalive(raw: u64) -> u64 {
    if raw == 0 {
        0
    } else {
        raw.clamp(5, 3600)
    }
}

/// Numeric `binding: config` field: the connection form writes these into
/// `external_config`; the top-level connection object stays as fallback for
/// hand-edited configs and older payloads (same convergence as `read_only`,
/// which the timeout/keepalive fields previously lacked — the form values
/// never reached the parser and the defaults always won).
pub(in crate::core::model) fn config_u64(
    external_config: Option<&serde_json::Map<String, Value>>,
    connection: &serde_json::Map<String, Value>,
    key: &str,
) -> Option<u64> {
    external_config
        .and_then(|config| config.get(key))
        .or_else(|| connection.get(key))
        .and_then(Value::as_u64)
}

/// Parses the `setEnv` connection field: one `KEY=VALUE` entry per line,
/// semicolons tolerated as separators (mirrors the `totp_secret` input
/// convention), blank entries ignored, entries trimmed. Strict by design —
/// "prefer failing the connection to silently misconfiguring remote
/// commands": every invalid entry is reported with its content in one
/// aggregated error. Duplicate keys follow `ssh SetEnv` semantics: the last
/// entry wins. This is the client-specified half of ssh's SetEnv/SendEnv —
/// no local environment is ever transmitted.
pub fn parse_set_env(raw: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut vars: Vec<(String, String)> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for entry in raw.split(['\n', ';']) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some((key, value)) = entry.split_once('=') else {
            errors.push(format!("'{entry}' (missing '=' separator)"));
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() {
            errors.push(format!("'{entry}' (empty key)"));
            continue;
        }
        if key.chars().any(char::is_whitespace) {
            errors.push(format!("'{entry}' (key must not contain whitespace)"));
            continue;
        }
        if key.contains('\0') {
            errors.push(format!("'{entry}' (NUL byte in key)"));
            continue;
        }
        if value.contains('\0') {
            errors.push(format!("'{entry}' (NUL byte in value)"));
            continue;
        }
        match vars.iter_mut().find(|(existing, _)| existing == key) {
            Some(slot) => slot.1 = value.to_string(),
            None => vars.push((key.to_string(), value.to_string())),
        }
    }
    if errors.is_empty() {
        Ok(vars)
    } else {
        Err(format!("Invalid setEnv entries: {}", errors.join("; ")))
    }
}

/// Host fields must be a dialable hostname/IP: no internal whitespace and no
/// URI scheme syntax (`ssh://…`) that would silently become a bogus TCP
/// target instead of failing fast at parse time.
pub(in crate::core::model) fn validate_host_field(host: String) -> Result<String, String> {
    if host.chars().any(char::is_whitespace) {
        return Err(format!(
            "Invalid SSH host '{host}': must not contain whitespace"
        ));
    }
    if host.contains("://") {
        return Err(format!(
            "Invalid SSH host '{host}': use a bare hostname, not a URI"
        ));
    }
    Ok(host)
}

/// Reads a credential field verbatim (no trimming): passwords and
/// passphrases may legitimately start or end with spaces.
pub(in crate::core::model) fn credential_string(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub(in crate::core::model) fn optional_string(
    object: Option<&serde_json::Map<String, Value>>,
    key: &str,
) -> String {
    object
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub(in crate::core::model) fn string_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("Missing SSH {key}"))
}
