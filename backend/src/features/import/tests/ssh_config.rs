use super::*;

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
