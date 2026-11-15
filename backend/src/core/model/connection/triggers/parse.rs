//! Trigger parse layer.

use super::*;

mod tssh;
mod validate;

use tssh::*;
pub(crate) use validate::validate_pattern_text;
use validate::*;

/// Parses `external_config.triggers` into a validated [`TriggersConfig`].
/// Accepts three input shapes:
/// - the raw JSON object (host lifecycle payloads, smoke tests);
/// - the JSON-string form (connection-form textarea), optionally carrying a
///   top-level `"enabled": false` to switch the rules off without deleting
///   them;
/// - tssh (trzsz-ssh) text-form rules (`ExpectCount` / `ExpectPatternN` /
///   `ExpectSendTextN` …, with or without the `#!!` ssh-config comment
///   prefix) — detected whenever the text is not valid JSON but mentions an
///   `Expect*` directive (tssh 自动交互输入兼容，0.4.77).
///
/// Missing/empty/`stages: []`/`enabled: false`/`ExpectCount 0` mean the
/// feature is off (`Ok(None)`); everything invalid is a hard error
/// (contract D7).
pub fn parse_triggers(
    raw: Option<&serde_json::Value>,
    secrets: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<TriggersConfig>, String> {
    let Some(value) = raw else {
        return Ok(None);
    };
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                return Ok(None);
            }
            match serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text) {
                Ok(object) => parse_object(&object, secrets),
                Err(error) => {
                    // A mangled JSON document must surface as a JSON error;
                    // only non-JSON text that names an Expect* directive is
                    // reinterpreted as the tssh text form.
                    if looks_like_tssh_text(text) {
                        parse_tssh_text(text, secrets)
                    } else {
                        Err(format!("triggers: invalid JSON: {error}"))
                    }
                }
            }
        }
        serde_json::Value::Object(object) => parse_object(object, secrets),
        other => Err(format!(
            "triggers: expected a JSON object or JSON string, got {}",
            json_type_name(other)
        )),
    }
}
