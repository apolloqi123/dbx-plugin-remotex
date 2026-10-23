//! Windterm session importer.

use super::*;

/// Resolved bits of `user.config` needed for credential decryption.
#[derive(Debug, Default)]
pub(super) struct WindTermConfig {
    /// Raw salt bytes (`application.fingerprint`), hex-decoded when possible.
    fingerprint: Option<Vec<u8>>,
    /// `application.masterPassword` switch.
    master_password_enabled: bool,
}

impl WindTermConfig {
    fn from_user_config(user_config: Option<&[u8]>) -> WindTermConfig {
        let Some(bytes) = user_config else {
            return WindTermConfig::default();
        };
        let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
            return WindTermConfig::default();
        };
        let application = value.get("application");
        WindTermConfig {
            fingerprint: application
                .and_then(|app| app.get("fingerprint"))
                .and_then(Value::as_str)
                .map(fingerprint_bytes),
            master_password_enabled: application
                .and_then(|app| app.get("masterPassword"))
                .map(config_flag_is_on)
                .unwrap_or(false),
        }
    }
}

/// Lenient truthiness for `user.config` flags (bool true, "true", 1).
pub(super) fn config_flag_is_on(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::String(flag) => flag.trim().eq_ignore_ascii_case("true"),
        Value::Number(flag) => flag.as_u64() == Some(1),
        _ => false,
    }
}

/// Salt bytes: hex-decode the fingerprint when it looks like hex (WindTerm
/// stores a hex string), otherwise use the raw string bytes.
pub(super) fn fingerprint_bytes(value: &str) -> Vec<u8> {
    let cleaned = value.trim().trim_start_matches("0x");
    if let Ok(bytes) = data_encoding::HEXUPPER_PERMISSIVE.decode(cleaned.as_bytes()) {
        return bytes;
    }
    value.trim().as_bytes().to_vec()
}

/// PBKDF2-HMAC-SHA3-512 → 48 bytes split into an AES-256 key and a CBC IV.
pub(super) fn derive_windterm_key(master_password: &str, salt: &[u8]) -> ([u8; 32], [u8; 16]) {
    let mut derived = [0u8; WINDTERM_DERIVED_BYTES];
    pbkdf2::pbkdf2_hmac::<sha3::Sha3_512>(
        master_password.as_bytes(),
        salt,
        WINDTERM_PBKDF2_ROUNDS,
        &mut derived,
    );
    let mut key = [0u8; 32];
    let mut iv = [0u8; 16];
    key.copy_from_slice(&derived[..32]);
    iv.copy_from_slice(&derived[32..]);
    (key, iv)
}

/// AES-256-CBC/PKCS7 decryption; `None` on any failure (wrong password,
/// tampered blob, bad padding) — callers skip the credentials.
pub(super) fn windterm_decrypt(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
) -> Option<Zeroizing<Vec<u8>>> {
    use aes::cipher::block_padding::Pkcs7;
    use aes::cipher::{BlockModeDecrypt, KeyIvInit};
    type Aes256CbcDecryptor = cbc::Decryptor<aes::Aes256>;
    Aes256CbcDecryptor::new_from_slices(key, iv)
        .ok()?
        .decrypt_padded_vec::<Pkcs7>(ciphertext)
        .ok()
        .map(Zeroizing::new)
}

/// Parses a WindTerm `.sessions` JSON array. `user_config` supplies the
/// fingerprint salt and the master-password switch; when the switch is on
/// the caller must supply `master_password` (else the dedicated error is
/// returned). Items that are not SSH sessions are skipped; per-item
/// credential failures degrade to `ImportedAuth::None` (warn semantics).
pub fn parse_windterm(
    sessions_json: &[u8],
    user_config: Option<&[u8]>,
    master_password: Option<&str>,
) -> Result<Vec<ImportedSession>, String> {
    let items: Vec<Value> = serde_json::from_slice(sessions_json)
        .map_err(|error| format!("WindTerm sessions file is not a JSON array: {error}"))?;
    let config = WindTermConfig::from_user_config(user_config);
    if config.master_password_enabled && master_password.is_none() {
        return Err(WINDTERM_MASTER_PASSWORD_REQUIRED.to_string());
    }
    let mut sessions = Vec::new();
    for item in &items {
        if let Some(session) = parse_windterm_item(item, &config, master_password) {
            push_preview_session(&mut sessions, session)?;
        }
    }
    Ok(sessions)
}

pub(super) fn parse_windterm_item(
    item: &Value,
    config: &WindTermConfig,
    master_password: Option<&str>,
) -> Option<ImportedSession> {
    let session = item.get("session")?;
    let protocol = session
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !protocol.eq_ignore_ascii_case("SSH") {
        return None;
    }
    let target = session
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (user, host) = split_windterm_target(target);
    if host.is_empty() {
        return None;
    }
    let label = session
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(&host);
    let port = windterm_port(session.get("port"));
    let mut imported = ImportedSession::new(
        label.to_string(),
        host,
        port,
        if user.is_empty() {
            "root".to_string()
        } else {
            user
        },
    );
    imported.group_path = session
        .get("group")
        .and_then(Value::as_str)
        .map(|group| {
            group
                .split('>')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    imported.description = session
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    imported.auth = windterm_credentials(session.get("autoLogin"), config, master_password);
    Some(imported)
}

/// `user@host` split on the last `@` (user names may contain `@`); no `@`
/// means the default user applies.
pub(super) fn split_windterm_target(target: &str) -> (String, String) {
    match target.trim().rsplit_once('@') {
        Some((user, host)) => (user.trim().to_string(), host.trim().to_string()),
        None => (String::new(), target.trim().to_string()),
    }
}

/// Port as number or numeric string; falls back to 22.
pub(super) fn windterm_port(value: Option<&Value>) -> u16 {
    value
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        })
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port > 0)
        .unwrap_or(22)
}

/// Credential chain for `session.autoLogin`:
/// 1. plaintext JSON (no master password in use),
/// 2. base64 → AES-256-CBC (PBKDF2 key from master password + fingerprint),
/// 3. anything else → no credentials for this item.
pub(super) fn windterm_credentials(
    raw: Option<&Value>,
    config: &WindTermConfig,
    master_password: Option<&str>,
) -> ImportedAuth {
    let Some(raw) = raw
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return ImportedAuth::None;
    };
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return auto_login_auth(&value);
    }
    let Ok(ciphertext) = BASE64.decode(raw) else {
        return ImportedAuth::None;
    };
    let Some(master_password) = master_password else {
        return ImportedAuth::None;
    };
    let Some(salt) = config.fingerprint.as_deref() else {
        return ImportedAuth::None;
    };
    let (key, iv) = derive_windterm_key(master_password, salt);
    let Some(plaintext) = windterm_decrypt(&key, &iv, &ciphertext) else {
        return ImportedAuth::None;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&plaintext) else {
        return ImportedAuth::None;
    };
    auto_login_auth(&value)
}

/// Reads the decrypted (or plaintext) autoLogin JSON: `PasswordEnabled` +
/// `Password` wins, then `Public Key.<platform>.path/pass`, else none. No
/// credential string is copied into `ImportedAuth`.
pub(super) fn auto_login_auth(value: &Value) -> ImportedAuth {
    let password_enabled = match value.get("PasswordEnabled") {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(flag)) => flag.trim().eq_ignore_ascii_case("true"),
        _ => false,
    };
    let password = value
        .get("Password")
        .and_then(Value::as_str)
        .filter(|password| !password.is_empty());
    if password_enabled && password.is_some() {
        return ImportedAuth::Password { has_secret: true };
    }
    let public_key = value.get("Public Key").or_else(|| value.get("PublicKey"));
    let key_entry = public_key.and_then(|public_key| {
        ["windows", "linux", "mac"]
            .iter()
            .find_map(|platform| public_key.get(*platform))
    });
    let key_path = key_entry
        .and_then(|entry| entry.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let key_passphrase = key_entry
        .and_then(|entry| entry.get("pass"))
        .and_then(Value::as_str)
        .filter(|pass| !pass.is_empty());
    if key_path.is_some() || key_passphrase.is_some() {
        return ImportedAuth::PrivateKey {
            path: key_path.map(expand_home_prefix),
            has_secret: key_passphrase.is_some(),
        };
    }
    ImportedAuth::None
}
