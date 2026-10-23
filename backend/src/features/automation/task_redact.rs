//! Task provider redact layer.

use super::*;

// ---------------------------------------------------------------------------
// Secret redaction (ADR §10: logs/events never carry credential material)
// ---------------------------------------------------------------------------

/// Accumulates connection secret material and scrubs it from every emitted
/// message. Values shorter than 4 chars are ignored (redacting "a" would
/// mangle normal text while leaking nothing meaningful).
#[derive(Default)]
pub(crate) struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub(super) fn add(&mut self, secret: &str) {
        let secret = secret.trim();
        if secret.chars().count() >= 4 && !self.secrets.iter().any(|s| s == secret) {
            self.secrets.push(secret.to_string());
        }
    }

    /// Collects every credential the connect pipeline holds for this
    /// connection, including jump-host secrets (their dial errors never
    /// embed them, but defense in depth is free here).
    pub(crate) fn from_connection(connection: &crate::core::model::StoredConnection) -> Self {
        let mut redactor = Self::default();
        redactor.add(&connection.password);
        redactor.add(&connection.sudo_password);
        redactor.add(&connection.totp_secret);
        redactor.add(&connection.private_key_passphrase);
        redactor.add(&connection.private_key);
        for jump in &connection.jump_hosts {
            redactor.add(&jump.password);
            redactor.add(&jump.private_key_passphrase);
            redactor.add(&jump.totp_secret);
        }
        redactor
    }

    pub(crate) fn scrub(&self, text: &str) -> String {
        let mut text = text.to_string();
        for secret in &self.secrets {
            if text.contains(secret.as_str()) {
                text = text.replace(secret.as_str(), "[redacted]");
            }
        }
        text
    }
}

// ---------------------------------------------------------------------------
// Log plumbing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogStream {
    Stdout,
    Stderr,
    System,
}

impl LogStream {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::System => "system",
        }
    }

    /// Default level per the contract's `debug|info|warn|error` vocabulary:
    /// remote stderr is `warn` (frequent and benign for `tail -F`-style
    /// commands), not `error`.
    pub(super) fn default_level(&self) -> &'static str {
        match self {
            Self::Stderr => "warn",
            _ => "info",
        }
    }
}

/// Formats a unix timestamp as RFC3339 UTC (`1970-01-01T00:00:00Z` style).
/// chrono is not a dependency of this crate, and the contract's event shape
/// wants a self-describing timestamp on every log line.
pub(crate) fn rfc3339(timestamp_secs: u64) -> String {
    let days = (timestamp_secs / 86_400) as i64;
    let secs_of_day = timestamp_secs % 86_400;
    // Howard Hinnant's civil_from_days: days since epoch → y/m/d (proleptic
    // Gregorian, valid for the whole u64 second range we can ever see).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60
    )
}

pub(super) fn rfc3339_now() -> String {
    rfc3339(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

/// Line-oriented buffering for a channel stream: complete lines are emitted
/// as they complete; a partial line longer than [`PARTIAL_LINE_LIMIT`] is
/// flushed early so progress stays visible.
#[derive(Default)]
pub(super) struct LineBuffer {
    pending: Vec<u8>,
}

impl LineBuffer {
    pub(super) fn push(&mut self, data: &[u8], mut emit: impl FnMut(String)) {
        self.pending.extend_from_slice(data);
        while let Some(position) = self.pending.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..position).collect();
            self.pending.remove(0); // the '\n'
            emit(String::from_utf8_lossy(&line).into_owned());
        }
        if self.pending.len() >= PARTIAL_LINE_LIMIT {
            let drained = std::mem::take(&mut self.pending);
            emit(String::from_utf8_lossy(&drained).into_owned());
        }
    }

    pub(super) fn flush(&mut self, mut emit: impl FnMut(String)) {
        if !self.pending.is_empty() {
            let drained = std::mem::take(&mut self.pending);
            emit(String::from_utf8_lossy(&drained).into_owned());
        }
    }
}
