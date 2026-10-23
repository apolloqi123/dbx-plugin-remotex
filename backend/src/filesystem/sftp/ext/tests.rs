use super::*;

/// remote-edit 校验基址与前端下载目标同源（#175 回归）：downloadDir
/// 偏好指向自定义目录时，`<偏好>/remote-edit/` 下的文件必须通过校验，
/// 即使 env 覆盖指向别处（前端只读偏好，偏好必须赢）。
#[test]
fn remote_edit_validation_follows_download_dir_preference() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let custom = tempfile::tempdir().expect("custom dir");
    // 偏好写自定义下载目录；文件落在 <自定义>/remote-edit/stamp/a.txt。
    crate::core::preferences::save_preferences(
        data_dir.path(),
        &serde_json::json!({ "downloadDir": custom.path().to_string_lossy() }),
    )
    .expect("save preference");
    let stamp = custom.path().join("remote-edit").join("20261011000000");
    std::fs::create_dir_all(&stamp).expect("mkdir stamp");
    let file = stamp.join("a.txt");
    std::fs::write(&file, b"x").expect("write fixture");
    // env 覆盖指向另一个目录（旧实现会解析到它并失败）。
    let env_override = tempfile::tempdir().expect("env dir");
    let validated = validate_remote_edit_path(&file, data_dir.path(), |key| {
        if key == crate::filesystem::local::downloads::DOWNLOAD_DIR_ENV {
            Some(std::ffi::OsString::from(env_override.path()))
        } else {
            std::env::var_os(key)
        }
    })
    .expect("preference-based base must validate");
    assert_eq!(validated, file.canonicalize().expect("canonicalize"));

    // 无偏好（全新 data_dir）：回落 env/USERPROFILE 基址，跨目录文件
    // 依旧被拒（守卫仍在）。
    let clean_data_dir = tempfile::tempdir().expect("clean data dir");
    let error = validate_remote_edit_path(&file, clean_data_dir.path(), |key| {
        if key == crate::filesystem::local::downloads::DOWNLOAD_DIR_ENV {
            Some(std::ffi::OsString::from(env_override.path()))
        } else {
            std::env::var_os(key)
        }
    })
    .expect_err("cross-base file must stay rejected");
    assert!(error.contains("remote-edit"), "{error}");
}

#[test]
fn shell_quote_wraps_and_escapes_single_quotes() {
    assert_eq!(shell_quote("/var/log/app log"), "'/var/log/app log'");
    assert_eq!(shell_quote("it's"), r"'it'\''s'");
    assert_eq!(shell_quote("a'b'c"), r"'a'\''b'\''c'");
    assert_eq!(shell_quote("'; rm -rf /"), r"''\''; rm -rf /'");
}

#[test]
fn format_mode_masks_type_bits() {
    assert_eq!(format_mode(0o100644), "0644");
    assert_eq!(format_mode(0o040755), "0755");
    assert_eq!(format_mode(0o120777), "0777");
}

#[test]
fn common_parent_dir_handles_single_and_nested_paths() {
    // Single file: the file name is dropped, unlike tiny-rdm.
    assert_eq!(
        common_parent_dir(&["/var/log/app.log".to_string()]),
        "/var/log"
    );
    // Siblings share their directory.
    assert_eq!(
        common_parent_dir(&["/a/b.txt".to_string(), "/a/c.txt".to_string()]),
        "/a"
    );
    // Nested sources keep the deeper shared directory.
    assert_eq!(
        common_parent_dir(&["/a/b/c.txt".to_string(), "/a/b/d/e.txt".to_string()]),
        "/a/b"
    );
    // Disjoint paths fall back to the root.
    assert_eq!(
        common_parent_dir(&["/x/1".to_string(), "/y/2".to_string()]),
        "/"
    );
    // A source that is itself an ancestor archives relative to its parent.
    assert_eq!(
        common_parent_dir(&["/tmp".to_string(), "/tmp/a.txt".to_string()]),
        "/"
    );
}

#[test]
fn relative_to_parent_strips_the_common_directory() {
    assert_eq!(
        relative_to_parent("/var/log/app.log", "/var/log"),
        "app.log"
    );
    assert_eq!(relative_to_parent("/a/b/c", "/a"), "b/c");
    assert_eq!(relative_to_parent("/etc", "/"), "etc");
}

#[test]
fn archive_command_quotes_every_argument() {
    let command = build_archive_command(
        "/tmp/arc.tar.gz.tmp",
        "/var/log",
        &["my app.log".to_string(), "it's".to_string()],
    );
    assert_eq!(
        command,
        "tar -czf '/tmp/arc.tar.gz.tmp' -C '/var/log' -- 'my app.log' 'it'\\''s'"
    );
}

#[test]
fn tar_z_flag_dispatches_by_extension() {
    assert!(tar_z_flag("/tmp/a.tar.gz").unwrap());
    assert!(tar_z_flag("/tmp/a.TGZ").unwrap());
    assert!(!tar_z_flag("/tmp/a.tar").unwrap());
    let zip = tar_z_flag("/tmp/a.zip").unwrap_err();
    assert!(zip.contains("unsupported"), "{zip}");
    assert!(tar_z_flag("/tmp/a.rar").is_err());
}

#[test]
fn unsafe_tar_members_are_detected_before_extraction() {
    // `..` 段（GNU tar 解包不中和）与绝对路径成员都必须拒绝——sudo 车道
    // 下解包以 root 运行，逃出目标目录即任意路径写。
    assert!(contains_unsafe_members("../evil"));
    assert!(contains_unsafe_members("dir/../../evil"));
    assert!(contains_unsafe_members("/abs/path"));
    assert!(contains_unsafe_members("./dir/../evil"));
    // 正常成员、嵌套路径、空行放行。
    assert!(!contains_unsafe_members("good.txt\ndir/nested.txt\n"));
    assert!(!contains_unsafe_members("./root/sub/file.log"));
}

#[test]
fn tar_commands_carry_the_z_flag_and_destination() {
    assert_eq!(
        build_tar_list_command("/tmp/a.tgz", true),
        "tar -tzf '/tmp/a.tgz'"
    );
    assert_eq!(
        build_extract_command("/tmp/a.tar", "/opt/app", false),
        "mkdir -p '/opt/app' && tar -xf '/tmp/a.tar' -C '/opt/app' --"
    );
    assert_eq!(
        build_extract_command("/tmp/a.tgz", "/opt/app", true),
        "mkdir -p '/opt/app' && tar -xzf '/tmp/a.tgz' -C '/opt/app' --"
    );
}

/// 引号包裹补充：归档名/目标目录含空格或单引号（远端用户目录常态）
/// 时，listing/extract 命令的每个参数都必须被 shell_quote 包裹并转义，
/// 否则嵌入的 `;`/反引号会被远端 shell 当作新命令执行（对标 Tabby
/// "用户路径永不进入 shell 语法位"的约定）。
#[test]
fn tar_commands_quote_names_with_spaces_and_quotes() {
    // 未压缩 .tar 的 listing 走无 z 形态。
    assert_eq!(
        build_tar_list_command("/tmp/my archive.tar", false),
        "tar -tf '/tmp/my archive.tar'"
    );
    // 单引号名：'\'' 转义后整条命令仍是单参数语义。
    assert_eq!(
        build_tar_list_command("/tmp/it's.tar.gz", true),
        r"tar -tzf '/tmp/it'\''s.tar.gz'"
    );
    assert_eq!(
        build_extract_command("/tmp/my archive.tar.gz", "/opt/my app", true),
        "mkdir -p '/opt/my app' && tar -xzf '/tmp/my archive.tar.gz' -C '/opt/my app' --"
    );
    // 恶意注入段不得逃出引号位。
    let hostile = build_extract_command("/tmp/evil'; rm -rf / #.tar.gz", "/opt/app", true);
    assert!(
        hostile.contains(r"'/tmp/evil'\''; rm -rf / #.tar.gz'"),
        "{hostile}"
    );
    assert!(hostile.starts_with("mkdir -p '/opt/app' && tar -xzf '"));
}

#[test]
fn top_level_entries_reduce_tar_listings() {
    assert_eq!(
        unique_top_level_entries("./dir1/\n./dir1/a.txt\ndir2/b/c\nroot.txt\n"),
        vec![
            "dir1".to_string(),
            "dir2".to_string(),
            "root.txt".to_string()
        ]
    );
    // Path traversal members and noise never produce entries.
    assert!(unique_top_level_entries("../evil\n..\n\n").is_empty());
}

#[test]
fn direct_write_size_limit_rejects_oversized_payloads() {
    assert_eq!(ensure_direct_write_size(0).unwrap(), ());
    assert_eq!(ensure_direct_write_size(MAX_DIRECT_WRITE_SIZE).unwrap(), ());
    let error = ensure_direct_write_size(MAX_DIRECT_WRITE_SIZE + 1).unwrap_err();
    assert!(error.contains("streaming upload"), "{error}");
}

#[test]
fn decode_direct_write_payload_handles_base64_edges() {
    assert_eq!(decode_direct_write_payload("").unwrap(), Vec::<u8>::new());
    assert_eq!(
        decode_direct_write_payload("aGVsbG8=").unwrap(),
        b"hello".to_vec()
    );
    // Exactly 4 MiB of decoded zeros stays within the cap (boundary).
    let boundary = format!("{}AA==", "A".repeat(4 * (MAX_DIRECT_WRITE_SIZE - 1) / 3));
    assert!(decode_direct_write_payload(&boundary).is_ok());
    assert!(decode_direct_write_payload("not*base64").is_err());
}

#[test]
fn direct_write_paths_live_next_to_the_target() {
    let (temporary, backup) = direct_write_paths("/home/user/file.txt", "abc");
    assert_eq!(temporary, "/home/user/.dbx-part-abc");
    assert_eq!(backup, "/home/user/.dbx-part-abc.backup");
    let (root_temp, _) = direct_write_paths("/file.txt", "abc");
    assert_eq!(root_temp, "/.dbx-part-abc");
}

/// M15 watch/upload 并发回传隔离证明：`write_bytes`（`watch/upload` 的
/// 提交路径）在每次调用里生成一个新的 `Uuid::new_v4()` 作为 task_id，
/// 因此并发回传——无论是同一个远端文件还是不同文件——各自的
/// `.dbx-part-<uuid>` 临时文件与 backup 都互不重名，最后一步的原子
/// rename 各自落到各自目标上，互不干扰。
#[test]
fn concurrent_write_bytes_calls_never_share_a_staging_file() {
    // Simulate N concurrent round-trips exactly as write_bytes names
    // them: one fresh uuid per in-flight call.
    let mut task_ids: Vec<String> = Vec::new();
    for _ in 0..64 {
        task_ids.push(uuid::Uuid::new_v4().to_string());
    }
    assert_eq!(
        task_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        64,
        "uuid task ids are unique per call"
    );
    // Same target: every concurrent commit stages under its own name.
    let mut staging: std::collections::BTreeSet<String> = Default::default();
    for task_id in &task_ids {
        let (temporary, backup) = direct_write_paths("/srv/notes.md", task_id);
        assert!(staging.insert(temporary));
        assert!(staging.insert(backup));
    }
    // Different targets: parents differ, so names can never cross either.
    let (temp_a, backup_a) = direct_write_paths("/srv/a.md", &task_ids[0]);
    let (temp_b, backup_b) = direct_write_paths("/etc/b.conf", &task_ids[1]);
    assert_ne!(temp_a, temp_b);
    assert_ne!(backup_a, backup_b);
    // Every staging name lives next to its own target, never elsewhere.
    assert!(temp_a.starts_with("/srv/") && backup_a.starts_with("/srv/"));
    assert!(temp_b.starts_with("/etc/") && backup_b.starts_with("/etc/"));
}

#[test]
fn exec_failures_report_status_and_output() {
    assert!(check_exec_success(&json!({ "exitCode": 0 }), "extract").is_ok());
    let error = check_exec_success(&json!({ "exitCode": 2, "output": "tar: eof\n" }), "archive")
        .unwrap_err();
    assert!(error.starts_with("archive exited with status 2: tar: eof"));
}

#[test]
fn clean_unique_name_rejects_paths_and_bounds_length() {
    assert_eq!(clean_unique_name("  report.pdf ").unwrap(), "report.pdf");
    assert!(clean_unique_name("").is_err());
    assert!(clean_unique_name("a/b.pdf").is_err());
    assert!(clean_unique_name("a\\b.pdf").is_err());
    assert!(clean_unique_name(".").is_err());
    assert!(clean_unique_name("..").is_err());
    let long = "x".repeat(300);
    assert_eq!(clean_unique_name(&long).unwrap().chars().count(), 255);
}

#[test]
fn unique_name_candidate_inserts_before_the_last_extension() {
    assert_eq!(unique_name_candidate("report.pdf", 1), "report(1).pdf");
    assert_eq!(
        unique_name_candidate("archive.tar.gz", 12),
        "archive.tar(12).gz"
    );
    assert_eq!(unique_name_candidate("report", 3), "report(3)");
    // Hidden files keep their leading dot untouched.
    assert_eq!(unique_name_candidate(".bashrc", 2), ".bashrc(2)");
}

#[test]
fn next_unique_name_picks_the_first_free_candidate() {
    let free = |_: &str| false;
    assert_eq!(
        next_unique_name("report.pdf", free),
        Some(("report.pdf".to_string(), false))
    );
    let taken = |candidate: &str| candidate == "report.pdf" || candidate == "report(1).pdf";
    assert_eq!(
        next_unique_name("report.pdf", taken),
        Some(("report(2).pdf".to_string(), true))
    );
    // Exhausting the probe budget yields None (handler turns it into an error).
    let everything = |_: &str| true;
    assert_eq!(next_unique_name("report.pdf", everything), None);
}

#[test]
fn unique_name_candidates_start_with_the_plain_name() {
    let candidates = unique_name_candidates("a.txt");
    assert_eq!(candidates.first().unwrap(), "a.txt");
    assert_eq!(candidates.get(1).unwrap(), "a(1).txt");
    assert_eq!(candidates.len(), (UNIQUE_NAME_PROBE_LIMIT + 1) as usize);
    assert_eq!(candidates.last().unwrap(), "a(999).txt");
}

#[test]
fn join_remote_name_handles_the_root() {
    assert_eq!(join_remote_name("/", "a.txt"), "/a.txt");
    assert_eq!(join_remote_name("/tmp/up", "a.txt"), "/tmp/up/a.txt");
}

#[test]
fn clean_link_target_keeps_relative_paths_and_rejects_junk() {
    // Relative targets are valid symlink semantics and must survive
    // verbatim (no normalization, no shell quoting involved).
    assert_eq!(
        clean_link_target("../shared/data").unwrap(),
        "../shared/data"
    );
    assert_eq!(clean_link_target("/abs/target").unwrap(), "/abs/target");
    // A target that is only blanks is still a valid (if odd) name; only
    // emptiness and NUL are protocol-level junk.
    assert!(clean_link_target("").is_err());
    assert!(clean_link_target("a\0b").is_err());
}

// —— M23（R1）exists 错误语义：两分支都只认 NO_SUCH_FILE —— //

#[test]
fn raw_exists_decision_maps_no_such_file_only() {
    // 可 stat 到：存在。
    assert!(raw_exists_decision(Ok(RawAttrs::default())).unwrap());
    // SSH_FX_NO_SUCH_FILE：不存在（不算错误）。
    assert!(!raw_exists_decision(Err("SFTP raw lstat failed with status 2".to_string())).unwrap());
    // 权限拒绝（status 3）：如实上抛，绝不误报 exists:false。
    let error =
        raw_exists_decision(Err("SFTP raw lstat failed with status 3".to_string())).unwrap_err();
    assert!(error.contains("status 3"), "{error}");
    // 无码错误（死通道等）：同样上抛。
    assert!(raw_exists_decision(Err("SFTP raw lstat failed: early eof".to_string())).is_err());
}

#[test]
fn high_level_exists_decision_maps_no_such_file_only() {
    use russh_sftp::client::error::Error as SftpError;
    use russh_sftp::protocol::{Status, StatusCode};
    let no_such = SftpError::Status(Status {
        id: 1,
        status_code: StatusCode::NoSuchFile,
        error_message: "no such file".to_string(),
        language_tag: String::new(),
    });
    assert!(!high_level_exists_decision(&no_such).unwrap());
    // 权限拒绝：上抛（错误串保留服务器消息），绝不误报 exists:false。
    let denied = high_level_exists_decision(&SftpError::Status(Status {
        id: 2,
        status_code: StatusCode::PermissionDenied,
        error_message: "permission denied".to_string(),
        language_tag: String::new(),
    }))
    .unwrap_err();
    assert!(denied.contains("permission denied"), "{denied}");
    // 通道/协议类错误：同样上抛。
    assert!(high_level_exists_decision(&SftpError::UnexpectedPacket).is_err());
}

// —— M16 latin-1 裸包写路径的纯逻辑 —— //

#[test]
fn raw_paths_next_to_reuses_the_target_directory_bytes() {
    // 目标字节里可以有任何非 UTF-8 序列：目录段原样复用，ASCII 暂存名拼接。
    let target = b"/tmp/caf\xe9/upload/report.pdf".to_vec();
    let (temporary, backup) = raw_paths_next_to(&target, ".dbx-part-abc", ".dbx-part-abc.backup");
    assert_eq!(temporary, b"/tmp/caf\xe9/upload/.dbx-part-abc".to_vec());
    assert_eq!(backup, b"/tmp/caf\xe9/upload/.dbx-part-abc.backup".to_vec());
    // 根目录目标：暂存名落在 / 下。
    let (root_temp, root_backup) = raw_paths_next_to(b"/a.txt", ".dbx-part-abc", ".b");
    assert_eq!(root_temp, b"/.dbx-part-abc".to_vec());
    assert_eq!(root_backup, b"/.b".to_vec());
}

#[test]
fn unique_name_raw_candidates_encode_wire_prefix_and_display_names() {
    let candidates = unique_name_raw_candidates("/tmp/caf%E9", "naïve.txt").unwrap();
    // 目录前缀来自列表回传的 wire 形式（%E9 → 0xE9），候选名是用户新输入
    // 的显示文本按 latin-1 编码（ï → 0xEF）；(n) 是 ASCII 增量。
    assert_eq!(candidates[0].1, b"/tmp/caf\xe9/na\xefve.txt".to_vec());
    assert_eq!(candidates[1].1, b"/tmp/caf\xe9/na\xefve(1).txt".to_vec());
    assert_eq!(candidates[0].0, "naïve.txt");
    assert_eq!(candidates[1].0, "naïve(1).txt");
    assert_eq!(candidates.len(), (UNIQUE_NAME_PROBE_LIMIT + 1) as usize);
    // 字面 %XX 的新输入名保持字面量（不二次转义）。
    let literal = unique_name_raw_candidates("/tmp", "100%.pdf").unwrap();
    assert_eq!(literal[0].1, b"/tmp/100%.pdf".to_vec());
    // 非法输入仍然被拒绝。
    assert!(unique_name_raw_candidates("/tmp", "a/b").is_err());
}

/// 两条 latin-1 路径还原分工的对照证明：touch/symlink-create/exists/
/// rename-unique/上传 start 的「wire 前缀 + 新输入末段」走
/// `write_path_bytes`；sftp/write、upload-local、symlink-update 的
/// 「整条 wire 路径」走 `unescape_wire`。同一服务器名字节在两种来源下
/// 还原结果一致；wire 里字面 %XX 已被 escape_wire 自转义为 %25，还原不吞。
#[test]
fn latin1_path_resolution_splits_by_path_origin() {
    // 服务器字节 b"caf\xe9.txt" 的两种来源：
    // (1) 列表 wire 路径（整条还原）——sftp/write 的 previewPath。
    assert_eq!(
        sftp_name::unescape_wire("/tmp/caf%E9.txt"),
        b"/tmp/caf\xe9.txt".to_vec()
    );
    // (2) wire 前缀 + 显示末段——touch 的 joinRemote(currentPath, name)。
    // 末段 "caf%E9.txt" 是显示文本：字面 %XX 保持 ASCII 字面量（用户输入
    // 未转义；真服务器的 é 名此时会以显示文本 é 传来，见下一条断言）。
    assert_eq!(
        sftp_name::write_path_bytes("/tmp/caf%E9.txt"),
        b"/tmp/caf%E9.txt".to_vec()
    );
    // 同一场景换成显示文本输入 é：末段编码回 0xE9。
    assert_eq!(
        sftp_name::write_path_bytes("/tmp/caf%E9/caf\u{e9}.txt"),
        b"/tmp/caf\xe9/caf\xe9.txt".to_vec()
    );
    // wire 自转义闭环：字面 "100%.txt" 在 wire 里是 "100%25.txt"，整条
    // 还原不吞 %；显示输入 "100%.txt" 直接字面量。
    assert_eq!(
        sftp_name::unescape_wire("/tmp/100%25.txt"),
        b"/tmp/100%.txt".to_vec()
    );
    assert_eq!(
        sftp_name::write_path_bytes("/tmp/100%.txt"),
        b"/tmp/100%.txt".to_vec()
    );
}
