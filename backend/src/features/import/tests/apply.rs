use super::*;

#[test]
fn securecrt_parses_sessions_groups_hex_ports_and_defaults() {
    let sessions = parse_securecrt(SECURECRT_EXPORT).unwrap();
    assert_eq!(sessions.len(), 3);
    let web = &sessions[0];
    assert_eq!(web.name, "web01");
    assert_eq!(web.group_path, vec!["Prod"]);
    assert_eq!(web.host, "192.168.1.10");
    // 000008ae is SecureCRT's zero-padded hex dword shape for 2222.
    assert_eq!(web.port, 2222);
    assert_eq!(web.username, "deploy");
    // The export only carries encrypted password blobs: password
    // semantics with no material plus the encrypted note.
    assert_eq!(web.auth, ImportedAuth::Password { has_secret: false });
    assert_eq!(web.secret_note, SECRET_NOTE_ENCRYPTED);
    // Decimal fallback for hand-written dword values.
    assert_eq!(sessions[1].port, 34);
    // Defaults: no port → 22, no user → root, root-level key → no group.
    assert_eq!(sessions[2].name, "root-level");
    assert_eq!(sessions[2].port, 22);
    assert_eq!(sessions[2].username, "root");
    assert!(sessions[2].group_path.is_empty());
    // Non-SSH2 protocols and host-less keys are skipped, and the
    // Default key outside Sessions never becomes a session.
}

#[test]
fn securecrt_dword_ports_prefer_hex_then_decimal() {
    assert_eq!(parse_securecrt_dword("00000016"), Some(22));
    assert_eq!(parse_securecrt_dword("000008ae"), Some(2222));
    assert_eq!(parse_securecrt_dword("0000FFFF"), Some(65535));
    // Not the 8-digit shape → decimal.
    assert_eq!(parse_securecrt_dword("34"), Some(34));
    assert_eq!(parse_securecrt_dword("22"), Some(22));
    // Junk and the zero port degrade to None (caller defaults to 22).
    assert_eq!(parse_securecrt_dword("junk"), None);
    assert_eq!(parse_securecrt_dword("00000000"), None);
    assert_eq!(parse_securecrt_dword(""), None);
}

#[test]
fn securecrt_deeply_nested_keys_fail_closed() {
    // A1 回归：恶意深嵌套（500 层）必须快速拒绝，而不是逐闭合标签克隆
    // 整个祖先栈（O(N²) CPU 放大）。
    let mut text = String::from("<VanDyke version=\"9.0\"><key name=\"Sessions\">");
    for depth in 0..500 {
        text.push_str(&format!("<key name=\"folder{depth}\">"));
    }
    for _ in 0..500 {
        text.push_str("</key>");
    }
    text.push_str("</key></VanDyke>");
    let error = parse_securecrt(&text).expect_err("deep nesting must be rejected");
    assert!(error.contains("nesting"), "{error}");
    // 真实导出的嵌套深度（Sessions > 文件夹 > 会话）不受影响。
    let sessions = parse_securecrt(SECURECRT_EXPORT).unwrap();
    assert_eq!(sessions.len(), 3);
}

#[test]
fn imported_auth_debug_has_no_credential_fields() {
    let password = ImportedAuth::Password { has_secret: true };
    let key = ImportedAuth::PrivateKey {
        path: Some("/home/u/id_ed25519".to_string()),
        has_secret: true,
    };
    let rendered = format!("{password:?} {key:?}");
    assert!(rendered.contains("has_secret"), "{rendered}");
    assert!(rendered.contains("/home/u/id_ed25519"), "{rendered}");
    assert!(!rendered.contains("value"), "{rendered}");
    assert!(!rendered.contains("content"), "{rendered}");
    assert!(!rendered.contains("passphrase"), "{rendered}");
}

#[test]
fn securecrt_entity_and_attribute_decoding() {
    let xml = concat!(
        "<VanDyke><key name=\"Sessions\"><key name=\"gr&amp;oup\"><key name=\"sess\">\n",
        "<string name=\"Protocol Name\">SSH2</string>\n",
        "<string name=\"Hostname\">h&#111;st&#x2F;range</string>\n",
        "<string name=\"Username\">u&quot;q</string>\n",
        "</key></key></key></VanDyke>",
    );
    let sessions = parse_securecrt(xml).unwrap();
    assert_eq!(sessions.len(), 1);
    // Frame names and field values decode named and numeric entities.
    assert_eq!(sessions[0].name, "sess");
    assert_eq!(sessions[0].host, "host/range");
    assert_eq!(sessions[0].username, "u\"q");
    assert_eq!(sessions[0].group_path, vec!["gr&oup"]);
}

#[test]
fn securecrt_malformed_xml_fails_closed() {
    // Unterminated tag / comment / closing tag.
    assert!(parse_securecrt("<key name=\"Sessions\"").is_err());
    assert!(parse_securecrt("<!-- never closed").is_err());
    assert!(parse_securecrt("</key").is_err());
    // Unclosed <key> frames.
    assert!(parse_securecrt("<key name=\"Sessions\"><key name=\"a\">").is_err());
    // Unbalanced </key>.
    assert!(parse_securecrt("</key>").is_err());
    // Value element closed by a different tag.
    assert!(parse_securecrt(
        "<key name=\"Sessions\"><key name=\"a\"><string name=\"Hostname\">h</key></key>"
    )
    .is_err());
    // Unquoted attribute value.
    assert!(parse_securecrt("<key name=Sessions></key>").is_err());
    // Unknown / unterminated entity.
    assert!(parse_securecrt(
        "<key name=\"Sessions\"><key name=\"a\"><string name=\"Hostname\">a&b;</string></key></key>"
    )
    .is_err());
    // Structurally valid input without any <key> tree parses to zero
    // sessions instead of failing (bad content ≠ bad structure).
    assert!(parse_securecrt("").unwrap().is_empty());
    assert!(parse_securecrt("plain text, no xml").unwrap().is_empty());
}

// -- FinalShell ---------------------------------------------------------

#[test]
fn finalshell_zip_parses_folders_sessions_and_skips() {
    let folder1 = "{\"id\":\"f1\",\"name\":\"Prod\",\"parent_id\":\"root\"}";
    let folder2 = "{\"id\":\"f2\",\"name\":\"Web\",\"parent_id\":\"f1\"}";
    let zip = xshell_zip(&[
        ("conn/folder.json", folder1),
        ("conn/sub/folder.json", folder2),
        (
            "conn/web_connect_config.json",
            &finalshell_config_json("web1", "10.1.0.1", ""),
        ),
        // Defaults: no port/user/name → 22 / root / host.
        (
            "conn/db_connect_config.json",
            "{\"host\":\"10.1.0.2\",\"conection_type\":100}",
        ),
        // Deleted, non-SSH and junk entries are skipped.
        (
            "conn/deleted_connect_config.json",
            "{\"host\":\"10.1.0.3\",\"conection_type\":100,\"delete_time\":5}",
        ),
        (
            "conn/rdp_connect_config.json",
            "{\"host\":\"10.1.0.4\",\"conection_type\":200}",
        ),
        ("conn/readme.txt", "ignore me"),
    ]);
    let sessions = parse_finalshell(&zip).unwrap();
    assert_eq!(sessions.len(), 2);
    let web = &sessions[0];
    assert_eq!(web.name, "web1");
    assert_eq!(web.host, "10.1.0.1");
    assert_eq!(web.port, 2222);
    assert_eq!(web.username, "deploy");
    // Nested folder.json files chain into a group path.
    assert_eq!(web.group_path, vec!["Prod", "Web"]);
    assert_eq!(web.description, "primary");
    // FinalShell passwords stay encrypted in the source: no material,
    // flagged in the preview.
    assert_eq!(web.auth, ImportedAuth::Password { has_secret: false });
    assert_eq!(web.secret_note, SECRET_NOTE_ENCRYPTED);
    assert_eq!(sessions[1].name, "10.1.0.2");
    assert_eq!(sessions[1].port, 22);
    assert_eq!(sessions[1].username, "root");
    assert!(sessions[1].group_path.is_empty());
}

#[test]
fn finalshell_bare_json_fallback_and_fail_closed() {
    // A single connection object and an array both work without a ZIP.
    let single = finalshell_config_json("solo", "10.1.1.1", "");
    let sessions = parse_finalshell(single.as_bytes()).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].host, "10.1.1.1");
    let array = format!("[{single},{{\"host\":\"10.1.1.2\",\"conection_type\":100}}]");
    let sessions = parse_finalshell(array.as_bytes()).unwrap();
    assert_eq!(sessions.len(), 2);
    // Neither ZIP nor JSON → readable failure.
    let error = parse_finalshell(b"definitely not a zip and not json").unwrap_err();
    assert!(error.contains("neither a ZIP"), "unexpected error: {error}");
    // JSON scalar is rejected.
    assert!(parse_finalshell(b"42").is_err());
    // ZIP with folders but no connections fails.
    let zip = xshell_zip(&[("conn/folder.json", "{\"id\":\"f1\",\"name\":\"Prod\"}")]);
    let error = parse_finalshell(&zip).unwrap_err();
    assert!(
        error.contains("*_connect_config.json"),
        "unexpected error: {error}"
    );
    // Malformed entries inside a ZIP degrade to skipped, not fatal.
    let zip = xshell_zip(&[
        ("conn/bad_connect_config.json", "{not json"),
        (
            "conn/good_connect_config.json",
            "{\"host\":\"10.1.2.1\",\"conection_type\":100}",
        ),
        ("conn/missing_id_folder.json", "{\"name\":\"broken\"}"),
    ]);
    let sessions = parse_finalshell(&zip).unwrap();
    assert_eq!(sessions.len(), 1);
}

#[test]
fn finalshell_group_chain_breaks_on_cycles() {
    let folder_a = "{\"id\":\"f1\",\"name\":\"A\",\"parent_id\":\"f2\"}";
    let folder_b = "{\"id\":\"f2\",\"name\":\"B\",\"parent_id\":\"f1\"}";
    let zip = xshell_zip(&[
        ("conn/folder.json", folder_a),
        ("conn/sub/folder.json", folder_b),
        (
            "conn/c_connect_config.json",
            "{\"host\":\"10.1.3.1\",\"conection_type\":100,\"parent_id\":\"f1\"}",
        ),
    ]);
    let sessions = parse_finalshell(&zip).unwrap();
    assert_eq!(sessions.len(), 1);
    // The loop bound stops the f1→f2→f1 cycle after one pass each.
    assert_eq!(sessions[0].group_path, vec!["B", "A"]);
    // "root" and "0" parent ids mean the top level.
    let zip = xshell_zip(&[
        ("conn/folder.json", folder_a),
        ("conn/sub/folder.json", folder_b),
        (
            "conn/r_connect_config.json",
            "{\"host\":\"10.1.3.2\",\"conection_type\":100,\"parent_id\":\"root\"}",
        ),
    ]);
    let sessions = parse_finalshell(&zip).unwrap();
    assert!(sessions[0].group_path.is_empty());
}

// -- Electerm -----------------------------------------------------------

#[test]
fn electerm_array_bookmarks_with_defaults() {
    let json =
        format!("[{ELECTERM_BOOKMARK},{{\"id\":\"bm2\",\"host\":\"10.2.0.2\",\"type\":\"SSH\"}}]");
    let sessions = parse_electerm(json.as_bytes()).unwrap();
    assert_eq!(sessions.len(), 2);
    let web = &sessions[0];
    assert_eq!(web.name, "web");
    assert_eq!(web.host, "10.2.0.1");
    assert_eq!(web.port, 2222);
    assert_eq!(web.username, "deploy");
    // Electerm bookmarks carry no credential material: password bookmarks
    // keep the kind with the not-carried note.
    assert_eq!(web.auth, ImportedAuth::Password { has_secret: false });
    assert_eq!(web.secret_note, SECRET_NOTE_NOT_CARRIED);
    assert!(web.group_path.is_empty());
    // Defaults: no port → 22, no user → root, no title → host, no
    // auth_type → no auth at all.
    assert_eq!(sessions[1].name, "10.2.0.2");
    assert_eq!(sessions[1].port, 22);
    assert_eq!(sessions[1].username, "root");
    assert_eq!(sessions[1].auth, ImportedAuth::None);
    assert_eq!(sessions[1].secret_note, "");
}

#[test]
fn electerm_object_form_resolves_group_paths() {
    let json = json!({
        "bookmarks": [
            { "id": "bm1", "title": "edge", "host": "10.2.1.1", "type": "ssh", "port": 22 },
            { "id": "bm2", "title": "hidden", "host": "10.2.1.2", "type": "ssh", "enable_ssh": false },
            { "id": "bm3", "title": "serial", "host": "COM3", "type": "serial" },
            { "id": "bm4", "title": "badport", "host": "10.2.1.4", "type": "ssh", "port": 70000 },
        ],
        "bookmarkGroups": [
            { "id": "g1", "title": "Prod", "bookmarkIds": [], "bookmarkGroupIds": ["g2"] },
            { "id": "g2", "title": "Web", "bookmarkIds": ["bm1"], "bookmarkGroupIds": [] },
        ],
    })
    .to_string();
    let sessions = parse_electerm(json.as_bytes()).unwrap();
    // hidden (enable_ssh=false), serial (type) and badport (out-of-range
    // port) are skipped.
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, "edge");
    // bm1 lives in the child group (Web), whose parent is Prod: the path
    // walks bookmarkGroupIds parent chains bottom-up.
    assert_eq!(sessions[0].group_path, vec!["Prod", "Web"]);
    assert_eq!(sessions[0].auth, ImportedAuth::None);
}

#[test]
fn electerm_group_cycles_stop_and_malformed_fails_closed() {
    let json = json!({
        "bookmarks": [{ "id": "bm1", "host": "10.2.2.1", "type": "ssh" }],
        "bookmarkGroups": [
            { "id": "g1", "title": "A", "bookmarkIds": ["bm1"], "bookmarkGroupIds": ["g2"] },
            { "id": "g2", "title": "B", "bookmarkIds": [], "bookmarkGroupIds": ["g1"] },
        ],
    })
    .to_string();
    let sessions = parse_electerm(json.as_bytes()).unwrap();
    assert_eq!(sessions[0].group_path, vec!["B", "A"]);

    assert!(parse_electerm(b"not json").is_err());
    // An object without a bookmarks array and a scalar both fail closed.
    assert!(parse_electerm(b"{\"bookmarkGroups\":[]}").is_err());
    assert!(parse_electerm(b"42").is_err());
    // Broken entries inside the array are skipped instead of fatal.
    let json = json!([
        { "host": "10.2.2.2", "type": "ssh" },          // no id → ok, id is optional
        { "id": "x", "type": "sftp", "host": "10.2.2.3" }, // wrong type
        "junk",
    ])
    .to_string();
    let sessions = parse_electerm(json.as_bytes()).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].host, "10.2.2.2");
}

// -- Termius ------------------------------------------------------------

#[test]
fn termius_export_maps_hosts_configs_identities_groups() {
    let sessions = parse_termius(TERMIUS_EXPORT.as_bytes()).unwrap();
    assert_eq!(sessions.len(), 2);
    let web = &sessions[0];
    assert_eq!(web.name, "web");
    assert_eq!(web.host, "10.3.0.1");
    // Port and identity resolve through the ssh_config reference.
    assert_eq!(web.port, 2222);
    assert_eq!(web.username, "deploy");
    // Plaintext identity password becomes metadata only, no note.
    assert_eq!(web.auth, ImportedAuth::Password { has_secret: true });
    assert_eq!(web.secret_note, "");
    assert_eq!(web.group_path, vec!["Prod", "Web"]);
    // Defaults: no label → host, no group/config → 22 / root / no group.
    assert_eq!(sessions[1].name, "10.3.0.2");
    assert_eq!(sessions[1].port, 22);
    assert_eq!(sessions[1].username, "root");
    assert!(sessions[1].group_path.is_empty());
}

#[test]
fn termius_encrypted_blobs_are_dropped_and_pem_keys_are_carried() {
    // Identity i2 holds an encrypted sync blob for a password.
    let hosts_with_identity = TERMIUS_EXPORT.replace(
        "\"id\":\"h2\",\"address\":\"10.3.0.2\"",
        "\"id\":\"h2\",\"address\":\"10.3.0.2\",\"identity\":{\"id\":\"i2\"},\
         \"ssh_key\":{\"id\":\"k1\"}",
    );
    let sessions = parse_termius(hosts_with_identity.as_bytes()).unwrap();
    let encrypted = &sessions[1];
    // The blob is never decoded: password semantics without material
    // plus the encrypted note.
    assert_eq!(encrypted.username, "ops");
    assert_eq!(encrypted.auth, ImportedAuth::Password { has_secret: false });
    assert_eq!(encrypted.secret_note, SECRET_NOTE_ENCRYPTED);

    // An identity with an ssh_key reference reports key-auth metadata only;
    // PEM content and its passphrase never enter the preview model.
    let key_only = TERMIUS_EXPORT
        .replace(
            "\"id\":\"i2\",\"username\":\"ops\",\"password\":\"BAAAAAAA0123456789012345678901234567890123456789\"",
            "\"id\":\"i2\",\"username\":\"ops\",\"password\":\"\",\"ssh_key\":{\"id\":\"k1\"}",
        )
        .replace(
            "\"id\":\"h2\",\"address\":\"10.3.0.2\"",
            "\"id\":\"h2\",\"address\":\"10.3.0.2\",\"identity\":{\"id\":\"i2\"}",
        );
    let sessions = parse_termius(key_only.as_bytes()).unwrap();
    match &sessions[1].auth {
        ImportedAuth::PrivateKey { path, has_secret } => {
            assert!(path.is_none());
            assert!(*has_secret);
        }
        other => panic!("expected private key auth, got {other:?}"),
    }
    assert_eq!(sessions[1].secret_note, "");
}

#[test]
fn termius_malformed_input_fails_closed() {
    assert!(parse_termius(b"not json").is_err());
    assert!(parse_termius(b"{\"data\":{}}").is_err());
    assert!(parse_termius(b"{\"hosts\":\"nope\"}").is_err());
    assert!(parse_termius(b"[]").is_err());
    // Broken host entries are skipped, not fatal.
    let hosts = json!({
        "hosts": [
            { "id": "h1", "address": "" },
            { "id": "h2", "address": "10.3.1.2" },
            "junk",
        ]
    })
    .to_string();
    let sessions = parse_termius(hosts.as_bytes()).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].host, "10.3.1.2");
}

#[test]
fn termius_helpers_edge_cases() {
    // The blob heuristic matches the NyaTerm importer's shape check.
    assert!(is_termius_encrypted_blob(
        "BAAAAAAA0123456789012345678901234567890123456789"
    ));
    assert!(!is_termius_encrypted_blob("BA"));
    assert!(!is_termius_encrypted_blob(
        "BAAAAAAA012345678901234567890123456789012345678!"
    ));
    assert!(!is_termius_encrypted_blob("plain password"));
}

// -- OpenSSH config ------------------------------------------------------

#[test]
fn parsed_preview_auth_never_retains_plaintext_credentials() {
    let sessions_json = json!([{
        "session": {
            "protocol": "SSH",
            "target": "root@10.2.0.1",
            "autoLogin": json!({ "PasswordEnabled": true, "Password": "do-not-retain" }).to_string(),
        }
    }])
    .to_string();
    let sessions = parse_windterm(sessions_json.as_bytes(), None, None).unwrap();
    let rendered = format!("{:?}", sessions[0]);
    assert!(!rendered.contains("do-not-retain"), "{rendered}");
    let exported = normalized_export("windterm", &sessions, &[0])
        .unwrap()
        .to_string();
    assert!(!exported.contains("do-not-retain"), "{exported}");
}

#[test]
fn normalized_export_never_contains_password_or_key_material() {
    let sessions = sample_sessions();
    let exported = normalized_export("windterm", &sessions, &[0, 1]).unwrap();
    let rendered = exported.to_string();
    assert!(rendered.contains("schemaVersion"));
    assert!(rendered.contains("/home/u/key"));
    assert!(rendered.contains("\"hasSecret\":true"));
    assert!(rendered.contains("\"keyPath\":\"/home/u/key\""));
    assert!(!rendered.contains("\"password\":"));
    assert!(!rendered.contains("\"privateKey\":"));
    assert!(!rendered.contains("\"passphrase\":"));
    assert_eq!(exported["sessions"][0]["auth"]["kind"], "password");
    assert_eq!(exported["sessions"][0]["auth"]["hasSecret"], true);
    assert!(exported["sessions"][0]["auth"].get("password").is_none());
}

// -- Streaming preview protocol -----------------------------------------

#[test]
fn sniff_kind_matches_every_wizard_source_extension() {
    for (file_name, want) in [
        ("sessions.mxtsessions", "moba"),
        ("Sessions.MXTSESSIONS", "moba"),
        ("xshell-backup.xts", "xshell"),
        ("folders.sessions", "windterm"),
        ("export.xml", "securecrt"),
        ("connect.zip", "finalshell"),
        ("config", "sshconfig"),
        ("weird.unknown", "sshconfig"),
    ] {
        assert_eq!(sniff_kind(file_name, b""), want, "{file_name}");
    }
}

#[test]
fn sniff_kind_splits_json_sources_by_top_level_shape() {
    assert_eq!(
        sniff_kind("t.json", br#"{"data": {"hosts": [{"label": "a"}]}}"#),
        "termius"
    );
    assert_eq!(sniff_kind("t.json", br#"{"hosts": []}"#), "termius");
    assert_eq!(
        sniff_kind("t.json", br#"[{"host": "1.2.3.4"}]"#),
        "electerm"
    );
    assert_eq!(sniff_kind("t.json", br#"{"bookmarks": []}"#), "electerm");
    // 坏 JSON 也归 Electerm，让解析器报出与向导一致的原文错误。
    assert_eq!(sniff_kind("t.json", b"{not json"), "electerm");
}

#[test]
fn session_field_values_maps_credentials_by_path_only() {
    let values = session_field_values(&sample_sessions()[0]);
    assert_eq!(values["display_name"], "web");
    assert_eq!(values["host"], "10.0.0.1");
    assert_eq!(values["port"], 22);
    assert_eq!(values["username"], "root");
    assert_eq!(values["authentication"], "password");
    assert!(
        values.get("password").is_none(),
        "password must never be filled"
    );
    assert!(values.get("private_key_path").is_none());

    let values = session_field_values(&sample_sessions()[1]);
    assert_eq!(values["authentication"], "private-key");
    assert_eq!(values["private_key_path"], "/home/u/key");
    assert!(
        values.get("private_key").is_none(),
        "key content must never be filled"
    );

    let none = ImportedSession {
        auth: ImportedAuth::None,
        ..sample_sessions()[0].clone()
    };
    assert_eq!(session_field_values(&none)["authentication"], "none");
}

#[test]
fn form_action_reuses_parsers_for_first_session_mapping() {
    // 真实 MobaXterm 文本走 parse_uploaded 全管线（与向导同一入口），
    // 首条会话的 fieldValues 与手写映射一致。
    let text = concat!(
        "[Bookmarks]\n",
        "SubRep=Prod\\Web\n",
        "web1=#109#0%192.168.1.10%22%deploy%pw%...\n",
        "db1=#109#0%10.0.0.5%%%pw%...\n",
    );
    let sessions = parse_uploaded("moba", text.as_bytes(), None, None).unwrap();
    assert_eq!(sessions.len(), 2);
    let values = session_field_values(&sessions[0]);
    assert_eq!(values["display_name"], "web1");
    assert_eq!(values["host"], "192.168.1.10");
    assert_eq!(values["username"], "deploy");
    assert_eq!(values["authentication"], "none");
}

#[test]
fn read_session_file_reports_sniffed_kind_and_bytes() {
    let dir = std::env::temp_dir().join(format!("dbx-import-action-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bookmarks.mxtsessions");
    std::fs::write(&path, "[Bookmarks]\n").unwrap();
    let (kind, bytes) = read_session_file(&path).unwrap();
    assert_eq!(kind, "moba");
    assert_eq!(bytes, b"[Bookmarks]\n");
    let missing = read_session_file(&dir.join("absent.sessions"));
    assert!(missing.is_err(), "missing file must be a readable error");
    std::fs::remove_dir_all(&dir).unwrap();
}
