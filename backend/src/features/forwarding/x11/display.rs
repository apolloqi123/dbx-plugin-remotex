//! Metrics support.

use super::*;

/// Upper bound for a display number: the TCP form maps to port `6000 + n`,
/// which must stay a valid `u16`.
pub(crate) const MAX_DISPLAY_NUMBER: u32 = u16::MAX as u32 - 6000;

/// Local X server endpoint resolved from a DISPLAY string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisplayTarget {
    /// Display number (`:10.0` -> 10). The screen suffix is ignored: the
    /// bridged connection carries the client's own screen selection.
    pub(crate) number: u32,
    /// Where the local X server listens.
    pub(crate) server: DisplayServer,
}

/// Where the local X server listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DisplayServer {
    /// Preferred transport: AF_UNIX socket (the X(7) `/tmp/.X11-unix/X<n>`
    /// convention, or an explicit path — macOS launchd hands out per-boot
    /// socket paths, Windows has no unix sockets).
    UnixSocket(PathBuf),
    /// `host:n` form: pinned to loopback `6000 + n`. The DISPLAY hostname is
    /// deliberately ignored so a remote-influenced value cannot steer the
    /// local dial away from 127.0.0.1 (spike §3.5).
    TcpLoopback(u16),
}

/// Parses a DISPLAY value into the local endpoint to bridge to (X(7)):
/// `:0[.s]`, `unix:0`, `unix:/path[:0]` and `/path:0` (macOS launchd) are
/// unix sockets; `host:0[.s]` is TCP loopback `6000 + n`. `None` on
/// anything unparsable — callers surface that as "no local X server"
/// (macOS: XQuartz, Windows: VcXsrv).
pub(crate) fn parse_display(display: &str) -> Option<DisplayTarget> {
    let text = display.trim();
    // `unix:` prefix (X(7)): a bare number, or an explicit socket path with
    // an optional display-number suffix.
    if let Some(rest) = text.strip_prefix("unix:") {
        if rest.is_empty() {
            return Some(unix_display(0));
        }
        if let Some(path) = rest.strip_prefix('/') {
            let (path, number) = match path.rsplit_once(':') {
                Some((head, number)) => (head, number),
                None => (path, ""),
            };
            let number = if number.is_empty() {
                0
            } else {
                parse_display_number(number)?
            };
            return Some(DisplayTarget {
                number,
                server: DisplayServer::UnixSocket(PathBuf::from(format!("/{path}"))),
            });
        }
        let number = parse_display_number(rest)?;
        return Some(unix_display(number));
    }
    // General form `[host]:number[.screen]`. Splitting on the LAST colon
    // keeps explicit socket paths (`/tmp/launchd…/org.xquartz:0`) intact.
    let (host, number) = text.rsplit_once(':')?;
    let number = parse_display_number(number)?;
    if host.is_empty() {
        return Some(unix_display(number));
    }
    // Explicit unix socket path in host position (macOS launchd DISPLAY).
    if host.starts_with('/') {
        return Some(DisplayTarget {
            number,
            server: DisplayServer::UnixSocket(PathBuf::from(host)),
        });
    }
    let port = u16::try_from(6000u32.checked_add(number)?).ok()?;
    Some(DisplayTarget {
        number,
        server: DisplayServer::TcpLoopback(port),
    })
}

pub(crate) fn unix_display(number: u32) -> DisplayTarget {
    DisplayTarget {
        number,
        server: DisplayServer::UnixSocket(PathBuf::from(format!("/tmp/.X11-unix/X{number}"))),
    }
}

/// `10` or `10.0` -> 10. Digits only, bounded so `6000 + n` stays a u16.
pub(crate) fn parse_display_number(text: &str) -> Option<u32> {
    let (number, screen) = match text.split_once('.') {
        Some((number, screen)) => (number, screen),
        None => (text, ""),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(number) || !(screen.is_empty() || digits(screen)) {
        return None;
    }
    let number: u32 = number.parse().ok()?;
    (number <= MAX_DISPLAY_NUMBER).then_some(number)
}
