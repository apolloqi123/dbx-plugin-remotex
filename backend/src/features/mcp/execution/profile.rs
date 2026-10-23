//! MCP execution helpers.

use super::*;

/// Resolves the optional `quickSudoProfile` argument (id or exact name)
/// before any connection I/O, so unknown references fail fast instead of
/// dialing first.
pub(crate) fn resolve_profile_reference(
    store: &sudo_profiles::SudoProfileStore,
    arguments: &Value,
) -> Result<Option<sudo_profiles::SudoProfile>, String> {
    let Some(reference) = arguments
        .get("quickSudoProfile")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    sudo_profiles::find_by_ref(store, reference)
        .cloned()
        .map(Some)
        .ok_or_else(|| format!("Quick Sudo profile '{reference}' not found"))
}

/// Fills auth gaps from a resolved global Quick Sudo profile: fields the
/// caller set explicitly always win; unset fields fall back to the profile.
pub(crate) fn apply_profile_fallbacks(auth: &mut SudoAuth, profile: &sudo_profiles::SudoProfile) {
    if auth.password.is_empty() && !profile.sudo_password.trim().is_empty() {
        auth.password = profile.sudo_password.trim().to_string();
    }
    if auth.totp_secrets.is_empty() {
        auth.totp_secrets = exec::parse_totp_secrets(&profile.totp_secret);
    }
    if auth.password_prompt_hint.is_empty() {
        auth.password_prompt_hint = exec::sanitize_prompt_hint(&profile.password_prompt_hint);
    }
    if auth.totp_prompt_hint.is_empty() {
        auth.totp_prompt_hint = exec::sanitize_prompt_hint(&profile.totp_prompt_hint);
    }
    if auth.flow_mode.is_none() {
        auth.flow_mode = Some(AuthFlowMode::parse(&profile.auth_flow_mode));
    }
}

/// Re-applies the caller's explicit per-call arguments on top of the
/// source-resolved base auth: values the caller actually sent always win,
/// whatever the connection's declared sudo source contributed.
pub(crate) fn apply_explicit_argument_overrides(auth: &mut SudoAuth, arguments: &Value) {
    let explicit = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    if let Some(password) = explicit("sudoPassword") {
        auth.password = password.to_string();
    }
    if let Some(secret) = explicit("totpSecret") {
        auth.totp_secrets = exec::parse_totp_secrets(secret);
    }
    if let Some(hint) = explicit("passwordPromptHint") {
        auth.password_prompt_hint = exec::sanitize_prompt_hint(hint);
    }
    if let Some(hint) = explicit("totpPromptHint") {
        auth.totp_prompt_hint = exec::sanitize_prompt_hint(hint);
    }
    if let Some(mode) = explicit("authFlowMode") {
        auth.flow_mode = Some(AuthFlowMode::parse(mode));
    }
}

pub(crate) fn sudo_auth(arguments: &Value) -> SudoAuth {
    let mut auth = SudoAuth::new(
        arguments
            .get("sudoPassword")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        arguments
            .get("password")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        arguments
            .get("totpSecret")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        Hints {
            password: arguments
                .get("passwordPromptHint")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            totp: arguments
                .get("totpPromptHint")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            flow_mode: arguments
                .get("authFlowMode")
                .and_then(Value::as_str)
                .map(AuthFlowMode::parse),
        },
    );
    auth.otp_ledger_scope = exec::otp_ledger_scope_for(
        arguments
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        arguments
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        arg_port_lossy(arguments),
    );
    auth
}
