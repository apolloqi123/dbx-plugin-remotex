//! Quick Sudo profile tests.

use super::*;
use crate::core::vault::KeyfileProvider;

/// Test-only secret assembled at runtime: obviously fake, and never a
/// literal credential in source.
fn test_secret(tag: &str) -> String {
    format!("{tag}-{}", uuid::Uuid::new_v4())
}

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dbx-sudo-profiles-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// All storage tests go through the keyfile tier: never the OS keychain
/// (macOS would pop an authorization dialog in CI/dev runs).
fn file_provider(dir: &Path) -> KeyfileProvider {
    KeyfileProvider::new(dir.join("vault.key"))
}

fn store_with(name: &str, password: &str) -> SudoProfileStore {
    let mut store = SudoProfileStore::default();
    let (profile, _) = save_profile(
        &mut store,
        &json!({ "name": name, "sudoPassword": password, "totpSecret": "JBSWY3DPEHPK3PXP" }),
    )
    .unwrap();
    store
        .bindings
        .insert("conn-1".to_string(), profile.id.clone());
    store
}

#[test]
fn create_update_keeps_secret_when_blank() {
    let secret = test_secret("keep");
    let mut store = store_with("ops", &secret);
    let id = store.profiles[0].id.clone();
    let (updated, created) = save_profile(
        &mut store,
        &json!({ "id": id, "name": "ops", "sudoPassword": "", "sudoUsePty": true }),
    )
    .unwrap();
    assert!(!created);
    assert_eq!(updated.sudo_password, secret);
    assert!(updated.sudo_use_pty);
    // Rewriting the secret works too.
    let replacement = test_secret("new");
    let (changed, _) = save_profile(
        &mut store,
        &json!({ "id": id, "name": "ops", "sudoPassword": replacement }),
    )
    .unwrap();
    assert_eq!(changed.sudo_password, replacement);
}

#[test]
fn explicit_clear_flags_wipe_secrets() {
    let secret = test_secret("clear");
    let mut store = store_with("ops", &secret);
    let id = store.profiles[0].id.clone();
    let (updated, _) = save_profile(
        &mut store,
        &json!({ "id": id, "name": "ops", "clearTotpSecret": true }),
    )
    .unwrap();
    assert!(updated.totp_secret.is_empty());
    assert_eq!(updated.sudo_password, secret);
}

#[test]
fn name_rules_are_enforced() {
    let mut store = store_with("ops", "x");
    assert!(save_profile(&mut store, &json!({ "name": "  " })).is_err());
    assert!(save_profile(&mut store, &json!({ "name": "a".repeat(65) })).is_err());
    // Case-insensitive uniqueness, trimmed compare.
    let error = save_profile(&mut store, &json!({ "name": " OPS " })).unwrap_err();
    assert!(error.contains("already in use"), "{error}");
    // Updating the same profile keeps its own name.
    let id = store.profiles[0].id.clone();
    assert!(save_profile(&mut store, &json!({ "id": id, "name": "OPS" })).is_ok());
}

#[test]
fn profile_cap_is_enforced() {
    let mut store = SudoProfileStore::default();
    for index in 0..MAX_PROFILES {
        save_profile(&mut store, &json!({ "name": format!("p{index}") })).unwrap();
    }
    assert!(save_profile(&mut store, &json!({ "name": "overflow" })).is_err());
}

#[test]
fn delete_cascades_bindings() {
    let mut store = store_with("ops", "x");
    let id = store.profiles[0].id.clone();
    assert!(delete_profile(&mut store, &id));
    assert!(!store.bindings.contains_key("conn-1"));
    assert!(!delete_profile(&mut store, &id));
}

#[test]
fn corrupted_file_falls_back_to_empty_store() {
    let dir = temp_dir();
    std::fs::write(store_path(&dir), "{not json").unwrap();
    let store = load_store_with(&dir, &file_provider(&dir));
    assert!(store.profiles.is_empty());
    assert!(store.bindings.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn store_roundtrip_preserves_profiles_and_bindings() {
    let dir = temp_dir();
    let store = store_with("ops", "s3cret");
    let provider = file_provider(&dir);
    save_store_with(&dir, &store, &provider).unwrap();
    let loaded = load_store_with(&dir, &provider);
    assert_eq!(loaded, store);
    assert_eq!(loaded.bindings, store.bindings);
    let _ = std::fs::remove_dir_all(&dir);
}

/// At-rest guarantee: the store file carries sealed envelopes only — no
/// plaintext secret, no legacy plaintext key names — plus the v2
/// envelope header naming the keyfile tier.
#[test]
fn stored_file_never_carries_plaintext_secrets() {
    let dir = temp_dir();
    let store = store_with("ops", "super-secret-password");
    save_store_with(&dir, &store, &file_provider(&dir)).unwrap();
    let text = std::fs::read_to_string(store_path(&dir)).unwrap();
    assert!(!text.contains("super-secret-password"));
    assert!(!text.contains("JBSWY3DPEHPK3PXP"));
    assert!(text.contains("sudoPasswordEnc"));
    assert!(!text.contains("sudoPassword\":"));
    assert!(!text.contains("totpSecret\":"));
    let file: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(file["version"], 2);
    assert_eq!(file["crypto"]["scheme"], "aead-v1");
    assert_eq!(file["crypto"]["storage"], "keyfile");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Empty secrets stay plaintext empty strings instead of envelopes.
#[test]
fn empty_secrets_stay_empty_strings_in_the_file() {
    let dir = temp_dir();
    let mut store = SudoProfileStore::default();
    save_profile(&mut store, &json!({ "name": "bare" })).unwrap();
    save_store_with(&dir, &store, &file_provider(&dir)).unwrap();
    let text = std::fs::read_to_string(store_path(&dir)).unwrap();
    assert!(text.contains("\"sudoPasswordEnc\": \"\""));
    assert!(text.contains("\"totpSecretEnc\": \"\""));
    let loaded = load_store_with(&dir, &file_provider(&dir));
    assert_eq!(loaded, store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A legacy v1 plaintext file is read normally and rewritten to the
/// encrypted v2 envelope right after load (best-effort migration).
#[test]
fn v1_plaintext_file_migrates_to_encrypted_v2() {
    let dir = temp_dir();
    let v1 = json!({
        "version": 1,
        "profiles": [{
            "id": "p1",
            "name": "ops",
            "sudoPassword": "plain-pass",
            "totpSecret": "JBSWY3DPEHPK3PXP",
            "authFlowMode": "password_then_otp",
            "createdAt": 1,
            "updatedAt": 2,
        }],
        "bindings": { "conn-1": "p1" },
    });
    std::fs::write(store_path(&dir), serde_json::to_string_pretty(&v1).unwrap()).unwrap();
    let provider = file_provider(&dir);
    let loaded = load_store_with(&dir, &provider);
    // Secrets survive into memory with metadata intact...
    assert_eq!(loaded.profiles[0].sudo_password, "plain-pass");
    assert_eq!(loaded.profiles[0].totp_secret, "JBSWY3DPEHPK3PXP");
    assert_eq!(
        loaded.bindings.get("conn-1").map(String::as_str),
        Some("p1")
    );
    // ...and the file is now the encrypted v2 envelope, no plaintext.
    let text = std::fs::read_to_string(store_path(&dir)).unwrap();
    assert!(!text.contains("plain-pass"));
    let file: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(file["version"], 2);
    assert_eq!(file["crypto"]["storage"], "keyfile");
    assert!(!file["profiles"][0]["sudoPasswordEnc"]
        .as_str()
        .unwrap_or_default()
        .is_empty());
    // Reloading the migrated file restores the same store.
    let reloaded = load_store_with(&dir, &provider);
    assert_eq!(reloaded, loaded);
    let _ = std::fs::remove_dir_all(&dir);
}

/// D4: an unreadable envelope degrades that field to empty (view reports
/// `sudoPasswordSet=false`) while metadata and sibling fields survive.
#[test]
fn corrupted_envelope_degrades_to_empty_secret() {
    let dir = temp_dir();
    let store = store_with("ops", "real-secret");
    let provider = file_provider(&dir);
    save_store_with(&dir, &store, &provider).unwrap();
    // Tamper with the stored envelope.
    let mut file: Value =
        serde_json::from_str(&std::fs::read_to_string(store_path(&dir)).unwrap()).unwrap();
    file["profiles"][0]["sudoPasswordEnc"] = json!("not-a-valid-envelope");
    std::fs::write(store_path(&dir), serde_json::to_string(&file).unwrap()).unwrap();
    let loaded = load_store_with(&dir, &provider);
    assert_eq!(loaded.profiles[0].sudo_password, "");
    assert_eq!(loaded.profiles[0].name, "ops");
    assert_eq!(loaded.profiles[0].totp_secret, "JBSWY3DPEHPK3PXP");
    let view = profile_view(&loaded.profiles[0]);
    assert_eq!(view["sudoPasswordSet"], false);
    assert_eq!(view["totpConfigured"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A key lost between save and load (keyfile deleted) opens every
/// envelope to empty without breaking the load path.
#[test]
fn lost_keyfile_degrades_secrets_but_keeps_metadata() {
    let dir = temp_dir();
    let store = store_with("ops", "real-secret");
    save_store_with(&dir, &store, &file_provider(&dir)).unwrap();
    std::fs::remove_file(dir.join("vault.key")).unwrap();
    // The replacement keyfile provider mints a fresh DEK: old envelopes
    // fail open-to-empty, the profile structure survives.
    let loaded = load_store_with(&dir, &file_provider(&dir));
    assert_eq!(loaded.profiles.len(), 1);
    assert_eq!(loaded.profiles[0].name, "ops");
    assert_eq!(loaded.profiles[0].sudo_password, "");
    assert_eq!(loaded.profiles[0].totp_secret, "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn views_never_echo_secrets() {
    let secret = test_secret("view");
    let store = store_with("ops", &secret);
    let views = list_views(&store);
    let rendered = serde_json::to_string(&views).unwrap();
    assert!(!rendered.contains(&secret));
    assert!(!rendered.contains("sudoPassword\":"));
    assert!(rendered.contains("\"sudoPasswordSet\":true"));
    let view = profile_view(&store.profiles[0].clone());
    assert_eq!(view["totpConfigured"], true);
}

#[test]
fn reveal_returns_raw_secrets_for_the_editor() {
    let secret = test_secret("reveal");
    let store = store_with("ops", &secret);
    let revealed = reveal_profile(&store, &store.profiles[0].id).expect("reveal");
    let profile = revealed["profile"].clone();
    assert_eq!(profile["sudoPassword"], secret);
    assert_eq!(profile["totpSecret"], "JBSWY3DPEHPK3PXP");
    assert_eq!(profile["name"], "ops");
    // Unknown ids fail instead of returning an empty shell.
    assert!(reveal_profile(&store, "missing").is_err());
}

#[test]
fn refs_resolve_by_id_then_name() {
    let store = store_with("ops", "x");
    let profile = &store.profiles[0];
    assert_eq!(find_by_ref(&store, &profile.id).unwrap().name, "ops");
    assert_eq!(find_by_ref(&store, "ops").unwrap().id, profile.id);
    assert!(find_by_ref(&store, "missing").is_none());
}

#[test]
fn bindings_validate_and_resolve() {
    let mut store = store_with("ops", "x");
    assert!(set_binding(&mut store, "conn-2", Some("nope")).is_err());
    let profile_id = store.profiles[0].id.clone();
    set_binding(&mut store, "conn-2", Some(&profile_id)).unwrap();
    assert!(bound_profile(&store, "conn-2").is_some());
    set_binding(&mut store, "conn-2", None).unwrap();
    assert!(bound_profile(&store, "conn-2").is_none());
    // Dangling binding resolves to none instead of failing.
    store
        .bindings
        .insert("conn-3".to_string(), "ghost".to_string());
    assert!(bound_profile(&store, "conn-3").is_none());
}

#[test]
fn apply_profile_overrides_source_and_falls_back_to_login() {
    let conn_secret = test_secret("conn");
    let store = store_with("ops", &test_secret("profile"));
    let profile = store.profiles[0].clone();
    let mut auth = SudoAuth::new(&conn_secret, "login-pass", "", Default::default());
    apply_profile(&mut auth, &profile, "login-pass");
    assert_eq!(auth.password, profile.sudo_password);
    assert_ne!(auth.password, conn_secret);
    assert!(!auth.totp_secrets.is_empty());

    let mut blank = profile.clone();
    blank.sudo_password = String::new();
    let mut auth = SudoAuth::new(&conn_secret, "login-pass", "", Default::default());
    apply_profile(&mut auth, &blank, "login-pass");
    // A bound profile without a password falls back to the login
    // password, not to the connection's own sudo secret.
    assert_eq!(auth.password, "login-pass");
}

#[test]
fn use_pty_follows_bound_profile() {
    let mut store = store_with("ops", "x");
    let id = store.profiles[0].id.clone();
    let (_, _) = save_profile(
        &mut store,
        &json!({ "id": id, "name": "ops", "sudoUsePty": true }),
    )
    .unwrap();
    let profile = &store.profiles[0];
    assert!(effective_use_pty(false, Some(profile)));
    assert!(effective_use_pty(true, None));
    assert!(!effective_use_pty(false, None));
}

/// Writes a v2 file whose `crypto.storage` header names the keychain tier
/// while the ciphertexts were sealed with the given provider — simulating
/// a legacy keychain-tier file for migration tests without touching the
/// OS keychain.
fn write_file_with_tier(
    dir: &Path,
    store: &SudoProfileStore,
    tier: &str,
    sealer: &dyn KeyProvider,
) {
    save_store_with(dir, store, sealer).unwrap();
    let path = store_path(dir);
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["crypto"]["storage"] = json!(tier);
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
}

#[test]
fn keychain_tier_file_migrates_to_keyfile_on_recovered_load() {
    let dir = temp_dir();
    let secret = test_secret("migrate");
    let store = store_with("ops", &secret);
    // Ciphertexts sealed with the keyfile DEK, header claiming keychain:
    // loading with the keyfile provider recovers the secrets, so the
    // loader must rewrite the file onto the keyfile tier.
    write_file_with_tier(&dir, &store, "keychain", &file_provider(&dir));

    let reloaded = load_store_with(&dir, &file_provider(&dir));
    assert_eq!(reloaded.profiles[0].sudo_password, secret);
    let raw = std::fs::read_to_string(store_path(&dir)).unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        value["crypto"]["storage"],
        json!("keyfile"),
        "file must migrate off the keychain tier"
    );
    assert!(
        keyfile_path(&dir).exists(),
        "keyfile DEK must exist after migration"
    );
    assert!(
        !raw.contains(&secret),
        "migrated file must stay encrypted at rest"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keychain_tier_file_with_unrecoverable_secrets_is_left_untouched() {
    let dir = temp_dir();
    let store = store_with("ops", &test_secret("lost"));
    // Seal with one keyfile, then claim keychain tier: loading with a
    // DIFFERENT keyfile cannot recover the secrets (simulates a denied /
    // lost keychain). The loader must not rewrite the file with blanks —
    // the keychain-tier header survives for a later successful attempt.
    write_file_with_tier(&dir, &store, "keychain", &file_provider(&dir));
    let wrong = KeyfileProvider::new(dir.join("other-vault.key"));
    // Drop the keyfile minted during sealing so the assertion below can
    // prove a failed load never creates one.
    std::fs::remove_file(keyfile_path(&dir)).unwrap();

    let reloaded = load_store_with(&dir, &wrong);
    assert_eq!(
        reloaded.profiles[0].sudo_password, "",
        "unrecoverable envelope degrades to empty"
    );

    let raw = std::fs::read_to_string(store_path(&dir)).unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        value["crypto"]["storage"],
        json!("keychain"),
        "unrecoverable file must not be rewritten"
    );
    assert!(
        !keyfile_path(&dir).exists(),
        "no keyfile DEK may be minted by a failed load"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn degraded_vault_refuses_to_overwrite_secrets_with_blanks() {
    let dir = temp_dir();
    let secret = test_secret("guard");
    let store = store_with("ops", &secret);
    // Persist a healthy, encrypted file first.
    save_store_with(&dir, &store, &file_provider(&dir)).unwrap();
    let before = std::fs::read_to_string(store_path(&dir)).unwrap();
    assert!(before.contains("sudoPasswordEnc"));

    // Saving through a provider whose DEK cannot resolve must be refused
    // instead of rewriting every envelope with "" (which a degraded load
    // would otherwise do even when the in-memory secrets look empty).
    let err = save_store_with(&dir, &store, &crate::core::vault::BrokenKeyProvider);
    let message = err.expect_err("degraded vault save must fail");
    assert!(message.contains("unavailable"), "error was: {message}");
    let after = std::fs::read_to_string(store_path(&dir)).unwrap();
    assert_eq!(after, before, "on-disk envelopes must be untouched");
    let _ = std::fs::remove_dir_all(&dir);
}
