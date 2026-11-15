use super::*;

#[test]
fn sudo_upload_staging_name_uses_dedicated_prefix() {
    let name = sudo_upload_staging_name("task-1");
    assert!(name.starts_with(".dbx-sudo-ul-") && name.ends_with(".part"));
    // 与下载车道前缀互不误伤（清理命令按前缀钉死命名空间）。
    assert!(!name.starts_with(".dbx-sudo-dl-"));
    assert_eq!(sudo_upload_staging_name("t"), ".dbx-sudo-ul-t.part");
}

#[test]
fn sudo_commit_body_quotes_paths_and_rolls_back() {
    let body = sudo_commit_body(
        "/home/u/.dbx-sudo-ul-t.part",
        "/root/a.bin",
        "/root/.dbx-upload-t.backup",
    );
    // 三个路径都单引号转义，让位→提交→清理/回滚次序固定。
    assert!(body.contains(
        "if [ -e '/root/a.bin' ]; then mv -f -- '/root/a.bin' '/root/.dbx-upload-t.backup'; fi"
    ));
    assert!(body.contains("if mv -f -- '/home/u/.dbx-sudo-ul-t.part' '/root/a.bin'; then rm -f -- '/root/.dbx-upload-t.backup'; exit 0; fi"));
    assert!(body.ends_with("if [ -e '/root/.dbx-upload-t.backup' ]; then mv -f -- '/root/.dbx-upload-t.backup' '/root/a.bin'; fi; exit 1"));
    // 嵌入单引号的路径经 POSIX '\'' 转义后整体仍是合法 shell。
    let quoted = sudo_commit_body("/home/u'/x", "/t'/y", "/b'/z");
    assert!(quoted.contains("'/home/u'\\''/x'"));
    assert!(quoted.contains("'/t'\\''/y'"));
}

#[test]
fn upload_progress_payload_marks_the_phase() {
    let staging = upload_progress_payload(
        "t1",
        "s1",
        Some("a.bin"),
        256,
        1024,
        UploadPhase::Staging,
        "running",
        CompressionMode::Gzip,
    );
    assert_eq!(staging["phase"], "staging");
    assert_eq!(staging["direction"], "upload");
    assert_eq!(staging["fileName"], "a.bin");
    assert_eq!(staging["transferred"], 256);
    assert_eq!(staging["size"], 1024);
    assert_eq!(staging["status"], "running");
    // M33：上传进度事件与下载同语义，恒带 compression 字段（含 none）。
    assert_eq!(staging["compression"], "gzip");

    let pushing = upload_progress_payload(
        "t1",
        "s1",
        None,
        512,
        1024,
        UploadPhase::Uploading,
        "running",
        CompressionMode::None,
    );
    assert_eq!(pushing["phase"], "uploading");
    assert_eq!(pushing["compression"], "none");
    // Non-start events carry no fileName; the workbench keeps its own.
    assert!(pushing.get("fileName").is_none());
}

#[test]
fn upload_cancel_error_tells_abort_reasons_apart() {
    assert_eq!(
        upload_cancel_error(Some("user")),
        "Upload cancelled by user"
    );
    assert_eq!(
        upload_cancel_error(Some(" ack-timeout ")),
        "Upload cancelled (ack-timeout)"
    );
    assert_eq!(upload_cancel_error(None), "Upload cancelled");
    assert_eq!(upload_cancel_error(Some("   ")), "Upload cancelled");
}

#[test]
fn numeric_or_absent_owner_values_ask_for_names() {
    assert!(owner_needs_name(&None));
    assert!(owner_needs_name(&Some("0".to_string())));
    assert!(owner_needs_name(&Some("1000".to_string())));
    // Already a name (SFTPv4+ attribute or a previous lookup): no work.
    assert!(!owner_needs_name(&Some("root".to_string())));
    // Defensive: an empty string would render as "-" either way.
    assert!(!owner_needs_name(&Some(String::new())));
}

#[test]
fn parse_ls_owner_map_reads_gnu_epoch_layout() {
    let output = "\
total 20
drwxr-xr-x  3 root root 4096 1720000000 .
drwxr-xr-x  1 root wheel 4096 1720000001 ..
-rw-r--r--  1 alice docker  123 1720000002 notes.txt
drwxr-xr-x  2 root root 4096 1720000003 sub dir with spaces
lrwxrwxrwx  1 root root   11 1720000004 link -> notes.txt
";
    let map = parse_ls_owner_map(output);
    assert_eq!(
        map.get("notes.txt"),
        Some(&("alice".to_string(), "docker".to_string()))
    );
    assert_eq!(
        map.get("sub dir with spaces"),
        Some(&("root".to_string(), "root".to_string()))
    );
    // Symlink arrow is stripped from the name key.
    assert_eq!(
        map.get("link"),
        Some(&("root".to_string(), "root".to_string()))
    );
    // "." / ".." come along but harmless: they never match SftpEntry names.
    assert_eq!(
        map.get("."),
        Some(&("root".to_string(), "root".to_string()))
    );
}

#[test]
fn parse_ls_owner_map_reads_classic_layout() {
    // BusyBox/BSD dates spread over three fields; owner/group stay in
    // fields 3/4 regardless.
    let output = "\
-rw-r--r--    1 root     root          4096 Jan 15 10:23 readme.md
-rw-r--r--    1 svc     deploy          123 Jan 15  2024 old.log
";
    let map = parse_ls_owner_map(output);
    assert_eq!(
        map.get("readme.md"),
        Some(&("root".to_string(), "root".to_string()))
    );
    assert_eq!(
        map.get("old.log"),
        Some(&("svc".to_string(), "deploy".to_string()))
    );
}

#[test]
fn parse_ls_owner_map_ignores_headers_and_noise() {
    assert!(parse_ls_owner_map("").is_empty());
    assert!(parse_ls_owner_map("total 20\n").is_empty());
    assert!(parse_ls_owner_map("ls: cannot open '/x': Permission denied\n").is_empty());
    // A row too short to carry owner/group is dropped, not misparsed.
    assert!(parse_ls_owner_map("-rw-r--r-- 1 x\n").is_empty());
}

#[test]
fn parse_ls_owner_map_keeps_first_row_for_duplicate_names() {
    let output = "\
-rw-r--r--  1 root root 1 1720000000 a.txt
-rw-r--r--  1 amy ops  2 1720000001 a.txt
";
    let map = parse_ls_owner_map(output);
    assert_eq!(
        map.get("a.txt"),
        Some(&("root".to_string(), "root".to_string()))
    );
}

#[test]
fn upload_meta_roundtrips_and_rejects_corruption() {
    let dir = temp_transfer_dir();
    let path = dir.join("upload-t1.json");
    write_upload_meta(&path, &json!({ "remotePath": "/srv/a.bin", "size": 9 })).unwrap();
    let meta = read_upload_meta(&path).unwrap();
    assert_eq!(meta["remotePath"], "/srv/a.bin");
    assert_eq!(meta["size"], 9);
    std::fs::write(&path, "{broken").unwrap();
    assert!(read_upload_meta(&path).is_none());
    assert!(read_upload_meta(&dir.join("missing.json")).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// T2d 字段类型错乱矩阵：断电/并发写坏/被外部工具改写的 meta 不得让
/// resume 接受错误语义。真实语义（实现决定断言）：
/// - `read_upload_meta` 只保证"能解析成 JSON 就返回"——数组/字符串
///   类型的 size 都能读出值，防线在读取方的 `as_str`/`as_u64`；
/// - `open_resume_spool` 对类型错乱字段一律拒绝（None/不匹配 → Err），
///   resume 永远不会把字符串 size 当成 0 或静默通过；
/// - 截断 JSON 连解析都不过，直接 None。
#[test]
fn upload_meta_field_corruption_is_rejected_not_misread() {
    use serde_json::Value;
    let dir = temp_transfer_dir();
    let meta_with = |text: &str, name: &str| {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        read_upload_meta(&path)
    };

    // size 是字符串："9" ≠ 9（u64），resume 必须拒绝而不是宽容转换。
    let stringy = meta_with(
        r#"{"remotePath":"/srv/a.bin","size":"9"}"#,
        "upload-t-str.json",
    )
    .unwrap();
    assert_eq!(stringy.get("size").and_then(Value::as_u64), None);
    let runtime = SshRuntime::new(dir.clone());
    std::fs::write(dir.join("upload-t-str.part"), b"xxxx").unwrap();
    assert!(runtime.open_resume_spool("t-str", "/srv/a.bin", 9).is_err());

    // offset 字段错乱（负数/字符串）：当前实现把 offset 放在二进制
    // 分块头部（BE u64），meta 里出现该字段属于垃圾数据，读取方忽略
    // 它即可——断言 meta 仍能解析且不会被任何字段误升级为合法 resume。
    let negative = meta_with(
        r#"{"remotePath":"/srv/a.bin","size":9,"offset":-4}"#,
        "upload-t-neg.json",
    )
    .unwrap();
    assert_eq!(negative.get("offset").and_then(Value::as_u64), None);
    // size 为负数同理：JSON 负数不满足 as_u64。
    let neg_size = meta_with(
        r#"{"remotePath":"/srv/a.bin","size":-9}"#,
        "upload-t-negsize.json",
    )
    .unwrap();
    assert_eq!(neg_size.get("size").and_then(Value::as_u64), None);

    // meta 是 JSON 数组而非对象：解析成功但 as_str/as_u64 全部落空，
    // resumable 扫描与 resume 校验都必须跳过/拒绝。
    let array = meta_with(r#"["/srv/a.bin",9]"#, "upload-t-arr.json").unwrap();
    assert!(array.is_array());
    assert_eq!(array.get("remotePath"), None);
    std::fs::write(dir.join("upload-t-arr.part"), b"xxxx").unwrap();
    assert!(runtime.open_resume_spool("t-arr", "/srv/a.bin", 9).is_err());

    // 截断 JSON（写一半断电）：连解析都不过，read_upload_meta 返回
    // None，resumable 扫描直接跳过。
    assert!(meta_with(r#"{"remotePath":"/srv/a.bin","size"#, "upload-t-trunc.json").is_none());
    // resumable 扫描同样只收健康 meta：上述四种坏件全部不出现在清单。
    std::fs::write(dir.join("upload-t-negsize.part"), b"xxxx").unwrap();
    std::fs::write(dir.join("upload-t-trunc.part"), b"xxxx").unwrap();
    let tasks = resumable_uploads_from(&dir, &[]);
    assert!(
        tasks.is_empty(),
        "corrupted metas must not resurface: {tasks:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resume_spool_validation_matches_meta() {
    let dir = temp_transfer_dir();
    // SshRuntime's transfer spool lives under <data_dir>/transfers.
    let transfer_dir = dir.join("transfers");
    std::fs::create_dir_all(&transfer_dir).unwrap();
    let runtime = SshRuntime::new(dir.clone());
    spool_fixture(&transfer_dir, "t1", "/srv/a.bin", 100, 40);
    // Matching request resumes at the spooled length.
    let (path, offset) = runtime.open_resume_spool("t1", "/srv/a.bin", 100).unwrap();
    assert_eq!(offset, 40);
    assert!(path.ends_with("upload-t1.part"));
    // Mismatched size / remote path / missing meta are refused.
    assert!(runtime.open_resume_spool("t1", "/srv/a.bin", 99).is_err());
    assert!(runtime
        .open_resume_spool("t1", "/srv/other.bin", 100)
        .is_err());
    assert!(runtime
        .open_resume_spool("gone", "/srv/a.bin", 100)
        .is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resumable_scan_lists_only_live_pending_uploads() {
    let dir = temp_transfer_dir();
    let transfer_dir = dir.join("transfers");
    std::fs::create_dir_all(&transfer_dir).unwrap();
    spool_fixture(&transfer_dir, "t-resume", "/srv/a.bin", 100, 40);
    spool_fixture(&transfer_dir, "t-live", "/srv/b.bin", 100, 10);
    // Empty spool, oversized spool, and meta-less files are skipped.
    spool_fixture(&transfer_dir, "t-empty", "/srv/c.bin", 100, 0);
    spool_fixture(&transfer_dir, "t-over", "/srv/d.bin", 10, 40);
    std::fs::write(transfer_dir.join("upload-t-nometa.part"), b"data").unwrap();
    std::fs::write(transfer_dir.join("upload-t-nospool.json"), "{}").unwrap();
    let tasks = resumable_uploads_from(&transfer_dir, &["t-live".to_string()]);
    assert_eq!(tasks.len(), 1, "only t-resume qualifies: {tasks:?}");
    assert_eq!(tasks[0]["taskId"], "t-resume");
    assert_eq!(tasks[0]["remotePath"], "/srv/a.bin");
    assert_eq!(tasks[0]["fileName"], "a.bin");
    assert_eq!(tasks[0]["size"], 100);
    assert_eq!(tasks[0]["resumableBytes"], 40);
    // A missing transfer dir yields an empty list, not an error.
    assert!(resumable_uploads_from(Path::new("/nonexistent-dbx-ssh"), &[]).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

// 启动清扫（评审 H-2/L-2）：下载侧临时件整目录清空、上传孤儿 meta 与
// 可再生的 .gz 清除、健康续传态（.part+.json）原样保留。
#[test]
fn sweep_stale_transfer_temps_keeps_resumable_spools_only() {
    let dir = temp_transfer_dir();
    let transfer_dir = dir.join("transfers");
    let downloads = transfer_dir.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    // 下载侧：不可续传，全部清。
    std::fs::write(downloads.join("download-x.part"), b"p").unwrap();
    std::fs::write(downloads.join("download-x.part.gz"), b"g").unwrap();
    std::fs::write(downloads.join("download-x.plain"), b"l").unwrap();
    std::fs::write(downloads.join("tree-x.tgz"), b"t").unwrap();
    std::fs::create_dir_all(downloads.join("stale-sub")).unwrap();
    // 上传侧：健康续传态保留；孤儿 meta 与无主 .gz 清除；超龄 spool
    // 连同 meta 回收（7 天 TTL，评审 H-2 后续）。
    std::fs::write(transfer_dir.join("upload-keep.part"), b"data").unwrap();
    std::fs::write(transfer_dir.join("upload-keep.json"), "{}").unwrap();
    std::fs::write(transfer_dir.join("upload-keep.part.gz"), b"gz").unwrap();
    std::fs::write(transfer_dir.join("upload-orphan.json"), "{}").unwrap();
    std::fs::write(transfer_dir.join("upload-orphan.part.gz"), b"gz").unwrap();
    std::fs::write(transfer_dir.join("upload-aged.part"), b"data").unwrap();
    std::fs::write(transfer_dir.join("upload-aged.json"), "{}").unwrap();
    let aged = std::fs::OpenOptions::new()
        .append(true)
        .open(transfer_dir.join("upload-aged.part"))
        .unwrap();
    aged.set_modified(
        std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 60 * 60),
    )
    .unwrap();
    drop(aged);
    sweep_stale_transfer_temps(&transfer_dir);
    assert!(
        std::fs::read_dir(&downloads).unwrap().next().is_none(),
        "download temps must be swept"
    );
    assert!(transfer_dir.join("upload-keep.part").exists());
    assert!(transfer_dir.join("upload-keep.json").exists());
    assert!(!transfer_dir.join("upload-keep.part.gz").exists());
    assert!(!transfer_dir.join("upload-orphan.json").exists());
    assert!(!transfer_dir.join("upload-orphan.part.gz").exists());
    assert!(!transfer_dir.join("upload-aged.part").exists());
    assert!(!transfer_dir.join("upload-aged.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 连接表单的 sudo_source 三选一决定生效的全局配置：off 从不提升；
/// custom 只沿用工作台绑定（0.4.x 兼容）；global 先按表单引用解析，
/// 再回落绑定；引用无法解析时退化为无配置而不是失败连接。
#[test]
fn effective_sudo_profile_follows_declared_source() {
    let parse = |external_config: Value| {
        StoredConnection::from_lifecycle_params(&serde_json::json!({
            "connection": {
                "id": "conn-src",
                "host": "example.com",
                "port": 22,
                "username": "user",
                "password": "login",
                "external_config": external_config
            }
        }))
        .unwrap()
    };
    let mut store = sudo_profiles::SudoProfileStore::default();
    let (profile, _) = sudo_profiles::save_profile(
        &mut store,
        &serde_json::json!({ "name": "ops", "sudoPassword": "p" }),
    )
    .unwrap();
    store
        .bindings
        .insert("conn-src".to_string(), profile.id.clone());

    let off = parse(serde_json::json!({ "sudo_source": "off" }));
    assert!(effective_sudo_profile(&off, &store).is_none());

    let custom = parse(serde_json::json!({ "sudo_source": "custom" }));
    assert_eq!(
        effective_sudo_profile(&custom, &store).unwrap().id,
        profile.id
    );

    let global_ref = parse(serde_json::json!({ "sudo_source": "global", "sudo_profile": "ops" }));
    assert_eq!(
        effective_sudo_profile(&global_ref, &store).unwrap().id,
        profile.id
    );
    let global_empty = parse(serde_json::json!({ "sudo_source": "global" }));
    assert_eq!(
        effective_sudo_profile(&global_empty, &store).unwrap().id,
        profile.id
    );

    // 引用未知配置时回落绑定；绑定也移除后退化为无配置。
    let ghost = parse(serde_json::json!({ "sudo_source": "global", "sudo_profile": "ghost" }));
    assert_eq!(
        effective_sudo_profile(&ghost, &store).unwrap().id,
        profile.id
    );
    store.bindings.remove("conn-src");
    assert!(effective_sudo_profile(&ghost, &store).is_none());
}

#[test]
fn transfer_paths_stay_next_to_target() {
    let (temporary, backup) = remote_transfer_paths("/home/user/file.txt", "task").unwrap();
    assert_eq!(temporary, "/home/user/.dbx-upload-task.part");
    assert_eq!(backup, "/home/user/.dbx-upload-task.backup");
}

/// T2c（Guacamole 路径矩阵）：无论目标形态如何，`.dbx-upload-<id>.part`
/// 与 `.backup` 都必须与最终目标**同目录**——原子 rename 只在同目录内
/// 可靠（跨目录 rename 会退化为 copy+unlink，或直接失败）。逐项断言
/// `.part` 路径的父目录与目标父目录一致，且不含残留尾斜杠（双斜杠
/// 会让远端 shell/SFTP 语义产生差异）。
#[test]
fn transfer_paths_anchor_in_target_directory_for_edge_targets() {
    let anchored = |target: &str, task: &str| {
        let (temporary, backup) = remote_transfer_paths(target, task).unwrap();
        assert!(backup.ends_with(&format!(".dbx-upload-{task}.backup")));
        assert!(!temporary.contains("//"), "{temporary}");
        assert!(!backup.contains("//"), "{backup}");
        temporary
    };
    // 根 `/`：rsplit_once('/') 的 parent 是空串，实现回落为 "/"，再经
    // trim_end_matches('/') 收敛为空串，最终拼成根下的隐藏临时文件。
    let root = anchored("/", "root");
    assert_eq!(root, "/.dbx-upload-root.part", "root target stays in /");
    // 目标带尾斜杠：rsplit_once 丢弃末尾空段，parent 落在 /data/dir
    // 本身——与目标真实父目录一致，原子 rename 前提成立，且不产生
    // 双斜杠（`.contains("//")` 断言兜底）。
    assert_eq!(
        anchored("/data/dir/", "trail"),
        "/data/dir/.dbx-upload-trail.part",
        "trailing slash keeps the temp in the same parent"
    );
    // `..` 段：函数入口做段级收敛（与 model::normalize_remote_path 同
    // 语义），临时件落进**解析后**的目标目录，与最终 rename 目标同目录，
    // 原子 rename 前提对遍历形态的目标同样成立。
    let dots = remote_transfer_paths("/data/a/../f.bin", "dots").unwrap();
    assert_eq!(dots.0, "/data/.dbx-upload-dots.part");
    // 多级/根级 `..`：收敛到根就停在根，绝不逃逸；相对父目录被弹空
    // （无锚定目录，落点会依赖远端 CWD）则与裸名一样显式报错。
    let dots_root = remote_transfer_paths("/data/a/../../f.bin", "dots-root").unwrap();
    assert_eq!(dots_root.0, "/.dbx-upload-dots-root.part");
    assert!(remote_transfer_paths("a/../../f.bin", "dots-collapse").is_err());
    // 目标文件名含空格/引号：文件名只影响 target 自身，不影响临时件
    // 目录选择；临时件名由 task_id（UUID）构成，天然免于引号/空格注入。
    assert_eq!(
        anchored("/data/my file\"it's\".bin", "quoted"),
        "/data/.dbx-upload-quoted.part"
    );
    assert_eq!(
        anchored("/data/quote'.bin", "single"),
        "/data/.dbx-upload-single.part"
    );
}

/// 缺父目录形态的拒绝路径：没有 `/` 的裸名没有父目录，无法落临时件，
/// 必须显式报错而不是拼出相对路径（相对路径会依赖远端 CWD，不可控）。
#[test]
fn transfer_paths_reject_parentless_targets() {
    assert!(remote_transfer_paths("bare-name.bin", "t").is_err());
}

#[test]
fn sudo_keepalive_registration_plan_replaces_stale_entries() {
    assert_eq!(
        SshRuntime::keepalive_registration_plan(Some("s-live"), "s-live"),
        KeepaliveRegistration::ReuseExisting
    );
    assert_eq!(
        SshRuntime::keepalive_registration_plan(Some("s-dead"), "s-live"),
        KeepaliveRegistration::ReplaceStale
    );
    assert_eq!(
        SshRuntime::keepalive_registration_plan(None, "s-live"),
        KeepaliveRegistration::InsertFresh
    );
}

/// `sftp/transfer/history`: live registry rows win over the persisted
/// rows of the same task (fresh progress, inherited start context),
/// stale persisted `running` rows from a dead sidecar surface as
/// failed, and the result is newest-first with limit/session filtering.
#[test]
fn transfer_history_merges_live_rows_over_persisted_ones() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let root = data_dir.path();
    transfer_history::record_transition(
        root,
        &json!({
            "taskId": "t-done", "sessionId": "s1", "connectionId": "c1",
            "direction": "download", "fileName": "done.bin", "size": 4,
            "transferred": 4, "status": "completed",
            "startedAt": 1_000, "finishedAt": 2_000,
        }),
    )
    .unwrap();
    transfer_history::record_transition(
        root,
        &json!({
            "taskId": "t-live", "sessionId": "s1", "connectionId": "c1",
            "direction": "download", "fileName": "live.bin", "size": 9,
            "transferred": 0, "status": "running",
            "startedAt": 3_000, "finishedAt": null,
        }),
    )
    .unwrap();
    transfer_history::record_transition(
        root,
        &json!({
            "taskId": "t-stale", "sessionId": "s1", "connectionId": "c1",
            "direction": "upload", "fileName": "stale.bin", "size": 2,
            "transferred": 0, "status": "running",
            "startedAt": 500, "finishedAt": null,
        }),
    )
    .unwrap();
    let runtime = SshRuntime::new(root.to_path_buf());
    runtime.downloads.lock().unwrap().insert(
        "t-live".to_string(),
        DownloadState {
            session_id: "s1".to_string(),
            remote_path: "/live.bin".to_string(),
            file_name: "live.bin".to_string(),
            size: 9,
            next_offset: 5,
            sink: None,
            emit_chunks: true,
            tree: None,
            sudo_tmp: None,
            latin1: false,
            throttle: Throttle::new(0),
            progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
            compression: CompressionMode::None,
            ready: false,
            serving_plain: false,
            staged_plain: None,
            gz_staging: None,
            remote_temp: None,
            final_dir: None,
            overwrite: false,
            read_handle: Arc::new(AsyncMutex::new(None)),
            raw_client: Arc::new(AsyncMutex::new(None)),
        },
    );
    let no_connection = |_: &str| String::new();
    let tasks = runtime
        .build_transfer_history(Some("s1"), None, 50, &no_connection)
        .unwrap();
    let tasks = tasks.get("tasks").and_then(Value::as_array).unwrap();
    assert_eq!(tasks.len(), 3);
    // Newest first: the live task (startedAt 3000) leads.
    assert_eq!(tasks[0]["taskId"], "t-live");
    assert_eq!(tasks[0]["transferred"], 5, "live progress wins");
    assert_eq!(tasks[0]["status"], "running");
    assert_eq!(tasks[0]["startedAt"], 3_000, "start context is inherited");
    assert_eq!(tasks[1]["taskId"], "t-done");
    assert_eq!(tasks[1]["status"], "completed");
    // The stale row has no live counterpart: dead-sidecar downgrade (D7).
    assert_eq!(tasks[2]["taskId"], "t-stale");
    assert_eq!(tasks[2]["status"], "failed");
    assert!(
        tasks[2]["error"]
            .as_str()
            .unwrap()
            .contains("sidecar restart"),
        "unexpected error: {}",
        tasks[2]["error"]
    );
    // Limit caps the merged list after sorting.
    let limited = runtime
        .build_transfer_history(Some("s1"), None, 1, &no_connection)
        .unwrap();
    assert_eq!(limited["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(limited["tasks"][0]["taskId"], "t-live");
    // Unknown sessions yield an empty list.
    let other = runtime
        .build_transfer_history(Some("s2"), None, 50, &no_connection)
        .unwrap();
    assert!(other["tasks"].as_array().unwrap().is_empty());
}

/// Issue #18: the history order must be a total order — newest first by
/// `startedAt`, ties broken by `taskId`, live rows without a start
/// record on top — so repeated polls cannot reshuffle equal/missing
/// keys into different positions.
#[test]
fn transfer_history_order_is_total_and_stable() {
    let rows = vec![
        json!({ "taskId": "b", "startedAt": 2_000 }),
        json!({ "taskId": "a", "startedAt": 2_000 }),
        json!({ "taskId": "z" }),
        json!({ "taskId": "c", "startedAt": 3_000 }),
        json!({ "taskId": "y" }),
        json!({ "taskId": "a-old", "startedAt": 1 }),
    ];
    let mut sorted = rows.clone();
    sorted.sort_by(compare_history_rows);
    let ids: Vec<&str> = sorted
        .iter()
        .map(|task| task["taskId"].as_str().unwrap())
        .collect();
    // Missing timestamps lead (live rows), taskId tiebreak; then newest
    // started first, again tie-broken by taskId.
    assert_eq!(ids, vec!["y", "z", "c", "a", "b", "a-old"]);
    // Any permutation of the same rows yields the identical order —
    // the property the previous `startedAt`-only sort lacked.
    for start in 0..rows.len() {
        let mut rotated = rows.clone();
        rotated.rotate_left(start);
        rotated.sort_by(compare_history_rows);
        let rotated_ids: Vec<&str> = rotated
            .iter()
            .map(|task| task["taskId"].as_str().unwrap())
            .collect();
        assert_eq!(rotated_ids, ids);
    }
}

/// Issue #18: two live downloads whose start records never reached the
/// disk (lost cross-process write) used to render in HashMap iteration
/// order — a different order on every sidecar restart. The taskId
/// tiebreak makes the merged history deterministic.
#[test]
fn transfer_history_live_rows_without_start_context_sort_by_task_id() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    for task_id in ["t-zulu", "t-alpha", "t-mike"] {
        runtime.downloads.lock().unwrap().insert(
            task_id.to_string(),
            DownloadState {
                session_id: "s1".to_string(),
                remote_path: format!("/{task_id}.bin"),
                file_name: format!("{task_id}.bin"),
                size: 8,
                next_offset: 0,
                sink: None,
                emit_chunks: true,
                tree: None,
                sudo_tmp: None,
                latin1: false,
                throttle: Throttle::new(0),
                progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
                // sudo 车道不参与压缩通道（M33）：远端临时件机制独立。
                compression: CompressionMode::None,
                ready: false,
                serving_plain: false,
                staged_plain: None,
                gz_staging: None,
                remote_temp: None,
                final_dir: None,
                overwrite: false,
                read_handle: Arc::new(AsyncMutex::new(None)),
                raw_client: Arc::new(AsyncMutex::new(None)),
            },
        );
    }
    let no_connection = |_: &str| String::new();
    let tasks = runtime
        .build_transfer_history(Some("s1"), None, 50, &no_connection)
        .unwrap();
    let ids: Vec<String> = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["taskId"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, vec!["t-alpha", "t-mike", "t-zulu"]);
}

/// Regression: a second cancel after the first one already removed the task
/// entry used to hit `.expect("a transfer was present")` when the compression
/// prep loop still held a `DownloadPrep` record (download/upload/finishing all
/// gone, prep present). That panic killed a shared SDK worker thread; the
/// prep-only path must answer with a regular cancellation event instead.
#[tokio::test]
async fn cancel_transfer_with_only_prep_does_not_panic_and_reports_cancel() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let task_id = "t-prep-only";
    let gz_staging = data_dir.path().join("tree.tgz");
    std::fs::write(&gz_staging, b"staging").expect("seed gzip staging");
    runtime.downloads.lock().unwrap().insert(
        task_id.to_string(),
        DownloadState {
            session_id: "s-prep".to_string(),
            remote_path: "/remote/tree".to_string(),
            file_name: "tree".to_string(),
            size: 42,
            next_offset: 0,
            sink: None,
            emit_chunks: true,
            tree: None,
            sudo_tmp: None,
            latin1: false,
            throttle: Throttle::new(0),
            progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
            compression: CompressionMode::Gzip,
            ready: false,
            serving_plain: false,
            staged_plain: None,
            gz_staging: Some(gz_staging.clone()),
            remote_temp: None,
            final_dir: None,
            overwrite: false,
            read_handle: Arc::new(AsyncMutex::new(None)),
            raw_client: Arc::new(AsyncMutex::new(None)),
        },
    );
    runtime.download_preps.lock().unwrap().insert(
        task_id.to_string(),
        DownloadPrep {
            session_id: "s-prep".to_string(),
            cancelled: Arc::new(AtomicBool::new(false)),
            cancel_reason: Arc::new(Mutex::new(None)),
            phase: Arc::new(Mutex::new("compressing")),
            remote_temp: None,
            gz_staging: gz_staging.clone(),
        },
    );
    let (emitter, sink) = transfer_event_emitter();
    runtime
        .cancel_transfer(task_id, Some("user"), &emitter)
        .await
        .expect("first cancel removes the task entry");
    // This second call is the panic reproducer: downloads is empty, the prep
    // entry is still registered (its loop has not observed the flag yet).
    runtime
        .cancel_transfer(task_id, Some("ack-timeout"), &emitter)
        .await
        .expect("prep-only cancel must return a cancellation, not panic");

    let events = emitted_events(&sink);
    let cancel = events
        .iter()
        .filter(|event| {
            event["method"] == "sftp/transfer/progress" && event["params"]["status"] == "cancelled"
        })
        .last()
        .expect("a cancellation event must be emitted");
    assert_eq!(cancel["params"]["taskId"], task_id);
    assert_eq!(cancel["params"]["sessionId"], "s-prep");
    assert_eq!(cancel["params"]["direction"], "download");
    assert_eq!(cancel["params"]["compression"], "gzip");
    assert_eq!(cancel["params"]["phase"], "compressing");
    assert!(
        cancel["params"]["error"]
            .as_str()
            .unwrap()
            .contains("Download cancelled"),
        "unexpected cancel error: {}",
        cancel["params"]["error"]
    );
    assert!(
        !gz_staging.exists(),
        "the prep-only cancel still cleans its local staging artifact"
    );
    assert!(
        runtime.download_preps.lock().unwrap().contains_key(task_id),
        "the prep record is owned by its background loop; cancel only flags it"
    );
}

/// Start's poisoned prep-registry early return happens after the download
/// state was inserted. No prep task exists to own or clean that task, so the
/// registration failure must roll the registry entry and its local staging
/// assets (sink `.part`, tree staging root) back instead of leaking them.
#[tokio::test]
async fn poisoned_prep_registry_rolls_back_the_registered_download() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let sink_task = "t-prep-poison-sink";
    let download_dir = data_dir.path().join("downloads").to_string_lossy().into_owned();
    let sink = runtime
        .build_download_sink(sink_task, Some(&download_dir), None)
        .await
        .expect("download sink")
        .expect("save-to-local sink");
    let mut sink_state = tree_chunk_state(
        data_dir.path().join("unused-root"),
        Vec::new(),
        CompressionMode::Gzip,
        None,
    );
    sink_state.sink = Some(sink.clone());
    sink_state.tree = None;
    runtime
        .downloads
        .lock()
        .unwrap()
        .insert(sink_task.to_string(), sink_state);

    let tree_task = "t-prep-poison-tree";
    let tree_root = data_dir.path().join("tree-root");
    std::fs::create_dir_all(&tree_root).expect("tree root");
    std::fs::write(tree_root.join("partial.bin"), b"partial").expect("partial");
    let tree_state = tree_chunk_state(tree_root.clone(), Vec::new(), CompressionMode::Gzip, None);
    runtime
        .downloads
        .lock()
        .unwrap()
        .insert(tree_task.to_string(), tree_state);

    // Poison the prep registry without unwinding through the test: the panic
    // is caught and the std mutex stays poisoned.
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = runtime.download_preps.lock().unwrap();
        panic!("poison the prep registry");
    }));
    assert!(poisoned.is_err(), "the test must actually poison the mutex");
    assert!(runtime.download_preps.lock().is_err());

    for task_id in [sink_task, tree_task] {
        let error = runtime
            .register_download_prep(
                task_id,
                DownloadPrep {
                    session_id: "s-prep".to_string(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                    cancel_reason: Arc::new(Mutex::new(None)),
                    phase: Arc::new(Mutex::new("compressing")),
                    remote_temp: None,
                    gz_staging: data_dir.path().join("never.tgz"),
                },
            )
            .await
            .expect_err("a poisoned prep registry must fail registration");
        assert_eq!(error, "Download prep registry is poisoned");
    }

    assert!(
        runtime.downloads.lock().unwrap().is_empty(),
        "a failed registration must remove the inserted download state"
    );
    assert!(
        !sink.part_path.exists(),
        "the sink staging .part must be rolled back"
    );
    assert!(
        !tree_root.exists(),
        "the tree staging root must be rolled back"
    );
}

#[tokio::test]
async fn staged_download_skips_binary_frames_when_emit_chunks_is_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let staged = dir.path().join("staged.bin");
    std::fs::write(&staged, b"hello world").expect("staged");
    let runtime = SshRuntime::new(dir.path().to_path_buf());
    let task_id = "t-staged-no-frames";
    let download = DownloadState {
        session_id: "s-staged".to_string(),
        remote_path: "/remote/staged".to_string(),
        file_name: "staged.bin".to_string(),
        size: 11,
        next_offset: 0,
        sink: None,
        emit_chunks: false,
        tree: None,
        sudo_tmp: None,
        latin1: false,
        throttle: Throttle::new(0),
        progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
        compression: CompressionMode::Gzip,
        ready: true,
        serving_plain: false,
        staged_plain: Some(staged),
        gz_staging: None,
        remote_temp: None,
        final_dir: Some(dir.path().to_path_buf()),
        overwrite: false,
        read_handle: Arc::new(AsyncMutex::new(None)),
        raw_client: Arc::new(AsyncMutex::new(None)),
    };
    runtime
        .downloads
        .lock()
        .unwrap()
        .insert(task_id.to_string(), download.clone());
    let (emitter, sink) = framed_transfer_emitter();
    let response = runtime
        .download_staged_chunk(task_id, 0, download, &emitter)
        .await
        .expect("staged chunk");
    assert_eq!(response["length"].as_u64(), Some(11));
    assert!(
        framed_messages(&sink)
            .iter()
            .all(|message| !matches!(message, FramedMessage::Binary { .. })),
        "emitChunks=false must not push download bytes over IPC"
    );
}

#[tokio::test]
async fn concurrent_same_name_downloads_reserve_distinct_final_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(dir.path().to_path_buf());
    let part_a = dir.path().join("a.part");
    let part_b = dir.path().join("b.part");
    std::fs::write(&part_a, b"first").expect("part a");
    std::fs::write(&part_b, b"second").expect("part b");
    let file_a = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&part_a)
        .await
        .expect("open a");
    let file_b = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&part_b)
        .await
        .expect("open b");
    let sink_a = DownloadSink {
        part_path: part_a,
        final_dir: dir.path().to_path_buf(),
        overwrite: false,
        file: AsyncMutex::new(file_a),
    };
    let sink_b = DownloadSink {
        part_path: part_b,
        final_dir: dir.path().to_path_buf(),
        overwrite: false,
        file: AsyncMutex::new(file_b),
    };
    let (first, second) = tokio::join!(
        runtime.finalize_download_sink(&sink_a, "same.txt"),
        runtime.finalize_download_sink(&sink_b, "same.txt"),
    );
    let first = first.expect("first finalize");
    let second = second.expect("second finalize");
    assert_ne!(first, second, "same-name downloads must get distinct names");
    let mut contents = vec![
        std::fs::read_to_string(&first).expect("first content"),
        std::fs::read_to_string(&second).expect("second content"),
    ];
    contents.sort();
    assert_eq!(contents, vec!["first".to_string(), "second".to_string()]);
}

/// Terminal transitions that omit `connectionId` must inherit it from the
/// live session first, then from the persisted start row; an unknown session
/// with no start context stays unattributed instead of leaking into another
/// connection's scoped view. A full `SessionEntry` cannot be built in unit
/// tests (`russh::client::Handle` has no public constructor), so the live
/// session branch is pinned through the pure resolver and the persisted /
/// unknown branches through `record_transfer` itself.
#[test]
fn transfer_history_resolves_live_session_connection_id() {
    let bare = json!({ "taskId": "t1", "sessionId": "s1" });
    assert_eq!(resolve_transfer_connection_id(&bare, "c1", ""), "c1");
    assert_eq!(
        resolve_transfer_connection_id(&bare, "c1", "c-prev"),
        "c1",
        "a live session outranks the persisted start row"
    );
    let explicit = json!({ "taskId": "t1", "sessionId": "s1", "connectionId": "c-explicit" });
    assert_eq!(
        resolve_transfer_connection_id(&explicit, "c1", "c-prev"),
        "c-explicit"
    );
    assert_eq!(
        resolve_transfer_connection_id(&bare, "", "c-prev"),
        "c-prev"
    );
    assert_eq!(resolve_transfer_connection_id(&bare, "", ""), "");
}

#[tokio::test]
async fn transfer_history_persists_connection_id() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    transfer_history::record_transition(
        data_dir.path(),
        &json!({
            "taskId": "t-inherit", "sessionId": "s1", "connectionId": "c1",
            "direction": "download", "fileName": "a.bin", "size": 4,
            "transferred": 0, "status": "running",
            "startedAt": 1_000, "finishedAt": null,
        }),
    )
    .unwrap();
    runtime
        .record_transfer(json!({
            "taskId": "t-inherit", "sessionId": "s1",
            "direction": "download", "fileName": "a.bin", "size": 4,
            "transferred": 4, "status": "completed",
        }))
        .await;
    let rows = transfer_history::load_history(data_dir.path());
    let inherited = rows
        .iter()
        .find(|row| row["taskId"] == "t-inherit")
        .expect("terminal row");
    assert_eq!(inherited["connectionId"], "c1");

    runtime
        .record_transfer(json!({
            "taskId": "t-gone", "sessionId": "s-gone",
            "direction": "upload", "fileName": "b.bin", "size": 2,
            "transferred": 2, "status": "failed",
        }))
        .await;
    let rows = transfer_history::load_history(data_dir.path());
    let unknown = rows
        .iter()
        .find(|row| row["taskId"] == "t-gone")
        .expect("unknown-session row");
    assert_eq!(unknown["connectionId"], "");
}

#[test]
fn transfer_history_query_filters_by_connection_id() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let root = data_dir.path();
    for (task_id, scope) in [
        ("t-c1", Some("c1")),
        ("t-c2", Some("c2")),
        ("t-legacy", None),
    ] {
        let mut row = json!({
            "taskId": task_id, "sessionId": "s1",
            "direction": "download", "fileName": "a.bin", "size": 4,
            "transferred": 4, "status": "completed",
            "startedAt": 1_000, "finishedAt": 2_000,
        });
        if let Some(scope) = scope {
            row["connectionId"] = json!(scope);
        }
        transfer_history::record_transition(root, &row).unwrap();
    }
    let runtime = SshRuntime::new(root.to_path_buf());
    let no_connection = |_: &str| String::new();
    let scoped = runtime
        .build_transfer_history(None, Some("c1"), 50, &no_connection)
        .unwrap();
    let tasks = scoped["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1, "only c1 rows are visible");
    assert_eq!(tasks[0]["taskId"], "t-c1");
    let unscoped = runtime
        .build_transfer_history(None, None, 50, &no_connection)
        .unwrap();
    assert_eq!(
        unscoped["tasks"].as_array().unwrap().len(),
        3,
        "legacy callers still see the merged global history"
    );
}

/// Teardown removes the session before cleanup runs, so the connection id
/// must be captured by the caller and threaded through; otherwise the failed
/// row is unattributed and invisible in the owning connection's history.
#[tokio::test]
async fn cleanup_session_transfers_persists_the_captured_connection_id() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    runtime.downloads.lock().unwrap().insert(
        "t-close".to_string(),
        DownloadState {
            session_id: "s1".to_string(),
            remote_path: "/close.bin".to_string(),
            file_name: "close.bin".to_string(),
            size: 8,
            next_offset: 3,
            sink: None,
            emit_chunks: true,
            tree: None,
            sudo_tmp: None,
            latin1: false,
            throttle: Throttle::new(0),
            progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
            compression: CompressionMode::None,
            ready: false,
            serving_plain: false,
            staged_plain: None,
            gz_staging: None,
            remote_temp: None,
            final_dir: None,
            overwrite: false,
            read_handle: Arc::new(AsyncMutex::new(None)),
            raw_client: Arc::new(AsyncMutex::new(None)),
        },
    );
    runtime
        .cleanup_session_transfers("s1", "c1")
        .await
        .expect("cleanup");
    let rows = transfer_history::load_history(data_dir.path());
    let failed = rows
        .iter()
        .find(|row| row["taskId"] == "t-close")
        .expect("failed teardown row");
    assert_eq!(failed["connectionId"], "c1");
    assert_eq!(failed["status"], "failed");
}
