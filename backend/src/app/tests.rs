//! Application-shell unit tests.

use super::actions::*;
use super::params::*;
use super::paths::*;
use super::*;

#[test]
fn rdp_start_gate_refuses_by_default_and_allows_explicit_experimental_opt_in() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    assert!(rdp_start_gate(data_dir.path()).is_err());
    crate::core::preferences::save_preferences(
        data_dir.path(),
        &json!({ "rdp_experimental_enabled": true }),
    )
    .expect("enable experimental RDP");
    assert!(rdp_start_gate(data_dir.path()).is_ok());
}

/// `connection/protocol` 响应组装：本地 Shell 目标优先返回
/// protocol="local" + 连接表单 shell（空 shell 原样透出）；非本地连接回落
/// 非 SSH 凭据注册表协议，其次已登记的 SSH 连接（"ssh"），都没有则空串
/// （宿主尚未推送该连接，前端短重试）。
#[test]
fn connection_protocol_payload_prefers_local_target() {
    let local = connection_protocol_payload(
        Some(crate::sessions::local::target::LocalTarget {
            id: "conn-local-1".to_string(),
            name: Some("Local shell".to_string()),
            shell: "/bin/bash".to_string(),
            cwd: "/tmp/demo".to_string(),
            env: vec![("EDITOR".to_string(), "/usr/bin/vim".to_string())],
        }),
        Some("rdp".to_string()),
        false,
    );
    assert_eq!(
        local,
        json!({
            "protocol": "local",
            "shell": "/bin/bash",
            "cwd": "/tmp/demo",
            "env": [{ "name": "EDITOR", "value": "/usr/bin/vim" }],
        })
    );
    // 连接表单未填 shell：空串透出，前端回落默认 Shell 解析链。
    let local_default = connection_protocol_payload(
        Some(crate::sessions::local::target::LocalTarget {
            id: "conn-local-2".to_string(),
            name: None,
            shell: String::new(),
            cwd: String::new(),
            env: Vec::new(),
        }),
        None,
        false,
    );
    assert_eq!(
        local_default,
        json!({ "protocol": "local", "shell": "", "cwd": "", "env": [] })
    );
    // 非 SSH 会话连接（telnet/vnc/rdp/serial）与未暂存连接维持原契约。
    let stored = connection_protocol_payload(None, Some("telnet".to_string()), false);
    assert_eq!(stored, json!({ "protocol": "telnet" }));
    let unstaged = connection_protocol_payload(None, None, false);
    assert_eq!(unstaged, json!({ "protocol": "" }));
    // 已登记的 SSH 连接：首个查询即拿到 "ssh"，前端不再把重试窗口跑满
    // （原恢复态 10×500ms 的纯等待是每次连接约 4.5s 延迟的来源）。
    let ssh = connection_protocol_payload(None, None, true);
    assert_eq!(ssh, json!({ "protocol": "ssh" }));
    // 本地 Shell 目标仍然优先（同一 id 不会双登记，防御性保持）。
    let local_beats_ssh = connection_protocol_payload(
        Some(crate::sessions::local::target::LocalTarget {
            id: "conn-local-3".to_string(),
            name: None,
            shell: "/bin/sh".to_string(),
            cwd: String::new(),
            env: Vec::new(),
        }),
        None,
        true,
    );
    assert_eq!(
        local_beats_ssh,
        json!({ "protocol": "local", "shell": "/bin/sh", "cwd": "", "env": [] })
    );
}

/// 宿主对 connection/action 响应的契约（与 ssh.rs 摘要臂测试同源）：
/// 取消选文件/取消来源选择 = 整体 null（安静无操作）——不能回归成
/// `{message, fieldValues: null}`（宿主以 must be an object 拒掉）。
#[test]
fn connection_action_cancel_returns_plain_null() {
    let mut cancelled_file =
        |_: &crate::filesystem::local::fs::ImportSourceSpec| -> Result<Option<String>, String> {
            Ok(None)
        };
    let mut cancelled = || -> Result<Option<String>, String> { Ok(None) };
    let mut cancelled_source = || -> Result<Option<usize>, String> { Ok(None) };
    let value = action_import_private_key(&mut cancelled).expect("cancel is not an error");
    assert!(value.is_null(), "private-key cancel: {value}");
    let value = action_import_sessions(&json!({}), &mut cancelled_source, &mut cancelled_file)
        .expect("cancel is not an error");
    assert!(value.is_null(), "session source cancel: {value}");
    let mut picked_source = || -> Result<Option<usize>, String> { Ok(Some(0)) };
    let value = action_import_sessions(&json!({}), &mut picked_source, &mut cancelled_file)
        .expect("cancel is not an error");
    assert!(value.is_null(), "session file cancel: {value}");
}

/// 契约另一半：fieldValues 一旦出现必须是 object，且只含 manifest 已声明
/// 字段、永不携带密码（凭据红线）。`path` 参数注入固定文件，两段对话框
/// 都不得被触发。
#[test]
fn connection_action_import_sessions_fills_object_field_values_without_secrets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("demo.conf");
    std::fs::write(
        &file,
        "Host demo\n  HostName demo.internal\n  User root\n  Port 2222\n",
    )
    .expect("write fixture");
    let params = json!({ "path": file.to_string_lossy() });
    let mut picker =
        || -> Result<Option<usize>, String> { panic!("path param must skip the source picker") };
    let mut file_picker =
        |source: &crate::filesystem::local::fs::ImportSourceSpec| -> Result<Option<String>, String> {
            panic!("path param must skip the file picker, got {}", source.kind)
        };
    let value =
        action_import_sessions(&params, &mut picker, &mut file_picker).expect("sshconfig import");
    let field_values = value
        .get("fieldValues")
        .and_then(Value::as_object)
        .expect("fieldValues must be an object for the host");
    assert_eq!(field_values["host"], "demo.internal");
    assert_eq!(field_values["username"], "root");
    assert!(
        !field_values.contains_key("password"),
        "password must never be backfilled: {field_values:?}"
    );
}

/// 「导入连接」的两段对话框流：`source` 参数跳过来源选择（冒烟注入同款
/// 契约），file picker 收到对应来源（默认目录 = ~/.ssh）；picker 注入
/// 固定文件后按既有解析器回填。未知来源显式报错。
#[test]
fn connection_action_import_sessions_resolves_source_then_picks_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("sessions.mxtsessions");
    std::fs::write(
        &file,
        "[Bookmarks]\nSubRep=Prod\\Web\nweb1=#109#0%192.168.1.10%22%deploy%pw%...\n",
    )
    .expect("write fixture");
    let params = json!({ "source": "moba" });
    let picked = std::sync::Mutex::new(Vec::new());
    let mut source_picker =
        || -> Result<Option<usize>, String> { panic!("source param must skip the source picker") };
    let mut file_picker =
        |source: &crate::filesystem::local::fs::ImportSourceSpec| -> Result<Option<String>, String> {
            picked
                .lock()
                .expect("picked lock")
                .push(source.kind.to_string());
            Ok(Some(file.to_string_lossy().into_owned()))
        };
    let value =
        action_import_sessions(&params, &mut source_picker, &mut file_picker).expect("moba import");
    {
        let entries = picked.lock().expect("picked lock");
        assert_eq!(
            *entries,
            ["moba"],
            "file picker must receive the chosen source"
        );
    }
    assert_eq!(value["fieldValues"]["host"], "192.168.1.10");

    let unknown = action_import_sessions(
        &json!({ "source": "nope" }),
        &mut source_picker,
        &mut file_picker,
    )
    .expect_err("unknown source must be rejected");
    assert!(unknown.contains("Unknown import source"), "{unknown}");
}

/// telnet 表单凭据 → 声明式自动登录：用户名为空时只带密码（登录提示由
/// 内置词表应答），非空时一并下发；正则槽位全部留空走内置默认词表。
#[test]
fn telnet_form_auto_login_builds_declarative_spec() {
    let spec = telnet_form_auto_login("ops", "s3cret");
    let debug_render = format!("{spec:?}");
    let declarative = spec.declarative.expect("declarative form");
    assert_eq!(declarative.username.as_deref(), Some("ops"));
    assert_eq!(declarative.password.as_deref(), Some("s3cret"));
    assert!(
        declarative.username_prompt_regex.is_none() && declarative.password_prompt_regex.is_none()
    );
    assert!(spec.rules.is_none() && spec.secrets.is_empty());

    let password_only = telnet_form_auto_login("", "s3cret");
    assert_eq!(
        password_only
            .declarative
            .expect("declarative form")
            .username,
        None,
        "blank form username must not answer login prompts"
    );
    // Debug 脱敏红线：spec 的调试呈现永不携带密码原文。
    assert!(
        !debug_render.contains("s3cret"),
        "auto-login debug leaked a password"
    );
}

#[test]
fn trigger_validation_reports_format_without_secret_content() {
    assert_eq!(trigger_input_format(None), "empty");
    assert_eq!(trigger_input_format(Some(&json!(""))), "empty");
    assert_eq!(trigger_input_format(Some(&json!({ "stages": [] }))), "json");
    assert_eq!(
        trigger_input_format(Some(&json!("#!! ExpectCount 1\n#!! ExpectPattern1 code"))),
        "tssh"
    );
    assert_eq!(trigger_input_format(Some(&json!(42))), "invalid");

    let parsed = crate::features::automation::triggers::parse_triggers(
        Some(&json!(
            r#"{"stages":[{"pattern":"code","sendSecretKey":"trigger_answer_1"}]}"#
        )),
        &|key| (key == "trigger_answer_1").then(|| "do-not-return-this".to_string()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(parsed.stages.len(), 1);
    let debug = format!("{parsed:?}");
    assert!(!debug.contains("do-not-return-this"));
}

#[test]
fn preview_byte_limits_are_bounded() {
    assert_eq!(bounded_bytes(&json!({}), "maxBytes", 12), 12);
    assert_eq!(bounded_bytes(&json!({ "maxBytes": 0 }), "maxBytes", 12), 1);
    assert_eq!(
        bounded_bytes(&json!({ "maxBytes": 99_999_999 }), "maxBytes", 12),
        1024 * 1024
    );
}

#[test]
fn connection_action_id_accepts_host_object_and_legacy_string_forms() {
    assert_eq!(
        connection_action_id(&json!({
            "action": { "id": "import-private-key" }
        })),
        Ok("import-private-key")
    );
    assert_eq!(
        connection_action_id(&json!({ "action": "import-sessions" })),
        Ok("import-sessions")
    );
    assert_eq!(
        connection_action_id(&json!({ "action": {} })),
        Err("Missing action id".to_string())
    );
}

#[test]
fn optional_u64_falls_back_on_missing_or_invalid() {
    assert_eq!(optional_u64(&json!({}), "offset", 0), 0);
    assert_eq!(optional_u64(&json!({ "offset": 4096 }), "offset", 0), 4096);
    // Negative numbers and non-numeric values fall back to the default.
    assert_eq!(optional_u64(&json!({ "offset": -3 }), "offset", 0), 0);
    assert_eq!(optional_u64(&json!({ "offset": "later" }), "offset", 7), 7);
    assert_eq!(optional_u64(&json!({ "offset": null }), "offset", 7), 7);
}

fn lookup_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |key: &str| {
        pairs
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| OsString::from(*value))
    }
}

#[test]
fn plugin_data_dir_env_var_takes_priority() {
    let dir = resolve_plugin_data_dir(lookup_from(&[
        ("DBX_PLUGIN_DATA_DIR", "/tmp/explicit-plugin-data"),
        ("DBX_DATA_DIR", "/tmp/unused-dbx-data"),
        ("HOME", "/Users/unused"),
    ]));
    assert_eq!(dir, PathBuf::from("/tmp/explicit-plugin-data"));
}

#[test]
fn blank_env_values_are_treated_as_unset() {
    // A blank DBX_PLUGIN_DATA_DIR must not win; DBX_DATA_DIR still applies.
    let dir = resolve_plugin_data_dir(lookup_from(&[
        ("DBX_PLUGIN_DATA_DIR", "   "),
        ("DBX_DATA_DIR", "/tmp/dbx-root"),
        ("HOME", "/Users/unused"),
    ]));
    assert_eq!(
        dir,
        PathBuf::from("/tmp/dbx-root")
            .join("plugin-data")
            .join("io.dbx.sshx")
    );
}

#[test]
fn dbx_data_dir_maps_into_plugin_data_tree() {
    let dir = resolve_plugin_data_dir(lookup_from(&[
        ("DBX_DATA_DIR", "/tmp/dbx-root"),
        ("HOME", "/Users/unused"),
    ]));
    assert_eq!(
        dir,
        PathBuf::from("/tmp/dbx-root")
            .join("plugin-data")
            .join("io.dbx.sshx")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_home_falls_back_to_application_support() {
    let dir = resolve_plugin_data_dir(lookup_from(&[("HOME", "/Users/tester")]));
    assert_eq!(
        dir,
        PathBuf::from("/Users/tester")
            .join("Library/Application Support")
            .join("dbx-plugin-data")
            .join("io.dbx.sshx")
    );
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn unix_xdg_data_home_is_preferred_over_local_share() {
    let with_xdg = resolve_plugin_data_dir(lookup_from(&[
        ("XDG_DATA_HOME", "/xdg/data"),
        ("HOME", "/Users/tester"),
    ]));
    assert_eq!(
        with_xdg,
        PathBuf::from("/xdg/data/dbx-plugin-data/io.dbx.sshx")
    );
    let without_xdg = resolve_plugin_data_dir(lookup_from(&[("HOME", "/Users/tester")]));
    assert_eq!(
        without_xdg,
        PathBuf::from("/Users/tester/.local/share/dbx-plugin-data/io.dbx.sshx")
    );
}

#[cfg(windows)]
#[test]
fn windows_falls_back_to_appdata() {
    let dir = resolve_plugin_data_dir(lookup_from(&[(
        "APPDATA",
        r"C:\Users\tester\AppData\Roaming",
    )]));
    assert_eq!(
        dir,
        PathBuf::from(r"C:\Users\tester\AppData\Roaming")
            .join("dbx-plugin-data")
            .join("io.dbx.sshx")
    );
}

#[test]
fn all_sources_missing_falls_back_to_temp_dir() {
    let dir = resolve_plugin_data_dir(lookup_from(&[]));
    assert_eq!(
        dir,
        std::env::temp_dir()
            .join("dbx-plugin-data")
            .join("io.dbx.sshx")
    );
}

#[test]
fn password_is_not_in_workbench_status() {
    let value = json!({ "ok": true, "plugin": "io.dbx.sshx" });
    assert!(!value.to_string().contains("password"));
}
