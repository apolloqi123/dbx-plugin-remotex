//! Clipboard URI list codecs shared by the platform backends.
//!
//! Linux clipboard owners publish file lists either as `text/uri-list`
//! (RFC 2483) or as GNOME's `x-special/gnome-copied-files` (a `copy`/`cut`
//! header followed by the same URI lines). Windows uses CF_HDROP instead, but
//! its payload is built here too so the encoder is unit-tested on every host.
//!
//! Only absolute `file://` URIs are accepted; `#` comments and blank lines are
//! ignored. Nothing here reads file contents or logs paths.

use std::path::{Path, PathBuf};

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_component(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            out.push((high << 4) | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn encode_path(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        let keep = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/');
        if keep {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn path_from_file_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let (host, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    let decoded = decode_component(path)?;
    // Windows clipboard URIs look like file:///C:/Users/x; Explorer needs the
    // native separator back. Other hosts only accept POSIX-style absolute paths.
    let decoded = if cfg!(windows) {
        decoded
            .strip_prefix('/')
            .unwrap_or(&decoded)
            .replace('/', "\\")
    } else {
        decoded
    };
    let path = PathBuf::from(decoded);
    path.is_absolute().then_some(path)
}

/// Absolute paths from a `text/uri-list` / GNOME copied-files payload.
pub(crate) fn paths_from_uri_text(text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // GNOME header line: "copy" or "cut".
        if line.eq_ignore_ascii_case("copy") || line.eq_ignore_ascii_case("cut") {
            continue;
        }
        if let Some(path) = path_from_file_uri(line) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

fn file_uri(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let absolute = if normalized.starts_with('/') {
        normalized
    } else {
        format!("/{normalized}")
    };
    format!("file://{}", encode_path(&absolute))
}

/// RFC 2483 payload (CRLF-terminated lines).
pub(crate) fn uri_list_from_paths(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        out.push_str(&file_uri(path));
        out.push_str("\r\n");
    }
    out
}

/// GNOME `x-special/gnome-copied-files` payload (`copy` + LF-terminated URIs).
pub(crate) fn gnome_copied_files_from_paths(paths: &[PathBuf]) -> String {
    let mut out = String::from("copy\n");
    for path in paths {
        out.push_str(&file_uri(path));
        out.push('\n');
    }
    out
}

#[cfg_attr(not(windows), allow(dead_code))]
fn path_units(path: &Path) -> Vec<u16> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().collect()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().encode_utf16().collect()
    }
}

/// CF_HDROP payload: a 20-byte DROPFILES header (`pFiles=20`, `fWide=1`)
/// followed by NUL-terminated UTF-16 paths and a final NUL.
/// Only the Windows backend encodes this; other hosts still unit-test it.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn build_dropfiles_payload(paths: &[PathBuf]) -> Vec<u8> {
    const HEADER_LEN: usize = 20;
    let mut units: Vec<u16> = Vec::new();
    for path in paths {
        units.extend(path_units(path));
        units.push(0);
    }
    units.push(0);
    let mut payload = Vec::with_capacity(HEADER_LEN + units.len() * 2);
    payload.extend_from_slice(&(HEADER_LEN as u32).to_le_bytes());
    payload.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    payload.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    payload.extend_from_slice(&0i32.to_le_bytes()); // fNC
    payload.extend_from_slice(&1i32.to_le_bytes()); // fWide
    for unit in units {
        payload.extend_from_slice(&unit.to_le_bytes());
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn parses_uri_list_comments_headers_and_crlf() {
        let text = "# comment\r\n\r\ncopy\r\nfile:///tmp/a.txt\r\nfile:///tmp/sp%20ace.txt\r\n";
        assert_eq!(
            paths_from_uri_text(text),
            paths(&["/tmp/a.txt", "/tmp/sp ace.txt"])
        );
    }

    #[test]
    fn ignores_remote_hosts_relative_and_non_file_uris() {
        let text =
            "file://remote/tmp/a\nhttp://example.com/x\n/tmp/relative\nfile://localhost/tmp/b\n";
        assert_eq!(paths_from_uri_text(text), paths(&["/tmp/b"]));
    }

    #[test]
    fn round_trips_posix_and_unicode_paths() {
        let source = paths(&["/tmp/a b/文件.txt", "/tmp/100%.txt"]);
        let encoded = uri_list_from_paths(&source);
        assert!(encoded.contains("file:///tmp/a%20b/%E6%96%87%E4%BB%B6.txt"));
        assert!(encoded.contains("file:///tmp/100%25.txt"));
        assert_eq!(paths_from_uri_text(&encoded), source);
    }

    #[test]
    fn gnome_payload_has_copy_header_and_round_trips() {
        let source = paths(&["/tmp/a.txt"]);
        let encoded = gnome_copied_files_from_paths(&source);
        assert_eq!(encoded, "copy\nfile:///tmp/a.txt\n");
        assert_eq!(paths_from_uri_text(&encoded), source);
    }

    #[test]
    fn dropfiles_payload_is_wide_and_nul_terminated() {
        let payload = build_dropfiles_payload(&paths(&["/tmp/a.txt", "/tmp/b.txt"]));
        assert_eq!(&payload[..4], 20u32.to_le_bytes());
        assert_eq!(&payload[16..20], 1i32.to_le_bytes());
        let units: Vec<u16> = payload[20..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let names: Vec<String> = units
            .split(|unit| *unit == 0)
            .filter(|chunk| !chunk.is_empty())
            .map(String::from_utf16_lossy)
            .collect();
        assert_eq!(names, ["/tmp/a.txt", "/tmp/b.txt"]);
        assert_eq!(units.last(), Some(&0));
    }

    #[test]
    fn dropfiles_payload_keeps_unicode_names() {
        let payload = build_dropfiles_payload(&paths(&["/tmp/文件.txt"]));
        let units: Vec<u16> = payload[20..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert!(units.contains(&(0x6587)));
    }
}
