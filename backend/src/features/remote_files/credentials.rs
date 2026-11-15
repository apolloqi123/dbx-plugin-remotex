//! Encrypted storage for the remote-desktop file-transfer SSH password.
//!
//! The transfer window's password is sealed with the plugin vault (AES-256-GCM
//! under the 0600 `vault.key` DEK) and keyed by host/port/username. This lets
//! the companion session reconnect after a plugin restart without ever
//! persisting plaintext credentials or asking the user again.

use crate::core::vault::{self, KeyfileProvider, Vault};
use data_encoding::HEXLOWER;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const FIELD: &str = "remoteFilesPassword";
const FILE_NAME: &str = "remote-files-credentials.json";
const MAX_ENTRIES: usize = 64;
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn store_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

fn entry_id(host: &str, port: u16, username: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(host.as_bytes());
    hasher.update([0]);
    hasher.update(port.to_string().as_bytes());
    hasher.update([0]);
    hasher.update(username.as_bytes());
    HEXLOWER.encode(&hasher.finalize())
}

fn entries(data_dir: &Path) -> Map<String, Value> {
    let Ok(bytes) = fs::read(store_path(data_dir)) else {
        return Map::new();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Map::new();
    };
    value
        .get("entries")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn vault(data_dir: &Path) -> Vault {
    Vault::new(&KeyfileProvider::new(vault::keyfile_path(data_dir)))
}

fn write_entries(data_dir: &Path, entries: &Map<String, Value>) -> Result<(), String> {
    let target = store_path(data_dir);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create {}: {error}", parent.display()))?;
    }
    let payload = json!({ "version": 1, "entries": entries });
    let data = serde_json::to_vec_pretty(&payload)
        .map_err(|error| format!("Unable to encode saved credentials: {error}"))?;
    let tmp = target.with_extension("json.tmp");
    fs::write(&tmp, &data)
        .map_err(|error| format!("Unable to write {}: {error}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, &target)
        .map_err(|error| format!("Unable to write {}: {error}", target.display()))
}

pub(crate) fn load(data_dir: &Path, host: &str, port: u16, username: &str) -> Option<String> {
    let id = entry_id(host, port, username);
    let envelope = entries(data_dir).get(&id)?.as_str()?.to_string();
    let password = vault(data_dir).open(FIELD, &id, &envelope);
    (!password.is_empty()).then_some(password)
}

/// Saves a verified password. The caller should only invoke this after SSH
/// and SFTP both succeeded, so a typo never overwrites the working entry.
pub(crate) fn save(
    data_dir: &Path,
    host: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<(), String> {
    if password.is_empty() {
        return Ok(());
    }
    let _guard = WRITE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let id = entry_id(host, port, username);
    let envelope = vault(data_dir).seal(FIELD, &id, password);
    if envelope.is_empty() {
        return Err("Encrypted credential storage is unavailable".to_string());
    }
    let mut rows = entries(data_dir);
    rows.insert(id, Value::String(envelope));
    while rows.len() > MAX_ENTRIES {
        let Some(oldest) = rows.keys().next().cloned() else {
            break;
        };
        rows.remove(&oldest);
    }
    write_entries(data_dir, &rows)
}

pub(crate) fn forget(data_dir: &Path, host: &str, port: u16, username: &str) -> Result<(), String> {
    let _guard = WRITE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let id = entry_id(host, port, username);
    let mut rows = entries(data_dir);
    if rows.remove(&id).is_none() {
        return Ok(());
    }
    write_entries(data_dir, &rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_encrypted_and_roundtrips_per_host_user() {
        let dir = tempfile::tempdir().expect("data dir");
        save(dir.path(), "192.168.6.83", 22, "LSXD", "s3cret").expect("save");
        let raw = fs::read_to_string(store_path(dir.path())).expect("store");
        assert!(!raw.contains("s3cret"), "password leaked: {raw}");
        assert!(!raw.contains("LSXD"), "username leaked: {raw}");
        assert_eq!(
            load(dir.path(), "192.168.6.83", 22, "LSXD"),
            Some("s3cret".to_string())
        );
        assert_eq!(load(dir.path(), "192.168.6.83", 22, "other"), None);
        assert_eq!(load(dir.path(), "other", 22, "LSXD"), None);
    }

    #[test]
    fn forget_removes_only_the_matching_entry() {
        let dir = tempfile::tempdir().expect("data dir");
        save(dir.path(), "host-a", 22, "u", "a").expect("save a");
        save(dir.path(), "host-b", 22, "u", "b").expect("save b");
        forget(dir.path(), "host-a", 22, "u").expect("forget");
        assert_eq!(load(dir.path(), "host-a", 22, "u"), None);
        assert_eq!(load(dir.path(), "host-b", 22, "u"), Some("b".to_string()));
    }
}
