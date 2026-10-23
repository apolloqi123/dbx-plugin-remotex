//! Quick Sudo profiles: json layer.

use super::*;

pub(super) fn profile_from_json(value: &Value, vault: &Vault) -> Option<SudoProfile> {
    let object = value.as_object()?;
    let string = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let id = string("id");
    // v2 stores `sudoPasswordEnc`/`totpSecretEnc` envelopes; a legacy v1
    // file carries the plaintext keys under the old names. An empty envelope
    // falls back to the plaintext key so v1 reads flow through one path.
    // Open failures degrade to "" (D4): the profile structure survives.
    let secret = |field: &str, encrypted_key: &str, plain_key: &str| {
        let envelope = string(encrypted_key);
        if envelope.is_empty() {
            string(plain_key)
        } else {
            vault.open(field, &id, &envelope)
        }
    };
    Some(SudoProfile {
        id: id.clone(),
        name: string("name"),
        sudo_password: secret(FIELD_SUDO_PASSWORD, "sudoPasswordEnc", "sudoPassword"),
        totp_secret: secret(FIELD_TOTP_SECRET, "totpSecretEnc", "totpSecret"),
        auth_flow_mode: string("authFlowMode"),
        password_prompt_hint: string("passwordPromptHint"),
        totp_prompt_hint: string("totpPromptHint"),
        sudo_use_pty: object
            .get("sudoUsePty")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        created_at: object.get("createdAt").and_then(Value::as_u64).unwrap_or(0),
        updated_at: object.get("updatedAt").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// Storage shape (v2): secrets are sealed envelopes under the `…Enc` keys,
/// empty secrets stay empty strings, metadata stays plaintext.
pub(super) fn profile_json(profile: &SudoProfile, vault: &Vault) -> Value {
    json!({
        "id": profile.id,
        "name": profile.name,
        "sudoPasswordEnc": vault.seal(FIELD_SUDO_PASSWORD, &profile.id, &profile.sudo_password),
        "totpSecretEnc": vault.seal(FIELD_TOTP_SECRET, &profile.id, &profile.totp_secret),
        "authFlowMode": profile.auth_flow_mode,
        "passwordPromptHint": profile.password_prompt_hint,
        "totpPromptHint": profile.totp_prompt_hint,
        "sudoUsePty": profile.sudo_use_pty,
        "createdAt": profile.created_at,
        "updatedAt": profile.updated_at,
    })
}

/// Plaintext JSON shape used by `reveal_profile` (workbench editor prefill).
/// This is the only path that echoes raw secrets and it never touches disk.
fn profile_plain_json(profile: &SudoProfile) -> Value {
    json!({
        "id": profile.id,
        "name": profile.name,
        "sudoPassword": profile.sudo_password,
        "totpSecret": profile.totp_secret,
        "authFlowMode": profile.auth_flow_mode,
        "passwordPromptHint": profile.password_prompt_hint,
        "totpPromptHint": profile.totp_prompt_hint,
        "sudoUsePty": profile.sudo_use_pty,
        "createdAt": profile.created_at,
        "updatedAt": profile.updated_at,
    })
}

/// Caller-facing view: secret presence flags only, never the values.
pub fn profile_view(profile: &SudoProfile) -> Value {
    json!({
        "id": profile.id,
        "name": profile.name,
        "sudoPasswordSet": !profile.sudo_password.trim().is_empty(),
        "totpConfigured": !profile.totp_secret.trim().is_empty(),
        "authFlowMode": profile.auth_flow_mode,
        "passwordPromptHint": profile.password_prompt_hint,
        "totpPromptHint": profile.totp_prompt_hint,
        "sudoUsePty": profile.sudo_use_pty,
        "createdAt": profile.created_at,
        "updatedAt": profile.updated_at,
    })
}

/// Sorted by name (case-insensitive) so UI and MCP listings are stable.
pub fn list_views(store: &SudoProfileStore) -> Vec<Value> {
    let mut profiles = store.profiles.clone();
    profiles.sort_by_key(|left| left.name.to_lowercase());
    profiles.iter().map(profile_view).collect()
}

/// Workbench-only reveal of one profile including its raw secrets, so the
/// profile editor can prefill what the user stored. Not reachable from the
/// MCP surface (list/save views stay flag-only).
pub fn reveal_profile(store: &SudoProfileStore, id: &str) -> Result<Value, String> {
    let profile = store
        .profiles
        .iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| "Quick Sudo profile not found".to_string())?;
    Ok(json!({ "profile": profile_plain_json(profile) }))
}
