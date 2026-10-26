use super::*;

#[test]
fn confirm_gate_set_covers_exec_family_and_spares_read_tools() {
    for gated in [
        "ssh_exec",
        "ssh_exec_sudo",
        "ssh_run_bg",
        "ssh_multi_exec",
        "ssh_terminal_input",
        "sftp_write_file",
        "sftp_upload",
    ] {
        assert!(is_confirm_gated_tool(gated), "{gated} should be gated");
    }
    for spared in [
        "ssh_close",
        "sftp_list_dir",
        "sftp_stat",
        "sftp_read_file",
        "sftp_download",
    ] {
        assert!(!is_confirm_gated_tool(spared), "{spared} must not be gated");
    }
}

#[tokio::test]
async fn command_text_gates_refuse_destructive_and_unknown_on_read_only() {
    // 提取自 call_tool_inner 的命令文本闸（确认改写后二次过闸复用同一
    // 入口）：read-only 下破坏性命令无条件拒绝、白名单外命令拒绝、白名单
    // 内只读命令放行。
    let state = state();
    let error = state
        .enforce_command_text_gates("ssh_exec", "rm -rf /tmp/x", true, &json!({}))
        .await
        .unwrap_err();
    assert!(error.contains("read-only"), "got: {error}");
    let error = state
        .enforce_command_text_gates("ssh_exec", "curl example.test", true, &json!({}))
        .await
        .unwrap_err();
    assert!(
        error.contains("not recognized as read-only"),
        "got: {error}"
    );
    state
        .enforce_command_text_gates("ssh_exec", "ls /tmp", true, &json!({}))
        .await
        .expect("inspection command passes the text gates");
}

#[test]
fn read_bounded_line_stays_within_the_ceiling() {
    let max_line = 16;
    // 正常行：含换行原样返回。
    let mut reader = io::Cursor::new(b"ping\nnext\n".to_vec());
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::Line
    ));
    assert_eq!(raw, b"ping\n");
    // EOF 尾行（无换行、且在上限内）：不算超限，正常返回。
    let mut reader = io::Cursor::new(b"tail-no-nl".to_vec());
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::Line
    ));
    // 恰好 max_line 字节内容 + 换行：合法满行。
    let mut full_line = vec![b'a'; max_line];
    full_line.push(b'\n');
    let mut reader = io::Cursor::new(full_line);
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::Line
    ));
    assert_eq!(raw.len(), max_line + 1);
    // 超限：读满上限仍无换行 → OverLimit，残余被吞弃，下一条是后续行。
    let mut input = vec![b'x'; 100];
    input.extend_from_slice(b"\nok\n");
    let mut reader = io::Cursor::new(input);
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::OverLimit
    ));
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::Line
    ));
    assert_eq!(raw, b"ok\n");
    // EOF：空读。
    let mut reader = io::Cursor::new(Vec::<u8>::new());
    let mut raw = Vec::new();
    assert!(matches!(
        read_bounded_line(&mut reader, max_line, &mut raw).unwrap(),
        StdioLineRead::Eof
    ));
}

#[tokio::test]
async fn run_bg_respects_read_only_gate() {
    let mut state = state();
    state.global_read_only = true;
    let error = state
        .call_tool(
            "ssh_run_bg",
            &json!({
                "host": "example.test",
                "username": "op",
                "command": "sleep 30"
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("read-only"),
        "expected read-only refusal, got: {error}"
    );
}

#[tokio::test]
async fn inline_dial_inherits_registered_read_only_gate() {
    let state = state();
    // All credential values in tests are assembled at runtime — never
    // real credentials, never literals in source.
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-ro",
            "host": "prod.example.test",
            "port": 2222,
            "username": "deploy",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
            "read_only": true,
        }
    }))
    .unwrap();
    assert!(stored.read_only, "lifecycle payload must carry read_only");
    state
        .dbx_connections
        .write()
        .await
        .insert("conn-ro".to_string(), stored);
    // Same endpoint re-dialed inline (no connectionId, host case-insensitive):
    // the read-only gate still applies to write tools — the error fires
    // before run_tool, so no localPath is needed to distinguish it from
    // a plain validation error.
    let error = state
        .call_tool(
            "sftp_upload",
            &json!({
                "host": "PROD.example.test",
                "port": 2222,
                "username": "deploy",
                "remotePath": "/tmp/x",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("read-only"),
        "expected read-only refusal for inline dial of a registered host, got: {error}"
    );
    // A different username or host is not covered by that registration.
    for identity in [
        json!({"host": "prod.example.test", "port": 2222, "username": "other"}),
        json!({"host": "other.example.test", "username": "deploy"}),
    ] {
        let error = state
            .call_tool("sftp_upload", &identity, None)
            .await
            .unwrap_err();
        assert!(
            !error.contains("read-only"),
            "expected non-matching identity to pass the gate, got: {error}"
        );
    }
}

#[tokio::test]
async fn sudo_allowlist_gates_privileged_tools() {
    let state = state();
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-sudo-list",
            "host": "web.example.test",
            "port": 22,
            "username": "deploy",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
            "external_config": {
                "sudo_whitelist": "systemctl restart nginx\ndocker restart *"
            },
        }
    }))
    .unwrap();
    assert_eq!(stored.sudo_whitelist.len(), 2, "external_config must parse");
    state
        .dbx_connections
        .write()
        .await
        .insert("conn-sudo-list".to_string(), stored);

    // ssh_exec_sudo with an unlisted command: refused, patterns listed.
    let error = state
        .call_tool(
            "ssh_exec_sudo",
            &json!({
                "connectionId": "conn-sudo-list",
                "command": "systemctl restart mysql",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("not allowed by this connection's whitelist")
            && error.contains("docker restart *"),
        "expected allowlist refusal, got: {error}"
    );

    // A matching command passes the gate and fails later (no dial here).
    let error = state
        .call_tool(
            "ssh_exec_sudo",
            &json!({
                "connectionId": "conn-sudo-list",
                "command": "docker restart api",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("whitelist"),
        "expected matching command to pass the gate, got: {error}"
    );

    // Inline `sudo …` inside plain ssh_exec is gated too.
    let error = state
        .call_tool(
            "ssh_exec",
            &json!({
                "connectionId": "conn-sudo-list",
                "command": "sudo systemctl restart mysql",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("not allowed by this connection's whitelist"),
        "expected inline-sudo refusal, got: {error}"
    );

    // Inline identity fallback: re-dialing the same host without a
    // connectionId inherits the whitelist; a different host does not.
    let error = state
        .call_tool(
            "ssh_exec_sudo",
            &json!({
                "host": "web.example.test",
                "username": "deploy",
                "command": "reboot",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("not allowed by this connection's whitelist"),
        "expected identity-fallback refusal, got: {error}"
    );
    let error = state
        .call_tool(
            "ssh_exec_sudo",
            &json!({
                "host": "other.example.test",
                "username": "deploy",
                "command": "reboot",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("whitelist"),
        "expected other host to skip the allowlist, got: {error}"
    );
}

#[tokio::test]
async fn read_only_tools_respect_sensitive_path_denylist() {
    let mut read_only_state = state();
    read_only_state.global_read_only = true;
    for (name, arguments) in [
        (
            "sftp_read_file",
            json!({"host": "h.test", "username": "op", "path": "/root/.ssh/id_rsa"}),
        ),
        (
            "sftp_list_dir",
            json!({"host": "h.test", "username": "op", "path": "/root/.ssh"}),
        ),
        (
            "sftp_download",
            json!({"host": "h.test", "username": "op", "remotePath": "/etc/shadow",
                   "localPath": "/tmp/dbx-denylist-test"}),
        ),
        (
            "ssh_task_status",
            json!({"host": "h.test", "username": "op", "logPath": "/root/.bash_history"}),
        ),
    ] {
        let error = read_only_state
            .call_tool(name, &arguments, None)
            .await
            .unwrap_err();
        assert!(
            error.contains("sensitive"),
            "expected sensitive-path refusal for {name}, got: {error}"
        );
    }
    // Ordinary inspection paths still pass the gate (they fail later, at
    // the dial).
    let error = read_only_state
        .call_tool(
            "sftp_read_file",
            &json!({"host": "h.test", "username": "op", "path": "/var/log/app.log"}),
            None,
        )
        .await
        .unwrap_err();
    assert!(!error.contains("sensitive"), "got: {error}");
    // On normal connections the denylist does not apply — the operator
    // already grants full access.
    let normal_state = state();
    let error = normal_state
        .call_tool(
            "sftp_read_file",
            &json!({"host": "h.test", "username": "op", "path": "/root/.ssh/id_rsa"}),
            None,
        )
        .await
        .unwrap_err();
    assert!(!error.contains("sensitive"), "got: {error}");
}

#[tokio::test]
async fn exec_whitelist_refuses_sensitive_paths_on_read_only() {
    let mut state = state();
    state.global_read_only = true;
    let error = state
        .call_tool(
            "ssh_exec",
            &json!({
                "host": "h.test",
                "username": "op",
                "command": "cat /root/.ssh/id_rsa",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("not recognized"),
        "expected whitelist refusal for a sensitive path, got: {error}"
    );
}

#[tokio::test]
async fn sftp_download_refuses_sensitive_local_targets() {
    let state = state();
    for local in [
        "/home/u/.bashrc",
        "/home/u/.ssh/authorized_keys",
        "/etc/cron.d/payload",
        "/Library/LaunchDaemons/com.example.payload.plist",
    ] {
        let error = state
            .call_tool(
                "sftp_download",
                &json!({
                    "host": "h.test",
                    "username": "op",
                    "remotePath": "/tmp/payload.sh",
                    "localPath": local,
                }),
                None,
            )
            .await
            .unwrap_err();
        assert!(
            error.contains("Refusing to write the local sensitive path"),
            "expected local denylist refusal for {local}, got: {error}"
        );
    }
    // Ordinary destinations pass the denylist (they fail later, at the
    // dial).
    let error = state
        .call_tool(
            "sftp_download",
            &json!({
                "host": "h.test",
                "username": "op",
                "remotePath": "/tmp/payload.sh",
                "localPath": "/tmp/dbx-download-ok/payload.sh",
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("Refusing to write"),
        "expected plain target to pass the denylist, got: {error}"
    );
}

#[test]
fn terminal_input_gate_read_only_only_allows_control_sequences() {
    // 纯控制序列（回车/Ctrl+C）在只读连接放行。
    assert!(terminal_input_gate("\r", true, false, &[]).is_ok());
    assert!(terminal_input_gate("\u{3}", true, false, &[]).is_ok());
    assert!(terminal_input_gate("", true, false, &[]).is_ok());
    // 任何可见文本（哪怕只读命令）都拒绝：终端注入不在只读白名单模型内。
    let error = terminal_input_gate("echo hi\r", true, false, &[]).unwrap_err();
    assert!(error.contains("read-only"), "{error}");
}

#[test]
fn terminal_input_gate_destructive_lines_need_confirmation() {
    // 灾难行（递归删除系统根）：无确认拒绝，确认放行（可写连接）。
    // 非灾难目标的 rm -rf /tmp/x 只是 Unknown 写操作，不触发本门。
    let error = terminal_input_gate("rm -rf /\r", false, false, &[]).unwrap_err();
    assert!(error.contains("destructive"), "{error}");
    assert!(terminal_input_gate("rm -rf /\r", false, true, &[]).is_ok());
    // 只读连接上灾难行即使带确认也拒绝（与 ssh_exec 一致）。
    let error = terminal_input_gate("rm -rf /\r", true, true, &[]).unwrap_err();
    assert!(error.contains("read-only"), "{error}");
    // 多行取最坏：干净行 + 灾难行整体拒绝。
    let error = terminal_input_gate("df -h\rrm -rf /\r", false, false, &[]).unwrap_err();
    assert!(error.contains("destructive"), "{error}");
}

#[test]
fn terminal_input_gate_sudo_lines_follow_the_allowlist() {
    // 连接声明了白名单：sudo 行必须命中条目。
    let allowlist = vec![vec![
        "systemctl".to_string(),
        "restart".to_string(),
        "*".to_string(),
    ]];
    let error = terminal_input_gate("sudo yum update -y\r", false, false, &allowlist).unwrap_err();
    assert!(error.contains("whitelist"), "{error}");
    assert!(
        terminal_input_gate("sudo systemctl restart nginx\r", false, false, &allowlist).is_ok()
    );
    // 未声明白名单的连接不做该门（行为与 ssh_exec 一致）。
    assert!(terminal_input_gate("sudo yum update -y\r", false, false, &[]).is_ok());
}

#[tokio::test]
async fn run_bg_requires_destructive_confirmation() {
    let state = state();
    let error = state
        .call_tool(
            "ssh_run_bg",
            &json!({
                "host": "example.test",
                "username": "op",
                "command": "rm -rf /"
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("destructive"),
        "expected destructive refusal, got: {error}"
    );
}

#[test]
fn pre_exec_transport_errors_are_retryable_post_exec_are_not() {
    assert!(exec::is_pre_exec_transport_error(
        "Failed to open exec channel: Disconnected"
    ));
    assert!(exec::is_pre_exec_transport_error(
        "Failed to start command: broken pipe"
    ));
    // Post-exec failures must NOT look retryable: the command may have
    // already run server-side.
    assert!(!exec::is_pre_exec_transport_error(
        "Timed out waiting for the remote command to finish."
    ));
    assert!(!exec::is_pre_exec_transport_error(
        "sudo exited 1: wrong password"
    ));
}

#[test]
fn local_transfer_gate_confines_paths_to_allowed_roots() {
    let scratch = std::env::temp_dir().join(format!("mcp-gate-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let inside_dir = scratch.join("inside");
    std::fs::create_dir_all(&inside_dir).unwrap();
    std::fs::write(inside_dir.join("a.bin"), b"x").unwrap();
    let inside = std::fs::canonicalize(inside_dir.join("a.bin")).unwrap();
    let roots = vec![std::fs::canonicalize(&inside_dir).unwrap()];
    // Inside the only configured root passes…
    assert!(ensure_local_transfer_allowed_in(&roots, "/x", &inside).is_ok());
    // …anything else (even non-sensitive) is refused…
    let outside = std::fs::canonicalize("/usr").unwrap_or_else(|_| PathBuf::from("/usr"));
    let error = ensure_local_transfer_allowed_in(&roots, "/x", &outside).unwrap_err();
    assert!(error.contains("localTransferRoot"), "{error}");
    // …and the default-roots message names the setting to widen.
    let empty_roots: Vec<PathBuf> = vec![];
    let error = ensure_local_transfer_allowed_in(&empty_roots, "", &inside).unwrap_err();
    assert!(
        error.contains("outside the allowed transfer roots"),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn local_transfer_gate_blocklist_applies_inside_allowed_roots() {
    let scratch = std::env::temp_dir().join(format!("mcp-gate-bl-{}", std::process::id()));
    let secret_dir = scratch.join(".ssh");
    std::fs::create_dir_all(&secret_dir).unwrap();
    std::fs::write(secret_dir.join("id_rsa"), b"k").unwrap();
    let key = std::fs::canonicalize(secret_dir.join("id_rsa")).unwrap();
    let rc = std::fs::canonicalize(&scratch).unwrap().join(".bashrc");
    let roots = vec![std::fs::canonicalize(&scratch).unwrap()];
    assert!(ensure_local_transfer_allowed_in(&roots, "", &key).is_err());
    assert!(ensure_local_transfer_allowed_in(&roots, "", &rc).is_err());
    let _ = std::fs::remove_dir_all(&scratch);
}

#[tokio::test]
async fn destructive_commands_require_explicit_confirmation() {
    let state = state();
    // Refused without the flag; the refusal happens before credential
    // validation (no "password" complaint), proving the gate ordering.
    let refused = state
        .dispatch(json!({
            "jsonrpc": "2.0", "id": 30, "method": "tools/call",
            "params": { "name": "ssh_exec", "arguments": {
                "host": "203.0.113.1", "username": "u",
                "command": "mkfs.ext4 /dev/sda1",
            }},
        }))
        .await
        .unwrap();
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("confirmDestructive"),
        "unexpected: {message}"
    );
    assert!(
        !message.contains("password"),
        "gate must fire first: {message}"
    );

    // With the flag the gate passes and the call proceeds to parameter
    // validation (missing password), proving it was not blocked.
    let gated_through = state
        .dispatch(json!({
            "jsonrpc": "2.0", "id": 31, "method": "tools/call",
            "params": { "name": "ssh_exec", "arguments": {
                "host": "203.0.113.1", "username": "u",
                "command": "mkfs.ext4 /dev/sda1",
                "confirmDestructive": true,
            }},
        }))
        .await
        .unwrap();
    let message = gated_through["error"]["message"].as_str().unwrap();
    assert!(message.contains("password"), "unexpected: {message}");

    // The same gate covers the sudo tool, and pipelines leak through
    // chain segments.
    for tool in ["ssh_exec_sudo"] {
        let chained = state
            .dispatch(json!({
                "jsonrpc": "2.0", "id": 32, "method": "tools/call",
                "params": { "name": tool, "arguments": {
                    "host": "203.0.113.1", "username": "u",
                    "command": "uptime && shutdown -h now",
                }},
            }))
            .await
            .unwrap();
        let message = chained["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("confirmDestructive"),
            "unexpected: {message}"
        );
    }
}

/// Reliability round 5: sudo-prefixed and substitution-hidden
/// destructive commands demand `confirmDestructive` at the tool gate
/// too (they used to slip through as plain Unknown and run unconfirmed
/// on writable connections), while ordinary commands still pass the
/// gate through to the credential check.
#[tokio::test]
async fn sudo_and_substitution_destructive_variants_demand_confirmation() {
    let state = state();
    for command in [
        "sudo rm -rf /",
        "sudo -u root rm -rf /etc",
        "sudo sh -c 'rm -rf /etc'",
        "sh -c 'mkfs.ext4 /dev/sda1'",
        "echo $(rm -rf /)",
        "echo `shutdown -h now`",
    ] {
        let refused = state
            .call_tool(
                "ssh_exec",
                &json!({ "host": "203.0.113.1", "username": "u", "command": command }),
                None,
            )
            .await
            .unwrap_err();
        assert!(
            refused.contains("confirmDestructive"),
            "{command}: {refused}"
        );
        assert!(!refused.contains("password"), "{command}: {refused}");
    }
    // The gate did not over-fire: a benign command proceeds past it.
    let through = state
        .call_tool(
            "ssh_exec",
            &json!({ "host": "203.0.113.1", "username": "u", "command": "echo hi" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(through.contains("password"), "{through}");
}

/// Reliability round 5: the local download/denylist gate holds across
/// double-slash and `./` path aliasing.
#[test]
fn sensitive_local_path_gate_survives_path_shapes() {
    for path in [
        "/etc//cron.d/payload",
        "/etc/./cron.d/payload",
        "/etc/./profile",
        "/home/u//.ssh/authorized_keys",
        "/var/spool//cron/x",
        "/Library/./LaunchDaemons/evil.plist",
    ] {
        assert!(
            is_sensitive_local_path(path),
            "expected '{path}' to be a sensitive local path"
        );
    }
    for path in [
        "/tmp/ok.bin",
        "/home/u/project/cron-doc.txt",
        "/home/u/ssh-config-backup", // prefix must match components, not substrings
    ] {
        assert!(
            !is_sensitive_local_path(path),
            "expected '{path}' NOT to be a sensitive local path"
        );
    }
}

// —— 第五轮：会话/存储 churn 可靠性 ————————————————

#[tokio::test]
async fn read_only_connection_gates_writes_and_unknown_commands() {
    let state = McpState::shared(Arc::new(SshRuntime::new(
        std::env::temp_dir().join("dbx-mcp-readonly-test"),
    )));
    let lifecycle = json!({
        "connection": {
            "id": "conn-readonly",
            "name": "Prod bastion",
            "host": "192.0.2.10",
            "port": 22,
            "username": "ops",
            "password": "secret",
            "external_config": { "authentication": "password", "read_only": true },
        },
        "runtime": { "host": "192.0.2.10", "port": 22 },
        "operationId": "op-ro",
    });

    // Write-class tools are rejected on sight.
    for tool in ["sftp_write_file", "ssh_exec_sudo"] {
        let error = state
            .call_dbx_with(
                &json!({
                    "tool": tool,
                    "arguments": { "command": "uptime", "path": "/tmp/x", "content": "y" },
                    "lifecycle": lifecycle,
                }),
                None,
            )
            .await
            .expect_err("write tool must be refused on a read-only connection");
        assert!(error.contains("read-only"), "unexpected: {error}");
    }

    // ssh_exec survives only for provably read-only commands: an
    // unrecognized mutating command is refused by the whitelist (before
    // any dialing), while a destructive pattern is refused outright.
    let unknown = state
        .call_dbx_with(
            &json!({
                "tool": "ssh_exec",
                "arguments": { "command": "systemctl restart nginx" },
                "lifecycle": lifecycle,
            }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(unknown.contains("not recognized"), "unexpected: {unknown}");

    let destructive = state
        .call_dbx_with(
            &json!({
                "tool": "ssh_exec",
                "arguments": { "command": "rm -rf /etc" },
                "lifecycle": lifecycle,
            }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        destructive.contains("Refused on read-only"),
        "unexpected: {destructive}"
    );

    // confirmDestructive cannot override a read-only connection.
    let confirmed = state
        .call_dbx_with(
            &json!({
                "tool": "ssh_exec",
                "arguments": { "command": "rm -rf /etc", "confirmDestructive": true },
                "lifecycle": lifecycle,
            }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        confirmed.contains("Refused on read-only"),
        "unexpected: {confirmed}"
    );
}

// —— MCP 容错性专项（2026-09-13 测试覆盖审计）———

/// 安全门组合矩阵：只读门 × confirmDestructive × sudo 白名单 × 进程级
/// 只读开关 × connectionScope 的组合行为。核心不变量：
/// 白名单/确认位都不放大权限——灾难门永远在白名单之后仍然生效；
/// 只读门先于一切；scope 是 fail-closed 的第一道闸。
#[tokio::test]
async fn gate_combo_matrix_read_only_destructive_sudo_and_scope() {
    // 普通连接（sudo 白名单：精确 systemctl restart nginx + shutdown *）。
    let rw_state = state();
    rw_state.dbx_connections.write().await.insert(
        "conn-rw".to_string(),
        stored_connection(
            "conn-rw",
            "web.example.test",
            22,
            json!({ "external_config": {
                    "sudo_whitelist": "systemctl restart nginx\nshutdown *"
                } }),
        ),
    );
    // 只读连接（同白名单）：只读门必须先于白名单与确认位。
    let ro_state = state();
    ro_state.dbx_connections.write().await.insert(
        "conn-ro".to_string(),
        stored_connection(
            "conn-ro",
            "web.example.test",
            22,
            json!({ "read_only": true, "external_config": {
                    "sudo_whitelist": "systemctl restart nginx\nshutdown *"
                } }),
        ),
    );
    // 组合 1：非只读 + 灾难命令 + confirmDestructive:true → 过灾难门
    // （错误是拨号失败，不再是 confirm 提示）。
    let error = rw_state
        .call_tool(
            "ssh_exec",
            &json!({
                "connectionId": "conn-rw",
                "command": "shutdown -h now",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("confirmDestructive") && !error.contains("looks destructive"),
        "confirmed destructive command must proceed past the gate, got: {error}"
    );

    // 组合 2：非只读 + 灾难命令 + 无确认 → confirm 提示（对照组）。
    let error = rw_state
        .call_tool(
            "ssh_exec",
            &json!({ "connectionId": "conn-rw", "command": "shutdown -h now" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("confirmDestructive"), "{error}");

    // 组合 3：白名单命中的灾难命令（shutdown -h now ∈ shutdown *）→
    // 白名单放行不豁免灾难门，仍需 confirmDestructive。
    let error = rw_state
        .call_tool(
            "ssh_exec_sudo",
            &json!({ "connectionId": "conn-rw", "command": "shutdown -h now" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("confirmDestructive") && !error.contains("whitelist"),
        "whitelist must not bypass the destructive gate, got: {error}"
    );
    // 同命令带确认后过两道门（白名单 + 灾难），落到拨号失败。
    let error = rw_state
        .call_tool(
            "ssh_exec_sudo",
            &json!({
                "connectionId": "conn-rw",
                "command": "shutdown -h now",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("whitelist") && !error.contains("confirmDestructive"),
        "confirmed whitelisted command must pass both gates, got: {error}"
    );

    // 组合 4：只读连接 + ssh_exec_sudo（写类）+ 命令命中白名单 →
    // 只读门先行，白名单救不回写工具。
    let error = ro_state
        .call_tool(
            "ssh_exec_sudo",
            &json!({ "connectionId": "conn-ro", "command": "systemctl restart nginx" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("read-only"),
        "read-only gate must fire before the whitelist, got: {error}"
    );

    // 组合 5：只读连接 + 白名单巡检命令 + confirmDestructive:true →
    // 确认位不改变白名单判定，调用照常走到拨号。
    let error = ro_state
        .call_tool(
            "ssh_exec",
            &json!({
                "connectionId": "conn-ro",
                "command": "df -h",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("read-only") && !error.contains("not recognized"),
        "inspection command must stay available on read-only, got: {error}"
    );

    // 组合 6：只读连接 + 灾难命令 + confirmDestructive:true →
    // 确认位无法覆盖只读（同命令在非只读上只需要确认）。
    let error = ro_state
        .call_tool(
            "ssh_exec",
            &json!({
                "connectionId": "conn-ro",
                "command": "shutdown -h now",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("Refused on read-only"),
        "read-only must refuse destructive commands outright, got: {error}"
    );

    // 组合 7：进程级只读开关（DBX_SSH_MCP_READ_ONLY）× 非只读注册连接
    // × 灾难命令 + 确认 → 操作员开关压过连接自身的可写属性。
    let mut killed_state = state();
    killed_state.global_read_only = true;
    killed_state.dbx_connections.write().await.insert(
        "conn-rw".to_string(),
        stored_connection("conn-rw", "web.example.test", 22, json!({})),
    );
    let error = killed_state
        .call_tool(
            "ssh_exec",
            &json!({
                "connectionId": "conn-rw",
                "command": "shutdown -h now",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("Refused on read-only"),
        "process kill switch must dominate the connection flag, got: {error}"
    );
    // 同开关下白名单巡检命令仍可用（fail-closed 不等于全拒）：错误来自
    // 拨号层而不是任何一道门。
    let error = killed_state
        .call_tool(
            "ssh_exec",
            &json!({ "connectionId": "conn-rw", "command": "uptime" }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        !error.contains("read-only")
            && !error.contains("not recognized")
            && !error.contains("confirmDestructive"),
        "uptime must pass the gates and fail at dialing, got: {error}"
    );

    // 组合 8：connectionScope 非空 × 内联凭据 × 灾难命令 → scope 是
    // fail-closed 的第一道闸，报 scope 而非 confirm 提示。
    let scoped_state = state();
    scoped_state
        .settings_set(&json!({ "connectionScope": ["conn-rw"] }))
        .unwrap();
    let error = scoped_state
        .call_tool(
            "ssh_exec",
            &json!({
                "host": "web.example.test", "username": "deploy",
                "command": "shutdown -h now", "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("connectionScope"),
        "scope must refuse inline dials before the destructive gate, got: {error}"
    );
}

/// 组合矩阵补充：terminal_input 的只读 × 灾难 × sudo 白名单三闸叠加，
/// 以及灾难 + 确认后仍受白名单约束（确认位只解锁灾难门一层）。
#[tokio::test]
async fn terminal_input_combo_gates_stack_in_order() {
    let state = state();
    state.dbx_connections.write().await.insert(
        "conn-ti".to_string(),
        stored_connection(
            "conn-ti",
            "web.example.test",
            22,
            json!({ "external_config": { "sudo_whitelist": "systemctl restart nginx" } }),
        ),
    );
    // 灾难行 + confirmDestructive → 过灾难门，但 sudo 行仍受白名单拒绝。
    let error = state
        .call_tool(
            "ssh_terminal_input",
            &json!({
                "connectionId": "conn-ti",
                "input": "sudo reboot\r",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("not allowed by this connection's whitelist"),
        "confirm must not lift the sudo allowlist, got: {error}"
    );
    // 白名单命中的 sudo 行 + 确认 → 两闸皆过，落到会话引导错误。
    let error = state
        .call_tool(
            "ssh_terminal_input",
            &json!({
                "connectionId": "conn-ti",
                "input": "sudo systemctl restart nginx\r",
                "confirmDestructive": true,
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("terminal"),
        "allowed sudo line must pass both gates, got: {error}"
    );
}
