use super::*;

fn lookup_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |key: &str| {
        pairs
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| OsString::from(*value))
    }
}

#[test]
fn save_local_file_writes_records_history_and_yields_name() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let target = tempfile::tempdir().expect("target");
    let result = save_local_file(
        data_dir.path(),
        "session.gif",
        b"GIF89a",
        Some(target.path().to_str().unwrap()),
        None,
    )
    .expect("save");
    let local_path = result["localPath"].as_str().unwrap().to_string();
    assert_eq!(std::fs::read(&local_path).unwrap(), b"GIF89a");
    // 记入传输历史（local/reveal、local/open 的合法路径来源）。
    let history = crate::features::transfer::history::load_history(data_dir.path());
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["localPath"].as_str().unwrap(), local_path);
    assert_eq!(history[0]["status"].as_str().unwrap(), "completed");
    // 撞名让位：同名再保存得到 " (1)" 后缀而不是覆盖。
    let again = save_local_file(
        data_dir.path(),
        "session.gif",
        b"GIF89b",
        Some(target.path().to_str().unwrap()),
        None,
    )
    .expect("save again");
    assert!(again["localPath"]
        .as_str()
        .unwrap()
        .ends_with("session (1).gif"));
    assert_eq!(std::fs::read(&local_path).unwrap(), b"GIF89a");
    // 覆盖策略：同名直接替换，不让位。
    let replaced = save_local_file(
        data_dir.path(),
        "session.gif",
        b"GIF89c",
        Some(target.path().to_str().unwrap()),
        Some("overwrite"),
    )
    .expect("overwrite");
    assert!(replaced["localPath"]
        .as_str()
        .unwrap()
        .ends_with("session.gif"));
    assert_eq!(std::fs::read(&local_path).unwrap(), b"GIF89c");
}

#[test]
fn save_local_file_rejects_relative_dir_and_empty_payload() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    assert!(
        save_local_file(data_dir.path(), "a.gif", b"x", Some("relative/dir"), None).is_err()
    );
    assert!(save_local_file(data_dir.path(), "a.gif", b"", None, None).is_err());
}

#[test]
fn sanitize_strips_separators_and_traversal() {
    assert_eq!(sanitize_file_name("report.tar.gz"), "report.tar.gz");
    // Separators are gone, so ".."-heavy names can never traverse; the
    // residual dots are harmless (remote names never contain '/' anyway).
    assert_eq!(sanitize_file_name("../../etc/passwd"), "....etcpasswd");
    assert_eq!(sanitize_file_name("../.."), "download");
    assert_eq!(sanitize_file_name("a/b\\c"), "abc");
    assert_eq!(sanitize_file_name("name... "), "name");
    assert_eq!(sanitize_file_name("  "), "download");
    assert_eq!(sanitize_file_name(""), "download");
    assert_eq!(sanitize_file_name("we\nird"), "we ird");
}

#[test]
fn pick_download_path_avoids_collisions() {
    let base = tempfile::tempdir().expect("tempdir");
    let first = pick_download_path(base.path(), "log.txt");
    assert_eq!(first.file_name().unwrap(), "log.txt");
    std::fs::write(&first, b"x").expect("write");
    let second = pick_download_path(base.path(), "log.txt");
    assert_eq!(second.file_name().unwrap(), "log (1).txt");
    // A dotfile ("stem" is the whole name) must not become ".hidden (1)."
    let dot = pick_download_path(base.path(), ".hidden");
    assert_eq!(dot.file_name().unwrap(), ".hidden");
}

#[test]
fn downloads_base_dir_prefers_env_then_home() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let target = tempfile::tempdir().expect("tempdir");
    let dir = downloads_base_dir(
        lookup_from(&[(
            "DBX_SSH_DOWNLOAD_DIR",
            target.path().to_string_lossy().as_ref(),
        )]),
        data_dir.path(),
    );
    assert_eq!(dir, target.path());
    let home = tempfile::tempdir().expect("tempdir");
    let downloads = home.path().join("Downloads");
    std::fs::create_dir_all(&downloads).expect("mkdir");
    // Windows 下 home 走 USERPROFILE，其余平台走 HOME（与实现一致）。
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let dir = downloads_base_dir(
        lookup_from(&[(home_var, home.path().to_string_lossy().as_ref())]),
        data_dir.path(),
    );
    assert_eq!(dir, downloads);
}

#[test]
fn can_save_local_env_overrides_platform_default() {
    // env explicit off wins everywhere
    assert!(!can_save_local(lookup_from(&[("DBX_SSH_LOCAL_SAVE", "0")])));
    assert!(can_save_local(lookup_from(&[(
        "DBX_SSH_LOCAL_SAVE",
        "true"
    )])));
    // no env: Linux requires a display; a blank value is "unset", so it
    // must behave exactly like the absent case regardless of platform.
    assert_eq!(
        can_save_local(lookup_from(&[("DBX_SSH_LOCAL_SAVE", "  ")])),
        can_save_local(lookup_from(&[]))
    );
}

#[test]
fn reveal_requires_recorded_completed_download() {
    let history = vec![
        serde_json::json!({
            "taskId": "t1", "direction": "download", "status": "completed",
            "localPath": "/Downloads/a.txt"
        }),
        serde_json::json!({ "taskId": "t2", "direction": "upload", "status": "completed" }),
        serde_json::json!({ "taskId": "t3", "direction": "download", "status": "failed" }),
    ];
    assert!(is_recorded_download(
        &history,
        Path::new("/Downloads/a.txt")
    ));
    assert!(!is_recorded_download(
        &history,
        Path::new("/Downloads/b.txt")
    ));
    assert!(!is_recorded_download(&history, Path::new("/etc/passwd")));
    let rejected = reveal_validated(&history, Path::new("/etc/passwd"));
    assert!(rejected
        .unwrap_err()
        .contains("not saved by a completed download"));
}

#[test]
fn open_requires_recorded_completed_download() {
    let history = vec![serde_json::json!({
        "direction": "download", "status": "completed", "localPath": "/Downloads/a.txt"
    })];
    assert!(is_recorded_download(
        &history,
        Path::new("/Downloads/a.txt")
    ));
    assert!(!is_recorded_download(&history, Path::new("/etc/passwd")));
}

/// Issue #18: reveal must never fall through to an arbitrary OS default
/// (the Documents folder) when the recorded file is gone. The fallback
/// chain is recorded file → its parent folder → the download directory.
#[test]
fn reveal_target_falls_back_to_parent_then_download_dir() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let downloads = tempfile::tempdir().expect("downloads");
    // Existing file: reveal it itself.
    let file = downloads.path().join("keep.txt");
    std::fs::write(&file, b"x").expect("write");
    assert_eq!(reveal_target(&file, data_dir.path()), file);
    // Missing file inside an existing folder: reveal the folder so the
    // user still lands next to where the download was saved.
    let missing = downloads.path().join("moved-away.txt");
    assert_eq!(reveal_target(&missing, data_dir.path()), downloads.path());
    // Missing file in a missing folder: land in the download directory.
    let gone = downloads.path().join("no-such-dir").join("gone.txt");
    assert_eq!(reveal_target(&gone, downloads.path()), downloads.path());
}

/// Issue #18: the reveal fallback directory honors the configured
/// `downloadDir` preference (same directory downloads are saved into),
/// and falls back to the resolved default when it is unset or relative.
#[test]
fn reveal_download_dir_prefers_configured_then_default() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let configured = tempfile::tempdir().expect("configured");
    crate::core::preferences::save_preferences(
        data_dir.path(),
        &serde_json::json!({ "downloadDir": configured.path().to_string_lossy() }),
    )
    .expect("save");
    assert_eq!(reveal_download_dir(data_dir.path()), configured.path());
    // No preference: the resolved default is used and exists afterwards.
    let bare = tempfile::tempdir().expect("bare");
    let dir = reveal_download_dir(bare.path());
    assert!(dir.is_dir());
    // A relative preference is not trusted; the default wins.
    crate::core::preferences::save_preferences(
        data_dir.path(),
        &serde_json::json!({ "downloadDir": "relative/dir" }),
    )
    .expect("save relative");
    let fallback = reveal_download_dir(data_dir.path());
    assert_ne!(fallback, PathBuf::from("relative/dir"));
    assert!(fallback.is_absolute());
}

/// Issue #18: Explorer's /select argument must quote the path only. A
/// whole-argument-quoted "/select,C:\... (4).msi" is mis-parsed by
/// Explorer, which then opens its default folder (Documents).
#[test]
fn explorer_select_argument_quotes_path_only() {
    let arg = explorer_select_arg(Path::new(r"C:\Users\15754\Downloads\a (4).msi"));
    assert_eq!(arg, r#"/select,"C:\Users\15754\Downloads\a (4).msi""#);
    // Space-free paths keep the same canonical shape.
    assert_eq!(
        explorer_select_arg(Path::new(r"C:\Downloads\a.msi")),
        r#"/select,"C:\Downloads\a.msi""#
    );
}
