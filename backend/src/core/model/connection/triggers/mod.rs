//! Trigger configuration types, parsing and limits.
//!
//! Kept in core so the connection model and preferences layer can validate the
//! stored shape without depending on the automation feature.

use std::fmt;
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha1::Sha1;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use regex::Regex;

mod parse;
mod types;

pub use parse::*;
pub use types::*;

/// Stage-count ceiling (contract D8): guards against config abuse.
pub const MAX_STAGES: usize = 16;
/// Pattern length ceiling, aligned with `exec::MAX_PROMPT_HINT_LEN` (D8).
pub const MAX_PATTERN_LEN: usize = 512;
/// `timeoutSecs` range: 1..=600 (D8); absent means the default.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
pub const MAX_TIMEOUT_SECS: u64 = 600;
/// `sleepMs` range: 0..=5000 (D8); absent means the default.
pub const DEFAULT_SLEEP_MS: u64 = 100;
pub const MAX_SLEEP_MS: u64 = 5000;
/// Send-text cap: the `\|` split and the per-char `passSleep = each` shape
/// both multiply the answer into segments, so an unbounded answer could stall
/// the terminal read loop on a misconfigured rule.
pub const MAX_SEND_TEXT_LEN: usize = 4096;
/// Command-line cap for `sendCommand`/`password_command`/`passphrase_command`.
pub const MAX_COMMAND_LEN: usize = 1024;
/// Runtime cap on captured stdout of a trigger/credential command: these
/// commands answer prompts (passwords, OTPs), so multi-KB output is a bug.
pub const MAX_COMMAND_OUTPUT_BYTES: usize = 4096;
/// Fixed command timeout (contract D4).
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
/// The only secret slots a `sendSecretKey` may reference (contract D5); the
/// manifest provides one password field per slot bound to `connection_secrets`.
pub const SECRET_SLOT_KEYS: &[&str] = &["trigger_answer_1", "trigger_answer_2"];
/// Rolling match buffer ceiling (contract D2).
pub(crate) const MAX_BUFFER_BYTES: usize = 8 * 1024;
/// TOTP secret ceiling after base32 decode (RFC 4226 recommends ≥16 bytes;
/// 64 is a generous abuse cap).
const MAX_TOTP_SECRET_BYTES: usize = 64;
/// tssh (trzsz-ssh) `--enc-secret` obfuscation key, byte-for-byte identical to
/// tssh's embedded `secretEncodeKey` (tssh/config.go): hex blobs are
/// `nonce(12) || AES-256-GCM(ct||tag)` under this key. Obfuscation, not
/// encryption — the same trust level tssh itself documents.
pub(crate) const TSSH_ENC_SECRET_KEY: &[u8; 32] = b"THE_UNSAFE_KEY_FOR_ENCODING_ONLY";

/// Decrypts a tssh `--enc-secret` blob the exact way tssh's `decodeSecret`
/// does: hex-decode, split off the 12-byte nonce, AES-256-GCM open under the
/// fixed embedded key. `error_context` names the offending directive
/// (`ExpectSendPass2`, …) for the connection error.
pub(crate) fn decode_tssh_enc_secret(
    hex_text: &str,
    error_context: &str,
) -> Result<String, String> {
    let blob = data_encoding::HEXLOWER_PERMISSIVE
        .decode(hex_text.trim().as_bytes())
        .map_err(|_| format!("{error_context}: value is not a valid tssh --enc-secret hex blob"))?;
    if blob.len() < 12 + 16 {
        return Err(format!(
            "{error_context}: tssh --enc-secret blob is too short (expected nonce + ciphertext + tag)"
        ));
    }
    let cipher = Aes256Gcm::new(TSSH_ENC_SECRET_KEY.into());
    let plaintext = cipher
        .decrypt(&Nonce::try_from(&blob[..12]).expect("12-byte GCM nonce"), &blob[12..])
        .map_err(|_| {
            format!(
                "{error_context}: tssh --enc-secret blob failed to decrypt (was it produced by tssh --enc-secret?)"
            )
        })?;
    String::from_utf8(plaintext)
        .map_err(|_| format!("{error_context}: decrypted secret is not valid UTF-8"))
}

/// Decodes a TOTP shared secret: base32 (RFC 4648), tolerant of the shapes
/// authenticator apps and tssh accept — spaces/hyphens dropped, case folded,
/// missing `=` padding restored. Returns the raw HMAC key bytes.
pub(crate) fn decode_base32_secret(text: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = text
        .chars()
        .filter(|character| !matches!(character, ' ' | '-'))
        .map(|character| character.to_ascii_uppercase())
        .collect();
    let unpadded = cleaned.trim_end_matches('=');
    let mut padded = unpadded.to_string();
    if !unpadded.len().is_multiple_of(8) {
        let padding = 8 - (unpadded.len() % 8);
        padded.push_str(&"=".repeat(padding));
    }
    data_encoding::BASE32
        .decode(padded.as_bytes())
        // 与 `core::otp::decode_base32` 同一条纪律：TOTP 密钥不得出现在错误串里
        // （连接解析失败的文案会进会话错误面板与审计日志）。
        .map_err(|_| {
            format!(
                "invalid base32 TOTP secret (expected A-Z2-7, got {} characters)",
                unpadded.chars().count()
            )
        })
}

/// Generates the RFC 6238 TOTP code: HMAC-SHA1, 6 digits, 30-second step —
/// the pquerna/otp defaults tssh's `getTotpCode` uses, so codes are
/// byte-compatible with `tssh ExpectSendTotpN`.
pub(crate) fn totp_code(secret: &[u8], now_ms: u64) -> String {
    let counter = (now_ms / 1000) / 30;
    let mut mac =
        <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary =
        u32::from_be_bytes(digest[offset..offset + 4].try_into().expect("4 bytes")) & 0x7fff_ffff;
    format!("{:06}", binary % 1_000_000)
}
