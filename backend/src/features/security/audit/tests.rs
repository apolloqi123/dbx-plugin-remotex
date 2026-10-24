use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dbx-audit-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample_entry(ts_ms: u64) -> AuditEntry {
    AuditEntry {
        ts_ms,
        tool: "ssh_exec".to_string(),
        connection_id: "conn-1".to_string(),
        gate: GateOutcome::Pass,
        approval: ApprovalTrail::None,
        outcome: EntryOutcome::Ok,
        exit_code: Some(0),
        duration_ms: 12,
        mode: ExecMode::Embedded,
        command: Some("echo hi".to_string()),
        output: Some("hi".to_string()),
        error: None,
    }
}

#[test]
fn clear_truncates_active_and_drops_rotated_generation() {
    let dir = temp_dir("clear");
    append(&dir, &sample_entry(1)).unwrap();
    append(&dir, &sample_entry(2)).unwrap();
    // Simulate a rotated predecessor from an earlier fill.
    std::fs::write(rotated_path(&dir), b"{\"stale\":true}\n").unwrap();
    clear(&dir).unwrap();
    assert!(tail(&dir, 100, None).unwrap().is_empty());
    assert!(
        !rotated_path(&dir).exists(),
        "rotated generation must be dropped"
    );
    // Appends after a clear start the new generation cleanly.
    append(&dir, &sample_entry(3)).unwrap();
    assert_eq!(tools(&tail(&dir, 100, None).unwrap()), vec!["ssh_exec"]);
    // Clear on a missing file is a no-op success.
    let empty = temp_dir("clear-empty");
    clear(&empty).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&empty);
}

fn tools(entries: &[AuditEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.tool.as_str()).collect()
}

#[test]
fn gate_strings_match_the_protocol_contract() {
    let cases = [
        (GateOutcome::Pass, "pass"),
        (GateOutcome::WriteDenied, "write-denied"),
        (GateOutcome::WhitelistDenied, "whitelist-denied"),
        (GateOutcome::SensitivePath, "sensitive-path"),
        (
            GateOutcome::DestructiveUnconfirmed,
            "destructive-unconfirmed",
        ),
        (GateOutcome::SudoAllowlistDenied, "sudo-allowlist-denied"),
        (GateOutcome::ReadOnlyServer, "read-only-server"),
    ];
    for (value, text) in cases {
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::json!(text)
        );
    }
}

#[test]
fn approval_outcome_mode_strings_match_the_protocol_contract() {
    let approvals = [
        (ApprovalTrail::None, "none"),
        (ApprovalTrail::Prompt, "prompt"),
        (ApprovalTrail::Approved, "approved"),
        (ApprovalTrail::Denied, "denied"),
        (ApprovalTrail::Timeout, "timeout"),
        (ApprovalTrail::Remembered, "remembered"),
    ];
    for (value, text) in approvals {
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::json!(text)
        );
    }
    for (value, text) in [(EntryOutcome::Ok, "ok"), (EntryOutcome::Error, "error")] {
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::json!(text)
        );
    }
    for (value, text) in [
        (ExecMode::Stdio, "stdio"),
        (ExecMode::Embedded, "embedded"),
        (ExecMode::Terminal, "terminal"),
    ] {
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::json!(text)
        );
    }
}

#[test]
fn entry_serializes_the_exact_wire_shape() {
    let entry = sample_entry(1_730_000_000_000);
    // Key set (`to_value`'s map is unordered, so compare sorted).
    let value = serde_json::to_value(&entry).unwrap();
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "approval",
            "command",
            "connectionId",
            "durationMs",
            "error",
            "exitCode",
            "gate",
            "mode",
            "outcome",
            "output",
            "tool",
            "tsMs"
        ]
    );
    let object = serde_json::to_value(&entry).unwrap();
    assert_eq!(object["exitCode"], 0);
    assert_eq!(object["error"], serde_json::Value::Null, "None stays null");
    // Field order on the actual JSONL line the audit file stores.
    let line = serde_json::to_string(&entry).unwrap();
    let mut cursor = 0;
    for key in [
        "\"tsMs\":",
        "\"tool\":",
        "\"connectionId\":",
        "\"gate\":",
        "\"approval\":",
        "\"outcome\":",
        "\"exitCode\":",
        "\"durationMs\":",
        "\"mode\":",
        "\"command\":",
        "\"output\":",
        "\"error\":",
    ] {
        let at = line[cursor..]
            .find(key)
            .unwrap_or_else(|| panic!("{key} missing or out of order in {line}"));
        cursor += at + key.len();
    }
}

#[test]
fn append_then_tail_roundtrips_every_field() {
    let dir = temp_dir("roundtrip");
    append(&dir, &sample_entry(1_000)).unwrap();
    let mut denied = sample_entry(2_000);
    denied.tool = "ssh_exec_sudo".to_string();
    denied.connection_id = "conn-2".to_string();
    denied.gate = GateOutcome::DestructiveUnconfirmed;
    denied.approval = ApprovalTrail::Denied;
    denied.outcome = EntryOutcome::Error;
    denied.exit_code = None;
    denied.duration_ms = 340;
    denied.mode = ExecMode::Terminal;
    denied.command = Some("rm -rf /tmp/scratch".to_string());
    denied.output = None;
    denied.error = Some("needs confirmation".to_string());
    append(&dir, &denied).unwrap();

    let entries = tail(&dir, 10, None).unwrap();
    assert_eq!(entries.len(), 2);
    let (first, second) = (&entries[0], &entries[1]);
    assert_eq!(first.ts_ms, 1_000);
    assert_eq!(first.tool, "ssh_exec");
    assert_eq!(first.connection_id, "conn-1");
    assert_eq!(first.gate, GateOutcome::Pass);
    assert_eq!(first.approval, ApprovalTrail::None);
    assert_eq!(first.outcome, EntryOutcome::Ok);
    assert_eq!(first.exit_code, Some(0));
    assert_eq!(first.duration_ms, 12);
    assert_eq!(first.mode, ExecMode::Embedded);
    assert_eq!(first.command.as_deref(), Some("echo hi"));
    assert_eq!(first.output.as_deref(), Some("hi"));
    assert_eq!(first.error, None);

    assert_eq!(second.ts_ms, 2_000);
    assert_eq!(second.tool, "ssh_exec_sudo");
    assert_eq!(second.connection_id, "conn-2");
    assert_eq!(second.gate, GateOutcome::DestructiveUnconfirmed);
    assert_eq!(second.approval, ApprovalTrail::Denied);
    assert_eq!(second.outcome, EntryOutcome::Error);
    assert_eq!(second.exit_code, None, "None survives as null");
    assert_eq!(second.duration_ms, 340);
    assert_eq!(second.mode, ExecMode::Terminal);
    assert_eq!(second.command.as_deref(), Some("rm -rf /tmp/scratch"));
    assert_eq!(second.output, None);
    assert_eq!(second.error.as_deref(), Some("needs confirmation"));

    // Raw line keeps nulls in place (stable row shape).
    let raw = std::fs::read_to_string(audit_path(&dir)).unwrap();
    assert!(raw.contains("\"exitCode\":null"));
    assert!(raw.contains("\"error\":null"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tail_filters_by_ts_and_takes_the_newest_limit() {
    let dir = temp_dir("filter");
    for ts in 1..=6u64 {
        let mut entry = sample_entry(ts * 1_000);
        entry.tool = format!("t{ts}");
        append(&dir, &entry).unwrap();
    }
    // Newest N, still oldest → newest inside the window.
    assert_eq!(tools(&tail(&dir, 2, None).unwrap()), ["t5", "t6"]);
    // before_ts is exclusive: t4 (ts 4000) is not < 4000.
    assert_eq!(
        tools(&tail(&dir, 500, Some(4_000)).unwrap()),
        ["t1", "t2", "t3"]
    );
    assert!(tail(&dir, 0, None).unwrap().is_empty());
    assert_eq!(tail(&dir, 500, None).unwrap().len(), 6);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tail_skips_malformed_and_partial_lines() {
    let dir = temp_dir("badlines");
    append(&dir, &sample_entry(1_000)).unwrap();
    let path = audit_path(&dir);
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("{not json\n");
    text.push('\n');
    text.push_str("{\"tsMs\":2000,\"tool\":\"orphan\"}\n");
    std::fs::write(&path, text).unwrap();
    append(&dir, &sample_entry(3_000)).unwrap();

    let entries = tail(&dir, 500, None).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].ts_ms, 1_000);
    assert_eq!(entries[1].ts_ms, 3_000);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tail_on_missing_file_yields_empty() {
    let dir = temp_dir("missing");
    assert!(tail(&dir, 10, None).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn append_clamps_long_error_text() {
    let dir = temp_dir("clamp");
    let mut entry = sample_entry(1_000);
    entry.outcome = EntryOutcome::Error;
    entry.error = Some("e".repeat(4_096));
    append(&dir, &entry).unwrap();
    let loaded = tail(&dir, 10, None).unwrap();
    let error = loaded[0].error.as_deref().unwrap();
    assert_eq!(error.chars().count(), MAX_ERROR_CHARS);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn append_clamps_command_and_output_text() {
    let dir = temp_dir("clamp-cmd");
    let mut entry = sample_entry(1_000);
    entry.command = Some("c".repeat(2_048));
    entry.output = Some("o".repeat(4_096));
    append(&dir, &entry).unwrap();
    let loaded = tail(&dir, 10, None).unwrap();
    assert_eq!(
        loaded[0]
            .command
            .as_deref()
            .map(str::chars)
            .map(Iterator::count),
        Some(MAX_COMMAND_CHARS)
    );
    assert_eq!(
        loaded[0]
            .output
            .as_deref()
            .map(str::chars)
            .map(Iterator::count),
        Some(MAX_OUTPUT_CHARS)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_lines_without_command_output_still_replay() {
    // 0.4.77 之前的行没有 command/output 字段：tail 必须继续回放（缺省
    // 为 null），老审计文件不能因为新字段作废。
    let dir = temp_dir("legacy");
    let path = audit_path(&dir);
    std::fs::write(
        &path,
        "{\"tsMs\":1000,\"tool\":\"ssh_exec\",\"connectionId\":\"conn-1\",\"gate\":\"pass\",\"approval\":\"none\",\"outcome\":\"ok\",\"exitCode\":0,\"durationMs\":12,\"mode\":\"embedded\",\"error\":null}\n",
    )
    .unwrap();
    let entries = tail(&dir, 10, None).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].command, None);
    assert_eq!(entries[0].output, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn full_file_rotates_and_the_fresh_file_keeps_the_new_entry() {
    let dir = temp_dir("rotate");
    let path = audit_path(&dir);
    // Fill with ≥ ROTATE_BYTES of padding (garbage lines are fine).
    std::fs::write(&path, vec![b'x'; (ROTATE_BYTES + 1_024) as usize]).unwrap();
    append(&dir, &sample_entry(5_000)).unwrap();

    let rotated = std::fs::metadata(rotated_path(&dir)).unwrap();
    assert!(rotated.len() >= ROTATE_BYTES, "old generation preserved");
    let fresh = std::fs::read_to_string(&path).unwrap();
    assert!(
        fresh.contains("\"tool\":\"ssh_exec\""),
        "new file holds the entry"
    );
    // The padding was not valid JSON, so tail sees only the new entry.
    let entries = tail(&dir, 500, None).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].ts_ms, 5_000);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn concurrent_appends_within_one_process_all_land() {
    let dir = temp_dir("concurrent");
    let dir = std::sync::Arc::new(dir);
    let mut handles = Vec::new();
    for thread_index in 0..8u32 {
        let dir = dir.clone();
        handles.push(std::thread::spawn(move || {
            for index in 0..10u32 {
                let mut entry = sample_entry(1_000 + (thread_index * 100 + index) as u64);
                entry.tool = format!("tool-{thread_index}-{index}");
                append(&dir, &entry).unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    let entries = tail(&dir, 500, None).unwrap();
    assert_eq!(entries.len(), 80, "every append survives");
    let mut names: Vec<&str> = entries.iter().map(|e| e.tool.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 80, "no line torn into another");
    let _ = std::fs::remove_dir_all(&*dir);
}
