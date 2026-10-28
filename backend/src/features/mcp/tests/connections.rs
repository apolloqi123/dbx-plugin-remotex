use super::*;

#[test]
fn scope_allows_matches_id_name_and_host() {
    let scope = |entries: &[&str]| -> Vec<String> {
        entries.iter().map(|entry| entry.to_string()).collect()
    };
    // 空 entry 列表本身不授予权限；持久化空数组会在 ConnectionScope
    // 层映射为 Unrestricted，显式空环境变量则映射为 DenyAll。
    assert!(!scope_allows(&[], "c1", Some("prod"), "db.local"));
    let entries = scope(&["conn-9", "ops@LEGACY", "Web-01"]);
    assert!(scope_allows(&entries, "conn-9", None, "other.local"));
    assert!(scope_allows(&entries, "other-id", Some("ops@LEGACY"), "x")); // name 精确（大小写敏感）
    assert!(scope_allows(&entries, "other-id", None, "web-01")); // host 大小写不敏感
    assert!(!scope_allows(
        &entries,
        "other-id",
        Some("prod"),
        "db.local"
    ));
    assert!(!scope_allows(&entries, "other-id", None, "db.local"));
}

#[test]
fn env_permission_parsers_distinguish_unset_allowlist_and_deny_all() {
    assert_eq!(
        permission_mode_from_env(|_| Some("confirm".to_string())).as_deref(),
        Some("confirm")
    );
    // 未设置/未知值不覆盖。
    assert!(permission_mode_from_env(|_| None).is_none());
    assert!(permission_mode_from_env(|_| Some("yolo".to_string())).is_none());
    assert_eq!(
        scope_from_env(|_| Some(" a , b,,c ".to_string())),
        Some(ConnectionScope::AllowList(vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
        ]))
    );
    // 显式空列表是有效的 fail-closed 覆盖，绝不能与持久化空数组
    // （未限制）复用同一种状态。
    assert_eq!(
        scope_from_env(|_| Some(" , ".to_string())),
        Some(ConnectionScope::DenyAll)
    );
    assert!(scope_from_env(|_| None).is_none());
}

#[test]
fn connection_scope_policy_preserves_persisted_empty_as_unrestricted() {
    assert!(ConnectionScope::from_persisted(Vec::new()).allows("id", Some("name"), "host"));
    assert!(!ConnectionScope::DenyAll.allows("id", Some("name"), "host"));
    assert!(ConnectionScope::AllowList(vec!["host".to_string()]).allows("id", None, "HOST"));
}

#[test]
fn settings_roundtrip_permission_fields_and_env_override() {
    let state = state();
    // 默认 autonomous + 空作用域。
    let view = state.settings_get();
    assert_eq!(view["execPermissionMode"], "autonomous");
    assert_eq!(view["connectionScope"], json!([]));
    // 合法更新。
    let updated = state
        .settings_set(&json!({
            "execPermissionMode": "confirm",
            "connectionScope": ["conn-1", "Prod-DB"],
        }))
        .unwrap();
    assert_eq!(updated["execPermissionMode"], "confirm");
    assert_eq!(updated["connectionScope"], json!(["conn-1", "Prod-DB"]));
    assert_eq!(updated["persistedExecPermissionMode"], "confirm");
    // 非法 mode / 非 scope 数组被拒绝。
    assert!(state
        .settings_set(&json!({ "execPermissionMode": "yolo" }))
        .is_err());
    assert!(state
        .settings_set(&json!({ "connectionScope": "conn-1" }))
        .is_err());
    // 重读（含从盘加载路径）不丢。
    let reloaded = McpPermission::load(&state.limits_path);
    assert_eq!(reloaded.exec_permission_mode, "confirm");
    assert_eq!(
        reloaded.connection_scope,
        vec!["conn-1".to_string(), "Prod-DB".to_string()]
    );
    // size-limit 键与 permission 键同文件共存（merge 写入）。
    state
        .settings_set(&json!({ "maxReadBytes": 4096 }))
        .unwrap();
    let document: Value =
        serde_json::from_str(&std::fs::read_to_string(&state.limits_path).unwrap()).unwrap();
    assert_eq!(document["maxReadBytes"], 4096);
    assert_eq!(document["execPermissionMode"], "confirm");
}

#[test]
fn connection_list_result_filters_by_scope() {
    let stored = || {
        StoredConnection::from_lifecycle_params(&json!({
            "connection": { "id": "conn-1", "name": "prod", "host": "db.local",
                            "port": 22, "username": "ops", "password": "pw" }
        }))
        .unwrap()
    };
    // 持久化空作用域映射为未限制：两来源都全量。
    let all = connection_list_result(
        Ok(vec![
            json!({"id":"conn-9","name":"staging","host":"stg.local"}),
        ]),
        &[stored()],
        &ConnectionScope::Unrestricted,
    );
    assert_eq!(all["connections"].as_array().unwrap().len(), 2);
    // host 作用域：bridge 条目按 host 过滤，registry 条目按 id/name/host。
    let filtered = connection_list_result(
        Ok(vec![
            json!({"id":"conn-9","name":"staging","host":"stg.local"}),
            json!({"id":"conn-1","name":"prod","host":"db.local"}),
        ]),
        &[stored()],
        &ConnectionScope::AllowList(vec!["db.local".to_string()]),
    );
    let ids: Vec<&str> = filtered["connections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["conn-1"]);
    // registry 降级路径同样过滤。
    let degraded = connection_list_result(
        Err("bridge down".to_string()),
        &[stored()],
        &ConnectionScope::AllowList(vec!["conn-9".to_string()]),
    );
    assert_eq!(degraded["connections"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn quick_sudo_profiles_roundtrip_without_echoing_secrets() {
    let dir = std::env::temp_dir().join(format!("dbx-mcp-profiles-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = McpState::new(dir.clone());
    // Test-only secret assembled at runtime (never a real credential).
    let secret = format!("test-{}", uuid::Uuid::new_v4());

    // MCP 工具面已收（#203 工具瘦身）；store 行为经 runtime 方法直接覆盖。
    let saved = state
        .runtime
        .profiles_save(&json!({
            "name": "ops",
            "sudoPassword": secret,
            "totpSecret": "JBSWY3DPEHPK3PXP",
            "authFlowMode": "password_plus_otp",
            "sudoUsePty": true,
        }))
        .await
        .unwrap();
    assert_eq!(saved["created"], true);
    assert_eq!(saved["profile"]["sudoPasswordSet"], true);
    assert_eq!(saved["profile"]["totpConfigured"], true);

    let listed = state.runtime.profiles_list();
    let rendered = listed.to_string();
    assert!(!rendered.contains(&secret), "secret leaked: {rendered}");
    assert!(rendered.contains("\"sudoPasswordSet\":true"));

    // Duplicate names are rejected, unknown references report clearly.
    let duplicate = state.runtime.profiles_save(&json!({ "name": "OPS" })).await;
    assert!(duplicate.unwrap_err().contains("already in use"));
    let missing = resolve_profile_reference(
        &sudo_profiles::load_store(&dir),
        &json!({ "quickSudoProfile": "ghost" }),
    );
    assert!(missing.unwrap_err().contains("not found"));

    let id = saved["profile"]["id"].as_str().unwrap().to_string();
    let removed = state.runtime.profiles_delete(&id).await.unwrap();
    assert_eq!(removed["removed"], true);
    let listed = state.runtime.profiles_list();
    assert_eq!(listed["profiles"].as_array().unwrap().len(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn sudo_auth_resolution_follows_declared_source() {
    let dir = std::env::temp_dir().join(format!("dbx-mcp-sudo-src-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = McpState::new(dir.clone());
    let profile_secret = format!("profile-{}", uuid::Uuid::new_v4());
    let call_secret = format!("call-{}", uuid::Uuid::new_v4());

    let saved = state
        .runtime
        .profiles_save(&json!({
            "name": "src-ops",
            "sudoPassword": profile_secret,
            "totpSecret": "JBSWY3DPEHPK3PXP",
        }))
        .await
        .unwrap();
    assert_eq!(saved["created"], true);

    // Global source: the referenced global profile owns the credential
    // source (with TOTP), exactly like the workbench.
    let (stored, _) = source_connection("global", "src-ops");
    let auth = state
        .resolve_sudo_auth(&json!({}), None, Some(stored))
        .await
        .unwrap();
    assert_eq!(auth.password, profile_secret);
    assert!(!auth.totp_secrets.is_empty());

    // Explicit per-call arguments win over the declared source.
    let (stored, _) = source_connection("global", "src-ops");
    let auth = state
        .resolve_sudo_auth(&json!({ "sudoPassword": call_secret }), None, Some(stored))
        .await
        .unwrap();
    assert_eq!(auth.password, call_secret);

    // The per-call quickSudoProfile reference replaces the declared one
    // and is applied even without a saved connection (inline MCP calls).
    let explicit_profile = sudo_profiles::SudoProfile {
        id: "inline".to_string(),
        name: "inline".to_string(),
        sudo_password: call_secret.clone(),
        totp_secret: String::new(),
        auth_flow_mode: "password_only".to_string(),
        password_prompt_hint: String::new(),
        totp_prompt_hint: String::new(),
        sudo_use_pty: false,
        created_at: 0,
        updated_at: 0,
    };
    let (stored, _) = source_connection("global", "src-ops");
    let auth = state
        .resolve_sudo_auth(&json!({}), Some(explicit_profile.clone()), Some(stored))
        .await
        .unwrap();
    assert_eq!(auth.password, call_secret);
    assert!(auth.totp_secrets.is_empty());
    let auth = state
        .resolve_sudo_auth(&json!({}), Some(explicit_profile.clone()), None)
        .await
        .unwrap();
    assert_eq!(auth.password, call_secret);

    // Custom source without a binding: the connection's own sudo config
    // (empty here) degrades to the login password fallback.
    let (stored, login) = source_connection("custom", "");
    let auth = state
        .resolve_sudo_auth(&json!({}), None, Some(stored))
        .await
        .unwrap();
    assert_eq!(auth.password, login);

    // Off refuses like the workbench gate, unless the caller passes
    // explicit credentials.
    let (stored, _) = source_connection("off", "");
    let refused = state
        .resolve_sudo_auth(&json!({}), None, Some(stored))
        .await;
    assert!(refused.unwrap_err().contains("Quick Sudo is disabled"));
    let (stored, _) = source_connection("off", "");
    let auth = state
        .resolve_sudo_auth(&json!({ "sudoPassword": call_secret }), None, Some(stored))
        .await
        .unwrap();
    assert_eq!(auth.password, call_secret);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pool_ids_key_by_identity() {
    let id = connection_pool_id(&json!({ "host": "h", "port": 2222, "username": "u" }));
    assert_eq!(id, "mcp-u@h:2222");
    assert_eq!(
        connection_pool_id(&json!({ "host": "h", "username": "u" })),
        "mcp-u@h:22"
    );
}

#[test]
fn jump_hosts_parse_into_the_chain() {
    let connection = stored_connection_from_arguments(&json!({
        "host": "target", "username": "u", "password": "p",
        "jumpHosts": [
            { "host": "bastion", "port": 2222, "username": "ops", "password": "jp" },
            { "host": "inner", "username": "relay", "authentication": "private-key", "private_key_path": "/k" },
        ],
    }))
    .unwrap();
    assert_eq!(connection.jump_hosts.len(), 2);
    assert_eq!(connection.jump_hosts[0].port, 2222);
    assert_eq!(connection.jump_hosts[1].authentication, "private-key");

    let too_many = json!({
        "host": "t", "username": "u", "password": "p",
        "jumpHosts": [
            { "host": "a", "username": "u", "password": "p" },
            { "host": "b", "username": "u", "password": "p" },
            { "host": "c", "username": "u", "password": "p" },
            { "host": "d", "username": "u", "password": "p" },
        ],
    });
    assert!(stored_connection_from_arguments(&too_many)
        .err()
        .unwrap()
        .contains("At most 3"));

    let missing_password = json!({
        "host": "t", "username": "u", "password": "p",
        "jumpHosts": [{ "host": "bastion", "username": "ops" }],
    });
    assert!(stored_connection_from_arguments(&missing_password).is_err());
}

#[test]
fn connections_map_authentication_sensibly() {
    let by_key = stored_connection_from_arguments(&json!({
        "host": "h", "username": "u", "command": "x",
        "privateKeyPath": "/k", "totpSecret": "123456", "authFlowMode": "password_plus_otp"
    }))
    .unwrap();
    assert_eq!(by_key.authentication, AuthenticationMethod::PrivateKey);
    assert_eq!(by_key.totp_secret, "123456");
    assert!(stored_connection_from_arguments(&json!({ "host": "h", "username": "u" })).is_err());
}

/// MCP inline dials accept pasted key contents as the path alternative:
/// content-only selects private-key without touching the filesystem, and
/// a dial with neither path nor content is rejected up front.
#[test]
fn connections_accept_private_key_content() {
    let by_content = stored_connection_from_arguments(&json!({
        "host": "h", "username": "u", "command": "x",
        "privateKeyContent": "-----BEGIN OPENSSH PRIVATE KEY-----\nbody\n-----END OPENSSH PRIVATE KEY-----\n"
    }))
    .unwrap();
    assert_eq!(by_content.authentication, AuthenticationMethod::PrivateKey);
    assert!(by_content.private_key_path.is_empty());
    assert!(by_content.private_key.starts_with("-----BEGIN"));

    let explicit_password_mode = stored_connection_from_arguments(&json!({
        "host": "h", "username": "u", "command": "x",
        "authentication": "private-key-password",
        "privateKeyContent": "-----BEGIN OPENSSH PRIVATE KEY-----\nbody\n-----END OPENSSH PRIVATE KEY-----\n",
        "password": "login"
    }))
    .unwrap();
    assert_eq!(
        explicit_password_mode.authentication,
        AuthenticationMethod::PrivateKeyPassword
    );

    // 既有的推断语义保持不变：没有密钥材料时显式 private-key 仍按
    // 「有密码则回退 password」解析（内联拨号从不挂空路径的私钥）。
    let degraded = stored_connection_from_arguments(&json!({
        "host": "h", "username": "u", "command": "x",
        "authentication": "private-key", "password": "login"
    }))
    .unwrap();
    assert_eq!(degraded.authentication, AuthenticationMethod::Password);
    let neither = stored_connection_from_arguments(&json!({
        "host": "h", "username": "u", "command": "x",
        "authentication": "private-key"
    }))
    .unwrap_err();
    assert!(
        neither.contains("Password authentication requires a password"),
        "{neither}"
    );
}

#[test]
fn mcp_limits_default_and_clamp_into_ceilings() {
    let defaults = McpLimits::default();
    assert_eq!(defaults.max_read_bytes, 256 * 1024);
    assert_eq!(defaults.max_download_bytes, 1024 * 1024);
    assert_eq!(defaults.local_transfer_root, "");
    assert_eq!(
        McpLimits {
            max_read_bytes: 0,
            max_upload_bytes: u64::MAX,
            max_download_bytes: 0,
            local_transfer_root: String::new(),
        }
        .sanitized(),
        McpLimits {
            max_read_bytes: 1,
            max_upload_bytes: UPLOAD_LIMIT_CEILING,
            max_download_bytes: 1,
            local_transfer_root: String::new(),
        }
    );
    // Values inside the ceilings pass through untouched.
    let inside = McpLimits {
        max_read_bytes: 512 * 1024,
        max_upload_bytes: 64 * 1024 * 1024,
        max_download_bytes: 4 * 1024 * 1024,
        local_transfer_root: String::new(),
    };
    assert_eq!(inside.clone().sanitized(), inside);
}

#[test]
fn local_transfer_root_setting_accepts_absolute_and_empty_only() {
    assert_eq!(validated_transfer_root(&json!("")).unwrap(), "");
    assert_eq!(
        validated_transfer_root(&json!(ABS_TEST_ROOT)).unwrap(),
        ABS_TEST_ROOT
    );
    assert!(validated_transfer_root(&json!("relative/path")).is_err());
    assert!(validated_transfer_root(&json!(42)).is_err());
}

#[test]
fn local_transfer_settings_persist_and_roundtrip() {
    let dir = std::env::temp_dir().join(format!("mcp-limits-test-{}", std::process::id()));
    let path = dir.join("mcp-settings.json");
    let _ = std::fs::remove_file(&path);
    let defaults = McpLimits::load(&path);
    assert_eq!(defaults.local_transfer_root, "");
    let mut updated = defaults;
    updated.local_transfer_root = ABS_TEST_ROOT.to_string();
    write_settings_document(&path, &updated, &McpPermission::default()).unwrap();
    let reloaded = McpLimits::load(&path);
    assert_eq!(reloaded.local_transfer_root, ABS_TEST_ROOT);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn local_transfer_roots_default_to_temp_and_data_dir() {
    let data_dir = std::env::temp_dir().join(format!("mcp-data-{}", std::process::id()));
    std::fs::create_dir_all(&data_dir).unwrap();
    let roots = local_transfer_roots_for("", &data_dir).unwrap();
    let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let data = std::fs::canonicalize(&data_dir).unwrap();
    assert!(roots.contains(&temp), "{roots:?}");
    assert!(roots.contains(&data), "{roots:?}");
    // A configured root narrows the policy to exactly that root…
    let roots = local_transfer_roots_for(data.to_str().unwrap(), &data_dir).unwrap();
    assert_eq!(roots, vec![data.clone()]);
    // …and an unresolvable configured root is an error, not a fallback.
    assert!(local_transfer_roots_for("/nonexistent/mcp-root", &data_dir).is_err());
    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn mcp_limits_load_falls_back_on_missing_or_corrupt_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mcp-settings.json");
    // Missing file and unparseable content both fall back to defaults.
    assert_eq!(McpLimits::load(&path), McpLimits::default());
    std::fs::write(&path, "not json at all {{{").unwrap();
    assert_eq!(McpLimits::load(&path), McpLimits::default());
    // Partial/garbage fields fall back per-field and out-of-range values
    // are clamped on load.
    std::fs::write(
        &path,
        serde_json::to_string(&json!({
            "maxReadBytes": "bogus",
            "maxUploadBytes": 32 * 1024 * 1024,
            "maxDownloadBytes": 999u64 * 1024 * 1024 * 1024,
        }))
        .unwrap(),
    )
    .unwrap();
    let loaded = McpLimits::load(&path);
    assert_eq!(loaded.max_read_bytes, McpLimits::default().max_read_bytes);
    assert_eq!(loaded.max_upload_bytes, 32 * 1024 * 1024);
    assert_eq!(loaded.max_download_bytes, DOWNLOAD_LIMIT_CEILING);
}

#[test]
fn mcp_limits_persist_roundtrip() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mcp-settings.json");
    let limits = McpLimits {
        max_read_bytes: 128 * 1024,
        max_upload_bytes: 8 * 1024 * 1024,
        max_download_bytes: 2 * 1024 * 1024,
        local_transfer_root: String::new(),
    };
    write_settings_document(&path, &limits, &McpPermission::default()).unwrap();
    assert_eq!(McpLimits::load(&path), limits);
    // A fresh state over the same data dir picks the persisted values up
    // (this is how the --mcp stdio process shares the settings).
    let state = McpState::new(directory.path().to_path_buf());
    // settings_get carries the §1.3 permission keys too; the limit
    // fields must round-trip unchanged.
    let view = state.settings_get();
    let expected = limits.to_json();
    assert_eq!(view["maxReadBytes"], expected["maxReadBytes"]);
    assert_eq!(view["maxUploadBytes"], expected["maxUploadBytes"]);
    assert_eq!(view["maxDownloadBytes"], expected["maxDownloadBytes"]);
    assert_eq!(view["localTransferRoot"], expected["localTransferRoot"]);
}

#[test]
fn mcp_settings_set_validates_and_persists_partial_updates() {
    let directory = tempfile::tempdir().unwrap();
    let state = McpState::new(directory.path().to_path_buf());

    // Partial update: only the named field changes.
    let updated = state
        .settings_set(&json!({ "maxReadBytes": 64 * 1024 }))
        .unwrap();
    assert_eq!(updated["maxReadBytes"], 64 * 1024);
    assert_eq!(
        updated["maxUploadBytes"],
        McpLimits::default().max_upload_bytes
    );
    assert_eq!(state.settings_get(), updated);

    // Invalid values are rejected without touching the stored settings.
    for bad in [
        json!({ "maxReadBytes": 0 }),
        json!({ "maxReadBytes": READ_LIMIT_CEILING + 1 }),
        json!({ "maxUploadBytes": "big" }),
        json!({ "maxDownloadBytes": -1 }),
        json!({ "localTransferRoot": "relative/root" }),
        json!({ "localTransferRoot": 7 }),
    ] {
        assert!(
            state.settings_set(&bad).is_err(),
            "expected error for {bad}"
        );
    }
    assert_eq!(
        state.settings_get()["maxReadBytes"],
        64 * 1024,
        "rejected updates must not change the settings"
    );

    // The transfer root accepts an absolute path and clears on empty.
    let with_root = state
        .settings_set(&json!({ "localTransferRoot": ABS_TEST_ROOT }))
        .unwrap();
    assert_eq!(with_root["localTransferRoot"], ABS_TEST_ROOT);
    let cleared = state
        .settings_set(&json!({ "localTransferRoot": "" }))
        .unwrap();
    assert_eq!(cleared["localTransferRoot"], "");

    // The soft ceilings themselves are accepted.
    let ceilings = state
        .settings_set(&json!({
            "maxReadBytes": READ_LIMIT_CEILING,
            "maxUploadBytes": UPLOAD_LIMIT_CEILING,
            "maxDownloadBytes": DOWNLOAD_LIMIT_CEILING,
        }))
        .unwrap();
    assert_eq!(ceilings["maxReadBytes"], READ_LIMIT_CEILING);

    // Persistence: a second state over the same directory reloads the
    // last accepted values.
    let reloaded = McpState::new(directory.path().to_path_buf());
    assert_eq!(reloaded.settings_get(), ceilings);
}

/// Reliability round 5 (churn): 500 settings write cycles must keep the
/// persisted document bounded (fixed shape, no accumulation files) and
/// every newly-set value immediately visible (no stale-read staleness);
/// clamp semantics keep working across the whole churn.
#[tokio::test]
async fn settings_storage_churn_stays_bounded() {
    let state = state();
    state
        .settings_set(&json!({ "execPermissionMode": "autonomous" }))
        .expect("initial settings write");
    let baseline_files = std::fs::read_dir(state.limits_path.parent().unwrap())
        .expect("data dir")
        .count();
    for index in 0..500 {
        let mode = if index % 2 == 0 {
            "confirm"
        } else {
            "autonomous"
        };
        let upload: u64 = if index % 3 == 0 { 1024 } else { 2048 };
        state
            .settings_set(&json!({
                "execPermissionMode": mode,
                "maxUploadBytes": upload,
            }))
            .unwrap_or_else(|error| panic!("settings_set failed at {index}: {error}"));
        let settings = state.settings_get();
        assert_eq!(
            settings["execPermissionMode"],
            json!(mode),
            "mode not visible immediately at {index}"
        );
        assert_eq!(settings["maxUploadBytes"], json!(upload));
    }
    // The store stays a fixed-shape document: file size bounded, no new
    // files appeared next to it (no per-write accumulation).
    let metadata = std::fs::metadata(&state.limits_path).expect("settings file");
    assert!(
        metadata.len() < 4 * 1024,
        "settings document grew to {} bytes",
        metadata.len()
    );
    let after_files = std::fs::read_dir(state.limits_path.parent().unwrap())
        .expect("data dir")
        .count();
    assert_eq!(after_files, baseline_files, "data dir accumulated files");
    // Clamp semantics survive churn: over-ceiling values never persist.
    state
        .settings_set(&json!({ "maxUploadBytes": u64::MAX }))
        .expect_err("over-ceiling upload cap must be rejected");
}

/// L2: list views expose metadata plus credential PRESENCE flags only —
/// the runtime-composed secret never appears in any branch, and the
/// degraded branch carries the start/upgrade note.
#[test]
fn connection_list_views_never_echo_credentials() {
    let secret = format!("pw-{}", uuid::Uuid::new_v4());
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-list",
            "name": "Web",
            "host": "web.example.test",
            "port": 2222,
            "username": "deploy",
            "password": secret,
            "read_only": true,
        }
    }))
    .unwrap();
    assert_eq!(stored.name.as_deref(), Some("Web"));

    // Degraded branch: registry alone + note.
    let degraded = connection_list_result(
        Err("bridge down".to_string()),
        std::slice::from_ref(&stored),
        &ConnectionScope::Unrestricted,
    );
    assert_eq!(degraded["source"], "session-registry");
    assert!(
        degraded["note"]
            .as_str()
            .unwrap()
            .contains("upgrade the DBX app"),
        "expected the upgrade note: {degraded}"
    );
    let entry = &degraded["connections"][0];
    assert_eq!(entry["id"], "conn-list");
    assert_eq!(entry["name"], "Web");
    assert_eq!(entry["host"], "web.example.test");
    assert_eq!(entry["port"], 2222);
    assert_eq!(entry["username"], "deploy");
    assert_eq!(entry["authentication"], "password");
    assert_eq!(entry["readOnly"], true);
    assert_eq!(entry["passwordSet"], true);
    let rendered = degraded.to_string();
    assert!(!rendered.contains(&secret), "secret leaked: {rendered}");
    assert!(
        !rendered.contains("\"password\":"),
        "raw credential field leaked: {rendered}"
    );

    // Bridge branch: bridge data wins by id, registry-only ids append,
    // and no note is attached.
    let bridge_entry = json!({
        "id": "conn-list", "name": "Web", "host": "web.example.test",
        "port": 2222, "username": "deploy", "authentication": "password",
        "readOnly": false,
    });
    let extra = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-extra", "host": "db.example.test", "port": 22,
            "username": "ops", "password": format!("pw-{}", uuid::Uuid::new_v4()),
        }
    }))
    .unwrap();
    let merged = connection_list_result(
        Ok(vec![bridge_entry]),
        &[stored, extra],
        &ConnectionScope::Unrestricted,
    );
    assert_eq!(merged["source"], "dbx-app-bridge");
    assert!(merged.get("note").is_none(), "bridge hit must not degrade");
    let entries = merged["connections"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "bridge entry + registry-only append");
    assert_eq!(entries[0]["id"], "conn-list");
    // Bridge entries carry no credential flags (contract: absent means
    // the app did not send one).
    assert!(entries[0].get("passwordSet").is_none());
    assert_eq!(entries[1]["id"], "conn-extra");
}

/// stdio selector→id resolution over the bridge list: unique hit,
/// ambiguous (both candidates listed with host), and missing. The
/// endpoint selector disambiguates a duplicate name.
#[test]
fn resolve_connection_in_bridge_list_matches_name_and_endpoint() {
    let entries = vec![
        json!({ "id": "a", "name": "Web", "host": "h1", "port": 22, "username": "ops" }),
        // Trailing space in the payload name still matches after trim.
        json!({ "id": "b", "name": "Web ", "host": "h2", "port": 2222, "username": "deploy" }),
        json!({ "id": "c", "name": "DB", "host": "h3", "port": 22, "username": "ops" }),
    ];
    assert_eq!(
        resolve_connection_in_bridge_list(&entries, &json!({ "connectionName": "DB" })).unwrap(),
        "c"
    );
    let error = resolve_connection_in_bridge_list(&entries, &json!({ "connectionName": "Web" }))
        .unwrap_err();
    assert!(
        error.contains("ambiguous")
            && error.contains("a")
            && error.contains("b")
            && error.contains("h1")
            && error.contains("h2"),
        "expected candidates in the error: {error}"
    );
    // Endpoint narrows the duplicate name to exactly one candidate.
    assert_eq!(
        resolve_connection_in_bridge_list(
            &entries,
            &json!({ "connectionName": "Web", "host": "h2", "port": 2222, "username": "deploy" })
        )
        .unwrap(),
        "b"
    );
    // A complete endpoint alone reuses the saved connection.
    assert_eq!(
        resolve_connection_in_bridge_list(
            &entries,
            &json!({ "host": "H2", "port": 2222, "username": "deploy" })
        )
        .unwrap(),
        "b"
    );
    assert!(
        resolve_connection_in_bridge_list(&entries, &json!({ "connectionName": "ghost" }))
            .unwrap_err()
            .contains("No saved connection matched"),
        "missing selector must be a clear error"
    );
}

/// L3: lifecycle payloads carry the display name through; absent names
/// stay None (older hosts, inline dials).
#[test]
fn lifecycle_name_roundtrip_and_optional_default() {
    let named = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "c1", "name": "  Bastion  ", "host": "h", "port": 22,
            "username": "u", "password": "p",
        }
    }))
    .unwrap();
    assert_eq!(named.name.as_deref(), Some("Bastion"));
    let unnamed = StoredConnection::from_lifecycle_params(&json!({
        "connection": { "id": "c2", "host": "h", "port": 22, "username": "u", "password": "p" }
    }))
    .unwrap();
    assert_eq!(unnamed.name, None);
}

/// L3: two same-named registry entries make the reference ambiguous —
/// the error lists every candidate id + host so the caller can
/// disambiguate by id.
#[tokio::test]
async fn ambiguous_connection_name_lists_candidates() {
    let state = state();
    for (id, host) in [("conn-x", "10.0.0.1"), ("conn-y", "10.0.0.2")] {
        let stored = StoredConnection::from_lifecycle_params(&json!({
            "connection": {
                "id": id, "name": "Twin", "host": host, "port": 22,
                "username": "ops",
                "password": format!("pw-{}", uuid::Uuid::new_v4()),
            }
        }))
        .unwrap();
        state
            .dbx_connections
            .write()
            .await
            .insert(id.to_string(), stored);
    }
    let error = state
        .call_tool(
            "sftp_stat",
            &json!({ "connectionName": "Twin", "path": "/tmp" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("ambiguous")
            && error.contains("conn-x")
            && error.contains("conn-y")
            && error.contains("10.0.0.1")
            && error.contains("10.0.0.2"),
        "expected every candidate in the error: {error}"
    );
}

#[tokio::test]
async fn endpoint_identity_resolves_a_unique_saved_connection_without_id() {
    let state = state();
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-endpoint",
            "name": "Prod bastion",
            "host": "prod.example.test",
            "port": 2222,
            "username": "deploy",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
        }
    }))
    .unwrap();
    state
        .dbx_connections
        .write()
        .await
        .insert(stored.id.clone(), stored);

    let resolved = state
        .registered_connection_by_ref(&json!({
            "host": "PROD.example.test",
            "port": 2222,
            "username": "deploy",
        }))
        .await
        .unwrap()
        .expect("endpoint should resolve the saved connection");
    assert_eq!(resolved.id, "conn-endpoint");
}

#[tokio::test]
async fn connection_name_uses_endpoint_to_disambiguate_candidates() {
    let state = state();
    for (id, host) in [("conn-x", "10.0.0.1"), ("conn-y", "10.0.0.2")] {
        let stored = StoredConnection::from_lifecycle_params(&json!({
            "connection": {
                "id": id,
                "name": "Twin",
                "host": host,
                "port": 22,
                "username": "ops",
                "password": format!("pw-{}", uuid::Uuid::new_v4()),
            }
        }))
        .unwrap();
        state
            .dbx_connections
            .write()
            .await
            .insert(id.to_string(), stored);
    }

    let resolved = state
        .registered_connection_by_ref(&json!({
            "connectionName": "Twin",
            "host": "10.0.0.2",
            "port": 22,
            "username": "ops",
        }))
        .await
        .unwrap()
        .expect("name plus endpoint should resolve one candidate");
    assert_eq!(resolved.id, "conn-y");
}

#[tokio::test]
async fn conflicting_connection_selectors_are_rejected() {
    let state = state();
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-prod",
            "name": "Prod",
            "host": "prod.example.test",
            "port": 22,
            "username": "ops",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
        }
    }))
    .unwrap();
    state
        .dbx_connections
        .write()
        .await
        .insert(stored.id.clone(), stored);

    let error = state
        .registered_connection_by_ref(&json!({
            "connectionId": "conn-prod",
            "host": "other.example.test",
            "username": "ops",
        }))
        .await
        .unwrap_err();
    assert!(
        error.contains("does not match"),
        "unexpected error: {error}"
    );
}

/// Saved-connection addressing must reach the dialers: an sftp/exec call
/// referencing a registered connection attempts the dial (a refused-port
/// error) instead of dying on the pre-fix pool lookup.
#[tokio::test]
async fn sftp_and_exec_resolve_saved_reference_before_dialing() {
    let state = state();
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-prod",
            "name": "Prod",
            "host": "127.0.0.1",
            "port": 1,
            "username": "ops",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
        }
    }))
    .unwrap();
    state
        .dbx_connections
        .write()
        .await
        .insert(stored.id.clone(), stored);

    for (tool, arguments) in [
        (
            "sftp_stat",
            json!({ "connectionName": "Prod", "path": "/tmp" }),
        ),
        (
            "ssh_exec",
            json!({ "connectionName": "Prod", "command": "true" }),
        ),
    ] {
        let error = state.call_tool(tool, &arguments, None).await.unwrap_err();
        assert!(
            !error.contains("Connection is not established")
                && !error.contains("Missing required parameter"),
            "{tool} bypassed saved-connection resolution: {error}"
        );
    }
}

/// 降级矩阵 ②：连接寻址双源（bridge registry vs session registry）
/// 同 id 冲突时桥数据胜出且不产生重复条目；不同 id 各自成列；作用域
/// 对两个来源都生效。
#[test]
fn connection_list_merge_resolves_dual_source_conflicts() {
    let bridge_entry = |id: &str, host: &str| {
        json!({
            "id": id, "name": id.to_uppercase(), "host": host, "port": 22,
            "username": "deploy", "authentication": "password", "readOnly": false,
        })
    };
    let registry = vec![
        // 同 id 不同 host：会话注册表声明更旧的元数据。
        stored_connection("conn-x", "old.example.test", 22, json!({})),
        // 不同 id：必须与桥条目并列出现。
        stored_connection("conn-y", "other.example.test", 2222, json!({})),
    ];
    let bridge = Ok(vec![
        bridge_entry("conn-x", "new.example.test"),
        bridge_entry("conn-y", "other.example.test"),
    ]);

    let merged = connection_list_result(bridge.clone(), &registry, &ConnectionScope::Unrestricted);
    assert_eq!(merged["source"], "dbx-app-bridge");
    let rows = merged["connections"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "id conflict must deduplicate, not append");
    let x = rows.iter().find(|row| row["id"] == "conn-x").unwrap();
    assert_eq!(
        x["host"], "new.example.test",
        "bridge metadata wins on an id conflict"
    );

    // 作用域对桥与注册表两来源都过滤：只留 conn-y。
    let scoped = connection_list_result(
        bridge,
        &registry,
        &ConnectionScope::AllowList(vec!["conn-y".to_string()]),
    );
    let rows = scoped["connections"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["id"], "conn-y");

    // 桥不可用时降级到注册表 + note（同 id 冲突无从发生）。
    let degraded = connection_list_result(
        Err("bridge down".into()),
        &registry,
        &ConnectionScope::Unrestricted,
    );
    assert_eq!(degraded["source"], "session-registry");
    assert_eq!(degraded["connections"].as_array().unwrap().len(), 2);
}

/// settings 联动回归：mcp/settings/set 的每个可调项都能立即驱动受影响
/// 工具的行为（传输根收窄、上传限额、confirm 档 fail-closed、scope 闸），
/// settings_get 回显生效值与 persisted 值。
#[tokio::test]
async fn settings_set_changes_drive_downstream_tool_behavior() {
    // (a) localTransferRoot 收窄：默认根内的文件过根闸，收窄后被拒。
    let st = state();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("inside.bin"), b"data").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("outside.bin"), b"data").unwrap();
    let upload_args = |path: &Path| {
        json!({
            "host": "web.example.test", "username": "deploy",
            "localPath": path.to_str().unwrap(), "remotePath": "/tmp/x",
        })
    };
    // 默认允许根（系统临时目录）放行两个文件 → 错误是拨号失败而非根拒绝。
    let error = st
        .call_tool(
            "sftp_upload",
            &upload_args(&outside.path().join("outside.bin")),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("allowed transfer roots") && !error.contains("localTransferRoot"),
        "default roots must allow temp files, got: {error}"
    );
    // 收窄到 root 后：根外文件在拨号前被拒。
    st.settings_set(&json!({ "localTransferRoot": root.path().to_str().unwrap() }))
        .unwrap();
    let error = st
        .call_tool(
            "sftp_upload",
            &upload_args(&outside.path().join("outside.bin")),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("localTransferRoot"),
        "narrowed root must refuse outside paths, got: {error}"
    );
    // 根内文件过根闸（错误回到拨号层）。
    let error = st
        .call_tool(
            "sftp_upload",
            &upload_args(&root.path().join("inside.bin")),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("localTransferRoot") && !error.contains("allowed transfer roots"),
        "in-root file must pass the root gate, got: {error}"
    );

    // (b) maxUploadBytes：设置即时生效，超限文件在拨号前被拒。
    let st = state();
    let big = tempfile::tempdir().unwrap();
    std::fs::write(big.path().join("big.bin"), vec![0u8; 64]).unwrap();
    st.settings_set(&json!({ "maxUploadBytes": 32 })).unwrap();
    let error = st
        .call_tool(
            "sftp_upload",
            &json!({
                "host": "web.example.test", "username": "deploy",
                "localPath": big.path().join("big.bin").to_str().unwrap(),
                "remotePath": "/tmp/x",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("exceeds the MCP upload limit"),
        "upload limit must apply immediately, got: {error}"
    );

    // (c) execPermissionMode=confirm：stdio 无审批通道 → 立即 fail-closed；
    // 切回 autonomous 后调用恢复（落到拨号错误）。
    let st = state();
    st.settings_set(&json!({ "execPermissionMode": "confirm" }))
        .unwrap();
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({ "host": "web.example.test", "username": "deploy", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("execPermissionMode=confirm"),
        "confirm mode must fail closed without an approval channel, got: {error}"
    );
    st.settings_set(&json!({ "execPermissionMode": "autonomous" }))
        .unwrap();
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({ "host": "web.example.test", "username": "deploy", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("execPermissionMode"),
        "switching back to autonomous must lift the gate, got: {error}"
    );

    // (d) connectionScope：非空即拒绝内联拨打（fail-closed），list 工具
    // 仍可用；清空后恢复。settings_get 回显生效值与 persisted 值。
    let st = state();
    st.settings_set(&json!({ "connectionScope": ["only-this"] }))
        .unwrap();
    let view = st.settings_get();
    assert_eq!(view["connectionScope"], json!(["only-this"]));
    assert_eq!(view["persistedConnectionScope"], json!(["only-this"]));
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({ "host": "web.example.test", "username": "deploy", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("connectionScope"), "{error}");
    let listed = st.call_tool("ssh_list_connections", &json!({}), None).await;
    assert!(listed.is_ok(), "list stays answerable under a scope");
    st.settings_set(&json!({ "connectionScope": [] })).unwrap();
    let error = st
        .call_tool(
            "ssh_exec",
            &json!({ "host": "web.example.test", "username": "deploy", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("connectionScope"),
        "clearing the scope must lift the gate, got: {error}"
    );
}
