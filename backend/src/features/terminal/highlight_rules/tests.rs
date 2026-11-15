use super::*;

fn save(store: &mut HighlightRuleStore, pattern: &str) -> HighlightRuleEntry {
    save_entry(store, &json!({ "pattern": pattern })).unwrap().0
}

#[test]
fn save_creates_with_defaults_and_updates_in_place() {
    let mut store = HighlightRuleStore::default();
    let (created_entry, created) = save_entry(&mut store, &json!({ "pattern": "ERROR" })).unwrap();
    assert!(created);
    assert!(!created_entry.id.is_empty());
    assert_eq!(created_entry.pattern, "ERROR");
    assert!(!created_entry.is_regex, "isRegex defaults to false");
    assert_eq!(created_entry.color, DEFAULT_COLOR);
    assert!(!created_entry.case_sensitive);
    assert!(created_entry.enabled);
    assert_eq!(created_entry.created_at, created_entry.updated_at);

    // Update keeps the id and bumps only the pattern + updatedAt.
    let id = created_entry.id.clone();
    let (updated, created) = save_entry(
        &mut store,
        &json!({ "id": id, "pattern": "FATAL", "isRegex": false, "color": "#22c55e",
                 "caseSensitive": true, "enabled": false }),
    )
    .unwrap();
    assert!(!created);
    assert_eq!(updated.id, id);
    assert_eq!(updated.pattern, "FATAL");
    assert_eq!(updated.color, "#22c55e");
    assert!(updated.case_sensitive);
    assert!(!updated.enabled);
    assert_eq!(store.rules.len(), 1);
}

#[test]
fn save_rejects_missing_oversized_or_malformed_input() {
    let mut store = HighlightRuleStore::default();
    // Pattern is required (trimmed non-empty).
    assert!(save_entry(&mut store, &json!({ "pattern": "   " })).is_err());
    assert!(save_entry(&mut store, &json!({})).is_err());
    // Length cap.
    assert!(save_entry(&mut store, &json!({ "pattern": "x".repeat(201) })).is_err());
    // Color must be #rrggbb.
    for bad_color in [
        "orange",
        "#12345",
        "#1234567",
        "#12h456",
        "f59e0b",
        "#ff00ff00",
    ] {
        assert!(
            save_entry(&mut store, &json!({ "pattern": "e", "color": bad_color })).is_err(),
            "expected color rejection for {bad_color}"
        );
    }
    // Updating a non-existent id is an error (never a silent create).
    assert!(save_entry(&mut store, &json!({ "id": "ghost", "pattern": "e" })).is_err());
    // Valid boundary: exactly 200 chars passes.
    assert!(save_entry(&mut store, &json!({ "pattern": "y".repeat(200) })).is_ok());
}

#[test]
fn cap_is_enforced_on_new_entries_only() {
    let mut store = HighlightRuleStore::default();
    for index in 0..MAX_RULES {
        save(&mut store, &format!("p{index}"));
    }
    assert!(save_entry(&mut store, &json!({ "pattern": "overflow" })).is_err());
    // Updating an existing rule stays within the cap.
    let id = store.rules[0].id.clone();
    assert!(save_entry(&mut store, &json!({ "id": id, "pattern": "ok" })).is_ok());
    assert_eq!(store.rules.len(), MAX_RULES);
}

#[test]
fn delete_reports_presence_and_is_idempotent() {
    let mut store = HighlightRuleStore::default();
    let entry = save(&mut store, "ERROR");
    assert!(delete_entry(&mut store, &entry.id));
    assert!(!delete_entry(&mut store, &entry.id));
    assert!(store.rules.is_empty());
}

#[test]
fn corrupted_file_falls_back_to_empty_store() {
    let dir = std::env::temp_dir().join(format!("dbx-hl-rules-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(store_path(&dir), "{not json").unwrap();
    assert!(load_store(&dir).rules.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn first_load_seeds_defaults_and_persists_them() {
    let dir = std::env::temp_dir().join(format!("dbx-hl-rules-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = load_or_seed_store(&dir);
    let patterns: Vec<&str> = store
        .rules
        .iter()
        .map(|entry| entry.pattern.as_str())
        .collect();
    assert_eq!(
        patterns,
        [
            "ERROR",
            "FATAL",
            "Permission denied",
            "No such file or directory",
            "command not found",
            "Connection refused",
            "No space left on device",
            "Failed to",
            "cannot",
            "Exception",
            "Traceback (most recent call last)",
            "FAIL",
            "denied",
            "timed out",
            "WARN",
            "deprecated",
            "SUCCESS",
            "active (running)",
            "done",
            "✓",
            "PASSED",
            r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b",
        ]
    );
    for entry in &store.rules {
        assert!(entry.enabled);
        assert_eq!(entry.created_at, entry.updated_at);
        // Only the IPv4 rule is a regex; only PASSED pins the case.
        assert_eq!(entry.is_regex, entry.id == "default-ipv4");
        assert_eq!(entry.case_sensitive, entry.id == "default-passed");
    }
    // The seed is persisted: the next load reads the file, and a user
    // deleting every rule afterwards is NOT reseeded (file exists).
    let reloaded = load_or_seed_store(&dir);
    assert_eq!(reloaded, store);
    let mut emptied = reloaded;
    emptied.rules.clear();
    save_store(&dir, &emptied).unwrap();
    assert!(load_or_seed_store(&dir).rules.is_empty());
    // Deterministic ids so defaults stay recognizable across installs.
    assert_eq!(store.rules[0].id, "default-error");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn store_roundtrip_preserves_rules_in_order() {
    let dir = std::env::temp_dir().join(format!("dbx-hl-rules-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = HighlightRuleStore::default();
    save(&mut store, "ERROR");
    save(&mut store, "FATAL");
    save_store(&dir, &store).unwrap();
    let loaded = load_store(&dir);
    assert_eq!(loaded, store);
    assert_eq!(loaded.rules.len(), 2);
    // The persisted file is 0600 on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(store_path(&dir))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn views_carry_full_fields_sorted_by_created_at() {
    let mut store = HighlightRuleStore::default();
    let first = save(&mut store, "ERROR");
    let second = save(&mut store, "FATAL");
    // Simulate rules stored newest-first with distinct timestamps, then
    // verify the view still lists createdAt-ascending.
    store.rules[0].created_at = 2000;
    store.rules[0].updated_at = 2000;
    store.rules[1].created_at = 1000;
    store.rules[1].updated_at = 1000;
    store.rules.reverse();
    let views = list_views(&store);
    assert_eq!(views.len(), 2);
    assert_eq!(views[0]["pattern"], second.pattern, "older createdAt first");
    assert_eq!(views[1]["pattern"], first.pattern);
    for view in &views {
        let keys: Vec<&str> = view
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for expected in [
            "id",
            "pattern",
            "isRegex",
            "color",
            "caseSensitive",
            "enabled",
            "createdAt",
            "updatedAt",
        ] {
            assert!(keys.contains(&expected), "missing {expected} in {keys:?}");
        }
    }
}
