//! Telnet layer.

use super::*;

/// Resolves the auto-login spec into a validated [`AutoLoginPlan`]. The rule
/// form maps its secrets closure onto the protocol's fixed slot names
/// (`trigger_answer_N` = `secrets[N-1]`) so the shared `parse_triggers`
/// validation (unknown slot, empty slot) applies unchanged; the declarative
/// form is lowered in [`parse_declarative_auto_login`]. The two forms are
/// mutually exclusive — sending both is a configuration error (D7).
pub fn parse_auto_login(spec: &AutoLoginSpec) -> Result<Option<AutoLoginPlan>, String> {
    if spec.rules.is_some() && spec.declarative.is_some() {
        return Err(
            "telnet/start: autoLogin.rules and autoLogin.declarative are mutually exclusive"
                .to_string(),
        );
    }
    if let Some(declarative) = &spec.declarative {
        return parse_declarative_auto_login(declarative);
    }
    let slots = spec.secrets.clone();
    Ok(
        triggers::parse_triggers(spec.rules.as_ref(), &move |key| match key {
            "trigger_answer_1" => slots.first().filter(|value| !value.is_empty()).cloned(),
            "trigger_answer_2" => slots.get(1).filter(|value| !value.is_empty()).cloned(),
            _ => None,
        })?
        .map(|triggers| AutoLoginPlan {
            triggers,
            watch: None,
        }),
    )
}

/// Lowers the NyaTerm-style declarative form onto the shared expect engine:
/// username → plaintext stage, password → `trigger_answer_1` secret stage
/// (redacted in `Debug`, never logged, its value only ever lives in the send
/// plan). Prompt regexes compile through the shared triggers gate so invalid
/// patterns fail the start with a readable error naming the dialog field;
/// success/failure regexes build the [`DeclarativeWatch`].
pub(crate) fn parse_declarative_auto_login(
    declarative: &DeclarativeAutoLogin,
) -> Result<Option<AutoLoginPlan>, String> {
    let username = declarative.username.as_deref().unwrap_or("").trim();
    let password = declarative.password.as_deref().unwrap_or("");
    if username.is_empty() && password.is_empty() {
        return Err(
            "telnet/start: autoLogin.declarative needs at least a username or a password to send"
                .to_string(),
        );
    }
    let max_retries = declarative.max_retries.unwrap_or(0);
    if max_retries > DECL_MAX_RETRIES {
        return Err(format!(
            "telnet/start: autoLogin.maxRetries must be between 0 and {DECL_MAX_RETRIES}, got {max_retries}"
        ));
    }
    let username_pattern = decl_pattern_text(
        declarative.username_prompt_regex.as_deref(),
        "usernamePromptRegex",
        DECL_USERNAME_PROMPT_DEFAULT,
    )?;
    let password_pattern = decl_pattern_text(
        declarative.password_prompt_regex.as_deref(),
        "passwordPromptRegex",
        DECL_PASSWORD_PROMPT_DEFAULT,
    )?;

    let mut stages = Vec::new();
    if !username.is_empty() {
        stages.push(json!({
            "pattern": username_pattern,
            "sendText": format!("{username}\r"),
        }));
    }
    if !password.is_empty() {
        stages.push(json!({
            "pattern": password_pattern,
            "sendSecretKey": "trigger_answer_1",
        }));
    }
    // Username or password is non-empty above, so the stage list can never
    // be empty and `parse_triggers` cannot answer `Ok(None)` here.
    let password_slot = password.to_string();
    let triggers_config = triggers::parse_triggers(Some(&json!({ "stages": stages })), &|key| {
        match key {
            "trigger_answer_1" if !password_slot.is_empty() => Some(password_slot.clone()),
            _ => None,
        }
    })?
    .ok_or_else(|| "telnet/start: autoLogin.declarative lowered to no login stages".to_string())?;

    let success = decl_regex(
        declarative.success_regex.as_deref(),
        "successRegex",
        DECL_SUCCESS_DEFAULT,
    )?;
    let failure = decl_regex(
        declarative.failure_regex.as_deref(),
        "failureRegex",
        DECL_FAILURE_DEFAULT,
    )?;

    Ok(Some(AutoLoginPlan {
        triggers: triggers_config,
        watch: Some(DeclarativeWatch::new(success, failure, max_retries)),
    }))
}

/// Resolves one optional declarative pattern field to its effective text:
/// blank/absent falls back to the built-in default, a custom value passes the
/// shared [`triggers::validate_pattern_text`] gate (length ceiling + regex
/// dialect) so an invalid pattern fails with the field name prefixed.
pub(crate) fn decl_pattern_text(
    value: Option<&str>,
    field: &str,
    default: &str,
) -> Result<String, String> {
    match value.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => triggers::validate_pattern_text(text)
            .map(|_| text.to_string())
            .map_err(|error| format!("telnet/start: autoLogin.{field}: {error}")),
        None => Ok(default.to_string()),
    }
}

/// Compiles one optional watch regex (success/failure): same blank-falls-back
/// to the NyaTerm default rule as [`decl_pattern_text`].
pub(crate) fn decl_regex(value: Option<&str>, field: &str, default: &str) -> Result<Regex, String> {
    let custom = match value.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => {
            triggers::validate_pattern_text(text)
                .map_err(|error| format!("telnet/start: autoLogin.{field}: {error}"))?;
            Some(text)
        }
        None => None,
    };
    let pattern = custom.map_or_else(|| default.to_string(), str::to_string);
    Regex::new(&pattern).map_err(|error| format!("telnet/start: autoLogin.{field}: {error}"))
}
