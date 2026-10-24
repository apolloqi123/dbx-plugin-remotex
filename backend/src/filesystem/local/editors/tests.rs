use super::*;

// Separator-normalized probe: on a non-Windows host Path::join produces
// '/' separators even for Windows-style relative parts, so both sides are
// normalized to '/' before comparing.
fn probe_from(paths: &[String]) -> impl Fn(&Path) -> bool + '_ {
    let wanted: Vec<String> = paths.iter().map(|p| p.replace('\\', "/")).collect();
    move |path: &Path| {
        let text = path.to_string_lossy().replace('\\', "/");
        wanted.contains(&text)
    }
}

fn lister_from(dirs: &[PathBuf]) -> impl Fn(&Path) -> Vec<PathBuf> + '_ {
    move |_dir: &Path| dirs.to_vec()
}

fn fake_env(apps: &[&str]) -> EnvPaths {
    EnvPaths {
        local_appdata: Some(PathBuf::from(r"C:\Users\u\AppData\Local")),
        program_files: Some(PathBuf::from(r"C:\Program Files")),
        program_files_x86: Some(PathBuf::from(r"C:\Program Files (x86)")),
        windows_dir: Some(PathBuf::from(r"C:\Windows")),
        path_dirs: vec![PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")],
        applications_dirs: apps.iter().map(PathBuf::from).collect(),
    }
}

#[test]
fn macos_catalog_probes_applications() {
    let env = fake_env(&["/Applications"]);
    let app_paths = [
        "/Applications/Visual Studio Code.app".to_string(),
        "/Applications/wpsoffice.app".to_string(),
        "/System/Applications/TextEdit.app".to_string(),
    ];
    let probe = probe_from(&app_paths);
    let catalog = build_catalog("macos", &env, &probe, &lister_from(&[]));
    let by_id = |id: &str| catalog.iter().find(|e| e.id == id).expect("entry");
    assert!(by_id("vscode").available);
    assert!(by_id("wps").available);
    assert!(!by_id("msword").available);
    // TextEdit is always present via the system path.
    assert!(by_id("textedit").available);
    match &by_id("vscode").launch {
        LaunchSpec::OpenApp { app_id } => assert_eq!(app_id, "Visual Studio Code"),
        other => panic!("expected openApp, got {other:?}"),
    }
}

#[test]
fn windows_catalog_resolves_literal_and_versioned_paths() {
    let env = fake_env(&[]);
    let win_paths = [
        r"C:\Users\u\AppData\Local\Programs\Microsoft VS Code\Code.exe".to_string(),
        r"C:\Users\u\AppData\Local\Kingsoft\WPS Office\11.1.0.12345\office6\wps.exe".to_string(),
        r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE".to_string(),
    ];
    let probe = probe_from(&win_paths);
    // Real list_subdirs returns absolute children of the scanned dir.
    let lister = |dir: &Path| vec![dir.join("11.1.0.12345")];
    let catalog = build_catalog("windows", &env, &probe, &lister);
    let by_id = |id: &str| catalog.iter().find(|e| e.id == id).expect("entry");
    assert!(by_id("vscode").available);
    assert!(by_id("wps-writer").available);
    assert!(by_id("msword").available);
    assert!(!by_id("msexcel").available);
    match &by_id("wps-writer").launch {
        LaunchSpec::Exe { exe, args } => {
            let normalized = exe.replace('\\', "/");
            assert!(normalized.ends_with("office6/wps.exe"), "exe={exe}");
            assert_eq!(args, &["{file}"]);
        }
        other => panic!("expected exe, got {other:?}"),
    }
}

#[test]
fn linux_catalog_probes_path() {
    let env = fake_env(&[]);
    let linux_paths = [
        "/usr/local/bin/code".to_string(),
        "/usr/bin/gedit".to_string(),
    ];
    let probe = probe_from(&linux_paths);
    let catalog = build_catalog("linux", &env, &probe, &lister_from(&[]));
    let by_id = |id: &str| catalog.iter().find(|e| e.id == id).expect("entry");
    assert!(by_id("vscode").available);
    let exe = match &by_id("vscode").launch {
        LaunchSpec::Exe { exe, .. } => exe.as_str(),
        _ => panic!("exe expected"),
    };
    assert_eq!(exe, "/usr/local/bin/code");
    assert!(!by_id("sublime").available);
}

#[test]
fn editors_list_payload_has_shape() {
    let payload = editors_list_payload();
    let platform = payload["platform"].as_str().expect("platform");
    assert!(["macos", "windows", "linux", "other"].contains(&platform));
    let editors = payload["editors"].as_array().expect("editors array");
    assert!(!editors.is_empty());
    for editor in editors {
        assert!(editor["id"].as_str().is_some());
        assert!(editor["name"].as_str().is_some());
        assert!(editor["available"].is_boolean());
        assert!(editor["launch"].is_object());
        assert!(editor["suggestedExtensions"].is_array());
    }
}

#[test]
fn tokenizer_handles_quotes_and_windows_paths() {
    assert_eq!(
        tokenize_command("code --wait {file}").expect("tokens"),
        ["code", "--wait", "{file}"]
    );
    // Windows path in quotes keeps its separators.
    assert_eq!(
        tokenize_command(r#""C:\Program Files\App\app.exe" -n {file}"#).expect("tokens"),
        [r"C:\Program Files\App\app.exe", "-n", "{file}"]
    );
    // Unquoted backslashes are literal (Windows-style).
    assert_eq!(
        tokenize_command(r"C:\Tools\app.exe {file}").expect("tokens")[0],
        r"C:\Tools\app.exe"
    );
    // Quoted escape only eats the quote/backslash itself.
    assert_eq!(tokenize_command(r#""a\"b""#).expect("tokens")[0], "a\"b");
    assert_eq!(tokenize_command("").expect("empty"), Vec::<String>::new());
    assert!(tokenize_command("\"unterminated").is_err());
    assert!(tokenize_command("'unterminated").is_err());
}

#[test]
fn substitute_file_replaces_placeholder_or_appends() {
    let path = Path::new("/tmp/a b.txt");
    assert_eq!(
        substitute_file(&["--wait".into(), "{file}".into()], path),
        ["--wait", "/tmp/a b.txt"]
    );
    // No placeholder: path appended at the end.
    assert_eq!(
        substitute_file(&["-n".into()], path),
        ["-n", "/tmp/a b.txt"]
    );
    // Every occurrence is replaced.
    assert_eq!(
        substitute_file(&["{file}".into(), "-o".into(), "{file}".into()], path),
        ["/tmp/a b.txt", "-o", "/tmp/a b.txt"]
    );
}

#[test]
fn launch_spec_from_params_resolves_catalog_and_custom() {
    // Unknown id is an error, not a silent fallback.
    assert!(launch_spec_from_params(&json!({ "editorId": "nope" })).is_err());
    // Custom command: first token is the exe, the rest are args.
    let spec = launch_spec_from_params(&json!({
        "custom": { "name": "Mine", "command": "code --wait {file}" }
    }))
    .expect("custom spec");
    match spec {
        LaunchSpec::Exe { exe, args } => {
            assert_eq!(exe, "code");
            assert_eq!(args, ["--wait", "{file}"]);
        }
        other => panic!("exe expected, got {other:?}"),
    }
    // Empty command is rejected.
    assert!(launch_spec_from_params(&json!({ "custom": { "command": "   " } })).is_err());
    // Neither id nor custom is an error.
    assert!(launch_spec_from_params(&json!({})).is_err());
}

#[test]
fn open_with_requires_recorded_download() {
    let history = vec![serde_json::json!({
        "direction": "download", "status": "completed",
        "localPath": "/Downloads/a.txt"
    })];
    let spec = LaunchSpec::Exe {
        exe: "true".into(),
        args: vec![],
    };
    let launched = std::cell::Cell::new(false);
    {
        let launcher = |_: &Path, _: &LaunchSpec| {
            launched.set(true);
            Ok(())
        };
        open_with_validated_with(&history, Path::new("/Downloads/a.txt"), &spec, &launcher)
            .expect("allowed");
    }
    assert!(launched.get());
    launched.set(false);
    let rejected =
        open_with_validated_with(&history, Path::new("/etc/passwd"), &spec, &|_, _| Ok(()));
    assert!(rejected
        .unwrap_err()
        .contains("not saved by a completed download"));
    assert!(!launched.get(), "rejected paths must not launch");
}

#[test]
fn list_subdirs_sorts_versions_newest_first() {
    let base = tempfile::tempdir().expect("tempdir");
    for name in ["11.1.0.1", "12.0.0.9", "10.9.9.9"] {
        std::fs::create_dir_all(base.path().join(name)).expect("mkdir");
    }
    std::fs::write(base.path().join("loose.txt"), b"x").expect("file");
    let dirs = list_subdirs(base.path());
    let names: Vec<String> = dirs
        .iter()
        .map(|d| d.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["12.0.0.9", "11.1.0.1", "10.9.9.9"]);
}
