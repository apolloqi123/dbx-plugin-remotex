//! Securecrt session importer.

use super::*;

/// One `<key name="...">` frame with its collected `<string>/<dword>` leaves
/// and the names of the enclosing frames (outermost first).
pub(super) struct SecureCrtFrame {
    name: String,
    fields: Vec<(String, String)>,
    ancestors: Vec<String>,
}

/// Parses a SecureCRT XML session export (`Tools → Export Session Settings`).
/// Sessions are the `<key>` frames nested below the `Sessions` root whose
/// `Protocol Name` is SSH2; folders between `Sessions` and the session frame
/// become the group path.
///
/// SecureCRT only ever writes encrypted password blobs to the export, so —
/// like the NyaTerm importer — nothing is decrypted here: every session keeps
/// `ImportedAuth::Password { has_secret: false }` plus the [`SECRET_NOTE_ENCRYPTED`]
/// preview note.
///
/// The XML subset is scanned by hand (no XML crate dependency): the
/// declaration, comments, CDATA, self-closing tags, the five named entities
/// and numeric character references are handled; anything else fails closed
/// with a readable error.
pub fn parse_securecrt(text: &str) -> Result<Vec<ImportedSession>, String> {
    let frames = scan_securecrt_xml(text)?;
    let mut sessions = Vec::new();
    for frame in &frames {
        if let Some(session) = securecrt_session_from_frame(frame) {
            push_preview_session(&mut sessions, session)?;
        }
    }
    Ok(sessions)
}

pub(super) fn scan_securecrt_xml(text: &str) -> Result<Vec<SecureCrtFrame>, String> {
    const ERR_PREFIX: &str = "Invalid SecureCRT XML";
    /// Nesting ceiling for `<key>` frames. Real exports nest
    /// VanDyke > Sessions > folders > session (a handful of levels); a hostile
    /// input nesting thousands deep would clone the whole ancestor stack per
    /// closing tag (O(N²) CPU amplification), so deeper frames fail closed.
    const MAX_KEY_DEPTH: usize = 32;
    let raw = text.as_bytes();
    let mut frames: Vec<SecureCrtFrame> = Vec::new();
    // Stack of open `<key>` frames: (name, collected fields).
    let mut stack: Vec<(String, Vec<(String, String)>)> = Vec::new();
    // Open value element: (tag name, field name, collected raw bytes).
    let mut field: Option<(&'static str, String, Vec<u8>)> = None;
    let mut index = 0usize;
    while index < raw.len() {
        if raw[index] != b'<' {
            if let Some((_, _, value)) = field.as_mut() {
                value.push(raw[index]);
            }
            index += 1;
            continue;
        }
        let rest = &raw[index..];
        if rest.starts_with(b"<!--") {
            let Some(end) = find_subslice(rest, b"-->") else {
                return Err(format!("{ERR_PREFIX}: unterminated comment"));
            };
            index += end + 3;
            continue;
        }
        if rest.starts_with(b"<![CDATA[") {
            let Some(end) = find_subslice(rest, b"]]>") else {
                return Err(format!("{ERR_PREFIX}: unterminated CDATA section"));
            };
            // CDATA outside a value element is unusual for SecureCRT exports
            // but harmless — ignore it.
            if let Some((_, _, value)) = field.as_mut() {
                value.extend_from_slice(&rest[9..end]);
            }
            index += end + 3;
            continue;
        }
        if rest.starts_with(b"<?") || rest.starts_with(b"<!") {
            let Some(end) = find_subslice(rest, b">") else {
                return Err(format!("{ERR_PREFIX}: unterminated declaration"));
            };
            index += end + 1;
            continue;
        }
        if rest.starts_with(b"</") {
            let Some(end) = find_subslice(rest, b">") else {
                return Err(format!("{ERR_PREFIX}: unterminated closing tag"));
            };
            let name = std::str::from_utf8(&rest[2..end])
                .map_err(|_| format!("{ERR_PREFIX}: closing tag is not UTF-8"))?
                .trim();
            index += end + 1;
            if let Some((tag, field_name, value)) = field.take() {
                if tag != name {
                    return Err(format!("{ERR_PREFIX}: </{name}> closes <{tag}>"));
                }
                let text = decode_xml_entities(String::from_utf8_lossy(&value).trim())?;
                if let Some(frame) = stack.last_mut() {
                    frame.1.push((field_name, text));
                }
                continue;
            }
            if name == "key" {
                let Some((frame_name, fields)) = stack.pop() else {
                    return Err(format!("{ERR_PREFIX}: unbalanced </key>"));
                };
                let ancestors = stack.iter().map(|(name, _)| name.clone()).collect();
                if frames.len() >= MAX_PREVIEW_SESSIONS {
                    return Err(format!(
                        "Import exceeds the {MAX_PREVIEW_SESSIONS} session limit"
                    ));
                }
                frames.push(SecureCrtFrame {
                    name: frame_name,
                    fields,
                    ancestors,
                });
            }
            continue;
        }
        // Opening tag: find the closing '>' honoring quoted attributes.
        let Some(end) = find_tag_end(rest) else {
            return Err(format!("{ERR_PREFIX}: unterminated tag"));
        };
        let tag_text = std::str::from_utf8(&rest[1..end])
            .map_err(|_| format!("{ERR_PREFIX}: tag is not UTF-8"))?;
        index += end + 1;
        let self_closing = tag_text.ends_with('/');
        let (tag_name, attrs) = parse_xml_tag(tag_text.trim_end_matches('/'))?;
        if field.is_some() {
            return Err(format!(
                "{ERR_PREFIX}: unexpected <{tag_name}> inside a value"
            ));
        }
        match (tag_name, self_closing) {
            ("key", false) => {
                if stack.len() >= MAX_KEY_DEPTH {
                    return Err(format!(
                        "{ERR_PREFIX}: <key> nesting exceeds the {MAX_KEY_DEPTH} level limit"
                    ));
                }
                let name = xml_attr(&attrs, "name").unwrap_or_default().to_string();
                stack.push((name, Vec::new()));
            }
            ("key", true) => {}
            ("string" | "dword", _) => {
                let Some(name) = xml_attr(&attrs, "name").map(str::to_owned) else {
                    return Err(format!(
                        "{ERR_PREFIX}: <{tag_name}> value without a name attribute"
                    ));
                };
                if self_closing {
                    if let Some(frame) = stack.last_mut() {
                        frame.1.push((name, String::new()));
                    }
                } else {
                    // Tag names are matched against this static str when the
                    // closing tag arrives, so keep them in that domain.
                    let tag: &'static str = if tag_name == "string" {
                        "string"
                    } else {
                        "dword"
                    };
                    field = Some((tag, name, Vec::new()));
                }
            }
            // The `VanDyke` root element and unknown elements are ignored.
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(format!("{ERR_PREFIX}: unclosed <key> frame"));
    }
    Ok(frames)
}

/// Index of the `>` that terminates a tag, honoring quoted attribute values.
pub(super) fn find_tag_end(rest: &[u8]) -> Option<usize> {
    let mut quote = 0u8;
    for (offset, byte) in rest.iter().copied().enumerate() {
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = byte,
            b'>' => return Some(offset),
            _ => {}
        }
    }
    None
}

/// Attributes of one XML tag, decoded.
pub(super) type XmlAttrs = Vec<(String, String)>;

/// Splits `name attr="v" attr2='v2'` into the tag name and its attributes.
/// Attribute values must be quoted — SecureCRT always quotes them, and a
/// malformed export should fail closed rather than mis-parse.
pub(super) fn parse_xml_tag(tag_text: &str) -> Result<(&str, XmlAttrs), String> {
    let trimmed = tag_text.trim();
    let Some(name_end) = trimmed.find(|byte: char| byte.is_whitespace()) else {
        return Ok((trimmed, Vec::new()));
    };
    let name = &trimmed[..name_end];
    let mut attrs = Vec::new();
    let mut rest = trimmed[name_end..].trim_start();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else {
            return Err(format!(
                "Invalid SecureCRT XML: attribute without a value in <{name}>"
            ));
        };
        let attr_name = rest[..eq].trim().to_string();
        let after_eq = rest[eq + 1..].trim_start();
        let quote = after_eq.as_bytes().first().copied().unwrap_or(0);
        if quote != b'"' && quote != b'\'' {
            return Err(format!(
                "Invalid SecureCRT XML: unquoted attribute value in <{name}>"
            ));
        }
        let value_part = &after_eq[1..];
        let Some(close) = value_part.find(quote as char) else {
            return Err(format!(
                "Invalid SecureCRT XML: unterminated attribute in <{name}>"
            ));
        };
        attrs.push((attr_name, decode_xml_entities(&value_part[..close])?));
        rest = value_part[close + 1..].trim_start();
    }
    Ok((name, attrs))
}

pub(super) fn xml_attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// The five named entities plus decimal/hex numeric character references.
/// Unknown or unterminated entities fail closed — SecureCRT never emits them
/// and silently keeping the raw bytes would corrupt host names.
pub(super) fn decode_xml_entities(text: &str) -> Result<String, String> {
    if !text.contains('&') {
        return Ok(text.to_string());
    }
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find('&') {
        decoded.push_str(&rest[..position]);
        let after = &rest[position + 1..];
        let Some(semicolon) = after.find(';') else {
            return Err("Invalid SecureCRT XML: unterminated entity".to_string());
        };
        let entity = &after[..semicolon];
        let replacement = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity.strip_prefix('#').and_then(parse_xml_char_ref),
        };
        let Some(replacement) = replacement else {
            return Err(format!("Invalid SecureCRT XML: unknown entity &{entity};"));
        };
        decoded.push(replacement);
        rest = &after[semicolon + 1..];
    }
    decoded.push_str(rest);
    Ok(decoded)
}

/// `#22` / `#x16` → char; rejects surrogates and out-of-range code points.
pub(super) fn parse_xml_char_ref(body: &str) -> Option<char> {
    let value = if let Some(hex) = body.strip_prefix('x').or_else(|| body.strip_prefix('X')) {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        body.parse::<u32>().ok()?
    };
    char::from_u32(value)
}

/// Builds an imported session from one `<key>` frame. Returns `None` for
/// frames outside the `Sessions` tree, non-SSH2 protocols and sessions
/// without a usable host (bad entries degrade to skipped, like Xshell).
pub(super) fn securecrt_session_from_frame(frame: &SecureCrtFrame) -> Option<ImportedSession> {
    let sessions_index = frame.ancestors.iter().position(|name| name == "Sessions")?;
    let field = |name: &str| {
        frame
            .fields
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let protocol = field("Protocol Name").unwrap_or_default();
    if !protocol.eq_ignore_ascii_case("SSH2") {
        return None;
    }
    let host = field("Hostname")
        .map(str::trim)
        .filter(|host| !host.is_empty())?;
    // SSH2 sessions store the port under "[SSH2] Port" (plain "Port" for the
    // rest); both spellings are accepted. SecureCRT writes dwords as
    // zero-padded 8-digit lowercase hex (`00000016` = 22), so hex wins when
    // the shape matches, with a decimal fallback for hand-written exports
    // (the NyaTerm importer parses decimals only, which misreads real
    // exports).
    let port = field("[SSH2] Port")
        .or_else(|| field("Port"))
        .and_then(parse_securecrt_dword)
        .unwrap_or(22);
    let username = field("Username")
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .unwrap_or("root");
    let group_path: Vec<String> = frame.ancestors[sessions_index + 1..]
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    let mut session = ImportedSession::new(
        if frame.name.trim().is_empty() {
            host.to_string()
        } else {
            frame.name.clone()
        },
        host.to_string(),
        port,
        username.to_string(),
    );
    session.group_path = group_path;
    session.auth = ImportedAuth::Password { has_secret: false };
    session.secret_note = SECRET_NOTE_ENCRYPTED.to_string();
    Some(session)
}

/// SecureCRT dword values: zero-padded 8-digit lowercase hex first
/// (`00000016` = 22), decimal fallback otherwise. `None` for junk — the
/// caller falls back to port 22.
pub(super) fn parse_securecrt_dword(value: &str) -> Option<u16> {
    let trimmed = value.trim();
    if trimmed.len() == 8 && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        if let Ok(port) = u16::from_str_radix(trimmed, 16) {
            return (port > 0).then_some(port);
        }
    }
    trimmed.parse::<u16>().ok().filter(|port| *port > 0)
}
