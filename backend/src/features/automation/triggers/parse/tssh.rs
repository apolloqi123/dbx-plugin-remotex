//! tssh (trzsz-ssh) text-form parsing: directive tokenizing plus the
//! `ExpectCount` / `Expect*` line grammar, normalized into the same JSON
//! object shape the validator consumes.

use super::*;

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
