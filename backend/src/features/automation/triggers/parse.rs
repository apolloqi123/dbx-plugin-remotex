//! Trigger parse layer.

use super::*;

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

/// True when the trimmed textarea text names at least one tssh `Expect*`
/// directive, so failed JSON parsing falls through to the tssh text parser
/// instead of a JSON error.
pub(super) fn looks_like_tssh_text(text: &str) -> bool {
    text.lines().any(|line| {
        let line = strip_tssh_comment_prefix(line).trim_start();
        match line.split_whitespace().next() {
            Some(token) => {
                let token = token.trim_matches('"').to_ascii_lowercase();
                token.starts_with("expect")
            }
            None => false,
        }
    })
}

/// Drops an optional tssh comment prefix: `#!!` (ssh-config embedding),
/// `#!`, or a plain `#`.
pub(super) fn strip_tssh_comment_prefix(line: &str) -> &str {
    let trimmed = line.trim_start();
    for prefix in ["#!!", "#!", "#"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest;
        }
    }
    trimmed
}

/// Splits `Directive value…` at the first whitespace run; the value keeps its
/// inner spacing (patterns and answers may contain spaces) and loses one
/// optional pair of surrounding double quotes.
pub(super) fn split_tssh_directive(line: &str) -> Option<(String, &str)> {
    let mut parts = line.splitn(2, char::is_whitespace);
    let directive = parts.next()?.trim().to_ascii_lowercase();
    if directive.is_empty() {
        return None;
    }
    let value = parts.next().unwrap_or("").trim();
    let value = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value);
    Some((directive, value))
}

/// Splits a directive name into its alphabetic head and 1-based stage index
/// (`expectpattern3` → `("expectpattern", 3)`).
pub(super) fn split_tssh_index(directive: &str) -> (&str, Option<usize>) {
    match directive
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_ascii_digit())
    {
        Some((boundary, _)) if boundary + 1 < directive.len() => (
            &directive[..=boundary],
            directive[boundary + 1..].parse::<usize>().ok(),
        ),
        _ => (directive, None),
    }
}

pub(super) fn tssh_error(line_no: usize, message: &str) -> String {
    format!("triggers: tssh text input, line {line_no}: {message}")
}

pub(super) fn tssh_pattern_value(value: &str, line_no: usize) -> Result<String, String> {
    if value.len() > MAX_PATTERN_LEN {
        return Err(tssh_error(
            line_no,
            &format!("pattern exceeds {MAX_PATTERN_LEN} characters"),
        ));
    }
    // The JSON form stays strict (D7), but pasted tssh rules were written for
    // tssh's matcher: a value that is not a valid regex falls back to a
    // literal substring match (tssh's own README example `*assword` is not a
    // valid regex either).
    if Regex::new(value).is_ok() {
        Ok(value.to_string())
    } else {
        Ok(regex::escape(value))
    }
}

/// Parses the tssh (trzsz-ssh) text form: `ExpectCount` / `ExpectTimeout` /
/// `ExpectSleepMS` / `ExpectPassSleep` globals plus per-stage
/// `ExpectPatternN` / `ExpectSendTextN` / `ExpectSendOtpN` and
/// `ExpectCaseSendTextN <pattern> <text>`. `ExpectCount 0` explicitly
/// disables the engine. The tssh ciphertext/TOTP answer directives are fully
/// supported (0.4.77): `ExpectSendPassN` / `ExpectCaseSendPassN` /
/// `ExpectSendEncTotpN` / `ExpectSendEncOtpN` decrypt `--enc-secret` blobs
/// exactly like tssh (`decode_tssh_enc_secret`), and `ExpectSendTotpN`
/// generates RFC 6238 codes — both mapping onto the plugin's
/// `sendSecret` / `sendTotp` / `sendCommand` answers.
pub(super) fn parse_tssh_text(
    text: &str,
    secrets: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<TriggersConfig>, String> {
    let mut expect_count: Option<u64> = None;
    let mut timeout_secs: Option<u64> = None;
    let mut sleep_ms: Option<u64> = None;
    let mut pass_sleep: Option<PassSleep> = None;
    let mut stages: std::collections::BTreeMap<usize, serde_json::Map<String, serde_json::Value>> =
        std::collections::BTreeMap::new();

    // Inserts one answer field after enforcing the one-answer-per-stage rule
    // (tssh resolves duplicates by directive priority; a paste carrying two
    // answers for the same stage is pathological, so reject it).
    let insert_answer = |stages: &mut std::collections::BTreeMap<
        usize,
        serde_json::Map<String, serde_json::Value>,
    >,
                         number: usize,
                         field: &str,
                         value: String,
                         line_no: usize|
     -> Result<(), String> {
        let stage = stages.entry(number).or_default();
        if [
            "sendText",
            "sendSecretKey",
            "sendSecret",
            "sendTotp",
            "sendCommand",
        ]
        .iter()
        .any(|key| stage.contains_key(*key))
        {
            return Err(tssh_error(line_no, "duplicate stage answer"));
        }
        stage.insert(field.to_string(), serde_json::Value::String(value));
        Ok(())
    };

    for (offset, raw_line) in text.lines().enumerate() {
        let line_no = offset + 1;
        let line = strip_tssh_comment_prefix(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let (directive, value) = split_tssh_directive(line)
            .ok_or_else(|| tssh_error(line_no, "expected 'Directive value'"))?;
        let (name, index) = split_tssh_index(&directive);
        let stage_number = || -> Result<usize, String> {
            index
                .filter(|index| (1..=MAX_STAGES).contains(index))
                .ok_or_else(|| {
                    tssh_error(
                        line_no,
                        &format!("{name} requires a stage number between 1 and {MAX_STAGES}"),
                    )
                })
        };
        match name {
            "expectcount" => {
                expect_count = Some(value.parse::<u64>().map_err(|_| {
                    tssh_error(
                        line_no,
                        &format!("ExpectCount must be an integer, got '{value}'"),
                    )
                })?);
            }
            "expecttimeout" => {
                let seconds = value.parse::<u64>().map_err(|_| {
                    tssh_error(
                        line_no,
                        &format!("ExpectTimeout must be an integer, got '{value}'"),
                    )
                })?;
                if !(1..=MAX_TIMEOUT_SECS).contains(&seconds) {
                    return Err(tssh_error(
                        line_no,
                        &format!("ExpectTimeout must be between 1 and {MAX_TIMEOUT_SECS}"),
                    ));
                }
                timeout_secs = Some(seconds);
            }
            "expectsleepms" => {
                let millis = value.parse::<u64>().map_err(|_| {
                    tssh_error(
                        line_no,
                        &format!("ExpectSleepMS must be an integer, got '{value}'"),
                    )
                })?;
                if millis > MAX_SLEEP_MS {
                    return Err(tssh_error(
                        line_no,
                        &format!("ExpectSleepMS must be between 0 and {MAX_SLEEP_MS}"),
                    ));
                }
                sleep_ms = Some(millis);
            }
            "expectpasssleep" => {
                pass_sleep = Some(PassSleep::parse(&value.to_ascii_lowercase()).ok_or_else(
                    || tssh_error(line_no, "ExpectPassSleep must be no, none, each or enter"),
                )?);
            }
            "expectpattern" => {
                let number = stage_number()?;
                stages.entry(number).or_default().insert(
                    "pattern".to_string(),
                    serde_json::Value::String(tssh_pattern_value(value, line_no)?),
                );
            }
            // tssh ExpectSendOtpN runs a local command and sends its stdout —
            // exactly the plugin's sendCommand.
            "expectsendtext" | "expectsendotp" => {
                let field = if name == "expectsendtext" {
                    "sendText"
                } else {
                    "sendCommand"
                };
                let number = stage_number()?;
                insert_answer(&mut stages, number, field, value.to_string(), line_no)?;
            }
            // tssh ExpectSendPassN: --enc-secret blob, decrypted with tssh's
            // own fixed key and sent as a secret (Enter appended).
            "expectsendpass" => {
                let number = stage_number()?;
                let decoded = decode_tssh_enc_secret(value, &format!("ExpectSendPass{number}"))?;
                insert_answer(&mut stages, number, "sendSecret", decoded, line_no)?;
            }
            // tssh ExpectSendTotpN: plaintext base32 secret; the code is
            // generated at match time.
            "expectsendtotp" => {
                let number = stage_number()?;
                decode_base32_secret(value).map_err(|error| tssh_error(line_no, &error))?;
                insert_answer(&mut stages, number, "sendTotp", value.to_string(), line_no)?;
            }
            // tssh ExpectSendEncTotpN: --enc-secret blob whose plaintext is
            // the base32 TOTP secret.
            "expectsendenctotp" => {
                let number = stage_number()?;
                let decoded = decode_tssh_enc_secret(value, &format!("ExpectSendEncTotp{number}"))?;
                decode_base32_secret(&decoded).map_err(|error| tssh_error(line_no, &error))?;
                insert_answer(&mut stages, number, "sendTotp", decoded, line_no)?;
            }
            // tssh ExpectSendEncOtpN: --enc-secret blob whose plaintext is a
            // local command producing the dynamic password.
            "expectsendencotp" => {
                let number = stage_number()?;
                let decoded = decode_tssh_enc_secret(value, &format!("ExpectSendEncOtp{number}"))?;
                insert_answer(&mut stages, number, "sendCommand", decoded, line_no)?;
            }
            "expectcasesendtext" => {
                let number = stage_number()?;
                let (case_pattern, case_text) =
                    value.split_once(char::is_whitespace).ok_or_else(|| {
                        tssh_error(line_no, "ExpectCaseSendText requires '<pattern> <answer>'")
                    })?;
                let case_pattern = case_pattern.trim();
                let stage = stages.entry(number).or_default();
                if stage.contains_key("casePattern") {
                    return Err(tssh_error(line_no, "duplicate case rule"));
                }
                stage.insert(
                    "casePattern".to_string(),
                    serde_json::Value::String(tssh_pattern_value(case_pattern.trim(), line_no)?),
                );
                stage.insert(
                    "caseSendText".to_string(),
                    serde_json::Value::String(case_text.trim().to_string()),
                );
            }
            // tssh ExpectCaseSendPassN: case pre-match answering a decrypted
            // --enc-secret secret.
            "expectcasesendpass" => {
                let number = stage_number()?;
                let (case_pattern, case_value) =
                    value.split_once(char::is_whitespace).ok_or_else(|| {
                        tssh_error(line_no, "ExpectCaseSendPass requires '<pattern> <secret>'")
                    })?;
                let case_pattern = case_pattern.trim();
                let decoded = decode_tssh_enc_secret(
                    case_value.trim(),
                    &format!("ExpectCaseSendPass{number}"),
                )?;
                let stage = stages.entry(number).or_default();
                if stage.contains_key("casePattern") {
                    return Err(tssh_error(line_no, "duplicate case rule"));
                }
                stage.insert(
                    "casePattern".to_string(),
                    serde_json::Value::String(tssh_pattern_value(case_pattern, line_no)?),
                );
                stage.insert(
                    "caseSendSecret".to_string(),
                    serde_json::Value::String(decoded),
                );
            }
            _ if name.starts_with("expectcasesend") || name.starts_with("expectsend") => {
                return Err(tssh_error(
                    line_no,
                    &format!(
                        "'{directive}' is not a supported tssh directive; supported forms: ExpectSendTextN, ExpectSendPassN, ExpectSendTotpN, ExpectSendEncTotpN, ExpectSendOtpN, ExpectSendEncOtpN, ExpectCaseSendTextN, ExpectCaseSendPassN"
                    ),
                ));
            }
            _ => {
                return Err(tssh_error(
                    line_no,
                    &format!("unknown directive '{directive}'"),
                ));
            }
        }
    }

    // tssh gates the whole feature on ExpectCount; an explicit 0 means off.
    // When the count is absent, pasted stages are taken at face value instead
    // of silently doing nothing (tssh's default 0 would be a paste foot-gun).
    let count = expect_count.unwrap_or(stages.len() as u64);
    if count == 0 {
        return Ok(None);
    }
    if stages.is_empty() {
        return Ok(None);
    }
    let mut object = serde_json::Map::new();
    if let Some(seconds) = timeout_secs {
        object.insert("timeoutSecs".to_string(), serde_json::Value::from(seconds));
    }
    if let Some(millis) = sleep_ms {
        object.insert("sleepMs".to_string(), serde_json::Value::from(millis));
    }
    if let Some(pacing) = pass_sleep {
        object.insert(
            "passSleep".to_string(),
            serde_json::Value::String(pacing.name().to_string()),
        );
    }
    let mut stage_list = Vec::new();
    for (number, stage) in stages {
        if (number as u64) > count {
            break;
        }
        stage_list.push(serde_json::Value::Object(stage));
    }
    object.insert("stages".to_string(), serde_json::Value::Array(stage_list));
    parse_object(&object, secrets)
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
