//! Xshell session importer.

use super::*;

/// Parses an Xshell `.xts` archive: every `.xsh` entry is one session. Entry
/// names on Chinese Windows installs are GBK-encoded, so raw bytes are
/// decoded as UTF-8 first (modern exports) with a GBK fallback. Only SSH
/// sessions are kept; archive expansion is bounded by [`MAX_ZIP_ENTRIES`],
/// [`MAX_ENTRY_BYTES`], and [`MAX_TOTAL_BYTES`].
pub fn parse_xshell(zip_bytes: &[u8]) -> Result<Vec<ImportedSession>, String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|error| format!("Failed to read Xshell archive: {error}"))?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(format!(
            "Xshell archive has more than {MAX_ZIP_ENTRIES} entries"
        ));
    }
    let mut sessions = Vec::new();
    let mut total_bytes: u64 = 0;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("Failed to read Xshell archive entry {index}: {error}"))?;
        if entry.is_dir() {
            continue;
        }
        let declared = entry.size();
        if declared > MAX_ENTRY_BYTES {
            return Err(format!(
                "Xshell archive entry expands beyond the {} MB per-entry limit",
                MAX_ENTRY_BYTES / (1024 * 1024)
            ));
        }
        total_bytes += declared;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(format!(
                "Xshell archive expands beyond the {} MB total limit",
                MAX_TOTAL_BYTES / (1024 * 1024)
            ));
        }
        // Header-declared size is honored by the reader too, but a hostile
        // archive can lie: cap the actual read as well.
        let name = decode_zip_name(entry.name_raw());
        if !name.to_ascii_lowercase().ends_with(".xsh") {
            continue;
        }
        let mut limited = entry.take(MAX_ENTRY_BYTES + 1);
        let mut content = Vec::new();
        limited
            .read_to_end(&mut content)
            .map_err(|error| format!("Failed to read Xshell entry '{name}': {error}"))?;
        if content.len() as u64 > MAX_ENTRY_BYTES {
            return Err(format!(
                "Xshell archive entry '{name}' exceeds the per-entry size limit"
            ));
        }
        let text = String::from_utf8_lossy(&content);
        if let Some(session) = parse_xsh_file(&name, &text) {
            push_preview_session(&mut sessions, session)?;
        }
    }
    Ok(sessions)
}

/// UTF-8 first (ASCII is identical under GBK), GBK fallback for Chinese
/// Windows exports, lossy as the last resort. Never fails.
pub(super) fn decode_zip_name(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(name) => name.to_string(),
        Err(_) => {
            // No BOM handling: entry names never carry BOMs, and BOM
            // sniffing would silently swallow a leading 0xFF 0xFE.
            let (decoded, had_errors) = encoding_rs::GBK.decode_without_bom_handling(raw);
            if had_errors {
                String::from_utf8_lossy(raw).into_owned()
            } else {
                decoded.into_owned()
            }
        }
    }
}

/// Parses one `.xsh` INI file. Returns `None` when the entry is not an SSH
/// session (protocol or host missing) — a bad entry never fails the import.
pub(super) fn parse_xsh_file(entry_name: &str, text: &str) -> Option<ImportedSession> {
    let sections = parse_ini_sections(text);
    let connection = find_section(&sections, "CONNECTION")?;
    let protocol = section_value(connection, "Protocol")?;
    if !protocol.eq_ignore_ascii_case("SSH") {
        return None;
    }
    let host = section_value(connection, "Host")?;
    let port = section_value(connection, "Port")
        .and_then(|port| port.parse::<u16>().ok())
        .filter(|port| *port > 0)
        .unwrap_or(22);
    let username = find_section(&sections, "CONNECTION:AUTHENTICATION")
        .and_then(|auth| section_value(auth, "UserName"))
        .unwrap_or("root")
        .to_string();
    let auth = find_section(&sections, "CONNECTION:AUTHENTICATION")
        .and_then(|auth| section_value(auth, "UserKey"))
        .map(|key| ImportedAuth::PrivateKey {
            path: Some(key.to_string()),
            has_secret: false,
        })
        .unwrap_or(ImportedAuth::None);
    let mut session = ImportedSession::new(
        xsh_session_name(entry_name),
        host.to_string(),
        port,
        username,
    );
    session.auth = auth;
    session.group_path = xsh_group_path(entry_name);
    Some(session)
}

/// Session display name: the `.xsh` entry's file stem.
pub(super) fn xsh_session_name(entry_name: &str) -> String {
    let path = entry_name.replace('\\', "/");
    let file = path.rsplit('/').next().unwrap_or(&path);
    strip_xsh_suffix(file).to_string()
}

pub(super) fn strip_xsh_suffix(name: &str) -> &str {
    match name.get(name.len().saturating_sub(4)..) {
        Some(suffix) if suffix.eq_ignore_ascii_case(".xsh") => {
            // The suffix is ASCII, so the byte boundary is a char boundary.
            &name[..name.len() - 4]
        }
        _ => name,
    }
}

/// Group path from the ZIP directory layout: `Xshell/Sessions/Prod/web.xsh`
/// → `["Prod"]`; root-level entries have no group. The last path component
/// is always the file name and is dropped.
pub(super) fn xsh_group_path(entry_name: &str) -> Vec<String> {
    let path = entry_name.replace('\\', "/");
    let lowered = path.to_ascii_lowercase();
    let prefix_end = if let Some(index) = lowered.find("xshell/sessions/") {
        index + "xshell/sessions/".len()
    } else if let Some(index) = lowered.find("xshell/") {
        index + "xshell/".len()
    } else {
        0
    };
    let relative = path.get(prefix_end..).unwrap_or(&path);
    let components: Vec<String> = relative
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    components
        .split_last()
        .map(|(_, directories)| directories.to_vec())
        .unwrap_or_default()
}
