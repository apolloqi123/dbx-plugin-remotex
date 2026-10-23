//! Termius session importer.

use super::*;

/// Flattened reference tables for a Termius export: hosts point at
/// `ssh_config` entries (port), which point at `identities` (username,
/// password), which point at `keys` (PEM material).
#[derive(Debug, Default)]
pub(super) struct TermiusConfig {
    port: Option<u64>,
    identity_id: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct TermiusIdentity {
    username: String,
    password: String,
    ssh_key_id: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct TermiusKey {
    passphrase: String,
    private_key: String,
}

#[derive(Debug, Default)]
pub(super) struct TermiusGroup {
    label: String,
    parent_id: Option<String>,
}

/// Parses a Termius JSON export (`Settings → Export`): an object whose
/// optional `data` wrapper holds `hosts`, `groups`, `ssh_configs`,
/// `identities` and `keys` arrays. Field mapping mirrors the NyaTerm
/// importer: the host label falls back to the address, the username to
/// `root`, and the port resolves through the host's `ssh_config`. Secret
/// fields that arrive as encrypted sync blobs (base64 starting with the
/// `BA` version prefix) are dropped, never decrypted — the session keeps
/// password semantics with the [`SECRET_NOTE_ENCRYPTED`] note. Plaintext
/// passwords and PEM private keys are carried over.
pub fn parse_termius(bytes: &[u8]) -> Result<Vec<ImportedSession>, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Termius export file is not valid JSON: {error}"))?;
    let root = value.get("data").unwrap_or(&value);
    let Some(hosts) = root.get("hosts").and_then(Value::as_array) else {
        return Err("Termius export JSON has no hosts array".to_string());
    };
    let groups = termius_groups(root.get("groups"));
    let configs = termius_configs(root.get("ssh_configs"));
    let identities = termius_identities(root.get("identities"));
    let keys = termius_keys(root.get("keys").or_else(|| root.get("ssh_keys")));
    let mut sessions = Vec::new();
    for host in hosts {
        if let Some(session) = termius_session(host, &groups, &configs, &identities, &keys) {
            push_preview_session(&mut sessions, session)?;
        }
    }
    Ok(sessions)
}

pub(super) fn termius_groups(value: Option<&Value>) -> HashMap<String, TermiusGroup> {
    let Some(items) = value.and_then(Value::as_array) else {
        return HashMap::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let id = json_text(object, "id")?.to_string();
            Some((
                id,
                TermiusGroup {
                    label: normalize_import_text(json_text(object, "label")),
                    parent_id: json_id_ref(object, "parent").map(str::to_owned),
                },
            ))
        })
        .collect()
}

pub(super) fn termius_configs(value: Option<&Value>) -> HashMap<String, TermiusConfig> {
    let Some(items) = value.and_then(Value::as_array) else {
        return HashMap::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let id = json_text(object, "id")?.to_string();
            Some((
                id,
                TermiusConfig {
                    port: json_u64(object, "port").filter(|port| *port > 0),
                    identity_id: json_id_ref(object, "identity")
                        .or_else(|| json_id_ref(object, "identity_id"))
                        .map(str::to_owned),
                },
            ))
        })
        .collect()
}

pub(super) fn termius_identities(value: Option<&Value>) -> HashMap<String, TermiusIdentity> {
    let Some(items) = value.and_then(Value::as_array) else {
        return HashMap::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let id = json_text(object, "id")?.to_string();
            Some((
                id,
                TermiusIdentity {
                    username: normalize_import_text(json_text(object, "username")),
                    password: normalize_import_text(json_text(object, "password")),
                    ssh_key_id: json_id_ref(object, "ssh_key")
                        .or_else(|| json_id_ref(object, "ssh_key_id"))
                        .map(str::to_owned),
                },
            ))
        })
        .collect()
}

pub(super) fn termius_keys(value: Option<&Value>) -> HashMap<String, TermiusKey> {
    let Some(items) = value.and_then(Value::as_array) else {
        return HashMap::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let id = json_text(object, "id")?.to_string();
            Some((
                id,
                TermiusKey {
                    passphrase: normalize_import_text(json_text(object, "passphrase")),
                    private_key: normalize_import_text(
                        json_text(object, "privateKey")
                            .or_else(|| json_text(object, "private_key")),
                    ),
                },
            ))
        })
        .collect()
}

pub(super) fn termius_session(
    host: &Value,
    groups: &HashMap<String, TermiusGroup>,
    configs: &HashMap<String, TermiusConfig>,
    identities: &HashMap<String, TermiusIdentity>,
    keys: &HashMap<String, TermiusKey>,
) -> Option<ImportedSession> {
    let object = host.as_object()?;
    let address = json_text(object, "address")
        .map(str::trim)
        .filter(|address| !address.is_empty())?;
    let config = json_id_ref(object, "ssh_config").and_then(|id| configs.get(id));
    let port = json_u64(object, "port")
        .filter(|port| *port > 0)
        .or(config.and_then(|config| config.port))
        .and_then(|port| u16::try_from(port).ok())
        .unwrap_or(22);
    let identity = json_id_ref(object, "identity")
        .or(config.and_then(|config| config.identity_id.as_deref()))
        .and_then(|id| identities.get(id));
    let mut username = json_text(object, "username")
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .unwrap_or_else(|| {
            identity
                .map(|identity| identity.username.as_str())
                .unwrap_or_default()
        });
    if username.is_empty() {
        username = "root";
    }
    let name = json_text(object, "label")
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(address);
    // Password: the host entry wins, then the identity.
    let mut password_raw = normalize_import_text(json_text(object, "password"));
    if password_raw.is_empty() {
        password_raw = identity
            .map(|identity| identity.password.as_str())
            .unwrap_or_default()
            .to_string();
    }
    let password_present = !password_raw.is_empty();
    let plaintext_password = password_present && !is_termius_encrypted_blob(&password_raw);
    // Key material is never carried into the preview model: we only derive
    // whether a plaintext PEM/passphrase existed before the parsed JSON drops.
    let key = identity
        .and_then(|identity| identity.ssh_key_id.as_deref())
        .and_then(|id| keys.get(id));
    let has_key_content = key
        .map(|key| key.private_key.trim())
        .is_some_and(|private_key| private_key.starts_with("-----BEGIN"));
    let has_key_passphrase = key
        .map(|key| key.passphrase.as_str())
        .is_some_and(|passphrase| !passphrase.is_empty() && !is_termius_encrypted_blob(passphrase));
    let mut session = ImportedSession::new(
        name.to_string(),
        address.to_string(),
        port,
        username.to_string(),
    );
    session.group_path = termius_group_path(
        json_id_ref(object, "group").or_else(|| json_text(object, "group_id")),
        groups,
    );
    let encrypted_password = password_present && !plaintext_password;
    let (auth, note) = if plaintext_password {
        (ImportedAuth::Password { has_secret: true }, None)
    } else if encrypted_password {
        (
            ImportedAuth::Password { has_secret: false },
            Some(SECRET_NOTE_ENCRYPTED),
        )
    } else if has_key_content || has_key_passphrase {
        (
            ImportedAuth::PrivateKey {
                path: None,
                has_secret: has_key_content || has_key_passphrase,
            },
            None,
        )
    } else {
        (ImportedAuth::None, None)
    };
    session.auth = auth;
    session.secret_note = note.unwrap_or_default().to_string();
    Some(session)
}

/// Walks the group parent chain bottom-up; a `visited` set breaks cycles.
pub(super) fn termius_group_path(
    group_id: Option<&str>,
    groups: &HashMap<String, TermiusGroup>,
) -> Vec<String> {
    let Some(mut current) = group_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Vec::new();
    };
    let mut path = Vec::new();
    let mut visited = std::collections::HashSet::new();
    loop {
        if !visited.insert(current.to_string()) {
            break;
        }
        let Some(group) = groups.get(current) else {
            break;
        };
        if !group.label.is_empty() {
            path.push(group.label.clone());
        }
        let Some(parent) = group
            .parent_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            break;
        };
        current = parent;
    }
    path.reverse();
    path
}

/// Termius encrypts secret fields into base64 blobs whose decoded form starts
/// with the `BA` version prefix — the same heuristic as the NyaTerm importer
/// (`is_termius_encrypted_value`). Encrypted material is never decoded here.
pub(super) fn is_termius_encrypted_blob(value: &str) -> bool {
    value.len() >= 40
        && value.starts_with("BA")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
}
