//! Quick Sudo profiles: storage layer.

use super::*;

pub(super) fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn store_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(FILE_NAME)
}

/// Loads the store; a missing or corrupted file yields an empty store so a
/// bad file can never break connecting (same policy as mcp-settings.json).
/// The DEK provider comes from the file's `crypto.storage` tier (first use:
/// keychain with keyfile fallback). A legacy v1 plaintext file carrying
/// non-empty secrets is rewritten to the encrypted v2 format best-effort.
pub fn load_store(data_dir: &Path) -> SudoProfileStore {
    let text = std::fs::read_to_string(store_path(data_dir)).unwrap_or_default();
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return SudoProfileStore::default();
    };
    let provider = resolve_provider(storage_from_value(&value), data_dir);
    load_from_value(&value, data_dir, provider.as_ref())
}

/// Injection variant of `load_store` with an explicit DEK provider, used by
/// tests (Keyfile + tempdir, never the OS keychain) and callers that already
/// resolved a provider. Same v1→v2 migration behavior as `load_store`.
#[cfg(test)]
pub fn load_store_with(data_dir: &Path, provider: &dyn KeyProvider) -> SudoProfileStore {
    let text = std::fs::read_to_string(store_path(data_dir)).unwrap_or_default();
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return SudoProfileStore::default();
    };
    load_from_value(&value, data_dir, provider)
}

fn load_from_value(value: &Value, data_dir: &Path, provider: &dyn KeyProvider) -> SudoProfileStore {
    let vault = Vault::new(provider);
    let store = parse_store(value, &vault);
    let recovered_secret = store
        .profiles
        .iter()
        .any(|profile| !profile.sudo_password.is_empty() || !profile.totp_secret.is_empty());
    // Best-effort rewrites on load; a failed rewrite only warns and never
    // blocks. v1 plaintext files carrying secrets are upgraded to the
    // encrypted format with the same tier. Files on the keychain tier are
    // migrated to the keyfile tier once their secrets have been recovered:
    // the OS keychain re-prompts for authorization whenever the sidecar
    // binary changes, so keychain is only kept when explicitly opted into.
    // Migration requires recovered plaintext — when the keychain was denied
    // the parsed secrets are empty and the file is left untouched instead of
    // being resealed with blanks.
    if !is_encrypted_file(value) && recovered_secret {
        if let Err(error) = save_store_with(data_dir, &store, provider) {
            eprintln!("[ssh] failed to migrate quick-sudo-profiles.json to encrypted v2: {error}");
        }
    } else if storage_from_value(value) == Some(KeyStorage::Keychain) && recovered_secret {
        let keyfile = KeyfileProvider::new(keyfile_path(data_dir));
        match save_store_with(data_dir, &store, &keyfile) {
            Ok(()) => {
                eprintln!("[ssh] migrated quick-sudo-profiles.json DEK from keychain to keyfile");
                delete_keychain_dek();
            }
            Err(error) => eprintln!(
                "[ssh] failed to migrate quick-sudo-profiles.json off the keychain tier: {error}"
            ),
        }
    }
    store
}

fn parse_store(value: &Value, vault: &Vault) -> SudoProfileStore {
    let profiles = value
        .get("profiles")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|profile| profile_from_json(profile, vault))
                .collect()
        })
        .unwrap_or_default();
    let bindings = value
        .get("bindings")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(connection, profile)| {
                    let profile = profile.as_str()?;
                    (!connection.is_empty() && !profile.is_empty())
                        .then(|| (connection.clone(), profile.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    SudoProfileStore { profiles, bindings }
}

fn storage_from_value(value: &Value) -> Option<KeyStorage> {
    value
        .get("crypto")
        .and_then(|crypto| crypto.get("storage"))
        .and_then(Value::as_str)
        .and_then(KeyStorage::parse)
}

/// v2 files carry `version >= 2` (anything else is treated as legacy
/// plaintext and migrated on load when it holds secrets).
fn is_encrypted_file(value: &Value) -> bool {
    value.get("version").and_then(Value::as_u64).unwrap_or(1) >= STORAGE_VERSION
}

/// Persists atomically (tmp + rename) with 0600 permissions on Unix. The DEK
/// provider follows the existing file's `crypto.storage` tier (first write:
/// keychain with keyfile fallback); all secret fields are sealed via
/// `crate::core::vault` before hitting disk.
pub fn save_store(data_dir: &Path, store: &SudoProfileStore) -> Result<(), String> {
    let text = std::fs::read_to_string(store_path(data_dir)).unwrap_or_default();
    let storage = serde_json::from_str::<Value>(&text)
        .ok()
        .as_ref()
        .and_then(storage_from_value);
    let provider = resolve_provider(storage, data_dir);
    save_store_with(data_dir, store, provider.as_ref())
}

/// Injection variant of `save_store` with an explicit DEK provider, used by
/// tests and callers that already resolved a provider. The provider's tier is
/// recorded in the `crypto.storage` header.
pub fn save_store_with(
    data_dir: &Path,
    store: &SudoProfileStore,
    provider: &dyn KeyProvider,
) -> Result<(), String> {
    let vault = Vault::new(provider);
    let path = store_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let value = json!({
        "version": STORAGE_VERSION,
        "crypto": {
            "scheme": CRYPTO_SCHEME,
            "storage": vault.storage().as_str(),
        },
        "profiles": store
            .profiles
            .iter()
            .map(|profile| profile_json(profile, &vault))
            .collect::<Vec<_>>(),
        "bindings": store.bindings,
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Failed to encode Quick Sudo profiles: {error}"))?;
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
