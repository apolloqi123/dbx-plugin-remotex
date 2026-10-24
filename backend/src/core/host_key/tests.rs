use super::*;

#[test]
fn changed_key_message_names_the_identity() {
    let message = host_key_changed_error("example.test", 2222, 4, "known_hosts").to_string();
    assert!(message.contains("example.test:2222"));
    assert!(message.contains("line 4"));
}

#[test]
fn lists_and_removes_known_host_entries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known_hosts");
    // ed25519 public key in openssh layout; host fields cover port forms.
    let key_line =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1234567890abcdefghijklmnopqrstuvwxyz test-key";
    std::fs::write(
        &path,
        format!(
            "# comment\nexample.test {key_line}\n[example.test]:2222 {key_line}\nother.test {key_line}\n"
        ),
    )
    .unwrap();

    let verifier = HostKeyVerifier::new(path.clone());
    let entries = verifier.list_known_hosts().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].host_field, "example.test");
    assert_eq!(entries[0].comment, "test-key");
    assert!(entries[0].fingerprint.starts_with("SHA256:") || entries[0].fingerprint.is_empty());

    // Removing port 22 drops both canonical forms of that port only.
    let removed = verifier.remove_known_host("example.test", 22).unwrap();
    assert_eq!(removed, 1);
    let entries = verifier.list_known_hosts().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].host_field, "[example.test]:2222");

    let removed = verifier.remove_known_host("example.test", 2222).unwrap();
    assert_eq!(removed, 1);
    let entries = verifier.list_known_hosts().unwrap();
    assert_eq!(
        entries,
        vec![KnownHostEntry {
            host_field: "other.test".to_string(),
            key_type: "ssh-ed25519".to_string(),
            fingerprint: entries[0].fingerprint.clone(),
            comment: "test-key".to_string(),
            marker: None,
        }]
    );

    // Removing a missing host is not an error.
    assert_eq!(verifier.remove_known_host("missing.test", 22).unwrap(), 0);
}

#[test]
fn lists_and_removes_marker_entries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known_hosts");
    let key_line =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1234567890abcdefghijklmnopqrstuvwxyz ca-key";
    std::fs::write(
        &path,
        format!(
            "@cert-authority *.prod.example {key_line}\n@revoked evil.test {key_line}\nplain.test {key_line}\n"
        ),
    )
    .unwrap();

    let verifier = HostKeyVerifier::new(path.clone());
    let entries = verifier.list_known_hosts().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].marker.as_deref(), Some("@cert-authority"));
    assert_eq!(entries[0].host_field, "*.prod.example");
    assert_eq!(entries[1].marker.as_deref(), Some("@revoked"));
    assert_eq!(entries[1].host_field, "evil.test");
    assert_eq!(entries[2].marker, None);
    assert_eq!(entries[2].host_field, "plain.test");
    assert!(entries.iter().all(|entry| entry.key_type == "ssh-ed25519"));

    // Removal matches the host token of marked lines, not the marker.
    let removed = verifier.remove_known_host("evil.test", 22).unwrap();
    assert_eq!(removed, 1);
    let entries = verifier.list_known_hosts().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.host_field != "evil.test"));

    // Wildcard CA entries are only removed by their literal host field.
    let removed = verifier.remove_known_host("*.prod.example", 22).unwrap();
    assert_eq!(removed, 1);
    let entries = verifier.list_known_hosts().unwrap();
    assert_eq!(
        entries,
        vec![KnownHostEntry {
            host_field: "plain.test".to_string(),
            key_type: "ssh-ed25519".to_string(),
            fingerprint: entries[0].fingerprint.clone(),
            comment: "ca-key".to_string(),
            marker: None,
        }]
    );
}

#[test]
fn listing_missing_store_is_empty() {
    let directory = tempfile::tempdir().unwrap();
    let verifier = HostKeyVerifier::new(directory.path().join("known_hosts"));
    assert!(verifier.list_known_hosts().unwrap().is_empty());
}

#[test]
fn listing_survives_adversarial_known_hosts_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known_hosts");
    let key_line =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1234567890abcdefghijklmnopqrstuvwxyz ok";
    // Hashed hostname entry (OpenSSH privacy form), CRLF endings, blank
    // lines, comments, a line with an unparseable key, a bare marker, a
    // stray marker fragment, a keyless host line, and one megabyte-scale
    // junk line — the listing must tolerate all of them without
    // panicking and without listing the junk.
    let junk_line = format!("junk.test {}", "A".repeat(1024 * 1024));
    let content = format!(
        "|1|c2FsdA==|aGFzaA== {key_line}\r\n\
             # leading comment\n\
             \n   \n\
             plain.test {key_line} trailing comment here\n\
             garbage.test ssh-ed25519 not-a-valid-key ssh-ed25519 also-broken\n\
             @cert-authority\n\
             @revoked @broken\n\
             broken.test\n\
             {junk_line}\r\n"
    );
    std::fs::write(&path, &content).unwrap();

    let verifier = HostKeyVerifier::new(path.clone());
    let entries = verifier.list_known_hosts().unwrap();
    let hosts: Vec<&str> = entries
        .iter()
        .map(|entry| entry.host_field.as_str())
        .collect();
    // Hashed entries stay visible; the line with an unparseable key is
    // still listed (with an empty fingerprint) so the store stays
    // inspectable, while junk/malformed lines are skipped entirely.
    assert_eq!(
        hosts,
        vec!["|1|c2FsdA==|aGFzaA==", "plain.test", "garbage.test"]
    );
    assert_eq!(entries[1].comment, "ok trailing comment here");
    assert!(entries[2].fingerprint.is_empty());

    // Removal must not delete the hashed entry, comments, unparseable or
    // junk lines while removing the plain entry.
    let removed = verifier.remove_known_host("plain.test", 22).unwrap();
    assert_eq!(removed, 1);
    let remaining = std::fs::read_to_string(&path).unwrap();
    assert!(remaining.contains("|1|c2FsdA==|aGFzaA=="));
    assert!(remaining.contains("# leading comment"));
    assert!(remaining.contains("@cert-authority"));
    assert!(remaining.contains(&junk_line));
    assert!(remaining.contains("garbage.test"));
    assert!(!remaining.contains("plain.test"));
}

#[test]
fn removal_ignores_nonstandard_marker_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known_hosts");
    let key_line =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1234567890abcdefghijklmnopqrstuvwxyz x";
    std::fs::write(
        &path,
        format!("@unknownmarker plain.test {key_line}\nplain.test {key_line}\n"),
    )
    .unwrap();

    let verifier = HostKeyVerifier::new(path.clone());
    // Only OpenSSH markers put the host in the second token; a fake
    // marker line is matched on its literal first token, so removing
    // "plain.test" must drop exactly one line.
    let removed = verifier.remove_known_host("plain.test", 22).unwrap();
    assert_eq!(removed, 1);
    let remaining = std::fs::read_to_string(&path).unwrap();
    assert!(remaining.contains("@unknownmarker"));
    // The fake-marker line stays in the file but is not listable (its
    // host field looks like a stray marker fragment).
    assert!(verifier.list_known_hosts().unwrap().is_empty());
}

/// Builds an in-memory ed25519 key so TOFU tests need no ssh-keygen.
fn generated_test_key() -> russh::keys::ssh_key::PublicKey {
    let private = russh::keys::ssh_key::PrivateKey::random(
        &mut rand::rng(),
        russh::keys::ssh_key::Algorithm::Ed25519,
    )
    .unwrap();
    private.public_key().clone()
}

/// Control environment so `check()`'s system-store leg never consults the
/// developer's real ~/.ssh/known_hosts (the tempdir HOME makes that path
/// missing, matching a fresh machine).
struct HomeIsolationGuard {
    previous: Option<std::ffi::OsString>,
}

impl HomeIsolationGuard {
    fn new(home: &std::path::Path) -> Self {
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let previous = std::env::var_os(var);
        std::env::set_var(var, home);
        Self { previous }
    }
}

impl Drop for HomeIsolationGuard {
    fn drop(&mut self) {
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        match &self.previous {
            Some(value) => std::env::set_var(var, value),
            None => std::env::remove_var(var),
        }
    }
}

#[test]
fn check_learn_check_closes_the_trust_loop() {
    let home = tempfile::tempdir().unwrap();
    let _guard = HomeIsolationGuard::new(home.path());
    let directory = tempfile::tempdir().unwrap();
    let verifier = HostKeyVerifier::new(directory.path().join("known_hosts"));
    let key = generated_test_key();

    // First sight: unknown (electerm's "prompts for an unknown host key
        // once" precondition).
    assert_eq!(
        verifier.check("tofu.test", 2222, &key).unwrap(),
        HostKeyState::Unknown
    );

    // Trust-on-first-use records the key, and a second check must now be
    // Trusted — the "second connection does not prompt again" semantic.
    verifier.learn("tofu.test", 2222, &key).unwrap();
    assert_eq!(
        verifier.check("tofu.test", 2222, &key).unwrap(),
        HostKeyState::Trusted
    );

    // The same key under the plugin's canonical port-form must also be
    // trusted (learn wrote an [host]:port entry for the non-default port).
    let entries = verifier.list_known_hosts().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].host_field, "[tofu.test]:2222");
}

#[test]
fn learned_key_mismatch_still_rejects_with_changed_key() {
    let home = tempfile::tempdir().unwrap();
    let _guard = HomeIsolationGuard::new(home.path());
    let directory = tempfile::tempdir().unwrap();
    let verifier = HostKeyVerifier::new(directory.path().join("known_hosts"));

    let trusted_key = generated_test_key();
    verifier.learn("rotate.test", 22, &trusted_key).unwrap();
    assert_eq!(
        verifier.check("rotate.test", 22, &trusted_key).unwrap(),
        HostKeyState::Trusted
    );

    // A different key on the same host must surface the changed-key
    // error (MITM guard) rather than silently falling back to Unknown.
    let rotated_key = generated_test_key();
    let error = verifier.check("rotate.test", 22, &rotated_key).unwrap_err();
    assert!(error.to_string().contains("changed"), "{error}");
}
