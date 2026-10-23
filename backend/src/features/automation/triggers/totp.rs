//! Trigger totp layer.

use super::*;

/// Decodes a TOTP shared secret: base32 (RFC 4648), tolerant of the shapes
/// authenticator apps and tssh accept — spaces/hyphens dropped, case folded,
/// missing `=` padding restored. Returns the raw HMAC key bytes.
pub(super) fn decode_base32_secret(text: &str) -> Result<Vec<u8>, String> {
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
        .map_err(|_| format!("invalid base32 TOTP secret (expected A-Z2-7, got '{text}')"))
}

/// Generates the RFC 6238 TOTP code: HMAC-SHA1, 6 digits, 30-second step —
/// the pquerna/otp defaults tssh's `getTotpCode` uses, so codes are
/// byte-compatible with `tssh ExpectSendTotpN`.
pub(super) fn totp_code(secret: &[u8], now_ms: u64) -> String {
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

/// Decrypts a tssh `--enc-secret` blob the exact way tssh's `decodeSecret`
/// does: hex-decode, split off the 12-byte nonce, AES-256-GCM open under the
/// fixed embedded key. `error_context` names the offending directive
/// (`ExpectSendPass2`, …) for the connection error.
pub(super) fn decode_tssh_enc_secret(
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
