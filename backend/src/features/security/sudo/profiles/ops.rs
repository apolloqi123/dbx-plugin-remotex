//! Quick Sudo profiles: ops layer.

use super::*;

use crate::core::time::unix_now_secs;

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Quick Sudo profile name is required".to_string());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!(
            "Quick Sudo profile name is limited to {MAX_NAME_LEN} characters"
        ));
    }
    Ok(name.to_string())
}

/// Creates or updates one profile from the shared camelCase parameter shape
/// (protocol `sudo/profiles/save` and the MCP save tool). Empty
/// `sudoPassword`/`totpSecret` keeps the stored value; explicit
/// `clearSudoPassword`/`clearTotpSecret` booleans wipe them. Returns the
/// saved profile plus whether it was newly created.
pub fn save_profile(
    store: &mut SudoProfileStore,
    params: &Value,
) -> Result<(SudoProfile, bool), String> {
    let name = validate_name(
        params
            .get("name")
            .and_then(Value::as_str)
            .ok_or("Missing name")?,
    )?;
    let existing_id = params
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let existing =
        existing_id.and_then(|id| store.profiles.iter().position(|profile| profile.id == id));
    // Case-insensitive uniqueness so the name stays an unambiguous MCP reference.
    if let Some(duplicate) = store.profiles.iter().find(|profile| {
        profile.name.eq_ignore_ascii_case(&name)
            && existing
                .map(|index| store.profiles[index].id != profile.id)
                .unwrap_or(true)
    }) {
        return Err(format!(
            "Quick Sudo profile name '{}' is already in use",
            duplicate.name
        ));
    }

    let string_input = |key: &str| -> Option<String> {
        params
            .get(key)
            .and_then(Value::as_str)
            .map(|value| value.to_string())
    };
    let flow_mode = match string_input("authFlowMode") {
        None => None,
        Some(value) if value.trim().is_empty() => None,
        Some(value) => {
            let trimmed = value.trim();
            if !matches!(
                trimmed,
                "off" | "password_only" | "password_plus_otp" | "password_then_otp"
            ) {
                return Err(format!("Unsupported authFlowMode '{trimmed}'"));
            }
            Some(trimmed.to_string())
        }
    };

    let created = existing.is_none();
    if created && store.profiles.len() >= MAX_PROFILES {
        return Err(format!(
            "At most {MAX_PROFILES} Quick Sudo profiles are supported"
        ));
    }
    let now = unix_now_secs();
    let mut profile = match existing {
        Some(index) => store.profiles[index].clone(),
        None => SudoProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.clone(),
            sudo_password: String::new(),
            totp_secret: String::new(),
            auth_flow_mode: AuthFlowMode::PasswordThenOtp.name().to_string(),
            password_prompt_hint: String::new(),
            totp_prompt_hint: String::new(),
            sudo_use_pty: false,
            created_at: now,
            updated_at: now,
        },
    };
    profile.name = name;
    if let Some(value) = string_input("sudoPassword") {
        if !value.is_empty() {
            profile.sudo_password = value;
        }
    }
    if params
        .get("clearSudoPassword")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        profile.sudo_password = String::new();
    }
    if let Some(value) = string_input("totpSecret") {
        if !value.trim().is_empty() {
            profile.totp_secret = value;
        }
    }
    if params
        .get("clearTotpSecret")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        profile.totp_secret = String::new();
    }
    if let Some(value) = flow_mode {
        profile.auth_flow_mode = value;
    }
    if let Some(value) = string_input("passwordPromptHint") {
        profile.password_prompt_hint = exec::sanitize_prompt_hint(&value);
    }
    if let Some(value) = string_input("totpPromptHint") {
        profile.totp_prompt_hint = exec::sanitize_prompt_hint(&value);
    }
    if let Some(value) = params.get("sudoUsePty").and_then(Value::as_bool) {
        profile.sudo_use_pty = value;
    }
    profile.updated_at = now;

    match existing {
        Some(index) => store.profiles[index] = profile.clone(),
        None => store.profiles.push(profile.clone()),
    }
    Ok((profile, created))
}

/// Removes a profile and every binding pointing at it. Returns false when
/// the id is unknown (mirrors `ssh/knownHosts/remove` semantics).
pub fn delete_profile(store: &mut SudoProfileStore, id: &str) -> bool {
    let before = store.profiles.len();
    store.profiles.retain(|profile| profile.id != id);
    let removed = store.profiles.len() != before;
    if removed {
        store.bindings.retain(|_, profile_id| profile_id != id);
    }
    removed
}

/// Resolves an id-or-exact-name reference (MCP ergonomics).
pub fn find_by_ref<'a>(store: &'a SudoProfileStore, reference: &str) -> Option<&'a SudoProfile> {
    store
        .profiles
        .iter()
        .find(|profile| profile.id == reference)
        .or_else(|| {
            store
                .profiles
                .iter()
                .find(|profile| profile.name == reference)
        })
}

/// The profile bound to a connection, if any. Dangling bindings fall back to
/// "no binding" instead of breaking the connection.
pub fn bound_profile<'a>(
    store: &'a SudoProfileStore,
    connection_id: &str,
) -> Option<&'a SudoProfile> {
    let profile_id = store.bindings.get(connection_id)?;
    store
        .profiles
        .iter()
        .find(|profile| profile.id.as_str() == profile_id)
}

/// Persists a connection binding. `None` clears it; `Some` must reference an
/// existing profile.
pub fn set_binding(
    store: &mut SudoProfileStore,
    connection_id: &str,
    profile_id: Option<&str>,
) -> Result<(), String> {
    match profile_id {
        None => {
            store.bindings.remove(connection_id);
        }
        Some(profile_id) => {
            if !store
                .profiles
                .iter()
                .any(|profile| profile.id == profile_id)
            {
                return Err(format!("Quick Sudo profile '{profile_id}' not found"));
            }
            store
                .bindings
                .insert(connection_id.to_string(), profile_id.to_string());
        }
    }
    Ok(())
}

/// Overlays a bound profile onto resolved connection auth: the profile owns
/// the whole credential source, so an empty profile password falls back to
/// the login password (never to the connection's own sudo secret). Field
/// assignment keeps the auth's OTP usage bookkeeping intact.
pub fn apply_profile(auth: &mut SudoAuth, profile: &SudoProfile, login_password: &str) {
    let profile_password = profile.sudo_password.trim();
    auth.password = if profile_password.is_empty() {
        login_password.to_string()
    } else {
        profile_password.to_string()
    };
    auth.totp_secrets = exec::parse_totp_secrets(&profile.totp_secret);
    auth.password_prompt_hint = exec::sanitize_prompt_hint(&profile.password_prompt_hint);
    auth.totp_prompt_hint = exec::sanitize_prompt_hint(&profile.totp_prompt_hint);
    auth.flow_mode = Some(AuthFlowMode::parse(&profile.auth_flow_mode));
}

/// A bound profile's PTY preference replaces the connection's.
pub fn effective_use_pty(connection_pty: bool, profile: Option<&SudoProfile>) -> bool {
    profile
        .map(|profile| profile.sudo_use_pty)
        .unwrap_or(connection_pty)
}
