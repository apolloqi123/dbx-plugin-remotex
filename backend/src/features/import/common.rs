//! Common session importer.

use super::*;

/// Non-sensitive authentication metadata carried by one imported preview.
///
/// This is deliberately not a credential container: plaintext passwords,
/// private-key content and key passphrases must never survive parsing into the
/// preview model. Parsers may inspect those fields in `Zeroizing` temporaries
/// only to derive this metadata before the temporary is dropped.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportedAuth {
    Password {
        has_secret: bool,
    },
    PrivateKey {
        path: Option<String>,
        has_secret: bool,
    },
    None,
}

/// One imported session in the common shape shared by all parsers.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedSession {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub group_path: Vec<String>,
    pub description: String,
    pub auth: ImportedAuth,
    /// Optional preview note code ([`SECRET_NOTE_ENCRYPTED`] /
    /// [`SECRET_NOTE_NOT_CARRIED`]) explaining missing credential material.
    pub secret_note: String,
}

impl ImportedSession {
    pub(super) fn new(name: String, host: String, port: u16, username: String) -> ImportedSession {
        ImportedSession {
            name,
            host,
            port,
            username,
            group_path: Vec::new(),
            description: String::new(),
            auth: ImportedAuth::None,
            secret_note: String::new(),
        }
    }
}

/// Kind label shared by preview responses and stored entries.
pub fn auth_kind(auth: &ImportedAuth) -> &'static str {
    match auth {
        ImportedAuth::Password { .. } => "password",
        ImportedAuth::PrivateKey { .. } => "private-key",
        ImportedAuth::None => "none",
    }
}

/// True when the auth carries decryptable secret material (a key *path*
/// alone is not a secret).
pub(super) fn has_secret(auth: &ImportedAuth) -> bool {
    match auth {
        ImportedAuth::Password { has_secret } | ImportedAuth::PrivateKey { has_secret, .. } => {
            *has_secret
        }
        ImportedAuth::None => false,
    }
}

/// Bounded accumulator shared by every parser. It checks before `Vec::push`,
/// so hostile exports cannot construct an unbounded session preview only to be
/// truncated later by `finish`.
pub(super) fn push_preview_session(
    sessions: &mut Vec<ImportedSession>,
    session: ImportedSession,
) -> Result<(), String> {
    if sessions.len() >= MAX_PREVIEW_SESSIONS {
        return Err(format!(
            "Import exceeds the {MAX_PREVIEW_SESSIONS} session limit"
        ));
    }
    sessions.push(session);
    Ok(())
}

/// Expands the `$(HomeDir)` / `~/` path prefixes against the local home
/// directory; relative paths stay as-is (they are relative to the sessions
/// file, which is only ever kept as a string here).
pub(super) fn expand_home_prefix(path: &str) -> String {
    for token in ["$(HomeDir)", "~"] {
        if let Some(rest) = path.strip_prefix(token) {
            let rest = rest.trim_start_matches(['/', '\\']);
            if let Some(home) = home_dir() {
                return if rest.is_empty() {
                    home
                } else {
                    format!("{home}/{rest}")
                };
            }
        }
    }
    path.to_string()
}

pub(super) fn home_dir() -> Option<String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|value| value.to_string_lossy().into_owned())
}

/// Trimmed description, empty when absent.
pub(super) fn normalize_import_text(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// Lenient field readers for the JSON formats (Electerm, Termius, FinalShell
/// bare fallback): absent fields and wrong types degrade to `None` / empty
/// instead of failing the whole import.
pub(super) fn json_text<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

pub(super) fn json_u64(object: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    match object.get(key) {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

pub(super) fn json_text_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `{"id": "x"}` reference object or a bare string id.
pub(super) fn json_id_ref<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Option<&'a str> {
    match object.get(key) {
        Some(Value::Object(inner)) => inner.get("id").and_then(Value::as_str),
        Some(Value::String(text)) if !text.is_empty() => Some(text.as_str()),
        _ => None,
    }
}

/// First index of `needle` in `haystack`, searching from `from`.
pub(super) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|&index| &haystack[index..index + needle.len()] == needle)
}
