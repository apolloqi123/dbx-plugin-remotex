//! Actionable SSH private-key decode failures (pure text classification).

pub(crate) fn is_pem_base64_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '/' || c == '+' || c == '='
}

/// Detects the one-line authorized-keys form and PEM public/certificate
/// envelopes so a pasted public key is reported as such instead of as an
/// opaque decode failure.
pub(crate) fn looks_like_public_key(text: &str) -> bool {
    let trimmed = text.trim_start();
    for header in [
        "-----BEGIN PUBLIC KEY-----",
        "-----BEGIN OPENSSH PUBLIC KEY-----",
        "-----BEGIN SSH2 PUBLIC KEY-----",
        "-----BEGIN CERTIFICATE-----",
    ] {
        if trimmed.contains(header) {
            return true;
        }
    }
    let first = trimmed.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let (Some(kind), Some(blob)) = (parts.next(), parts.next()) else {
        return false;
    };
    let looks_like_algorithm = kind.starts_with("ssh-")
        || kind.starts_with("ecdsa-")
        || kind.starts_with("sk-ssh-")
        || kind.starts_with("sk-ecdsa-");
    looks_like_algorithm && blob.len() >= 40 && blob.chars().all(is_pem_base64_char)
}

pub(crate) fn has_html_escaped_entities(text: &str) -> bool {
    text.contains("&lt;")
        || text.contains("&gt;")
        || text.contains("&quot;")
        || text.contains("&amp;")
}

/// Turns a raw `decode_secret_key` failure into an actionable message while
/// always preserving the underlying decoder error verbatim (never swallow
/// detail). Classification is a pure function of the key text and the error,
/// so tests cover every branch without an SSH server. This only rewords the
/// failure — authentication flow, passphrase handling, and decoding itself
/// are untouched.
pub(crate) fn private_key_decode_failure(
    key_text: &str,
    error: &russh::keys::Error,
    passphrase_provided: bool,
) -> String {
    fn detailed(guidance: &str, error: &russh::keys::Error) -> String {
        format!("Failed to decode SSH private key: {guidance} (decoder error: {error})")
    }
    let raw = error.to_string();
    if key_text.contains("PuTTY-User-Key-File-") {
        if matches!(error, russh::keys::Error::KeyIsEncrypted) || raw.contains("encrypted") {
            return detailed(
                "this PuTTY PPK key is encrypted; fill in the private key passphrase field \
                 for this connection, then reconnect",
                error,
            );
        }
        if raw.contains("MAC") {
            return detailed(
                "the PuTTY PPK did not verify — the passphrase appears to be incorrect, or \
                 the file is damaged; check the private key passphrase field or re-export \
                 the key from PuTTYgen",
                error,
            );
        }
        return detailed(
            "this PuTTY PPK file could not be parsed; export it from PuTTYgen again \
             (Conversions → Export OpenSSH key) and paste the OpenSSH-format key",
            error,
        );
    }
    if looks_like_public_key(key_text) {
        return detailed(
            "the pasted text looks like a PUBLIC key or certificate, not a private key; \
             paste the matching private key file contents, including the \
             -----BEGIN … PRIVATE KEY----- and -----END … PRIVATE KEY----- lines",
            error,
        );
    }
    if matches!(error, russh::keys::Error::KeyIsEncrypted) {
        return detailed(
            "this private key is encrypted; fill in the private key passphrase field for \
             this connection, then reconnect",
            error,
        );
    }
    if raw.contains("PKCS#5 algorithm") && raw.contains("unsupported") {
        return detailed(
            "the encrypted PKCS#8 key uses a PBKDF2 variant this decoder does not \
             support; re-export the key in OpenSSH format (ssh-keygen -p -f <keyfile>) \
             or without a passphrase",
            error,
        );
    }
    if key_text.contains("-----BEGIN EC PRIVATE KEY-----") && raw.starts_with("Der:") {
        return detailed(
            "legacy 'EC PRIVATE KEY' (SEC1) PEM is not supported by this decoder; convert \
             the key once (ssh-keygen -p -f <keyfile>, or openssl pkcs8 -topk8 -nocrypt \
             -in <keyfile>) and paste the converted key",
            error,
        );
    }
    if passphrase_provided
        && (raw.contains("cryptographic error") || raw.contains("Unpad") || raw.contains("MAC"))
    {
        return detailed(
            "the key did not decrypt with the stored passphrase — check or update the \
             private key passphrase field, or the key file is damaged",
            error,
        );
    }
    if raw.contains("Base64 decoding error") {
        return detailed(
            "the key body is not valid base64 — the text was probably truncated or \
             mangled while copying; re-copy the complete key from the original file",
            error,
        );
    }
    if matches!(error, russh::keys::Error::CouldNotReadKey) {
        if has_html_escaped_entities(key_text) {
            return detailed(
                "the text looks HTML-escaped (&lt;, &quot;, …); copy the key from the \
                 original file as plain text, not from a web page or chat window",
                error,
            );
        }
        if key_text.contains("-----BEGIN") {
            return detailed(
                "a PEM header was found but not in a recognizable form — the key was \
                 probably mangled by rich-text copy (altered dashes or extra characters); \
                 re-copy the key from the original file as plain text",
                error,
            );
        }
        return detailed(
            "no PEM private-key header (-----BEGIN … PRIVATE KEY-----) was found; paste \
             the complete key file contents, including the BEGIN and END lines",
            error,
        );
    }
    detailed("the key could not be decoded", error)
}
