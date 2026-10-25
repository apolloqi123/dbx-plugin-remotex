//! SSH auth support.

use super::*;
use crate::core::keys::decode_error::is_pem_base64_char;
pub(crate) use crate::core::keys::decode_error::private_key_decode_failure;

pub(crate) async fn authenticate_private_key_result(
    session: &mut Handle<SshClient>,
    connection: &StoredConnection,
) -> Result<AuthResult, String> {
    let key_text = resolve_private_key_text(connection).await?;
    // D9: passphrase_command 接入点——先用既有口令（或无口令）尝试解码；
    // 仅当密钥确实需要口令且命令已配置时，本地执行命令取回并重试一次。
    // 未加密密钥永远不会触发命令执行。
    let stored_passphrase = (!connection.private_key_passphrase.is_empty())
        .then_some(connection.private_key_passphrase.as_str());
    let decoded = match decode_secret_key(&key_text, stored_passphrase) {
        Ok(private_key) => Ok(private_key),
        Err(stored_error) => {
            if stored_passphrase.is_none() && !connection.passphrase_command.is_empty() {
                match resolve_passphrase_command(connection).await {
                    Some(passphrase) => decode_secret_key(&key_text, Some(&passphrase)),
                    // 命令失败：保留原始解码错误，认证链照常报错。
                    None => Err(stored_error),
                }
            } else {
                Err(stored_error)
            }
        }
    };
    let private_key = decoded.map_err(|error| {
        private_key_decode_failure(&key_text, &error, stored_passphrase.is_some())
    })?;
    let hash = session
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten();
    tokio::time::timeout(
        Duration::from_secs(connection.connect_timeout_secs),
        session.authenticate_publickey(
            &connection.username,
            PrivateKeyWithHashAlg::new(Arc::new(private_key), hash),
        ),
    )
    .await
    .map_err(|_| "SSH private-key authentication timed out".to_string())?
    .map_err(|error| format!("SSH private-key authentication failed: {error}"))
}

/// Private key source resolution for authentication: pasted key content
/// (`connection_secrets.private_key`) wins over the key path, mirroring the
/// form's either-or contract. Common Windows text artifacts (CRLF, UTF-8 BOM,
/// and leading whitespace) are normalized so keys pasted from Windows editors
/// or stored with Windows line endings still decode.
pub(crate) async fn resolve_private_key_text(
    connection: &StoredConnection,
) -> Result<String, String> {
    if !connection.private_key.is_empty() {
        return Ok(normalize_private_key_text(&connection.private_key));
    }
    let key_path = expand_private_key_path(&connection.private_key_path);
    let text = tokio::fs::read_to_string(&key_path)
        .await
        .map_err(|error| {
            format!(
                "Failed to read SSH private key '{}': {error}",
                key_path.display()
            )
        })?;
    Ok(normalize_private_key_text(&text))
}

/// Normalizes text artifacts around a PEM/PPK key. `russh` matches PEM begin
/// markers exactly at the start of a line, so a UTF-8 BOM or indentation on
/// the first line otherwise becomes the opaque `Could not read key` error.
/// Beyond the original CRLF/BOM/indent handling this also strips paste-time
/// pollution that can never occur in a real key file: zero-width characters,
/// `<br>` tags from web-page copies, trailing whitespace on marker lines,
/// smart dashes, and whitespace inside base64 body lines (terminal copy that
/// re-wrapped the text). Marker lines are rebuilt into their canonical
/// `-----BEGIN … -----` form. No key semantics change: directive lines such
/// as `DEK-Info:` are preserved as-is.
pub(crate) fn normalize_private_key_text(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.strip_prefix('\u{feff}').unwrap_or(&normalized);
    let cleaned = strip_invisible_paste_artifacts(normalized);
    let mut rebuilt: Vec<String> = Vec::new();
    let mut in_pem_body = false;
    for line in cleaned.split('\n') {
        let trimmed = line.trim();
        // Drop leading blank/indent-only lines, like the previous whole-text
        // trim_start did.
        if rebuilt.is_empty() && trimmed.is_empty() {
            continue;
        }
        if looks_like_marker_line(trimmed) {
            rebuilt.push(normalize_pem_marker_line(trimmed));
            in_pem_body = trimmed.contains("BEGIN");
            continue;
        }
        if in_pem_body {
            let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
            if !compact.is_empty() && compact.chars().all(is_pem_base64_char) {
                rebuilt.push(compact);
                continue;
            }
        }
        rebuilt.push(trimmed.to_string());
    }
    // Trailing per-line whitespace is already gone; keep the original tail
    // (including its final newline) so the resolved text stays byte-identical
    // apart from the pollution fixes.
    rebuilt.join("\n")
}

/// Removes characters and tags that only arrive via rich-text or web-page
/// copying: zero-width space/joiners, stray mid-text BOMs, and `<br>` tags
/// in any letter case. A valid key file never contains any of them, so the
/// removal cannot alter real key content.
pub(crate) fn strip_invisible_paste_artifacts(text: &str) -> String {
    let filtered: String = text
        .chars()
        .filter(|c| !matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{feff}'))
        .collect();
    // ASCII folding only: Unicode lowercasing can change byte lengths (e.g.
    // `İ`), which would shift the mask offsets and corrupt the removals —
    // the tags themselves are pure ASCII.
    let lower = filtered.to_ascii_lowercase();
    let mut remove = vec![false; filtered.len()];
    for tag in ["<br />", "<br/>", "<br>"] {
        let mut from = 0;
        while let Some(offset) = lower[from..].find(tag) {
            let start = from + offset;
            for byte in remove.iter_mut().skip(start).take(tag.len()) {
                *byte = true;
            }
            from = start + tag.len();
        }
    }
    let mut out = String::with_capacity(filtered.len());
    for (index, c) in filtered.char_indices() {
        if !remove[index] {
            out.push(c);
        }
    }
    out
}

/// ASCII base64 alphabet plus the padding sign, as PEM bodies use it.
/// Marker-line detection that also accepts smart dashes, which rich-text
/// editors substitute for the ASCII hyphens of `-----BEGIN … -----`.
pub(crate) fn looks_like_marker_line(line: &str) -> bool {
    let starts_with_dash = line
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '-' | '\u{2013}' | '\u{2014}' | '\u{2212}'));
    starts_with_dash && (line.contains("BEGIN") || line.contains("END"))
}

/// Rebuilds a PEM begin/end marker into the canonical five-dash form, fixing
/// smart dashes (— – −), doubled inner spaces, and trailing whitespace, all
/// of which make `russh`'s exact marker match fail with `Could not read key`.
pub(crate) fn normalize_pem_marker_line(line: &str) -> String {
    let ascii_dashes: String = line
        .chars()
        .map(|c| match c {
            '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            other => other,
        })
        .collect();
    let mut collapsed = String::with_capacity(ascii_dashes.len());
    let mut previous_was_space = false;
    for c in ascii_dashes.chars() {
        if c.is_whitespace() {
            if !previous_was_space {
                collapsed.push(' ');
                previous_was_space = true;
            }
        } else {
            collapsed.push(c);
            previous_was_space = false;
        }
    }
    let core = collapsed.trim().trim_matches('-').trim();
    if core.starts_with("BEGIN") || core.starts_with("END") {
        format!("-----{core}-----")
    } else {
        collapsed
    }
}

pub(crate) fn expand_private_key_path(path: &str) -> PathBuf {
    let Some(remainder) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) else {
        return PathBuf::from(path);
    };
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(remainder)
}
