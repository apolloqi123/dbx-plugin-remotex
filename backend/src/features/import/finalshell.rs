//! Finalshell session importer.

use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct FinalShellFolder {
    id: String,
    name: String,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    delete_time: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct FinalShellConnection {
    #[serde(default)]
    name: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: Option<u64>,
    #[serde(default)]
    user_name: Option<String>,
    #[serde(default)]
    parent_id: Option<String>,
    /// 100 is the SSH connection type; other types are skipped.
    #[serde(default)]
    conection_type: Option<i32>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    delete_time: u64,
}

/// Parses a FinalShell conn directory packed as a ZIP: `folder.json` files
/// build the group tree, `*_connect_config.json` files are the connections.
/// A bare JSON object (or array of objects) is accepted as a fallback for
/// single-file exports. Field semantics mirror the NyaTerm importer: only
/// SSH connections (`conection_type == 100`) that were not deleted survive.
///
/// FinalShell stores passwords with its own DES scheme; like NyaTerm, the
/// ciphertext is not carried over — sessions keep password semantics with no
/// material plus the [`SECRET_NOTE_ENCRYPTED`] preview note. Archive
/// expansion is bounded by the same caps as Xshell ([`MAX_ZIP_ENTRIES`],
/// [`MAX_ENTRY_BYTES`], [`MAX_TOTAL_BYTES`]).
pub fn parse_finalshell(zip_bytes: &[u8]) -> Result<Vec<ImportedSession>, String> {
    let (folders, connections) = read_finalshell_entries(zip_bytes)?;
    if connections.is_empty() {
        return Err(
            "FinalShell source does not contain any *_connect_config.json entries".to_string(),
        );
    }
    let mut sessions = Vec::new();
    for connection in connections {
        if let Some(session) = finalshell_session(connection, &folders) {
            push_preview_session(&mut sessions, session)?;
        }
    }
    Ok(sessions)
}

pub(super) fn read_finalshell_entries(
    zip_bytes: &[u8],
) -> Result<(HashMap<String, FinalShellFolder>, Vec<FinalShellConnection>), String> {
    if let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)) {
        if archive.len() > MAX_ZIP_ENTRIES {
            return Err(format!(
                "FinalShell archive has more than {MAX_ZIP_ENTRIES} entries"
            ));
        }
        let mut folders: HashMap<String, FinalShellFolder> = HashMap::new();
        let mut connections = Vec::new();
        let mut total_bytes: u64 = 0;
        for index in 0..archive.len() {
            let entry = archive.by_index(index).map_err(|error| {
                format!("Failed to read FinalShell archive entry {index}: {error}")
            })?;
            if entry.is_dir() {
                continue;
            }
            let declared = entry.size();
            if declared > MAX_ENTRY_BYTES {
                return Err(format!(
                    "FinalShell archive entry expands beyond the {} MB per-entry limit",
                    MAX_ENTRY_BYTES / (1024 * 1024)
                ));
            }
            total_bytes += declared;
            if total_bytes > MAX_TOTAL_BYTES {
                return Err(format!(
                    "FinalShell archive expands beyond the {} MB total limit",
                    MAX_TOTAL_BYTES / (1024 * 1024)
                ));
            }
            let name = decode_zip_name(entry.name_raw());
            let file_name = name.rsplit('/').next().unwrap_or(&name).to_string();
            if file_name != "folder.json" && !file_name.ends_with("_connect_config.json") {
                continue;
            }
            let mut limited = entry.take(MAX_ENTRY_BYTES + 1);
            let mut content = Vec::new();
            limited
                .read_to_end(&mut content)
                .map_err(|error| format!("Failed to read FinalShell entry '{name}': {error}"))?;
            if content.len() as u64 > MAX_ENTRY_BYTES {
                return Err(format!(
                    "FinalShell archive entry '{name}' exceeds the per-entry size limit"
                ));
            }
            if file_name == "folder.json" {
                // A malformed or deleted folder entry degrades to "no group",
                // a malformed connection entry to "skipped" — neither fails
                // the import.
                if let Ok(folder) = serde_json::from_slice::<FinalShellFolder>(&content) {
                    if folder.delete_time == 0 {
                        folders.insert(folder.id.clone(), folder);
                    }
                }
            } else if let Ok(connection) = serde_json::from_slice::<FinalShellConnection>(&content)
            {
                if connections.len() >= MAX_PREVIEW_SESSIONS {
                    return Err(format!(
                        "Import exceeds the {MAX_PREVIEW_SESSIONS} session limit"
                    ));
                }
                connections.push(connection);
            }
        }
        return Ok((folders, connections));
    }
    // Not a ZIP: accept a bare connection JSON (one object or an array).
    let value: Value = serde_json::from_slice(zip_bytes).map_err(|error| {
        format!("FinalShell source is neither a ZIP archive nor valid JSON: {error}")
    })?;
    let items = match value {
        item @ Value::Object(_) => vec![item],
        Value::Array(items) => items,
        _ => {
            return Err(
                "FinalShell JSON must be a connection object or an array of objects".to_string(),
            )
        }
    };
    let mut connections = Vec::new();
    for item in items {
        if let Ok(connection) = serde_json::from_value::<FinalShellConnection>(item) {
            if connections.len() >= MAX_PREVIEW_SESSIONS {
                return Err(format!(
                    "Import exceeds the {MAX_PREVIEW_SESSIONS} session limit"
                ));
            }
            connections.push(connection);
        }
    }
    Ok((HashMap::new(), connections))
}

pub(super) fn finalshell_session(
    connection: FinalShellConnection,
    folders: &HashMap<String, FinalShellFolder>,
) -> Option<ImportedSession> {
    if connection.delete_time != 0 || connection.conection_type != Some(100) {
        return None;
    }
    let host = connection.host.trim();
    if host.is_empty() {
        return None;
    }
    let port = connection
        .port
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port > 0)
        .unwrap_or(22);
    let username = connection
        .user_name
        .as_deref()
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .unwrap_or("root");
    let name = if connection.name.trim().is_empty() {
        host.to_string()
    } else {
        connection.name
    };
    let mut session = ImportedSession::new(name, host.to_string(), port, username.to_string());
    session.group_path = finalshell_group_path(connection.parent_id.as_deref(), folders);
    session.description = normalize_import_text(connection.description.as_deref());
    session.auth = ImportedAuth::Password { has_secret: false };
    session.secret_note = SECRET_NOTE_ENCRYPTED.to_string();
    Some(session)
}

/// Walks the `parent_id` folder chain bottom-up; `root` / `0` / empty ids end
/// the chain, and the loop bound prevents cycles from spinning forever.
pub(super) fn finalshell_group_path(
    parent_id: Option<&str>,
    folders: &HashMap<String, FinalShellFolder>,
) -> Vec<String> {
    let Some(mut current) = parent_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Vec::new();
    };
    if current == "root" || current == "0" {
        return Vec::new();
    }
    let mut path = Vec::new();
    for _ in 0..folders.len() {
        let Some(folder) = folders.get(current) else {
            break;
        };
        let name = folder.name.trim();
        if !name.is_empty() {
            path.push(name.to_string());
        }
        match folder.parent_id.as_deref().map(str::trim) {
            Some(next) if !next.is_empty() && next != "root" && next != "0" => current = next,
            _ => break,
        }
    }
    path.reverse();
    path
}
