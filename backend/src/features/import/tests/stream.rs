use super::*;

#[test]
fn preview_text_strips_ansi_escapes_before_length_cap() {
    assert_eq!(preview_text("\u{1b}[31mweb-01\u{1b}[0m"), "web-01");
    assert_eq!(preview_text("plain"), "plain");
    // 帽在剥除之后计算：剥除后恰好触界的字符串不再截断。
    let long = format!("{}\u{1b}[0m", "y".repeat(MAX_PREVIEW_TEXT_BYTES));
    assert_eq!(preview_text(&long), "y".repeat(MAX_PREVIEW_TEXT_BYTES));
}

#[test]
fn parsers_reject_session_counts_before_building_an_unbounded_preview_vec() {
    let mut text = String::from("[Bookmarks]\n");
    for index in 0..=MAX_PREVIEW_SESSIONS {
        text.push_str(&format!("s{index}=#109#0%10.0.0.1%22%root%\n"));
    }
    let error = parse_moba_ini(&text).expect_err("oversized preview must fail closed");
    assert!(error.contains("session limit"), "{error}");

    let entries = (0..=MAX_PREVIEW_SESSIONS)
        .map(|index| (format!("Xshell/Sessions/s{index}.xsh"), WEB_XSH.to_string()))
        .collect::<Vec<_>>();
    let refs = entries
        .iter()
        .map(|(name, body)| (name.as_str(), body.as_str()))
        .collect::<Vec<_>>();
    let error = parse_xshell(&xshell_zip(&refs))
        .expect_err("ZIP preview must fail before a giant session Vec");
    assert!(error.contains("session limit"), "{error}");
}

#[test]
fn startup_removes_legacy_import_store_without_exposing_its_contents() {
    let dir = std::env::temp_dir().join(format!("dbx-import-migrate-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let legacy = dir.join("imported-connections.json");
    std::fs::write(&legacy, r#"{"password":"do-not-leak"}"#).unwrap();
    assert!(remove_legacy_store(&dir).is_ok());
    assert!(!legacy.exists());
    assert!(
        remove_legacy_store(&dir).is_ok(),
        "missing legacy files are ignored"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn preview_start_enforces_task_and_process_memory_budgets() {
    let stream = ImportStream::default();
    for _ in 0..MAX_IMPORT_TASKS {
        stream
            .start(&json!({ "kind": "moba", "mainSize": 0 }))
            .unwrap();
    }
    assert!(stream
        .start(&json!({ "kind": "moba", "mainSize": 0 }))
        .is_err());
    let bounded = ImportStream::default();
    assert!(bounded
        .start(&json!({ "kind": "moba", "mainSize": MAX_IMPORT_MEMORY_BYTES + 1 }))
        .is_err());
}

#[test]
fn repeated_start_cancel_releases_every_slot_without_spawning_workers() {
    let stream = ImportStream::default();
    for _ in 0..MAX_IMPORT_TASKS * 32 {
        let task = stream
            .start(&json!({ "kind": "moba", "mainSize": 1 }))
            .unwrap();
        let task_id = task["taskId"].as_str().unwrap();
        assert!(stream.cancel(task_id));
    }
    assert_eq!(stream.active_task_count(), 0);
    assert_eq!(stream.reserved_bytes(), 0);
}

#[test]
fn start_lazily_reclaims_expired_tasks_before_enforcing_limits() {
    let stream = ImportStream::default();
    for _ in 0..MAX_IMPORT_TASKS {
        stream
            .start(&json!({ "kind": "moba", "mainSize": 1 }))
            .unwrap();
    }
    assert!(stream.expire_before(Instant::now() + IMPORT_TASK_TTL));
    assert_eq!(stream.active_task_count(), 0);
    assert!(stream
        .start(&json!({ "kind": "moba", "mainSize": 1 }))
        .is_ok());
}

#[test]
fn expired_preview_is_removed_before_next_operation() {
    let stream = ImportStream::default();
    let task = stream
        .start(&json!({ "kind": "moba", "mainSize": 1 }))
        .unwrap();
    let id = task["taskId"].as_str().unwrap();
    assert!(stream.expire_before(std::time::Instant::now() + IMPORT_TASK_TTL));
    assert!(stream.finish(id).is_err());
}

#[test]
fn streaming_preview_requires_contiguous_offsets_and_cleans_up_on_failure() {
    let stream = ImportStream::default();
    let started = stream
        .start(&json!({ "kind": "moba", "mainSize": 4 }))
        .unwrap();
    let task_id = started["taskId"].as_str().unwrap();
    assert_eq!(started["chunkSize"], IMPORT_CHUNK_LIMIT);
    let mut first = 0u64.to_be_bytes().to_vec();
    first.extend_from_slice(b"[Boo");
    assert_eq!(stream.append(task_id, "main", &first).unwrap(), 4);
    let mut wrong = 3u64.to_be_bytes().to_vec();
    wrong.extend_from_slice(b"x");
    assert!(stream.append(task_id, "main", &wrong).is_err());
    assert!(!stream.cancel(task_id));
}

#[test]
fn streaming_preview_finishes_with_sanitized_rows_and_discards_source() {
    let text = b"[Bookmarks]\nweb=#109#0%10.6.0.1%22%root%\n";
    let stream = ImportStream::default();
    let started = stream
        .start(&json!({ "kind": "moba", "mainSize": text.len() }))
        .unwrap();
    let task_id = started["taskId"].as_str().unwrap();
    let mut frame = 0u64.to_be_bytes().to_vec();
    frame.extend_from_slice(text);
    assert_eq!(
        stream.append(task_id, "main", &frame).unwrap(),
        text.len() as u64
    );
    let preview = stream.finish(task_id).unwrap();
    assert_eq!(preview["sessions"][0]["name"], "web");
    assert_eq!(preview["sessions"][0]["groupPath"], "");
    assert!(!preview.to_string().contains("password"));
    assert!(stream.finish(task_id).is_err());
}
