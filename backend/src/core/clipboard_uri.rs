//! Parsing for X11 clipboard file lists (`text/uri-list`,
//! `x-special/gnome-copied-files`). Used by the VNC worker (RFB text payloads
//! sometimes carry the URI list) and by the SSH-side clipboard probe.

const MAX_PATHS: usize = 200;
const MAX_LINE_BYTES: usize = 64 * 1024;

/// Extracts absolute remote paths from a URI list. Plain text is deliberately
/// ignored: copying `/etc/hosts` as text must not trigger a download.
pub(crate) fn parse_uri_list(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') || line == "copy" || line == "cut" {
            continue;
        }
        if line.len() > MAX_LINE_BYTES {
            continue;
        }
        let Some(path) = file_uri_path(line) else {
            continue;
        };
        if path.len() < 2 || path == "/" || path.contains('\0') {
            continue;
        }
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
        if paths.len() >= MAX_PATHS {
            break;
        }
    }
    paths
}

fn file_uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///abs` (empty authority) and `file://host/abs`.
    let path = if rest.starts_with('/') {
        rest
    } else {
        let slash = rest.find('/')?;
        &rest[slash..]
    };
    if !path.starts_with('/') {
        return None;
    }
    Some(percent_decode(path))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gnome_and_uri_list_shapes() {
        assert_eq!(
            parse_uri_list("copy\nfile:///home/apollo/.bash_history"),
            ["/home/apollo/.bash_history"]
        );
        assert_eq!(
            parse_uri_list("file://localhost/tmp/a%20b.txt\r\nfile://h/%E4%B8%AD.txt"),
            ["/tmp/a b.txt", "/中.txt"]
        );
    }

    #[test]
    fn ignores_plain_text_and_duplicates() {
        assert!(parse_uri_list("/etc/hosts").is_empty());
        assert_eq!(parse_uri_list("copy\n# comment\nfile:///a\nfile:///a"), ["/a"]);
    }

    #[test]
    fn rejects_root_and_bounds_the_list() {
        assert!(parse_uri_list("file:///").is_empty());
        let text = (0..MAX_PATHS + 20)
            .map(|index| format!("file:///tmp/f{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(parse_uri_list(&text).len(), MAX_PATHS);
    }
}
