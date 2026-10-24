//! TOTP secret parsing, code generation and OTP replay/rotation ledgers.

use super::*;

#[derive(Debug, Clone)]
pub enum TotpSecret {
    /// RFC 6238 shared secret with parameters.
    Key {
        key: Vec<u8>,
        digits: u32,
        period: u64,
        algorithm: TotpAlgorithm,
    },
    /// Static numeric code (4-10 digits) that is not time-based.
    Static(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

/// One OTP selection with the data the replay guard needs.
pub(in crate::core::exec) struct TotpSelection {
    /// Ledger-key prefix identifying the secret the code came from.
    pub(in crate::core::exec) fingerprint: String,
    pub(in crate::core::exec) code: String,
    /// Unix seconds after which the code's own window expires.
    pub(in crate::core::exec) valid_until: u64,
    /// Secret period (or the static fallback window).
    pub(super) period: u64,
}

/// Process-global ledger of OTP selections, keyed by usage key (see
/// [`current_totp_selection`]) with the selection's window expiry as the
/// value. Global — not per `SudoAuth` — because MCP exec calls resolve a
/// fresh instance per call while rotation must survive across calls, which
/// mirrors tiny-rdm's service-level `markOTPUsage`; in-memory only, cleared
/// on sidecar restart.
pub(in crate::core::exec) fn otp_usage_ledger() -> &'static Mutex<HashMap<String, u64>> {
    static LEDGER: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Process-global record of codes already auto-submitted, keyed by
/// `scope|secret-fingerprint|code` with their replay-window expiry
/// (`validUntil + period`, i.e. including the ±1-step acceptance slack).
/// Global for the same reason as [`otp_usage_ledger`].
pub(in crate::core::exec) fn committed_otp_ledger() -> &'static Mutex<HashMap<String, u64>> {
    static LEDGER: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Serializes tests that touch the process-global OTP ledgers and hands
/// each one a clean slate: hold the returned guard for the whole test so
/// parallel tests neither share marks nor wipe each other mid-run.
#[cfg(test)]
pub(in crate::core::exec) fn otp_ledger_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: OnceLock<Mutex<()>> = OnceLock::new();
    let serial = SERIAL.get_or_init(|| Mutex::new(()));
    let guard = serial.lock().unwrap_or_else(|poison| poison.into_inner());
    otp_usage_ledger()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clear();
    committed_otp_ledger()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clear();
    guard
}

/// Stable ledger-key prefix for a secret: a short SHA-256 fingerprint of
/// its canonical form (key material, digits, period, algorithm — or the
/// static code), so marks written by one resolved `SudoAuth` match lookups
/// from the next call regardless of how the secret list was ordered or
/// merged. Only the fingerprint is stored, never the secret.
pub(in crate::core::exec) fn otp_secret_fingerprint(secret: &TotpSecret) -> String {
    let canonical = match secret {
        TotpSecret::Key {
            key,
            digits,
            period,
            algorithm,
        } => format!("k:{}:{digits}:{period}:{algorithm:?}", HEXLOWER.encode(key)),
        TotpSecret::Static(code) => format!("s:{code}"),
    };
    let digest = Sha256::digest(canonical.as_bytes());
    HEXLOWER.encode(&digest)[..16].to_string()
}

/// Ledger scope for the connection a resolved [`SudoAuth`] serves: the OTP
/// usage/committed marks are keyed per target so back-to-back calls against
/// the same host rotate across their shared ledger while a second host
/// reusing the same secret is never blocked by the first host's history.
pub fn otp_ledger_scope_for(username: &str, host: &str, port: u16) -> String {
    format!("{username}@{host}:{port}")
}

/// Replay-window expiry for a committed code: its nominal validity plus one
/// extra step, because servers commonly accept the previous and next
/// window's code alongside the current one (the ±1-step slack).
pub fn totp_replay_window_expiry(valid_until: u64, period: u64) -> u64 {
    valid_until.saturating_add(period)
}

/// True while a committed code must not be auto-submitted again.
pub fn is_totp_in_replay_window(now: u64, committed_until: u64) -> bool {
    now <= committed_until
}

pub(in crate::core::exec) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

/// Parses one or more TOTP secrets separated by newlines or semicolons
/// (tiny-rdm's multi-secret `TOTPSecretRefs` equivalent).
pub fn parse_totp_secrets(input: &str) -> Vec<TotpSecret> {
    input
        .split(['\n', '\r', ';'])
        .filter_map(parse_totp_secret)
        .collect()
}

/// Parses a TOTP secret reference: an `otpauth://totp/…` URI, a base32 key,
/// or a static numeric code. Returns `None` for empty or unusable values.
pub fn parse_totp_secret(input: &str) -> Option<TotpSecret> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed
        .strip_prefix("otpauth://")
        .or_else(|| trimmed.strip_prefix("OTPAUTH://"))
    {
        return parse_otpauth(rest);
    }
    if is_numeric_otp(trimmed) {
        return Some(TotpSecret::Static(trimmed.to_string()));
    }
    let key = BASE32
        .decode(trimmed.to_ascii_uppercase().as_bytes())
        .ok()?;
    if key.is_empty() {
        return None;
    }
    Some(TotpSecret::Key {
        key,
        digits: 6,
        period: 30,
        algorithm: TotpAlgorithm::Sha1,
    })
}

pub(in crate::core::exec) fn parse_otpauth(rest: &str) -> Option<TotpSecret> {
    let (scheme_path, query) = rest.split_once('?').unwrap_or((rest, ""));
    if !scheme_path
        .split('/')
        .next()
        .is_some_and(|label| label.eq_ignore_ascii_case("totp"))
    {
        return None;
    }
    let mut secret = None;
    let mut digits = 6_u32;
    let mut period = 30_u64;
    let mut algorithm = TotpAlgorithm::Sha1;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "secret" => {
                secret = BASE32
                    .decode(value.to_ascii_uppercase().as_bytes())
                    .ok()
                    .filter(|decoded| !decoded.is_empty());
            }
            "digits" => digits = value.parse().ok().filter(|d| (6..=8).contains(d))?,
            "period" => period = value.parse().ok().filter(|&p| (1..=120).contains(&p))?,
            "algorithm" => {
                algorithm = match value.to_ascii_uppercase().as_str() {
                    "SHA256" => TotpAlgorithm::Sha256,
                    "SHA512" => TotpAlgorithm::Sha512,
                    _ => TotpAlgorithm::Sha1,
                };
            }
            _ => {}
        }
    }
    Some(TotpSecret::Key {
        key: secret?,
        digits,
        period,
        algorithm,
    })
}

pub(in crate::core::exec) fn is_numeric_otp(input: &str) -> bool {
    (4..=10).contains(&input.len()) && input.bytes().all(|byte| byte.is_ascii_digit())
}

pub(in crate::core::exec) fn hotp(
    key: &[u8],
    counter: u64,
    digits: u32,
    algorithm: TotpAlgorithm,
) -> String {
    let message = counter.to_be_bytes();
    let digest: Vec<u8> = match algorithm {
        TotpAlgorithm::Sha1 => {
            let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(key)
                .expect("HMAC accepts keys of any length");
            mac.update(&message);
            mac.finalize().into_bytes().to_vec()
        }
        TotpAlgorithm::Sha256 => {
            let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
                .expect("HMAC accepts keys of any length");
            mac.update(&message);
            mac.finalize().into_bytes().to_vec()
        }
        TotpAlgorithm::Sha512 => {
            let mut mac = <Hmac<Sha512> as KeyInit>::new_from_slice(key)
                .expect("HMAC accepts keys of any length");
            mac.update(&message);
            mac.finalize().into_bytes().to_vec()
        }
    };
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | digest[offset + 3] as u32;
    let code = binary % 10_u32.pow(digits);
    format!("{code:0width$}", width = digits as usize)
}
