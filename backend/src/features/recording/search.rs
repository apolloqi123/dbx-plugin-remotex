use super::*;

/// Reads one page of events (oldest first) for replay: `offset` counts
/// event lines (header excluded). Returns the events plus `hasMore`.
pub fn read_events(
    data_dir: &Path,
    recording_id: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    let path = cast_path(data_dir, recording_id)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("Failed to read recording: {error}"))?;
    let mut total = 0_usize;
    let mut events: Vec<Value> = Vec::new();
    for line in text.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let index = total;
        total += 1;
        if index >= offset && events.len() < limit.min(PAGE_LIMIT) {
            events.push(json!({
                "time": event.get("time").cloned().unwrap_or(json!(0)),
                "type": event.get("eventtype").cloned().unwrap_or(json!("o")),
                "data": event.get("eventdata").cloned().unwrap_or(json!("")),
            }));
        }
    }
    let has_more = offset + events.len() < total;
    Ok(json!({
        "recordingId": recording_id,
        "offset": offset,
        "total": total,
        "hasMore": has_more,
        "events": events,
    }))
}

/// Case-insensitive `contains` over lossy UTF-8 — recordings are terminal
/// output, so matching is byte-folded text, no locale collation.
fn contains_fold(haystack: &str, needle_lower: &str) -> bool {
    haystack.to_lowercase().contains(needle_lower)
}

/// Extracts a display excerpt anchored at the first match in `text`: the
/// matched word always shows in full, up to `max_chars` visible characters
/// follow it, and cut sides get an ellipsis. Pure so tests can pin the output.
pub(super) fn excerpt_around(text: &str, needle_lower: &str, max_chars: usize) -> String {
    let Some(relative) = text.to_lowercase().find(needle_lower) else {
        return String::new();
    };
    let chars: Vec<char> = text.chars().collect();
    let mut byte_index = 0;
    let mut char_index = 0;
    for (index, ch) in chars.iter().enumerate() {
        if byte_index + ch.len_utf8() > relative {
            char_index = index;
            break;
        }
        byte_index += ch.len_utf8();
        char_index = index + 1;
    }
    const ELLIPSIS: &str = "…";
    let lead = usize::from(char_index > 0);
    let window = max_chars
        .saturating_sub(lead)
        .saturating_sub(1)
        .max(needle_lower.chars().count());
    let start = char_index;
    let end = (char_index + window).min(chars.len());
    let mut out = String::new();
    if lead == 1 {
        out.push_str(ELLIPSIS);
    }
    out.extend(chars[start..end].iter());
    if end < chars.len() {
        out.push_str(ELLIPSIS);
    }
    out
}

/// Builds a single-line flattened text of one event's `eventdata`: ANSI/OSC
/// control sequences are stripped (searching must not miss "ERROR" hidden
/// behind a color escape) and line breaks become spaces so each hit renders
/// as one excerpt line.
pub(super) fn flatten_event_text(event: &Value) -> String {
    let raw = event.get("eventdata").and_then(Value::as_str).unwrap_or("");
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => {
                // CSI: swallow through the final byte (@-~); OSC: through BEL
                // or ST. Anything else: one following byte max.
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                } else if chars.peek() == Some(&']') {
                    chars.next();
                    for next in chars.by_ref() {
                        match next {
                            '\x07' => break,
                            '\x1b' => {
                                // ST 终止符（ESC \）：连反斜杠一起吞掉。
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                            _ => {}
                        }
                    }
                } else {
                    chars.next();
                }
            }
            '\r' => {}
            '\n' | '\t' => out.push(' '),
            other => out.push(other),
        }
    }
    out.trim().to_string()
}

/// Content-only full-text scan of one recording: returns up to
/// `max_hits` `{ time, excerpt }` rows. Excerpt window is the flattened
/// single-line text so hits stay readable in a compact list.
fn search_recording_content(
    path: &Path,
    needle_lower: &str,
    max_hits: usize,
) -> Option<Vec<Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut hits: Vec<Value> = Vec::new();
    let mut truncated = false;
    for line in text.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let flat = flatten_event_text(&event);
        if flat.is_empty() || !contains_fold(&flat, needle_lower) {
            continue;
        }
        if hits.len() >= max_hits {
            truncated = true;
            break;
        }
        hits.push(json!({
            "time": event.get("time").cloned().unwrap_or(json!(0)),
            "excerpt": excerpt_around(&flat, needle_lower, 120),
        }));
    }
    if truncated || !hits.is_empty() {
        Some(hits)
    } else {
        None
    }
}

/// `ssh/recording/search`: instant full-text scan over existing `.cast`
/// recordings (no persistent index; bounded by SEARCH_MAX_RECORDINGS). A hit
/// is a name match (host / recordingId contains `query`, case-insensitive —
/// listed with zero content hits) or a content match (flattened stdout text
/// contains `query`). Newest first, mirroring `list_recordings`. Empty query
/// yields an empty result set — the workbench shows the unfiltered list.
pub fn search_recordings(data_dir: &Path, query: &str) -> Vec<Value> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let dir = recordings_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut items: Vec<(std::time::SystemTime, Value)> = Vec::new();
    for entry in entries.flatten().take(SEARCH_MAX_RECORDINGS * 4) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("cast") {
            continue;
        }
        let Some(recording_id) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if items.len() >= SEARCH_MAX_RECORDINGS {
            break;
        }
        let Some(row) = describe_recording(&path, &recording_id) else {
            continue;
        };
        let host = row.get("host").and_then(Value::as_str).unwrap_or("");
        let name_match = contains_fold(host, &needle) || contains_fold(&recording_id, &needle);
        let hits = search_recording_content(&path, &needle, SEARCH_MAX_HITS);
        if !name_match && hits.is_none() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        items.push((
            modified,
            json!({
                "recordingId": recording_id,
                "sessionId": row.get("sessionId").cloned().unwrap_or(json!("")),
                "connectionId": row.get("connectionId").cloned().unwrap_or(json!("")),
                "host": row.get("host").cloned().unwrap_or(json!("")),
                "startedAt": row.get("startedAt").cloned().unwrap_or(json!(0)),
                "durationSecs": row.get("durationSecs").cloned().unwrap_or(json!(0)),
                "bytes": row.get("bytes").cloned().unwrap_or(json!(0)),
                "nameMatch": name_match,
                "hits": hits.unwrap_or_default(),
            }),
        ));
    }
    items.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    items.into_iter().map(|(_, item)| item).collect()
}
