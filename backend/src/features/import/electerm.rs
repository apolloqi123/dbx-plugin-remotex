//! Electerm session importer.

use super::*;

pub(super) struct ElectermGroup {
    id: String,
    title: String,
    bookmark_ids: Vec<String>,
    group_ids: Vec<String>,
}

/// Parses an Electerm bookmarks export: either a bare JSON array of bookmark
/// objects (no group information available) or the `{bookmarks,
/// bookmarkGroups}` object Electerm stores. Field semantics mirror the
/// NyaTerm importer: only `type == "ssh"` bookmarks with `enable_ssh` unset
/// or true survive; an out-of-range port skips the bookmark. Electerm keeps
/// credentials out of the bookmarks file, so password-auth bookmarks carry no
/// material and get the [`SECRET_NOTE_NOT_CARRIED`] preview note.
pub fn parse_electerm(bytes: &[u8]) -> Result<Vec<ImportedSession>, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Electerm bookmarks file is not valid JSON: {error}"))?;
    let (bookmarks, groups_raw) = match value {
        Value::Array(items) => (items, None),
        Value::Object(map) => {
            let Some(bookmarks) = map.get("bookmarks").and_then(Value::as_array).cloned() else {
                return Err("Electerm JSON object has no bookmarks array".to_string());
            };
            (bookmarks, map.get("bookmarkGroups").cloned())
        }
        _ => {
            return Err(
                "Electerm bookmarks file must be a JSON array or an object with a bookmarks array"
                    .to_string(),
            )
        }
    };
    let groups = electerm_groups(groups_raw.as_ref());
    let mut bookmark_groups: HashMap<String, String> = HashMap::new();
    for group in &groups {
        for bookmark_id in &group.bookmark_ids {
            bookmark_groups
                .entry(bookmark_id.clone())
                .or_insert_with(|| group.id.clone());
        }
    }
    let mut sessions = Vec::new();
    for bookmark in &bookmarks {
        if let Some(session) = electerm_session(bookmark, &groups, &bookmark_groups) {
            push_preview_session(&mut sessions, session)?;
        }
    }
    Ok(sessions)
}

pub(super) fn electerm_groups(raw: Option<&Value>) -> Vec<ElectermGroup> {
    let Some(items) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            Some(ElectermGroup {
                id: json_text(object, "id")?.to_string(),
                title: json_text(object, "title").unwrap_or_default().to_string(),
                bookmark_ids: json_text_list(object.get("bookmarkIds")),
                group_ids: json_text_list(object.get("bookmarkGroupIds")),
            })
        })
        .collect()
}

pub(super) fn electerm_session(
    bookmark: &Value,
    groups: &[ElectermGroup],
    bookmark_groups: &HashMap<String, String>,
) -> Option<ImportedSession> {
    let object = bookmark.as_object()?;
    let session_type = json_text(object, "type").unwrap_or_default();
    if !session_type.eq_ignore_ascii_case("ssh") {
        return None;
    }
    if object.get("enable_ssh") == Some(&Value::Bool(false)) {
        return None;
    }
    let host = json_text(object, "host")
        .map(str::trim)
        .filter(|host| !host.is_empty())?;
    let port = match json_u64(object, "port") {
        Some(port) if (1..=u64::from(u16::MAX)).contains(&port) => port as u16,
        // An explicit out-of-range port marks the bookmark as broken — skip
        // it (NyaTerm semantics), do not silently fall back to 22.
        Some(_) => return None,
        None => 22,
    };
    let username = json_text(object, "username")
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .unwrap_or("root");
    let name = json_text(object, "title")
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(host);
    let mut session = ImportedSession::new(
        name.to_string(),
        host.to_string(),
        port,
        username.to_string(),
    );
    session.group_path = json_text(object, "id")
        .and_then(|id| bookmark_groups.get(id))
        .map(|group_id| electerm_group_path(group_id, groups))
        .unwrap_or_default();
    if json_text(object, "auth_type")
        .map(str::trim)
        .is_some_and(|auth| auth.eq_ignore_ascii_case("password"))
    {
        session.auth = ImportedAuth::Password { has_secret: false };
        session.secret_note = SECRET_NOTE_NOT_CARRIED.to_string();
    }
    Some(session)
}

/// Resolves a group id to its full path by walking `bookmarkGroupIds` parent
/// chains; a `visited` set breaks cycles.
pub(super) fn electerm_group_path(group_id: &str, groups: &[ElectermGroup]) -> Vec<String> {
    let by_id: HashMap<&str, &ElectermGroup> = groups
        .iter()
        .map(|group| (group.id.as_str(), group))
        .collect();
    let parent_of: HashMap<&str, &str> = groups
        .iter()
        .flat_map(|group| {
            group
                .group_ids
                .iter()
                .map(move |child| (child.as_str(), group.id.as_str()))
        })
        .collect();
    let mut path = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut current = group_id;
    loop {
        if !visited.insert(current.to_string()) {
            break;
        }
        let Some(group) = by_id.get(current) else {
            break;
        };
        let title = group.title.trim();
        if !title.is_empty() {
            path.push(title.to_string());
        }
        let Some(parent) = parent_of.get(current) else {
            break;
        };
        current = parent;
    }
    path.reverse();
    path
}
