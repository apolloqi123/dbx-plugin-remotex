use super::*;

// -- MobaXterm ----------------------------------------------------------

#[test]
fn moba_parses_ssh_sessions_with_group_and_defaults() {
    let text = concat!(
        "[Bookmarks]\n",
        "SubRep=Prod\\Web\n",
        "ImgNum=42\n",
        "web1=#109#0%192.168.1.10%22%deploy%pw%...\n",
        "db1=#109#0%10.0.0.5%%%pw%...\n",
        "custom=#109#0%10.0.0.9%2222%ops%pw%...\n",
    );
    let sessions = parse_moba_ini(text).unwrap();
    assert_eq!(sessions.len(), 3);
    assert_eq!(sessions[0].name, "web1");
    assert_eq!(sessions[0].group_path, vec!["Prod", "Web"]);
    assert_eq!(sessions[0].host, "192.168.1.10");
    assert_eq!(sessions[0].port, 22);
    assert_eq!(sessions[0].username, "deploy");
    assert_eq!(sessions[0].auth, ImportedAuth::None);
    // Missing port and user fall back to 22 / root.
    assert_eq!(sessions[1].port, 22);
    assert_eq!(sessions[1].username, "root");
    assert_eq!(sessions[2].port, 2222);
}

#[test]
fn moba_skips_non_ssh_entries_and_other_sections() {
    let text = concat!(
        "[Bookmarks]\n",
        "shell=#1#0%192.168.1.10%22%root%\n",
        "telnet=#99#0%192.168.1.11%23%root%\n",
        "ssh=#109#0%192.168.1.12%22%root%\n",
        "[Bookmarks_2]\n",
        "in-other=#109#0%192.168.1.13%22%root%\n",
        "[Servers]\n",
        "wrong-section=#109#0%192.168.1.14%22%root%\n",
    );
    let sessions = parse_moba_ini(text).unwrap();
    let names: Vec<&str> = sessions.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["ssh", "in-other"]);
}

#[test]
fn moba_empty_sub_rep_means_no_group_and_bad_values_are_skipped() {
    let text = concat!(
        "[Bookmarks]\n",
        "SubRep=\n",
        "ok=#109#0%1.2.3.4%22%root%\n",
        "nohost=#109#0%%22%root%\n",
        "badport=#109#0%1.2.3.5%not-a-port%root%\n",
        "novalue=#109#0\n",
    );
    let sessions = parse_moba_ini(text).unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(sessions[0].group_path.is_empty());
    // Unparsable port falls back to 22.
    assert_eq!(sessions[1].port, 22);
}

#[test]
fn moba_tolerates_garbage_comments_and_missing_sections() {
    assert!(parse_moba_ini("").unwrap().is_empty());
    assert!(parse_moba_ini("not an ini at all\n;;;;\n")
        .unwrap()
        .is_empty());
    assert!(parse_moba_ini("[Bookmarks\nx=#109#0%1.2.3.4%22%root%\n")
        .unwrap()
        .is_empty());
    let text = "; comment\n# comment\n[Bookmarks]\n;another\nssh=#109#0%1.2.3.4%22%root%\n";
    let sessions = parse_moba_ini(text).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].host, "1.2.3.4");
}

// -- Xshell -------------------------------------------------------------

/// Builds an in-memory ZIP archive with the given entries.
fn xshell_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }
    buf.into_inner()
}

const WEB_XSH: &str = concat!(
    "[CONNECTION]\n",
    "Protocol=SSH\n",
    "Host=192.168.1.20\n",
    "Port=2222\n",
    "\n",
    "[CONNECTION:AUTHENTICATION]\n",
    "UserName=deploy\n",
    "UserKey=my-key\n",
    "\n",
    "[SCRIPT]\n",
    "something=else\n",
);

#[test]
fn xshell_parses_sessions_groups_and_key_auth() {
    let zip = xshell_zip(&[
        ("Xshell/Sessions/Prod/web.xsh", WEB_XSH),
        (
            "Xshell/Sessions/db.xsh",
            "[CONNECTION]\nProtocol=SSH\nHost=10.0.0.2\n",
        ),
        ("notes.txt", "ignore me"),
    ]);
    let sessions = parse_xshell(&zip).unwrap();
    assert_eq!(sessions.len(), 2);
    let web = &sessions[0];
    assert_eq!(web.name, "web");
    assert_eq!(web.host, "192.168.1.20");
    assert_eq!(web.port, 2222);
    assert_eq!(web.username, "deploy");
    assert_eq!(web.group_path, vec!["Prod"]);
    assert_eq!(
        web.auth,
        ImportedAuth::PrivateKey {
            path: Some("my-key".to_string()),
            has_secret: false,
        }
    );
    // Defaults: port 22, user root, no group at the Sessions root.
    assert_eq!(sessions[1].name, "db");
    assert_eq!(sessions[1].port, 22);
    assert_eq!(sessions[1].username, "root");
    assert!(sessions[1].group_path.is_empty());
    assert_eq!(sessions[1].auth, ImportedAuth::None);
}

#[test]
fn xshell_skips_non_ssh_and_non_xsh_entries() {
    let zip = xshell_zip(&[
        (
            "Xshell/Sessions/serial.xsh",
            "[CONNECTION]\nProtocol=SERIAL\nHost=COM3\n",
        ),
        (
            "Xshell/Sessions/telnet.xsh",
            "[CONNECTION]\nProtocol=Telnet\nHost=10.0.0.3\nPort=23\n",
        ),
        ("Xshell/Sessions/ssh.xsh", WEB_XSH),
        ("Xshell/backup/old.xsh.bak", "junk"),
    ]);
    let sessions = parse_xshell(&zip).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, "ssh");
}

#[test]
fn xshell_malformed_xsh_entries_are_skipped_not_fatal() {
    let zip = xshell_zip(&[
        ("garbage.xsh", "this is not an ini at all"),
        ("no-host.xsh", "[CONNECTION]\nProtocol=SSH\n"),
        ("good.xsh", WEB_XSH),
    ]);
    let sessions = parse_xshell(&zip).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, "good");
}

#[test]
fn xshell_rejects_invalid_archives_and_enforces_entry_cap() {
    assert!(parse_xshell(b"definitely not a zip").is_err());
    // One entry over the cap fails the whole archive.
    let mut entries = Vec::new();
    for index in 0..=MAX_ZIP_ENTRIES {
        entries.push((format!("s{index}.xsh"), String::from("[CONNECTION]\n")));
    }
    let refs: Vec<(&str, &str)> = entries
        .iter()
        .map(|(n, c)| (n.as_str(), c.as_str()))
        .collect();
    let zip = xshell_zip(&refs);
    let error = parse_xshell(&zip).unwrap_err();
    assert!(error.contains("more than"), "unexpected error: {error}");
    // Just under the cap parses fine.
    let refs: Vec<(&str, &str)> = entries
        .iter()
        .take(MAX_ZIP_ENTRIES)
        .map(|(n, c)| (n.as_str(), c.as_str()))
        .collect();
    assert!(parse_xshell(&xshell_zip(&refs)).is_ok());
}

#[test]
fn zip_entry_names_decode_utf8_first_then_gbk() {
    // ASCII and UTF-8 names decode verbatim.
    assert_eq!(decode_zip_name(b"web.xsh"), "web.xsh");
    assert_eq!(decode_zip_name("生产环境.xsh".as_bytes()), "生产环境.xsh");
    // GBK bytes for "生产环境.xsh" (Chinese Windows exports) are not
    // valid UTF-8, so the GBK fallback applies.
    let (gbk, _, had_errors) = encoding_rs::GBK.encode("生产环境.xsh");
    assert!(!had_errors);
    let mut raw = b"Xshell/Sessions/".to_vec();
    raw.extend_from_slice(&gbk);
    let decoded = decode_zip_name(&raw);
    assert_eq!(decoded, "Xshell/Sessions/生产环境.xsh");
    // Undecodable bytes degrade lossily instead of panicking.
    assert_eq!(decode_zip_name(&[0xff, 0xfe]), "\u{fffd}\u{fffd}");
}

// -- WindTerm -----------------------------------------------------------

const FINGERPRINT_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";

/// Test-only helper: encrypts like WindTerm does (AES-256-CBC/PKCS7,
/// key/IV derived via PBKDF2-HMAC-SHA3-512), so the parser round-trips
/// against an independent encryption path.
fn windterm_encrypt(master: &str, fingerprint_hex: &str, plaintext: &str) -> String {
    use aes::cipher::block_padding::Pkcs7;
    use aes::cipher::{BlockModeEncrypt, KeyIvInit};
    type Aes256CbcEncryptor = cbc::Encryptor<aes::Aes256>;
    let salt = fingerprint_bytes(fingerprint_hex);
    let (key, iv) = derive_windterm_key(master, &salt);
    let ciphertext = Aes256CbcEncryptor::new_from_slices(&key, &iv)
        .unwrap()
        .encrypt_padded_vec::<Pkcs7>(plaintext.as_bytes());
    BASE64.encode(ciphertext)
}

fn user_config_json(master_password: bool) -> String {
    json!({
        "application": {
            "fingerprint": FINGERPRINT_HEX,
            "masterPassword": master_password,
        }
    })
    .to_string()
}

#[test]
fn windterm_parses_ssh_sessions_with_defaults_and_groups() {
    let sessions_json = json!([
        {
            "session": {
                "protocol": "SSH",
                "target": "ops@10.1.1.5",
                "label": "gateway",
                "port": 2200,
                "group": "Cloud>AWS>us-east",
                "description": "primary",
            }
        },
        {
            "session": {
                "protocol": "ssh",
                "target": "10.1.1.6",
            }
        },
        {
            "session": { "protocol": "Telnet", "target": "10.1.1.7" }
        },
        { "note": "no session object" },
        {
            "session": { "protocol": "SSH", "target": "@10.1.1.8" }
        },
        {
            "session": { "protocol": "SSH", "target": "10.1.1.9:22", "label": "x" },
        }
    ])
    .to_string();
    let sessions = parse_windterm(sessions_json.as_bytes(), None, None).unwrap();
    assert_eq!(sessions.len(), 4);
    assert_eq!(sessions[0].name, "gateway");
    assert_eq!(sessions[0].host, "10.1.1.5");
    assert_eq!(sessions[0].port, 2200);
    assert_eq!(sessions[0].username, "ops");
    assert_eq!(sessions[0].group_path, vec!["Cloud", "AWS", "us-east"]);
    assert_eq!(sessions[0].description, "primary");
    assert_eq!(sessions[0].auth, ImportedAuth::None);
    // Defaults: no @ → root, no port → 22, no label → host.
    assert_eq!(sessions[1].username, "root");
    assert_eq!(sessions[1].port, 22);
    assert_eq!(sessions[1].name, "10.1.1.6");
    assert!(sessions[1].group_path.is_empty());
    assert_eq!(sessions[3].username, "root");
}

#[test]
fn windterm_plaintext_autologin_yields_password_auth() {
    let auto_login =
        json!({ "PasswordEnabled": true, "Password": "p@ss", "Public Key": {} }).to_string();
    let sessions_json = json!([{
        "session": {
            "protocol": "SSH",
            "target": "root@10.2.0.1",
            "label": "plain",
            "autoLogin": auto_login,
        }
    }])
    .to_string();
    let sessions = parse_windterm(sessions_json.as_bytes(), None, None).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].auth,
        ImportedAuth::Password { has_secret: true }
    );
}

#[test]
fn windterm_encrypted_autologin_roundtrips_with_master_password() {
    let auto_login = json!({
        "PasswordEnabled": true,
        "Password": "enc-p@ss",
        "Public Key": {
            "windows": { "path": "$(HomeDir)keys/id_ed25519", "pass": "key-phr" }
        }
    })
    .to_string();
    let mut sessions_json = json!([{
        "session": {
            "protocol": "SSH",
            "target": "deploy@10.3.0.1",
            "label": "encrypted",
            "autoLogin": windterm_encrypt("master-key", FINGERPRINT_HEX, &auto_login),
        }
    }])
    .to_string();
    let config = user_config_json(true);
    let sessions = parse_windterm(
        sessions_json.as_bytes(),
        Some(config.as_bytes()),
        Some("master-key"),
    )
    .unwrap();
    assert_eq!(sessions.len(), 1);
    // Password wins over the public key entry.
    assert_eq!(
        sessions[0].auth,
        ImportedAuth::Password { has_secret: true }
    );

    // Without the PasswordEnabled flag the public key entry applies, with
    // the $(HomeDir) prefix expanded.
    let key_only = json!({
        "Public Key": {
            "windows": { "path": "~/keys/id_rsa", "pass": "key-phr" }
        }
    })
    .to_string();
    sessions_json = json!([{
        "session": {
            "protocol": "SSH",
            "target": "deploy@10.3.0.1",
            "label": "keyed",
            "autoLogin": windterm_encrypt("master-key", FINGERPRINT_HEX, &key_only),
        }
    }])
    .to_string();
    let sessions = parse_windterm(
        sessions_json.as_bytes(),
        Some(config.as_bytes()),
        Some("master-key"),
    )
    .unwrap();
    match &sessions[0].auth {
        ImportedAuth::PrivateKey { path, has_secret } => {
            let path = path.as_deref().unwrap();
            assert!(path.ends_with("keys/id_rsa"), "unexpected path: {path}");
            assert_ne!(path, "~/keys/id_rsa");
            assert!(*has_secret, "passphrase is represented only as metadata");
        }
        other => panic!("expected private key auth, got {other:?}"),
    }
}

#[test]
fn windterm_master_password_switch_requires_the_password() {
    let auto_login = json!({ "PasswordEnabled": true, "Password": "x" }).to_string();
    let sessions_json = json!([{
        "session": {
            "protocol": "SSH",
            "target": "root@10.3.0.2",
            "autoLogin": windterm_encrypt("master-key", FINGERPRINT_HEX, &auto_login),
        }
    }])
    .to_string();
    let config = user_config_json(true);
    let error =
        parse_windterm(sessions_json.as_bytes(), Some(config.as_bytes()), None).unwrap_err();
    assert_eq!(error, "WindTerm master password is required");
    // Switch off: no password needed, but the blob can't be decrypted —
    // credentials are skipped, the session survives.
    let config_off = user_config_json(false);
    let sessions =
        parse_windterm(sessions_json.as_bytes(), Some(config_off.as_bytes()), None).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].auth, ImportedAuth::None);
}

#[test]
fn windterm_malformed_input_degrades_gracefully() {
    assert!(parse_windterm(b"not json", None, None).is_err());
    assert!(parse_windterm(b"{\"not\":\"an array\"}", None, None).is_err());
    // autoLogin that is neither JSON nor base64, and ciphertext that does
    // not decrypt: credentials skipped, sessions kept.
    let sessions_json = json!([
        { "session": { "protocol": "SSH", "target": "a@10.4.0.1", "autoLogin": "%%%garbage%%%" } },
        { "session": { "protocol": "SSH", "target": "b@10.4.0.2", "autoLogin": "AAAA" } },
    ])
    .to_string();
    let config = user_config_json(false);
    let sessions = parse_windterm(sessions_json.as_bytes(), Some(config.as_bytes()), None).unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].auth, ImportedAuth::None);
    assert_eq!(sessions[1].auth, ImportedAuth::None);
}

// -- SecureCRT ----------------------------------------------------------

const SECURECRT_EXPORT: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<VanDyke version=\"9.0\">\n",
    "\t<key name=\"Default\">\n",
    "\t\t<string name=\"Protocol Name\">SSH2</string>\n",
    "\t\t<dword name=\"[SSH2] Port\">00000016</dword>\n",
    "\t</key>\n",
    "\t<key name=\"Sessions\">\n",
    "\t\t<key name=\"Prod\">\n",
    "\t\t\t<key name=\"web01\">\n",
    "\t\t\t\t<string name=\"Protocol Name\">SSH2</string>\n",
    "\t\t\t\t<string name=\"Hostname\">192.168.1.10</string>\n",
    "\t\t\t\t<dword name=\"[SSH2] Port\">000008ae</dword>\n",
    "\t\t\t\t<string name=\"Username\">deploy</string>\n",
    "\t\t\t\t<string name=\"Password V2\">uH9kJ2vQ EncryptedBlob</string>\n",
    "\t\t\t</key>\n",
    "\t\t\t<key name=\"legacy\">\n",
    "\t\t\t\t<string name=\"Protocol Name\">SSH2</string>\n",
    "\t\t\t\t<string name=\"Hostname\">10.0.0.5</string>\n",
    "\t\t\t\t<dword name=\"Port\">34</dword>\n",
    "\t\t\t</key>\n",
    "\t\t\t<key name=\"telnet\">\n",
    "\t\t\t\t<string name=\"Protocol Name\">Telnet</string>\n",
    "\t\t\t\t<string name=\"Hostname\">10.0.0.6</string>\n",
    "\t\t\t</key>\n",
    "\t\t\t<key name=\"nohost\">\n",
    "\t\t\t\t<string name=\"Protocol Name\">SSH2</string>\n",
    "\t\t\t</key>\n",
    "\t\t</key>\n",
    "\t\t<key name=\"root-level\">\n",
    "\t\t\t<string name=\"Protocol Name\">SSH2</string>\n",
    "\t\t\t<string name=\"Hostname\">10.0.0.9</string>\n",
    "\t\t</key>\n",
    "\t</key>\n",
    "</VanDyke>\n",
);

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

fn finalshell_config_json(name: &str, host: &str, extra: &str) -> String {
    format!(
        "{{\"name\":\"{name}\",\"host\":\"{host}\",\"port\":2222,\"user_name\":\"deploy\",\
         \"parent_id\":\"f2\",\"conection_type\":100,\"description\":\"primary\"{extra}}}"
    )
}

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

const ELECTERM_BOOKMARK: &str = concat!(
    "{",
    "\"id\":\"bm1\",\"title\":\"web\",\"host\":\"10.2.0.1\",\"port\":2222,",
    "\"username\":\"deploy\",\"auth_type\":\"password\",\"type\":\"ssh\"",
    "}"
);

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

const TERMIUS_EXPORT: &str = concat!(
    "{",
    "\"data\":{",
    "\"hosts\":[",
    "{\"id\":\"h1\",\"label\":\"web\",\"address\":\"10.3.0.1\",",
    "\"group\":{\"id\":\"g2\"},\"ssh_config\":{\"id\":\"c1\"}},",
    "{\"id\":\"h2\",\"address\":\"10.3.0.2\"}",
    "],",
    "\"groups\":[",
    "{\"id\":\"g1\",\"label\":\"Prod\",\"parent\":null},",
    "{\"id\":\"g2\",\"label\":\"Web\",\"parent\":{\"id\":\"g1\"}}",
    "],",
    "\"ssh_configs\":[{\"id\":\"c1\",\"port\":2222,\"identity\":{\"id\":\"i1\"}}],",
    "\"identities\":[",
    "{\"id\":\"i1\",\"username\":\"deploy\",\"password\":\"plain-pw\"},",
    "{\"id\":\"i2\",\"username\":\"ops\",\"password\":\"BAAAAAAA0123456789012345678901234567890123456789\"}",
    "],",
    "\"keys\":[",
    "{\"id\":\"k1\",\"label\":\"key1\",\"privateKey\":\"-----BEGIN OPENSSH PRIVATE KEY-----\\nabc\\n-----END OPENSSH PRIVATE KEY-----\",\"passphrase\":\"key-phr\"}",
    "]",
    "}",
    "}",
);

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
fn sshconfig_parses_host_blocks_with_defaults_and_first_value_wins() {
    let config = concat!(
        "# global defaults\n",
        "Port 2222\n",
        "\n",
        "Host web1\n",
        "  Hostname 10.0.0.1\n",
        "  User deploy\n",
        "  Port 2223\n",
        "  IdentityFile ~/.ssh/id_ed25519\n",
        "\n",
        "Host db1\n",
        "  hostName 10.0.0.2\n",
        "  USER ops\n",
        "  port = 2224\n",
    );
    let sessions = parse_ssh_config(config).unwrap();
    assert_eq!(sessions.len(), 2);
    // The global Port (first value) wins over the per-host one; the
    // alias, Hostname, User and IdentityFile map through.
    assert_eq!(sessions[0].name, "web1");
    assert_eq!(sessions[0].host, "10.0.0.1");
    assert_eq!(sessions[0].port, 2222);
    assert_eq!(sessions[0].username, "deploy");
    assert!(sessions[0].group_path.is_empty());
    match &sessions[0].auth {
        ImportedAuth::PrivateKey { path, has_secret } => {
            let path = path.as_deref().unwrap();
            assert!(path.ends_with(".ssh/id_ed25519"), "unexpected path: {path}");
            assert!(!has_secret, "a key path alone is not a secret");
        }
        other => panic!("expected private key auth, got {other:?}"),
    }
    assert_eq!(sessions[0].secret_note, SECRET_NOTE_KEY_PATH_ONLY);
    // Keyword case and the `=` separator both parse.
    assert_eq!(sessions[1].host, "10.0.0.2");
    assert_eq!(sessions[1].username, "ops");
    assert_eq!(sessions[1].port, 2222);
    // No IdentityFile: no auth, no note.
    assert_eq!(sessions[1].auth, ImportedAuth::None);
    assert_eq!(sessions[1].secret_note, "");
}

#[test]
fn sshconfig_wildcards_carry_defaults_without_becoming_entries() {
    let config = concat!(
        "Host *.prod.example.com\n",
        "  User admin\n",
        "  ProxyJump bastion\n",
        "Host *\n",
        "  User fallback\n",
        "  ServerAliveInterval 60\n",
        "\n",
        "Host web1.prod.example.com\n",
        "  Hostname 10.1.0.1\n",
        "\n",
        "Host plain\n",
    );
    let sessions = parse_ssh_config(config).unwrap();
    // `*.prod.example.com` and `*` are skipped as entries but their
    // defaults apply to the concrete candidates.
    assert_eq!(sessions.len(), 2);
    let web = &sessions[0];
    assert_eq!(web.name, "web1.prod.example.com");
    assert_eq!(web.host, "10.1.0.1");
    // First value wins: the narrower wildcard block beats `Host *`.
    assert_eq!(web.username, "admin");
    assert_eq!(web.port, 22);
    // ProxyJump from the wildcard block is annotated for manual mapping.
    assert_eq!(web.description, "ProxyJump bastion (needs manual mapping)");
    // A candidate matching nothing but `Host *` takes its defaults.
    assert_eq!(sessions[1].name, "plain");
    assert_eq!(sessions[1].host, "plain");
    assert_eq!(sessions[1].username, "fallback");
    assert!(sessions[1].description.is_empty());
}

#[test]
fn sshconfig_multi_pattern_host_line_splits_into_entries() {
    let config = "Host web1 db1 *.prod !hidden\n  User ops\n";
    let sessions = parse_ssh_config(config).unwrap();
    // Only the concrete patterns become entries; `*.prod` and `!hidden`
    // never name a machine.
    let names: Vec<&str> = sessions.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["web1", "db1"]);
    assert_eq!(sessions[0].username, "ops");
}

#[test]
fn sshconfig_comma_separated_host_list_becomes_separate_entries() {
    // OpenSSH 模式列表最常见的写法：逗号分隔。不拆开会把 "web1,db2"
    // 整体当成一个具名主机，导入一条必然连不上的坏条目。
    let config = "Host web1,web2,*.prod,!nope db3\n  User ops\n";
    let sessions = parse_ssh_config(config).unwrap();
    let names: Vec<&str> = sessions.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["web1", "web2", "db3"]);
    // 两个条目共享同一块内的设置。
    assert!(sessions.iter().all(|s| s.username == "ops"));
}

#[test]
fn sshconfig_hostname_tokens_and_negation_patterns() {
    let config = concat!(
        "Host tunnel\n",
        "  Hostname %h.example.net\n",
        "  Port 2200\n",
        "Host !web2 *\n",
        "  User noc\n",
        "Host backup\n",
    );
    let sessions = parse_ssh_config(config).unwrap();
    assert_eq!(sessions.len(), 2);
    // %h in Hostname expands to the matched alias.
    assert_eq!(sessions[0].host, "tunnel.example.net");
    assert_eq!(sessions[0].port, 2200);
    // A negation-only pattern list has no concrete candidate, but `*`
    // still carries the default user to every entry.
    assert_eq!(sessions[1].name, "backup");
    assert_eq!(sessions[1].username, "noc");
}

#[test]
fn sshconfig_include_and_known_hosts_are_annotated_not_followed() {
    // The pipeline receives uploaded bytes only, so Include is never
    // followed (no recursion, hence no cycle to detect) and both
    // directives degrade to visible annotations.
    let config = concat!(
        "Host web1\n",
        "  Include ~/.ssh/config.d/*.conf\n",
        "  Include ~/.ssh/config.d/*.conf\n",
        "  UserKnownHostsFile /etc/ssh/ssh_known_hosts\n",
        "  User deploy\n",
    );
    let sessions = parse_ssh_config(config).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].description,
        concat!(
            "Include ~/.ssh/config.d/*.conf (not followed); ",
            "UserKnownHostsFile /etc/ssh/ssh_known_hosts (not carried)"
        )
    );
}

#[test]
fn sshconfig_proxy_command_is_annotated_and_never_executed() {
    let config = concat!(
        "Host jump\n",
        "  ProxyCommand nc -X connect -x proxy.internal:3128 %h %p\n",
        "  Hostname 10.2.0.1\n",
    );
    let sessions = parse_ssh_config(config).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].description,
        "ProxyCommand nc -X connect -x proxy.internal:3128 %h %p (not executed, needs manual mapping)"
    );
}

#[test]
fn sshconfig_match_host_user_applies_conditionally() {
    let config = concat!(
        "Host web1\n",
        "  Hostname 10.0.0.1\n",
        "  User deploy\n",
        "Host web2\n",
        "  Hostname 10.0.0.2\n",
        "\n",
        "Match user deploy host 10.0.0.*\n",
        "  ForwardAgent yes\n",
        "  Port 2222\n",
    );
    let sessions = parse_ssh_config(config).unwrap();
    assert_eq!(sessions.len(), 2);
    // web1 (deploy@10.0.0.1) satisfies both criteria, web2 does not.
    assert_eq!(sessions[0].port, 2222);
    assert_eq!(sessions[1].port, 22);
}

#[test]
fn sshconfig_unsupported_match_degrades_with_reason() {
    let config = concat!(
        "Host web1\n",
        "  Hostname 10.0.0.1\n",
        "Match exec /usr/bin/check\n",
        "  Port 9999\n",
        "Match localuser root\n",
        "  Port 8888\n",
    );
    let sessions = parse_ssh_config(config).unwrap();
    assert_eq!(sessions.len(), 1);
    // The unsupported blocks are skipped and the reasons surface on the
    // imported row; their settings are not applied.
    assert_eq!(
        sessions[0].description,
        concat!(
            "Match exec: settings not imported; ",
            "Match localuser: settings not imported"
        )
    );
    assert_eq!(sessions[0].port, 22);
    // The reason list is capped.
    let config = concat!(
        "Host web1\n",
        "Match exec a\nMatch localuser b\nMatch final\nMatch all\nMatch canonical\n",
    );
    let sessions = parse_ssh_config(config).unwrap();
    assert_eq!(
        sessions[0]
            .description
            .matches("settings not imported")
            .count(),
        MAX_SSH_CONFIG_NOTES
    );
}

#[test]
fn sshconfig_host_argument_is_capped_and_block_structure_survives() {
    // 敌意超长 Host 模式串：截断到参数帽，块结构保留——后续指令仍归属
    // 该块，而不是错落到前一个块（截断的模式匹配不到任何主机，语义
    // 自然降级为"块保留、模式失效"）。
    let giant = "x".repeat(MAX_SSH_CONFIG_ARG_BYTES + 1024);
    let config = format!("Host {giant}\n  Port 2222\n");
    let blocks = ssh_config_blocks(&config).unwrap();
    assert_eq!(blocks.len(), 2);
    let patterns = blocks[1].host_patterns.as_ref().unwrap().join(" ");
    assert_eq!(patterns.len(), MAX_SSH_CONFIG_ARG_BYTES);
    assert_eq!(
        blocks[1].directives,
        vec![("port".to_string(), "2222".to_string())]
    );
}

#[test]
fn preview_text_strips_ansi_escapes_before_length_cap() {
    assert_eq!(preview_text("\u{1b}[31mweb-01\u{1b}[0m"), "web-01");
    assert_eq!(preview_text("plain"), "plain");
    // 帽在剥除之后计算：剥除后恰好触界的字符串不再截断。
    let long = format!("{}\u{1b}[0m", "y".repeat(MAX_PREVIEW_TEXT_BYTES));
    assert_eq!(preview_text(&long), "y".repeat(MAX_PREVIEW_TEXT_BYTES));
}

#[test]
fn sshconfig_identityfile_paths_map_without_reading_keys() {
    let config = concat!(
        "Host a\n",
        "  IdentityFile ~/.ssh/id_rsa\n",
        "Host b\n",
        "  IdentityFile %d/keys/id_ed25519\n",
        "Host c\n",
        "  IdentityFile /opt/keys/custom\n",
    );
    let sessions = parse_ssh_config(config).unwrap();
    assert_eq!(sessions.len(), 3);
    for (index, expected_suffix) in [".ssh/id_rsa", "keys/id_ed25519", "custom"]
        .into_iter()
        .enumerate()
    {
        match &sessions[index].auth {
            ImportedAuth::PrivateKey { path, has_secret } => {
                let path = path.as_deref().unwrap();
                assert!(path.ends_with(expected_suffix), "unexpected path: {path}");
                assert!(!has_secret, "no key material may be represented");
            }
            other => panic!("expected private key auth, got {other:?}"),
        }
        assert_eq!(sessions[index].secret_note, SECRET_NOTE_KEY_PATH_ONLY);
    }
    // `~` and %d expand against the same home directory.
    match &sessions[0].auth {
        ImportedAuth::PrivateKey {
            path: Some(home_1), ..
        } => match &sessions[1].auth {
            ImportedAuth::PrivateKey {
                path: Some(home_2), ..
            } => {
                let home_1 = home_1.trim_end_matches(".ssh/id_rsa");
                let home_2 = home_2.trim_end_matches("keys/id_ed25519");
                assert_eq!(home_1, home_2, "both prefixes expand to HOME");
            }
            other => panic!("expected private key auth, got {other:?}"),
        },
        other => panic!("expected private key auth, got {other:?}"),
    }
}

#[test]
fn sshconfig_hostile_input_degrades_without_panicking() {
    // No Host lines at all: empty result, not an error.
    assert!(parse_ssh_config("").unwrap().is_empty());
    assert!(parse_ssh_config("only comments\n# and blanks\n\n")
        .unwrap()
        .is_empty());
    // Unterminated quote, keyword-only lines and giant values degrade to
    // skipped lines / directives.
    let config = concat!(
        "Host \"unterminated\n",
        "Host\n",
        "  Port\n",
        "  Port not-a-port\n",
        "  Hostname \n",
        "Host ok\n",
    );
    let sessions = parse_ssh_config(config).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, "ok");
    assert_eq!(sessions[0].port, 22);
    assert_eq!(sessions[0].host, "ok");
    // An argument beyond the per-directive budget is dropped silently.
    let giant = format!(
        "Host g\n  Hostname {}\n",
        "x".repeat(MAX_SSH_CONFIG_ARG_BYTES + 1)
    );
    let sessions = parse_ssh_config(&giant).unwrap();
    assert_eq!(sessions[0].host, "g");
    // Empty and negated-only pattern lists match nothing.
    let config = "Host !only-negated\n  User x\n";
    assert!(parse_ssh_config(config).unwrap().is_empty());
}

#[test]
fn sshconfig_line_and_session_caps_fail_closed() {
    let mut config = String::new();
    for _ in 0..=MAX_SSH_CONFIG_LINES {
        config.push_str("# line\n");
    }
    let error = parse_ssh_config(&config).expect_err("line budget must fail closed");
    assert!(error.contains("line limit"), "{error}");
    let mut config = String::new();
    for index in 0..=MAX_PREVIEW_SESSIONS {
        config.push_str(&format!("Host host-{index}\n"));
    }
    let error = parse_ssh_config(&config).expect_err("session budget must fail closed");
    assert!(error.contains("session limit"), "{error}");
}

#[test]
fn sshconfig_pattern_budget_fails_closed_on_hostile_globs() {
    // 200 wildcard blocks with long star-heavy patterns matched against
    // 500 candidates: the O(pattern × candidate) budget trips long before
    // any pathological CPU burn.
    let mut config = String::new();
    for _ in 0..200 {
        config.push_str(&format!("Host {}{}\n", "*a".repeat(100), "*"));
    }
    for index in 0..500 {
        config.push_str(&format!("Host candidate-{index:04}-of-500\n"));
    }
    let error = parse_ssh_config(&config).expect_err("pattern budget must fail closed");
    assert_eq!(error, SSH_CONFIG_MATCH_WORK_EXCEEDED);
}

#[test]
fn sshconfig_glob_and_pattern_list_semantics() {
    // Single patterns: `*`, `?`, case-insensitive, byte-exact multibyte.
    assert!(ssh_glob_match("*.prod", "web1.prod"));
    assert!(!ssh_glob_match("*.prod", "web1.prod.example"));
    assert!(ssh_glob_match("web?", "web1"));
    assert!(!ssh_glob_match("web?", "web12"));
    assert!(ssh_glob_match("WEB1", "web1"));
    assert!(ssh_glob_match("主机*", "主机01"));
    assert!(!ssh_glob_match("", "x"));
    assert!(ssh_glob_match("", ""));
    // Pattern lists: comma separation, `!` veto beats positives.
    let mut work = 0u64;
    assert_eq!(
        ssh_pattern_list_matches("web1,*.prod", "web1", &mut work),
        Some(true)
    );
    assert_eq!(
        ssh_pattern_list_matches("*.prod,!web1.prod", "web1.prod", &mut work),
        Some(false)
    );
    assert_eq!(
        ssh_pattern_list_matches("!*,web1", "web1", &mut work),
        Some(false)
    );
    // The budget reports exhaustion instead of burning CPU.
    let mut work = MAX_SSH_MATCH_WORK;
    assert_eq!(ssh_pattern_list_matches("*", "web1", &mut work), None);
}

#[test]
fn sshconfig_streaming_preview_roundtrips_sanitized() {
    let text = concat!(
        "Host web1\n",
        "  Hostname 10.7.0.1\n",
        "  User deploy\n",
        "  Port 2222\n",
        "  IdentityFile ~/.ssh/id_rsa\n",
        "  ProxyJump bastion\n",
    );
    let stream = ImportStream::default();
    let started = stream
        .start(&json!({ "kind": "sshconfig", "mainSize": text.len() }))
        .unwrap();
    let task_id = started["taskId"].as_str().unwrap();
    let mut frame = 0u64.to_be_bytes().to_vec();
    frame.extend_from_slice(text.as_bytes());
    assert_eq!(
        stream.append(task_id, "main", &frame).unwrap(),
        text.len() as u64
    );
    let preview = stream.finish(task_id).unwrap();
    assert_eq!(preview["sourceKind"], "sshconfig");
    assert_eq!(preview["sessions"][0]["name"], "web1");
    assert_eq!(preview["sessions"][0]["port"], 2222);
    assert_eq!(preview["sessions"][0]["authKind"], "private-key");
    assert_eq!(
        preview["sessions"][0]["secretNote"],
        SECRET_NOTE_KEY_PATH_ONLY
    );
    // The sanitized export keeps the key path but never any key material
    // or password fields (the no-credential invariant for the new source).
    let exported = preview["export"].to_string();
    assert!(exported.contains("keyPath"), "{exported}");
    assert!(!exported.contains("\"privateKey\""), "{exported}");
    assert!(!exported.contains("\"password\""), "{exported}");
    assert!(!exported.contains("\"passphrase\""), "{exported}");
    assert!(stream.finish(task_id).is_err());
}

#[test]
fn sshconfig_line_splitting_edge_cases() {
    // Comments, blanks and quoted arguments.
    assert_eq!(split_ssh_config_line("# comment"), None);
    assert_eq!(split_ssh_config_line("   "), None);
    let (keyword, argument) = split_ssh_config_line("Host \"a b\"").unwrap();
    assert_eq!(keyword, "host");
    assert_eq!(argument, "a b");
    // `=` separators with and without spaces.
    assert_eq!(
        split_ssh_config_line("HostName=x.example"),
        Some(("hostname".to_string(), "x.example".to_string()))
    );
    assert_eq!(
        split_ssh_config_line("Port = 22"),
        Some(("port".to_string(), "22".to_string()))
    );
    // An unterminated quote degrades to a skipped line.
    assert_eq!(split_ssh_config_line("IdentityFile \"no-close"), None);
    // Match criteria parsing: no-arg criteria get empty values.
    let criteria = parse_ssh_match_criteria("host *.prod user deploy");
    assert_eq!(
        criteria,
        vec![
            ("host".to_string(), "*.prod".to_string()),
            ("user".to_string(), "deploy".to_string()),
        ]
    );
    let criteria = parse_ssh_match_criteria("all");
    assert_eq!(criteria, vec![("all".to_string(), String::new())]);
    // A trailing keyword without its value degrades to an empty value.
    let criteria = parse_ssh_match_criteria("exec");
    assert_eq!(criteria, vec![("exec".to_string(), String::new())]);
}

// -- Sanitized export ----------------------------------------------------

fn sample_sessions() -> Vec<ImportedSession> {
    vec![
        ImportedSession {
            name: "web".to_string(),
            host: "10.0.0.1".to_string(),
            port: 22,
            username: "root".to_string(),
            group_path: vec!["Prod".to_string()],
            description: String::new(),
            auth: ImportedAuth::Password { has_secret: true },
            secret_note: String::new(),
        },
        ImportedSession {
            name: "keyed".to_string(),
            host: "10.0.0.2".to_string(),
            port: 2222,
            username: "ops".to_string(),
            group_path: Vec::new(),
            description: "d".to_string(),
            auth: ImportedAuth::PrivateKey {
                path: Some("/home/u/key".to_string()),
                has_secret: true,
            },
            secret_note: String::new(),
        },
    ]
}

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

#[test]
fn windterm_helpers_edge_cases() {
    // Target splitting keeps multi-@ users intact and trims spaces.
    assert_eq!(
        split_windterm_target(" user@mail.com @ host "),
        ("user@mail.com".to_string(), "host".to_string())
    );
    assert_eq!(
        split_windterm_target("only-host"),
        (String::new(), "only-host".to_string())
    );
    // Ports arrive as numbers or strings; junk falls back to 22.
    assert_eq!(windterm_port(Some(&json!(2222))), 2222);
    assert_eq!(windterm_port(Some(&json!("2222"))), 2222);
    assert_eq!(windterm_port(Some(&json!("nope"))), 22);
    assert_eq!(windterm_port(Some(&json!(0))), 22);
    assert_eq!(windterm_port(None), 22);
    // Fingerprint salt: hex decodes, junk stays raw bytes.
    assert_eq!(fingerprint_bytes("0xa1b2"), vec![0xa1, 0xb2]);
    assert_eq!(fingerprint_bytes("A1B2"), vec![0xa1, 0xb2]);
    assert_eq!(fingerprint_bytes("salt!"), b"salt!".to_vec());
    // Config flags are lenient.
    assert!(config_flag_is_on(&json!(true)));
    assert!(config_flag_is_on(&json!("true")));
    assert!(config_flag_is_on(&json!(1)));
    assert!(!config_flag_is_on(&json!(false)));
    assert!(!config_flag_is_on(&json!("off")));
    // Home prefix expansion without HOME keeps the path verbatim.
    assert_eq!(expand_home_prefix("relative/id_rsa"), "relative/id_rsa");
}

// -- Connection-form import action ---------------------------------------

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
