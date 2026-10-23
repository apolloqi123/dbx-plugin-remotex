//! Moba session importer.

use super::*;

/// Parses a MobaXterm `.mxtsessions` INI. Only `Bookmarks*` sections carry
/// sessions; `SubRep` holds the folder path (`\`-separated) applied to every
/// session in its section. SSH connection strings are the entries whose
/// type field starts with `109` (`#109#<subtype>%host%port%user%...`).
pub fn parse_moba_ini(text: &str) -> Result<Vec<ImportedSession>, String> {
    let mut sessions = Vec::new();
    for section in parse_ini_sections(text) {
        if !section.name.starts_with("Bookmarks") {
            continue;
        }
        // SubRep may appear anywhere in the section; resolve it first so
        // group assignment does not depend on key order.
        let group_path = section
            .entries
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("SubRep"))
            .map(|(_, value)| moba_group_path(value))
            .unwrap_or_default();
        for (key, value) in &section.entries {
            if key.is_empty() || key.eq_ignore_ascii_case("SubRep") {
                continue;
            }
            if let Some(mut session) = parse_moba_session_value(key, value) {
                session.group_path = group_path.clone();
                push_preview_session(&mut sessions, session)?;
            }
        }
    }
    Ok(sessions)
}

/// Splits one `#<type>#<subtype>%host%port%username%...` value. Returns
/// `None` for non-SSH types and for entries without a usable host.
pub(super) fn parse_moba_session_value(name: &str, value: &str) -> Option<ImportedSession> {
    let fields: Vec<&str> = value.split('%').collect();
    let type_field = fields.first()?.trim();
    // "#109#0" -> the segment between the leading '#' markers is the type.
    let type_str = type_field.trim_start_matches('#').split('#').next()?;
    if !type_str.starts_with("109") {
        return None;
    }
    let host = fields
        .get(1)
        .map(|host| host.trim())
        .filter(|host| !host.is_empty())?;
    let port = fields
        .get(2)
        .and_then(|port| port.trim().parse::<u16>().ok())
        .filter(|port| *port > 0)
        .unwrap_or(22);
    let username = fields
        .get(3)
        .map(|username| username.trim())
        .filter(|username| !username.is_empty())
        .unwrap_or("root");
    Some(ImportedSession::new(
        name.to_string(),
        host.to_string(),
        port,
        username.to_string(),
    ))
}

pub(super) fn moba_group_path(sub_rep: &str) -> Vec<String> {
    sub_rep
        .split('\\')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) struct IniSection {
    name: String,
    entries: Vec<(String, String)>,
}

/// Minimal line-oriented INI scan: `;`/`#` comment lines, `[section]`
/// headers, `key=value` pairs (first `=` splits). Never panics on odd bytes;
/// a malformed line is skipped.
pub(super) fn parse_ini_sections(text: &str) -> Vec<IniSection> {
    let mut sections: Vec<IniSection> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with([';', '#']) {
            continue;
        }
        if trimmed.starts_with('[') {
            if let Some(end) = trimmed.find(']') {
                if let Some(name) = trimmed.get(1..end) {
                    sections.push(IniSection {
                        name: name.trim().to_string(),
                        entries: Vec::new(),
                    });
                }
            }
            continue;
        }
        if let Some(eq) = trimmed.find('=') {
            let (key, value) = (trimmed.get(..eq), trimmed.get(eq + 1..));
            if let (Some(key), Some(value)) = (key, value) {
                if let Some(section) = sections.last_mut() {
                    section
                        .entries
                        .push((key.trim().to_string(), value.trim().to_string()));
                }
            }
        }
    }
    sections
}

pub(super) fn find_section<'a>(sections: &'a [IniSection], name: &str) -> Option<&'a IniSection> {
    sections
        .iter()
        .find(|section| section.name.eq_ignore_ascii_case(name))
}

pub(super) fn section_value<'a>(section: &'a IniSection, key: &str) -> Option<&'a str> {
    section
        .entries
        .iter()
        .rev()
        .find(|(entry_key, _)| entry_key.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}
