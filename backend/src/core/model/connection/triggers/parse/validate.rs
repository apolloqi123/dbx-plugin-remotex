//! Validation layer for the JSON shape of `external_config.triggers`: the
//! object form (host payload / parsed textarea JSON), per-stage answer rules
//! and the shared pattern gate. Pure string/JSON work only.

use super::*;

/// Shared validator for the object form (host object or parsed textarea
/// JSON). `enabled: false` keeps the stages stored but disables the engine.
pub(super) fn parse_object(
    object: &serde_json::Map<String, serde_json::Value>,
    secrets: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<TriggersConfig>, String> {
    match object.get("enabled") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::Bool(false)) => return Ok(None),
        Some(serde_json::Value::Bool(true)) => {}
        Some(other) => {
            return Err(format!(
                "triggers.enabled: expected a boolean, got {}",
                json_type_name(other)
            ))
        }
    }
    let stages = match object.get("stages") {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(serde_json::Value::Array(stages)) => stages,
        Some(other) => {
            return Err(format!(
                "triggers.stages: expected a JSON array, got {}",
                json_type_name(other)
            ))
        }
    };
    if stages.is_empty() {
        return Ok(None);
    }
    if stages.len() > MAX_STAGES {
        return Err(format!(
            "triggers.stages: at most {MAX_STAGES} stages are supported, got {}",
            stages.len()
        ));
    }
    let timeout_secs = match object.get("timeoutSecs") {
        None | Some(serde_json::Value::Null) => DEFAULT_TIMEOUT_SECS,
        Some(value) => value
            .as_u64()
            .filter(|value| (1..=MAX_TIMEOUT_SECS).contains(value))
            .ok_or_else(|| {
                format!("triggers.timeoutSecs: must be an integer between 1 and {MAX_TIMEOUT_SECS}")
            })?,
    };
    let sleep_ms = match object.get("sleepMs") {
        None | Some(serde_json::Value::Null) => DEFAULT_SLEEP_MS,
        Some(value) => value
            .as_u64()
            .filter(|value| *value <= MAX_SLEEP_MS)
            .ok_or_else(|| {
                format!("triggers.sleepMs: must be an integer between 0 and {MAX_SLEEP_MS}")
            })?,
    };
    let pass_sleep = match object.get("passSleep") {
        None | Some(serde_json::Value::Null) => PassSleep::None,
        Some(serde_json::Value::String(name)) => PassSleep::parse(name).ok_or_else(|| {
            format!("triggers.passSleep: must be none, each or enter, got '{name}'")
        })?,
        Some(other) => {
            return Err(format!(
                "triggers.passSleep: expected a string, got {}",
                json_type_name(other)
            ))
        }
    };
    let mut parsed = Vec::with_capacity(stages.len());
    for (index, stage) in stages.iter().enumerate() {
        let stage = stage.as_object().ok_or_else(|| {
            format!(
                "triggers.stages[{}]: expected a JSON object, got {}",
                index + 1,
                json_type_name(stage)
            )
        })?;
        parsed.push(parse_stage(stage, index, secrets)?);
    }
    Ok(Some(TriggersConfig {
        timeout_secs,
        sleep_ms,
        pass_sleep,
        stages: parsed,
    }))
}

pub(super) fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// Reads a required non-empty string field; `null` counts as absent.
pub(super) fn required_text<'a>(
    stage: &'a serde_json::Map<String, serde_json::Value>,
    field: &'a str,
) -> Result<Option<&'a str>, String> {
    match stage.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) if !text.is_empty() => Ok(Some(text)),
        Some(serde_json::Value::String(_)) => Err(format!("{field}: must not be empty")),
        Some(other) => Err(format!(
            "{field}: expected a string, got {}",
            json_type_name(other)
        )),
    }
}

pub(super) fn compile_pattern(text: &str, error_prefix: &str) -> Result<Regex, String> {
    if text.len() > MAX_PATTERN_LEN {
        return Err(format!(
            "{error_prefix}: pattern exceeds {MAX_PATTERN_LEN} characters"
        ));
    }
    Regex::new(text).map_err(|error| format!("{error_prefix}: {error}"))
}

/// Resolves a `sendSecretKey`/`caseSendSecretKey` reference against the
/// connection's secret slots (contract D5: exactly two known slots; an
/// unfilled slot is a configuration error, D7).
pub(super) fn resolve_secret(
    key: &str,
    error_prefix: &str,
    secrets: &dyn Fn(&str) -> Option<String>,
) -> Result<StageAnswer, String> {
    if !SECRET_SLOT_KEYS.contains(&key) {
        return Err(format!(
            "{error_prefix}: unknown secret slot '{key}' (expected trigger_answer_1 or trigger_answer_2)"
        ));
    }
    let value = secrets(key).unwrap_or_default();
    if value.is_empty() {
        return Err(format!(
            "{error_prefix}: secret slot '{key}' is empty; fill the trigger secret field on the connection or drop the sendSecretKey reference"
        ));
    }
    Ok(StageAnswer::Secret(value))
}

pub(super) fn parse_stage(
    stage: &serde_json::Map<String, serde_json::Value>,
    index: usize,
    secrets: &dyn Fn(&str) -> Option<String>,
) -> Result<TriggerStage, String> {
    let prefix = move |field: &str| format!("triggers.stages[{}].{field}", index + 1);
    let pattern_text = required_text(stage, "pattern")?
        .ok_or_else(|| prefix("pattern") + ": is required")?
        .to_string();
    let pattern = compile_pattern(&pattern_text, &prefix("pattern"))?;

    // Answer five-choose-one (contract §2.1).
    let send_text = required_text(stage, "sendText")?;
    let send_secret_key = required_text(stage, "sendSecretKey")?;
    let send_secret = required_text(stage, "sendSecret")?;
    let send_totp = required_text(stage, "sendTotp")?;
    let send_command = required_text(stage, "sendCommand")?;
    let answer_count = usize::from(send_text.is_some())
        + usize::from(send_secret_key.is_some())
        + usize::from(send_secret.is_some())
        + usize::from(send_totp.is_some())
        + usize::from(send_command.is_some());
    if answer_count == 0 {
        return Err(format!(
            "{}: exactly one of sendText, sendSecretKey, sendSecret, sendTotp or sendCommand is required",
            prefix("answer")
        ));
    }
    if answer_count > 1 {
        return Err(format!(
            "{}: sendText, sendSecretKey, sendSecret, sendTotp and sendCommand are mutually exclusive",
            prefix("answer")
        ));
    }
    let answer = if let Some(text) = send_text {
        if text.len() > MAX_SEND_TEXT_LEN {
            return Err(format!(
                "{}: answer exceeds {MAX_SEND_TEXT_LEN} characters",
                prefix("sendText")
            ));
        }
        StageAnswer::Text(text.to_string())
    } else if let Some(key) = send_secret_key {
        resolve_secret(key, &prefix("sendSecretKey"), secrets)?
    } else if let Some(secret) = send_secret {
        if secret.len() > MAX_SEND_TEXT_LEN {
            return Err(format!(
                "{}: answer exceeds {MAX_SEND_TEXT_LEN} characters",
                prefix("sendSecret")
            ));
        }
        StageAnswer::Secret(secret.to_string())
    } else if let Some(secret) = send_totp {
        let key = decode_base32_secret(secret)
            .map_err(|error| format!("{}: {error}", prefix("sendTotp")))?;
        if key.len() > MAX_TOTP_SECRET_BYTES {
            return Err(format!(
                "{}: TOTP secret exceeds {MAX_TOTP_SECRET_BYTES} bytes after base32 decoding",
                prefix("sendTotp")
            ));
        }
        StageAnswer::Totp(key)
    } else {
        let command = send_command.expect("answer_count == 1 guarantees one variant");
        if command.len() > MAX_COMMAND_LEN {
            return Err(format!(
                "{}: command exceeds {MAX_COMMAND_LEN} characters",
                prefix("sendCommand")
            ));
        }
        StageAnswer::Command(command.to_string())
    };

    // Optional case pre-match: casePattern + exactly one caseSend* (contract §2.1).
    let case_pattern_text = required_text(stage, "casePattern")?;
    let case_send_text = required_text(stage, "caseSendText")?;
    let case_send_secret_key = required_text(stage, "caseSendSecretKey")?;
    let case_send_secret = required_text(stage, "caseSendSecret")?;
    let case_answer_count = usize::from(case_send_text.is_some())
        + usize::from(case_send_secret_key.is_some())
        + usize::from(case_send_secret.is_some());
    if case_answer_count > 0 && case_pattern_text.is_none() {
        return Err(format!(
            "{}: caseSendText/caseSendSecretKey/caseSendSecret require casePattern",
            prefix("caseAnswer")
        ));
    }
    if case_pattern_text.is_some() && case_answer_count != 1 {
        return Err(format!(
            "{}: exactly one of caseSendText, caseSendSecretKey or caseSendSecret is required alongside casePattern",
            prefix("caseAnswer")
        ));
    }
    let case = match (
        case_pattern_text,
        case_send_text,
        case_send_secret_key,
        case_send_secret,
    ) {
        (Some(pattern), Some(text), None, None) => {
            if text.len() > MAX_SEND_TEXT_LEN {
                return Err(format!(
                    "{}: answer exceeds {MAX_SEND_TEXT_LEN} characters",
                    prefix("caseSendText")
                ));
            }
            Some(CaseRule {
                pattern: compile_pattern(pattern, &prefix("casePattern"))?,
                answer: StageAnswer::Text(text.to_string()),
            })
        }
        (Some(pattern), None, Some(key), None) => Some(CaseRule {
            pattern: compile_pattern(pattern, &prefix("casePattern"))?,
            answer: resolve_secret(key, &prefix("caseSendSecretKey"), secrets)?,
        }),
        (Some(pattern), None, None, Some(secret)) => {
            if secret.len() > MAX_SEND_TEXT_LEN {
                return Err(format!(
                    "{}: answer exceeds {MAX_SEND_TEXT_LEN} characters",
                    prefix("caseSendSecret")
                ));
            }
            Some(CaseRule {
                pattern: compile_pattern(pattern, &prefix("casePattern"))?,
                answer: StageAnswer::Secret(secret.to_string()),
            })
        }
        _ => None,
    };

    Ok(TriggerStage {
        pattern,
        answer,
        case,
    })
}

/// Shared pattern gate for consumers that build on the engine without going
/// through `parse_triggers` (the Telnet declarative auto-login): same length
/// ceiling, same regex dialect, bare error so the caller can prefix its own
/// field name. One compile path — no second validation vocabulary.
pub(crate) fn validate_pattern_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_PATTERN_LEN {
        return Err(format!("pattern exceeds {MAX_PATTERN_LEN} characters"));
    }
    Regex::new(text)
        .map(|_| ())
        .map_err(|error| error.to_string())
}
