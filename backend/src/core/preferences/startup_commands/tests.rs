use super::*;

#[test]
fn executes_only_for_plain_shell_sessions() {
    // 空串（含恰好空串）= 交互 shell，startup_commands 生效。
    assert!(executes_for(""));
    // 非空 remote_command 走 exec 替代 shell：语义冲突，永不注入。
    assert!(!executes_for("tail -f /var/log/syslog"));
    assert!(!executes_for(" "));
}

#[test]
fn parse_entry_defaults_and_clamps_delay() {
    // 缺省 delayMs → 300。
    assert_eq!(
        parse_entry(&json!({ "command": "cd /var/log" })),
        Some(StartupCommand {
            command: "cd /var/log".into(),
            delay_ms: 300
        })
    );
    // 超上限截断到 30s；字符串数字同样接受并钳制。
    assert_eq!(
        parse_entry(&json!({ "command": "x", "delayMs": 99_999 })),
        Some(StartupCommand {
            command: "x".into(),
            delay_ms: 30_000
        })
    );
    assert_eq!(
        parse_entry(&json!({ "command": "x", "delayMs": "5000" })),
        Some(StartupCommand {
            command: "x".into(),
            delay_ms: 5_000
        })
    );
    // 非法 delay 回默认；0 合法（不设等待）。
    assert_eq!(
        parse_entry(&json!({ "command": "x", "delayMs": "abc" })),
        Some(StartupCommand {
            command: "x".into(),
            delay_ms: 300
        })
    );
    assert_eq!(
        parse_entry(&json!({ "command": "x", "delayMs": 0 })),
        Some(StartupCommand {
            command: "x".into(),
            delay_ms: 0
        })
    );
    // 命令结尾的 CR/LF 剥掉（注入器自带回车），全空命令丢弃。
    assert_eq!(
        parse_entry(&json!({ "command": "echo hi\r\n" })),
        Some(StartupCommand {
            command: "echo hi".into(),
            delay_ms: 300
        })
    );
    assert_eq!(parse_entry(&json!({ "command": "\r\n" })), None);
    // 显式禁用丢弃。
    assert_eq!(
        parse_entry(&json!({ "command": "x", "enabled": false })),
        None
    );
    // 形状非法丢弃（命令缺失/非对象）。
    assert_eq!(parse_entry(&json!({ "delayMs": 10 })), None);
    assert_eq!(parse_entry(&json!("echo hi")), None);
}

#[test]
fn sanitize_entry_truncates_command_on_utf8_boundary() {
    // 4KiB 内原样保留；enabled 缺省 true。
    let kept = "x".repeat(MAX_COMMAND_BYTES);
    let sanitized = sanitize_entry(&json!({ "command": kept })).expect("kept");
    assert_eq!(
        sanitized["command"].as_str().unwrap().len(),
        MAX_COMMAND_BYTES
    );
    assert_eq!(sanitized["enabled"], true);
    assert_eq!(sanitized["delayMs"], 300);
    // 超长按字节截断；多字节字符不在边界上劈开。
    let multibyte = "界".repeat(MAX_COMMAND_BYTES); // 3 bytes each
    let sanitized = sanitize_entry(&json!({ "command": multibyte })).expect("truncated");
    let stored = sanitized["command"].as_str().unwrap();
    assert!(stored.len() <= MAX_COMMAND_BYTES);
    assert!(
        stored.len().is_multiple_of(3),
        "must land on a char boundary"
    );
    // 延迟钳制与 enabled 保留进落盘形状（编辑器要能回显禁用行）。
    let sanitized =
        sanitize_entry(&json!({ "command": "top", "delayMs": 1_000_000, "enabled": false }))
            .expect("disabled row kept");
    assert_eq!(sanitized["delayMs"], 30_000);
    assert_eq!(sanitized["enabled"], false);
    // 空命令不落盘。
    assert_eq!(sanitize_entry(&json!({ "command": "  " })), None);
    assert_eq!(sanitize_entry(&json!({ "no": "command" })), None);
}

#[test]
fn sanitize_store_shapes_and_caps() {
    // 非对象整体拒绝（save_preferences 报错给前端）。
    assert_eq!(sanitize_store(&json!(["x"])), None);
    assert_eq!(sanitize_store(&json!("nope")), None);
    let store = sanitize_store(&json!({
        "conn-a": {
            "enabled": true,
            "commands": [
                { "command": "echo one" },
                { "command": "echo two", "delayMs": 5, "enabled": false },
                "junk",
                { "command": "" },
            ],
        },
        "conn-b": "junk-entry",
    }))
    .expect("store");
    // conn-a：非法行丢弃、禁用行保留、enabled 透传。
    assert_eq!(store["conn-a"]["enabled"], true);
    let rows = store["conn-a"]["commands"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["command"], "echo one");
    assert_eq!(rows[1]["enabled"], false);
    // conn-b：形状非法的连接条目整条丢弃。
    assert!(store.get("conn-b").is_none());
    // enabled 缺省 false（开关默认关）。
    let store = sanitize_store(&json!({ "conn": { "commands": [] } })).expect("store");
    assert_eq!(store["conn"]["enabled"], false);
    // 每连接命令数上限。
    let many: Vec<Value> = (0..MAX_COMMANDS + 10)
        .map(|index| json!({ "command": format!("echo {index}") }))
        .collect();
    let store =
        sanitize_store(&json!({ "conn": { "enabled": true, "commands": many } })).expect("store");
    assert_eq!(
        store["conn"]["commands"].as_array().unwrap().len(),
        MAX_COMMANDS
    );
}

#[test]
fn active_commands_filter_by_master_switch_and_rows() {
    let store = json!({
        "on": { "enabled": true, "commands": [
            { "command": "first", "delayMs": 10 },
            { "command": "skipped", "enabled": false },
            { "command": "second" },
        ] },
        "off": { "enabled": false, "commands": [{ "command": "never" }] },
    });
    // 顺序保持、禁用行剔除。
    assert_eq!(
        active_commands(Some(&store), "on"),
        vec![
            StartupCommand {
                command: "first".into(),
                delay_ms: 10
            },
            StartupCommand {
                command: "second".into(),
                delay_ms: 300
            },
        ]
    );
    // 主开关关闭 → 空计划。
    assert!(active_commands(Some(&store), "off").is_empty());
    // 未知连接 / 缺 store → 空。
    assert!(active_commands(Some(&store), "other").is_empty());
    assert!(active_commands(None, "on").is_empty());
    assert!(active_commands(Some(&json!({})), "on").is_empty());
}

#[test]
fn active_commands_caps_read_path_at_max_commands() {
    // C1 回归：直读盘路径必须重申每连接 20 行上限——直接构造
    // preferences.json 不得注入任意行数（写路径本就 sanitize）。
    let many: Vec<Value> = (0..MAX_COMMANDS + 10)
        .map(|index| json!({ "command": format!("echo {index}") }))
        .collect();
    let store = json!({ "conn": { "enabled": true, "commands": many } });
    let plan = active_commands(Some(&store), "conn");
    assert_eq!(plan.len(), MAX_COMMANDS);
    // 截断保序：保留前 20 条，而不是后 20 条。
    assert_eq!(plan[0].command, "echo 0");
    assert_eq!(
        plan[MAX_COMMANDS - 1].command,
        format!("echo {}", MAX_COMMANDS - 1)
    );
}

#[test]
fn load_plan_reads_preferences_file_and_degrades_cleanly() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    // 无文件 → 空。
    assert!(load_plan(data_dir.path(), "conn").is_empty());
    // 正常落盘 → 读回（嵌套在 prefs 键下，与 preferences.json 布局一致）。
    std::fs::write(
        crate::core::preferences::store_path(data_dir.path()),
        json!({
            "version": 1,
            "prefs": { "startup_commands": {
                "conn": { "enabled": true, "commands": [{ "command": "echo hi", "delayMs": 25 }] },
            } },
        })
        .to_string(),
    )
    .expect("write prefs");
    assert_eq!(
        load_plan(data_dir.path(), "conn"),
        vec![StartupCommand {
            command: "echo hi".into(),
            delay_ms: 25
        }]
    );
    // 损坏文件 → 空而不是报错。
    std::fs::write(
        crate::core::preferences::store_path(data_dir.path()),
        "{junk",
    )
    .expect("write junk");
    assert!(load_plan(data_dir.path(), "conn").is_empty());
}

#[tokio::test]
async fn inject_sequence_types_commands_in_order_with_enter() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    let plan = vec![
        StartupCommand {
            command: "echo one".into(),
            delay_ms: 1,
        },
        StartupCommand {
            command: "echo two".into(),
            delay_ms: 1,
        },
    ];
    let worker = tokio::spawn(async move {
        inject_sequence(&plan, |payload| {
            let tx = tx.clone();
            async move { tx.send(payload).await.is_ok() }
        })
        .await
    });
    // 每条命令自带 \r 结尾，顺序与计划一致。
    assert_eq!(rx.recv().await.as_deref(), Some(&b"echo one\r"[..]));
    assert_eq!(rx.recv().await.as_deref(), Some(&b"echo two\r"[..]));
    assert!(worker.await.expect("injector"));
    // 通道关闭（会话关闭）→ 注入器停止并报未完成。
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
    drop(rx);
    let plan = vec![StartupCommand {
        command: "echo x".into(),
        delay_ms: 1,
    }];
    assert!(
        !inject_sequence(&plan, |payload| {
            let tx = tx.clone();
            async move { tx.send(payload).await.is_ok() }
        })
        .await
    );
}

#[tokio::test]
async fn inject_sequence_respects_the_delay_between_commands() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    // 两条各 40ms：总耗时 ≥ 80ms（首条前也等待，给 shell 起提示符留时间）。
    let plan = vec![
        StartupCommand {
            command: "a".into(),
            delay_ms: 40,
        },
        StartupCommand {
            command: "b".into(),
            delay_ms: 40,
        },
    ];
    let started = std::time::Instant::now();
    let worker = tokio::spawn(async move {
        inject_sequence(&plan, |payload| {
            let tx = tx.clone();
            async move { tx.send(payload).await.is_ok() }
        })
        .await
    });
    while rx.recv().await.is_some() {}
    assert!(worker.await.expect("injector"));
    assert!(
        started.elapsed() >= Duration::from_millis(80),
        "delays must hold"
    );
}

#[test]
fn input_payload_appends_enter() {
    assert_eq!(input_payload("cd /tmp"), b"cd /tmp\r".to_vec());
    assert_eq!(input_payload(""), b"\r".to_vec());
}
