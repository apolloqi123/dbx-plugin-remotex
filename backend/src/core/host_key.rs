use std::io;
use std::path::PathBuf;

use russh::keys::known_hosts::{check_known_hosts_path, learn_known_hosts_path};
use russh::keys::ssh_key::PublicKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyState {
    Trusted,
    Unknown,
}

/// One entry of the plugin's known_hosts store, in OpenSSH wire layout.
/// `marker` carries the OpenSSH marker of `@cert-authority` / `@revoked`
/// lines (the token before the host field); plain entries have `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownHostEntry {
    pub host_field: String,
    pub key_type: String,
    pub fingerprint: String,
    pub comment: String,
    pub marker: Option<String>,
}

pub struct HostKeyVerifier {
    known_hosts_path: PathBuf,
}

/// OpenSSH marker tokens that precede the host field on a known_hosts line.
const KNOWN_HOSTS_MARKERS: [&str; 2] = ["@cert-authority", "@revoked"];

/// Lines longer than this are treated as corrupt and skipped: a real
/// known_hosts line (marker + host + key + comment) stays far below, while a
/// megabyte-sized junk line must not be parsed, listed, or rewritten.
const MAX_KNOWN_HOSTS_LINE_LEN: usize = 8192;

fn is_known_hosts_marker(token: &str) -> bool {
    KNOWN_HOSTS_MARKERS.contains(&token)
}

/// Splits one known_hosts line into `(marker, host_field)`; `marker` is
/// `Some` only for genuine `@cert-authority` / `@revoked` lines. Returns
/// `None` for blank/comment lines, corrupt marker fragments, and overlong
/// junk lines.
fn split_known_hosts_line(line: &str) -> Option<(Option<String>, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.len() > MAX_KNOWN_HOSTS_LINE_LEN {
        return None;
    }
    let mut parts = line.split_whitespace();
    let first_token = parts.next()?;
    let (marker, host_field) = if is_known_hosts_marker(first_token) {
        let host_field = parts.next()?;
        (Some(first_token.to_string()), host_field)
    } else {
        (None, first_token)
    };
    if host_field.starts_with('@') || host_field == "|" {
        // Stray marker/hashed fragments without a host field are unusable.
        return None;
    }
    Some((marker, host_field.to_string()))
}

impl HostKeyVerifier {
    pub fn new(known_hosts_path: PathBuf) -> Self {
        Self { known_hosts_path }
    }

    /// Lists the entries of the plugin's own known_hosts store (the system
    /// ~/.ssh/known_hosts stays read-only).
    pub fn list_known_hosts(&self) -> Result<Vec<KnownHostEntry>, String> {
        let content = match std::fs::read_to_string(&self.known_hosts_path) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!(
                    "Failed to read {}: {error}",
                    self.known_hosts_path.display()
                ))
            }
        };
        let mut entries = Vec::new();
        for line in content.lines() {
            let Some((marker, host_field)) = split_known_hosts_line(line) else {
                continue;
            };
            let mut parts = line.split_whitespace();
            // Skip the marker (if any) and the host field; the remainder is
            // key type, key data, and the optional comment.
            let skip = usize::from(marker.is_some()) + 1;
            for _ in 0..skip {
                parts.next();
            }
            let (Some(key_type), Some(key_data)) = (parts.next(), parts.next()) else {
                continue;
            };
            let comment = parts.collect::<Vec<_>>().join(" ");
            let fingerprint = PublicKey::from_openssh(&format!("{key_type} {key_data}"))
                .ok()
                .map(|key| {
                    key.fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
                        .to_string()
                })
                .unwrap_or_default();
            entries.push(KnownHostEntry {
                host_field,
                key_type: key_type.to_string(),
                fingerprint,
                comment,
                marker,
            });
        }
        Ok(entries)
    }

    /// Removes entries matching `host:port` (or bare `host` for port 22)
    /// from the plugin's known_hosts store. Returns how many lines were
    /// removed; a changed key record disappears with the same match.
    pub fn remove_known_host(&self, host: &str, port: u16) -> Result<usize, String> {
        let content = match std::fs::read_to_string(&self.known_hosts_path) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(format!(
                    "Failed to read {}: {error}",
                    self.known_hosts_path.display()
                ))
            }
        };
        let patterns = if port == 22 {
            [host.to_string(), format!("[{host}]:22")]
        } else {
            [format!("[{host}]:{port}"), String::new()]
        };
        let mut kept = Vec::new();
        let mut removed = 0_usize;
        for line in content.lines() {
            // Overlong/corrupt lines are never matched: they stay in the file
            // untouched instead of being removed by an accidental pattern hit.
            if line.len() > MAX_KNOWN_HOSTS_LINE_LEN {
                kept.push(line);
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                kept.push(line);
                continue;
            }
            let mut tokens = trimmed.split_whitespace();
            let first = tokens.next().unwrap_or_default();
            // Only genuine OpenSSH markers move the host into the second
            // token; any other leading token is treated as the host itself.
            let host_token = if is_known_hosts_marker(first) {
                tokens.next().unwrap_or_default()
            } else {
                first
            };
            if patterns
                .iter()
                .any(|pattern| !pattern.is_empty() && host_token == pattern)
            {
                removed += 1;
                continue;
            }
            kept.push(line);
        }
        if removed == 0 {
            return Ok(0);
        }
        let mut output = kept.join("\n");
        if !output.is_empty() {
            output.push('\n');
        }
        std::fs::write(&self.known_hosts_path, output).map_err(|error| {
            format!(
                "Failed to write {}: {error}",
                self.known_hosts_path.display()
            )
        })?;
        Ok(removed)
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> Result<HostKeyState, io::Error> {
        let system = system_known_hosts_path()
            .map(|path| check_known_hosts_file(host, port, key, &path))
            .transpose()?
            .unwrap_or(HostKeyState::Unknown);
        // Always evaluate both stores. A changed key in either store must win
        // over a matching entry in the other store.
        let plugin = check_known_hosts_file(host, port, key, &self.known_hosts_path)?;
        Ok(
            if system == HostKeyState::Trusted || plugin == HostKeyState::Trusted {
                HostKeyState::Trusted
            } else {
                HostKeyState::Unknown
            },
        )
    }

    pub fn learn(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), io::Error> {
        if let Some(parent) = self.known_hosts_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        learn_known_hosts_path(host, port, key, &self.known_hosts_path).map_err(|error| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "Failed to persist host key for {host}:{port} to {}: {error}",
                    self.known_hosts_path.display()
                ),
            )
        })
    }
}

fn check_known_hosts_file(
    host: &str,
    port: u16,
    key: &PublicKey,
    path: &std::path::Path,
) -> Result<HostKeyState, io::Error> {
    match path.try_exists() {
        Ok(false) => return Ok(HostKeyState::Unknown),
        Ok(true) => {}
        Err(error) => return Err(error),
    }
    // russh treats every File::open failure as an empty known_hosts file. Probe
    // readability first so permission and I/O failures cannot silently disable
    // host-key verification.
    std::fs::File::open(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "Failed to read known_hosts file {}: {error}",
                path.display()
            ),
        )
    })?;
    match check_known_hosts_path(host, port, key, path) {
        Ok(true) => Ok(HostKeyState::Trusted),
        Err(russh::keys::Error::KeyChanged { line }) => Err(host_key_changed_error(
            host,
            port,
            line,
            &path.display().to_string(),
        )),
        Ok(false) => Ok(HostKeyState::Unknown),
        Err(error) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Failed to parse known_hosts file {}: {error}",
                path.display()
            ),
        )),
    }
}

fn system_known_hosts_path() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join(".ssh").join("known_hosts"))
}

fn host_key_changed_error(host: &str, port: u16, line: usize, store: &str) -> io::Error {
    io::Error::other(format!(
        "Host key for {host}:{port} changed (recorded at {store}, line {line}). This may indicate a man-in-the-middle attack."
    ))
}

#[cfg(test)]
mod tests;
