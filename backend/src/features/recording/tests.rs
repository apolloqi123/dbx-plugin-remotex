use super::*;

fn temp_dir() -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("dbx-session-recording-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn clear_recordings_deletes_only_cast_files() {
    let dir = temp_dir();
    let recordings = recordings_dir(&dir);
    std::fs::create_dir_all(&recordings).unwrap();
    std::fs::write(recordings.join("a.cast"), "{}").unwrap();
    std::fs::write(recordings.join("b.cast"), "{}").unwrap();
    std::fs::write(recordings.join("keep.txt"), "not a recording").unwrap();
    assert_eq!(clear_recordings(&dir), 2);
    assert!(!recordings.join("a.cast").exists());
    assert!(!recordings.join("b.cast").exists());
    assert!(recordings.join("keep.txt").exists());
    // 目录不存在时安全返回 0。
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(clear_recordings(&dir), 0);
}

#[cfg(unix)]
#[test]
fn recording_file_and_dir_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir();
    let recorder = SessionRecorder::start(&dir, "rec-perm", "c", "h", "s", 80, 24).unwrap();
    drop(recorder);
    // 录像含终端回显的全部输出：文件 0600、目录 0700，与 otp/preferences 同纪律。
    let meta = std::fs::metadata(recordings_dir(&dir).join("rec-perm.cast")).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let dir_meta = std::fs::metadata(recordings_dir(&dir)).unwrap();
    assert_eq!(dir_meta.permissions().mode() & 0o777, 0o700);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn start_observe_finish_writes_asciicast_v2() {
    let dir = temp_dir();
    let mut recorder =
        SessionRecorder::start(&dir, "rec-1", "conn-1", "web-01", "sess-1", 120, 40).unwrap();
    recorder.observe(b"hello ");
    recorder.observe(b"world\n");
    let summary = recorder.finish().unwrap();
    assert_eq!(summary["events"], 2);
    assert_eq!(summary["recordingId"], "rec-1");
    let text = std::fs::read_to_string(recordings_dir(&dir).join("rec-1.cast")).unwrap();
    let mut lines = text.lines();
    let head: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(head["version"], 2);
    assert_eq!(head["width"], 120);
    assert_eq!(head["meta"]["host"], "web-01");
    assert_eq!(head["meta"]["sessionId"], "sess-1");
    let events: Vec<Value> = lines
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 3, "two output events plus the stop marker");
    assert_eq!(events[0]["eventtype"], "o");
    assert_eq!(events[0]["eventdata"], "hello ");
    assert_eq!(events[1]["eventdata"], "world\n");
    assert!(events[0]["time"].as_f64().unwrap() <= events[1]["time"].as_f64().unwrap());
    // 尾标记记录墙钟时长：列表据此显示时长，静默录制也不会是 0。
    assert_eq!(events[2]["eventtype"], "m");
    assert_eq!(events[2]["eventdata"], "");
    assert!(events[2]["time"].as_f64().unwrap() >= events[1]["time"].as_f64().unwrap());
    assert_eq!(events[2]["time"], summary["durationSecs"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_bursts_and_huge_bursts_are_handled() {
    let dir = temp_dir();
    let mut recorder = SessionRecorder::start(&dir, "rec-2", "c", "h", "s", 80, 24).unwrap();
    recorder.observe(b"");
    let huge = vec![b'x'; MAX_EVENT_BYTES + 4096];
    recorder.observe(&huge);
    let summary = recorder.finish().unwrap();
    assert_eq!(summary["events"], 1, "empty bursts write nothing");
    assert_eq!(summary["bytes"], MAX_EVENT_BYTES as u64);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn list_reports_headers_and_duration_newest_first() {
    let dir = temp_dir();
    let mut older = SessionRecorder::start(&dir, "rec-a", "c", "h1", "s1", 80, 24).unwrap();
    older.observe(b"one\n");
    older.finish().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut newer = SessionRecorder::start(&dir, "rec-b", "c", "h2", "s2", 80, 24).unwrap();
    newer.observe(b"two\n");
    newer.finish().unwrap();

    let list = list_recordings(&dir);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["recordingId"], "rec-b", "newest first");
    assert_eq!(list[1]["recordingId"], "rec-a");
    assert_eq!(list[1]["host"], "h1");
    assert!(list[1]["startedAt"].as_u64().unwrap() > 0);
    assert!(list[1]["durationSecs"].as_f64().unwrap() >= 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn read_events_paginates_and_counts() {
    let dir = temp_dir();
    let mut recorder = SessionRecorder::start(&dir, "rec-3", "c", "h", "s", 80, 24).unwrap();
    for index in 0..7 {
        recorder.observe(format!("line-{index}\n").as_bytes());
    }
    recorder.finish().unwrap();
    let page = read_events(&dir, "rec-3", 0, 5).unwrap();
    // 7 条输出事件 + 1 条收尾标记（非输出），分页按文件行计。
    assert_eq!(page["total"], 8);
    assert_eq!(page["events"].as_array().unwrap().len(), 5);
    assert_eq!(page["hasMore"], true);
    let tail = read_events(&dir, "rec-3", 5, 5).unwrap();
    assert_eq!(tail["events"].as_array().unwrap().len(), 3);
    assert_eq!(tail["hasMore"], false);
    assert_eq!(
        tail["events"][1]["data"],
        json!("line-6\n"),
        "event order is oldest first"
    );
    assert_eq!(tail["events"][2]["type"], "m");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Issue #12 回归：「只点击了录制然后终止」产出仅含 header 的录制
/// （0 输出事件），必须照常列入列表且可经 RPC 同路径（cast_path + 删文件）
/// 删除，删除后从列表消失。
///
/// 静默录制同样要有真实时长：时长取自停止时写入的墙钟标记，而不是最后
/// 一条输出事件（没有输出时旧实现恒为 0 秒）。
#[test]
fn header_only_recording_is_listed_and_deletable() {
    let dir = temp_dir();
    let recorder = SessionRecorder::start(&dir, "rec-empty", "c", "h", "s", 80, 24).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(40));
    recorder.finish().unwrap();

    let list = list_recordings(&dir);
    assert_eq!(
        list.len(),
        1,
        "header-only recording still shows in the list"
    );
    assert_eq!(list[0]["recordingId"], "rec-empty");
    assert!(
        list[0]["durationSecs"].as_f64().unwrap() >= 0.03,
        "silent recording keeps its wall-clock duration"
    );

    let path = cast_path(&dir, "rec-empty").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let marker = text.lines().last().unwrap();
    let marker: Value = serde_json::from_str(marker).unwrap();
    assert_eq!(marker["eventtype"], "m");

    std::fs::remove_file(&path).unwrap();
    assert!(!path.exists());
    assert!(
        list_recordings(&dir).is_empty(),
        "deleted recording leaves the list"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 旧版本（无收尾标记、无输出事件）的录制：用文件修改时间与 header 启动
/// 时间估算墙钟时长，避免升级后旧记录永远显示 0 秒。
#[test]
fn legacy_header_only_recording_estimates_duration_from_mtime() {
    let dir = temp_dir();
    let recordings = recordings_dir(&dir);
    std::fs::create_dir_all(&recordings).unwrap();
    let started = crate::core::time::unix_now_secs().saturating_sub(42);
    let header = json!({
        "version": 2,
        "width": 80,
        "height": 24,
        "timestamp": started,
        "meta": { "sessionId": "s", "connectionId": "c", "host": "h" },
    });
    std::fs::write(recordings.join("rec-legacy.cast"), format!("{header}\n")).unwrap();

    let list = list_recordings(&dir);
    assert_eq!(list.len(), 1);
    let duration = list[0]["durationSecs"].as_f64().unwrap();
    assert!(
        (40.0..44.0).contains(&duration),
        "legacy header-only recording estimates ~42s, got {duration}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cast_path_rejects_traversal() {
    let dir = temp_dir();
    assert!(cast_path(&dir, "../escape").is_err());
    assert!(cast_path(&dir, "a/b").is_err());
    assert!(cast_path(&dir, "").is_err());
    assert!(cast_path(&dir, "ok-uuid").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn flatten_event_text_strips_control_sequences() {
    // 颜色/OSC 控制序列剥掉后命中词才不会被转义符藏起来；换行/制表压成
    // 空格（摘录保持单行），CR 直接丢弃。
    let event =
        json!({ "eventdata": "\x1b[31mER\x1b[0mROR\x1b]0;title\x07: boom\r\nnext\tline" });
    let flat = flatten_event_text(&event);
    assert_eq!(flat, "ERROR: boom next line");
    assert_eq!(flatten_event_text(&json!({ "eventdata": "" })), "");
    // 非法/悬空转义序列也被吞掉，不会把 ESC 泄进摘录。
    assert_eq!(flatten_event_text(&json!({ "eventdata": "a\x1bZb" })), "ab");
}

#[test]
fn excerpt_around_windows_on_match() {
    let text = "aaaaaaaaaa NEEDLE bbbbbbbbbb";
    // 窗口锚定在命中处：命中词完整显示，前面截断带前导省略号。
    assert_eq!(excerpt_around(text, "needle", 12), "…NEEDLE bbb…");
    // 预算足够时命中之后到结尾全部保留，只有前导省略号（窗口锚定命中处）。
    assert_eq!(excerpt_around(text, "needle", 40), "…NEEDLE bbbbbbbbbb");
    assert_eq!(excerpt_around("no match here", "zzz", 10), "");
    // 多字节字符按字符窗口截取，不切半个 code point；命中词保持完整。
    assert_eq!(excerpt_around("你好needle世界", "needle", 9), "…needle世…");
}

#[test]
fn search_matches_name_and_content_newest_first() {
    let dir = temp_dir();
    let mut older = SessionRecorder::start(&dir, "rec-a", "c", "web-01", "s1", 80, 24).unwrap();
    older.observe(b"health check passed\n");
    older.observe(b"failed to start service\n");
    older.observe(b"second health mention\n");
    older.finish().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut newer = SessionRecorder::start(&dir, "rec-b", "c", "db-02", "s2", 80, 24).unwrap();
    newer.observe(b"\x1b[32mHealth Check OK\x1b[0m\n");
    newer.finish().unwrap();
    std::fs::write(recordings_dir(&dir).join("not-a-cast.txt"), "health").unwrap();

    // 内容命中（大小写不敏感 + 转义符后面的词也算）。
    let hits = search_recordings(&dir, "HEALTH");
    assert_eq!(
        hits.len(),
        2,
        "both recordings mention health; newest first"
    );
    assert_eq!(hits[0]["recordingId"], "rec-b");
    assert_eq!(hits[0]["nameMatch"], false);
    let rec_b_hits = hits[0]["hits"].as_array().unwrap();
    assert_eq!(rec_b_hits.len(), 1);
    assert_eq!(rec_b_hits[0]["excerpt"], "Health Check OK");
    let rec_a_hits = hits[1]["hits"].as_array().unwrap();
    assert_eq!(rec_a_hits.len(), 2, "both matching events listed");
    assert_eq!(rec_a_hits[0]["excerpt"], "health check passed");
    assert!(rec_a_hits[0]["time"].as_f64().unwrap() <= rec_a_hits[1]["time"].as_f64().unwrap());

    // 名称命中：host 包含查询词即可，即使内容不匹配也列出（hits 空）。
    let by_host = search_recordings(&dir, "db-0");
    assert_eq!(by_host.len(), 1);
    assert_eq!(by_host[0]["recordingId"], "rec-b");
    assert_eq!(by_host[0]["nameMatch"], true);

    let by_id = search_recordings(&dir, "REC-A");
    assert_eq!(by_id.len(), 1);
    assert_eq!(by_id[0]["nameMatch"], true);

    // 空查询/无命中：空集。
    assert!(search_recordings(&dir, "").is_empty());
    assert!(search_recordings(&dir, "zzz-not-present").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_caps_hits_and_respects_missing_dir() {
    let dir = temp_dir();
    let mut recorder = SessionRecorder::start(&dir, "rec-cap", "c", "h", "s", 80, 24).unwrap();
    for index in 0..(SEARCH_MAX_HITS + 3) {
        recorder.observe(format!("oops {index}\n").as_bytes());
    }
    recorder.finish().unwrap();
    let hits = search_recordings(&dir, "oops");
    assert_eq!(hits.len(), 1);
    let rows = hits[0]["hits"].as_array().unwrap();
    assert_eq!(rows.len(), SEARCH_MAX_HITS);
    // 不存在的目录安全返回空。
    assert!(search_recordings(&temp_dir(), "oops").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn auto_record_flag_round_trips() {
    set_auto_record(true);
    assert!(auto_record_enabled());
    set_auto_record(false);
    assert!(!auto_record_enabled());
}
