use super::*;

#[tokio::test]
async fn dbx_bridge_registers_lifecycle_and_dispatches_tools() {
    let state = McpState::shared(Arc::new(SshRuntime::new(
        std::env::temp_dir().join("dbx-mcp-bridge-test"),
    )));
    let lifecycle = json!({
        "connection": {
            "id": "conn-ssh-1",
            "name": "Prod bastion",
            "host": "192.0.2.10",
            "port": 22,
            "username": "ops",
            "password": "secret",
            "external_config": { "authentication": "password", "quick_sudo": true },
            "connection_secrets": { "totp_secret": "JBSWY3DPEHPK3PXP" },
        },
        "runtime": { "host": "192.0.2.10", "port": 22 },
        "operationId": "op-1",
    });

    // A tool that needs no connection runs right away.
    let listed = state
        .call_dbx_with(
            &json!({ "tool": "ssh_list_known_hosts", "arguments": {}, "lifecycle": lifecycle }),
            None,
        )
        .await
        .unwrap();
    assert!(listed["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("knownHosts"));

    // The lifecycle connection is registered: a connection-bound tool now
    // gets past credential resolution (it fails on the missing command
    // parameter, proving connectionId routing works end to end).
    let err = state
        .call_dbx_with(&json!({ "tool": "ssh_exec", "arguments": {} }), None)
        .await
        .err()
        .unwrap();
    assert!(err.contains("command"), "unexpected error: {err}");
}

#[tokio::test]
async fn dbx_bridge_requires_complete_lifecycle_payloads() {
    let state = McpState::shared(Arc::new(SshRuntime::new(
        std::env::temp_dir().join("dbx-mcp-bridge-test-2"),
    )));
    let err = state
        .call_dbx_with(
            &json!({
                "tool": "ssh_exec",
                "arguments": { "command": "true" },
                "lifecycle": { "connection": { "id": "x", "username": "u", "password": "p" } },
            }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(err.contains("host"), "unexpected error: {err}");
}

/// L3: a lifecycle-registered connection is resolvable by its display
/// name — the call gets past the registry lookup and fails on the
/// missing tool parameter instead, proving name → connection routing.
#[tokio::test]
async fn connection_name_resolves_through_registry() {
    let mut state = McpState::shared(Arc::new(SshRuntime::new(
        std::env::temp_dir().join("dbx-mcp-name-test"),
    )));
    // Hermetic: keep the name-miss path off the real app bridge.
    state.bridge_fallback = false;
    let lifecycle = json!({
        "connection": {
            "id": "conn-named",
            "name": "Prod bastion",
            "host": "192.0.2.10",
            "port": 22,
            "username": "ops",
            "password": "secret",
        },
        "runtime": { "host": "192.0.2.10", "port": 22 },
        "operationId": "op-name",
    });
    let listed = state
        .call_dbx_with(
            &json!({ "tool": "ssh_list_known_hosts", "arguments": {}, "lifecycle": lifecycle }),
            None,
        )
        .await
        .unwrap();
    assert!(listed["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("knownHosts"));

    // connectionName (exact, as stored) reaches the tool's own parameter
    // validation: the registry lookup passed.
    let err = state
        .call_dbx_with(
            &json!({
                "tool": "ssh_exec",
                "arguments": { "connectionName": "Prod bastion" },
            }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(err.contains("command"), "unexpected error: {err}");

    // An unknown name gets the self-heal guidance (no bridge available
    // in tests is the same degradation as an older app).
    let err = state
        .call_dbx_with(
            &json!({
                "tool": "ssh_exec",
                "arguments": { "connectionName": "ghost", "command": "true" },
            }),
            None,
        )
        .await
        .err()
        .unwrap();
    assert!(
        err.contains("No connection named 'ghost'") && err.contains("ssh_list_connections"),
        "unexpected error: {err}"
    );
}

// —— M17：MCP 工具面文件名编码模式 ——

#[test]
fn mcp_encoding_prefers_connection_override_then_global() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let store = preferences::store_path(data_dir.path());
    // 只设连接覆盖：命中覆盖 → latin-1；未命中的连接 → 全局缺省 auto。
    std::fs::write(
        &store,
        r#"{"prefs":{"sftp_name_encoding_overrides":{"conn-1":"latin-1"}}}"#,
    )
    .expect("write prefs");
    assert_eq!(
        mcp_sftp_encoding(data_dir.path(), &json!({ "connectionId": "conn-1" })),
        sftp_name::NameEncoding::Latin1
    );
    assert_eq!(
        mcp_sftp_encoding(data_dir.path(), &json!({ "connectionId": "conn-2" })),
        sftp_name::NameEncoding::Auto
    );
    // 全局 latin-1：内联拨号（无 connectionId，dispatch 层也无法归一化）
    // 按未覆盖处理，跟随全局。
    std::fs::write(&store, r#"{"prefs":{"sftp_name_encoding":"latin-1"}}"#).expect("write prefs");
    assert_eq!(
        mcp_sftp_encoding(data_dir.path(), &json!({})),
        sftp_name::NameEncoding::Latin1
    );
    // 连接覆盖压过全局偏好（优先级链第三态）。
    std::fs::write(
        &store,
        r#"{"prefs":{"sftp_name_encoding":"latin-1","sftp_name_encoding_overrides":{"conn-9":"auto"}}}"#,
    )
    .expect("write prefs");
    assert_eq!(
        mcp_sftp_encoding(data_dir.path(), &json!({ "connectionId": "conn-9" })),
        sftp_name::NameEncoding::Auto
    );
}

#[test]
fn raw_list_items_display_paths_round_trip_to_server_bytes() {
    let entry = |name: &[u8], size: Option<u64>, permissions: Option<u32>| sftp_raw::RawEntry {
        name: name.to_vec(),
        attrs: sftp_raw::RawAttrs {
            size,
            permissions,
            mtime: Some(100),
            ..sftp_raw::RawAttrs::default()
        },
    };
    let items = raw_list_items(
        "/data",
        vec![
            entry(b"caf\xe9.txt", Some(12), Some(0o100644)),
            entry(b"dir\xe9", None, Some(0o040755)),
            entry(b".", None, Some(0o040755)),
            entry(b"..", None, None),
            entry(b"\xff\xfe.bin", Some(3), None),
        ],
    );
    // `.`/`..` 跳过；名字口径 = 显示形式（latin-1 解码，忠实可逆）。
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["name"], json!("caf\u{e9}.txt"));
    assert_eq!(items[0]["path"], json!("/data/caf\u{e9}.txt"));
    assert_eq!(items[0]["kind"], json!("file"));
    assert_eq!(items[0]["size"], json!(12));
    assert_eq!(items[0]["modifiedAt"], json!(100));
    assert_eq!(items[1]["name"], json!("dir\u{e9}"));
    assert_eq!(items[1]["path"], json!("/data/dir\u{e9}"));
    assert_eq!(items[1]["kind"], json!("directory"));
    // attrs 缺 permissions（非标准服务器）退回 file，不误判成目录。
    assert_eq!(items[2]["name"], json!("\u{ff}\u{fe}.bin"));
    assert_eq!(items[2]["kind"], json!("file"));

    // 往返闭环：AI 把列表返回的 path 原样回传给 sftp_remove 等写工具时，
    // latin1_encode_display 精确还原服务器原始字节（显示 → 字节是 latin-1
    // 解码的逆变换，目录 ASCII 前缀按字面量透传）。
    assert_eq!(
        sftp_name::latin1_encode_display(items[0]["path"].as_str().unwrap()),
        b"/data/caf\xe9.txt".to_vec()
    );
    // 子目录导航同样闭环：父列表返回的显示目录路径进入下一次列表/
    // rename 目标组合时还原字节。
    assert_eq!(
        sftp_name::latin1_encode_display(items[1]["path"].as_str().unwrap()),
        b"/data/dir\xe9".to_vec()
    );
    // rename 往返：源 = 列表回传显示路径；目标 = 同目录 + 新输入显示文本
    //（latin-1 域内字符映回同值字节）。
    assert_eq!(
        sftp_name::latin1_encode_display(items[0]["path"].as_str().unwrap()),
        b"/data/caf\xe9.txt".to_vec()
    );
    assert_eq!(
        sftp_name::latin1_encode_display("/data/caf\u{e9}2.txt"),
        b"/data/caf\xe92.txt".to_vec()
    );
}

// —— M18：MCP 工具面剩余 SFTP 工具的 latin-1 往返闭环 ——

/// 带请求日志的内存桩裸包客户端（复用 `sftp_raw::test_support` 的
/// duplex 桩服务器，不连 SSH）。请求日志记录的是去帧 payload：
/// `[type, id 4 字节, 包体]`。
async fn stub_raw_sftp(
    replies: Vec<Vec<u8>>,
) -> (
    sftp_raw::RawSftp<tokio::io::DuplexStream>,
    sftp_raw::test_support::RequestLog,
) {
    let log = sftp_raw::test_support::request_log();
    let (client_side, server_side) = tokio::io::duplex(4096);
    tokio::spawn(sftp_raw::test_support::scripted_server(
        server_side,
        replies,
        Some(log.clone()),
    ));
    let client = sftp_raw::RawSftp::init(client_side).await.unwrap();
    (client, log)
}

#[tokio::test]
async fn raw_stat_maps_v3_attrs_and_lstat_carries_latin1_path_bytes() {
    let (mut client, log) = stub_raw_sftp(vec![sftp_raw::test_support::attrs_body_full(
        12,
        0o100644,
        Some(111),
        Some(222),
    )])
    .await;
    let path = "/data/caf\u{e9}.txt";
    let attrs = client
        .lstat(&sftp_name::latin1_encode_display(path))
        .await
        .unwrap();
    // 裸包 attrs 无 uid/gid：调用方 shell 查询尽力而为（此处按缺省
    // None 验证装配口径），主元数据不受影响。
    let stat = raw_stat_json(path, attrs, None, None);
    assert_eq!(stat["size"], json!(12));
    assert_eq!(stat["permissions"], json!("0644"));
    assert_eq!(stat["modifiedAt"], json!(222));
    assert_eq!(stat["accessedAt"], json!(111));
    assert_eq!(stat["uid"], Value::Null);
    assert_eq!(stat["gid"], Value::Null);
    // 字节级闭环：LSTAT 帧携带显示路径还原出的服务器原始字节。
    let lstat = &sftp_raw::test_support::recorded_requests(&log)[1];
    assert_eq!(lstat[0], 7); // FXP_LSTAT
    assert_eq!(&lstat[9..], b"/data/caf\xe9.txt");
    // 响应里的 path 原样回传工具面即落回原始字节（精确逆变换）。
    assert_eq!(
        sftp_name::latin1_encode_display(stat["path"].as_str().unwrap()),
        b"/data/caf\xe9.txt".to_vec()
    );
}

#[tokio::test]
async fn raw_sftp_read_file_round_trips_latin1_path_and_flags_truncation() {
    // 截断：max_bytes=5，服务器回 11 字节 → truncated=true + 截到 5。
    let (mut client, log) = stub_raw_sftp(vec![
        sftp_raw::test_support::handle_body(b"h1"),
        sftp_raw::test_support::data_body(b"hello world"),
        sftp_raw::test_support::status_body(0),
    ])
    .await;
    let (data, truncated) = raw_sftp_read_file(&mut client, "/d/caf\u{e9}.txt", 0, 5)
        .await
        .unwrap();
    assert!(truncated);
    assert_eq!(data, b"hello".to_vec());
    // OPEN 帧路径字节 = 显示路径的 latin1_encode_display 逆变换
    // （payload = type + id + path_len + path + pflags + attrs）。
    let open = &sftp_raw::test_support::recorded_requests(&log)[1];
    assert_eq!(open[0], 3); // FXP_OPEN
    assert_eq!(&open[9..20], b"/d/caf\xe9.txt");

    // 未截断 + offset 分页起点显式携带（READ 循环读到 EOF 收尾）。
    let (mut client, log) = stub_raw_sftp(vec![
        sftp_raw::test_support::handle_body(b"h1"),
        sftp_raw::test_support::data_body(b"rld"),
        sftp_raw::test_support::status_body(1), // SSH_FX_EOF：数据读完
        sftp_raw::test_support::status_body(0), // CLOSE
    ])
    .await;
    let (data, truncated) = raw_sftp_read_file(&mut client, "/d/f", 8, 100)
        .await
        .unwrap();
    assert!(!truncated);
    assert_eq!(data, b"rld".to_vec());
    let read = &sftp_raw::test_support::recorded_requests(&log)[2];
    assert_eq!(read[0], 5); // FXP_READ
    assert_eq!(&read[11..19], &8_u64.to_be_bytes());
}

#[tokio::test]
async fn raw_sftp_write_file_chunks_at_32kib_and_round_trips_latin1_path() {
    let content = vec![0xA9_u8; sftp_raw::MAX_WRITE_CHUNK + 5];
    let (mut client, log) = stub_raw_sftp(vec![
        sftp_raw::test_support::status_body(2), // LSTAT 预检：目标不存在
        sftp_raw::test_support::handle_body(b"h1"),
        sftp_raw::test_support::status_body(0), // WRITE#1
        sftp_raw::test_support::status_body(0), // WRITE#2
        sftp_raw::test_support::status_body(0), // CLOSE
    ])
    .await;
    let written = raw_sftp_write_file(&mut client, "/d/caf\u{e9}.txt", &content, false)
        .await
        .unwrap();
    assert_eq!(written["bytes"], json!(content.len()));
    let requests = sftp_raw::test_support::recorded_requests(&log);
    // 覆盖预检与 OPEN 同一字节口径（显示路径 → 服务器原始字节）。
    let lstat = &requests[1];
    assert_eq!(lstat[0], 7); // FXP_LSTAT
    assert_eq!(&lstat[9..], b"/d/caf\xe9.txt");
    let open = &requests[2];
    assert_eq!(open[0], 3); // FXP_OPEN
    assert_eq!(&open[9..20], b"/d/caf\xe9.txt");
    // WRITE 按 32 KiB 切块、offset 显式推进（payload 坐标：type+id+
    // handle_len+handle 之后是 offset 8 字节）。
    let write1 = &requests[3];
    assert_eq!(write1[0], 6); // FXP_WRITE
    assert_eq!(&write1[11..19], &0_u64.to_be_bytes());
    let write2 = &requests[4];
    assert_eq!(
        &write2[11..19],
        &(sftp_raw::MAX_WRITE_CHUNK as u64).to_be_bytes()
    );
    assert_eq!(write2[23..].len(), 5);
}

#[tokio::test]
async fn raw_sftp_write_file_refuses_existing_target_without_overwrite() {
    let (mut client, _) =
        stub_raw_sftp(vec![sftp_raw::test_support::attrs_body(1, 0o100644)]).await;
    let error = raw_sftp_write_file(&mut client, "/d/caf\u{e9}.txt", b"x", false)
        .await
        .unwrap_err();
    assert!(error.contains("already exists"), "{error}");
}

#[tokio::test]
async fn raw_sftp_upload_round_trips_latin1_path_and_payload() {
    // M19 sftp_upload 裸包车道：覆盖预检 LSTAT（NO_SUCH_FILE）→ OPEN →
    // WRITE → CLOSE；OPEN 帧路径字节 = 显示路径 latin1_encode_display
    // 逆变换，载荷逐字节落 WRITE 帧（与 sftp_download 的往返闭环见
    // raw_sftp_upload_download_round_trip_closes_latin1_loop）。
    let payload = b"mcp upload \xE9 \xA9 payload".to_vec();
    let (mut client, log) = stub_raw_sftp(vec![
        sftp_raw::test_support::status_body(2), // LSTAT 预检：目标不存在
        sftp_raw::test_support::handle_body(b"h1"),
        sftp_raw::test_support::status_body(0), // WRITE
        sftp_raw::test_support::status_body(0), // CLOSE
    ])
    .await;
    let bytes = raw_sftp_write_bytes(&mut client, "/up/caf\u{e9}.bin", &payload, false)
        .await
        .unwrap();
    assert_eq!(bytes, payload.len());
    let requests = sftp_raw::test_support::recorded_requests(&log);
    // 覆盖预检与 OPEN 同一字节口径（显示路径 → 服务器原始字节）。
    let lstat = &requests[1];
    assert_eq!(lstat[0], 7); // FXP_LSTAT
    assert_eq!(&lstat[9..], b"/up/caf\xe9.bin");
    let open = &requests[2];
    assert_eq!(open[0], 3); // FXP_OPEN
    assert_eq!(&open[9..21], b"/up/caf\xe9.bin");
    // WRITE 帧尾部载荷逐字节一致（payload = type+id+handle_len+handle
    // +offset+data_len 之后是 data）。
    let write = &requests[3];
    assert_eq!(write[0], 6); // FXP_WRITE
    assert_eq!(&write[write.len() - payload.len()..], &payload[..]);
}

#[tokio::test]
async fn raw_sftp_download_round_trips_latin1_path_and_payload() {
    // M19 sftp_download 裸包车道：OPEN(READ) → READ → EOF → CLOSE；
    // OPEN 帧路径字节 = 显示路径 latin1_encode_display 逆变换，载荷
    // 逐字节回收（与 upload 的同路径闭环见下一条）。
    let payload = b"mcp download \xE9 \xA9 payload".to_vec();
    let (mut client, log) = stub_raw_sftp(vec![
        sftp_raw::test_support::handle_body(b"h1"),
        sftp_raw::test_support::data_body(&payload),
        sftp_raw::test_support::status_body(1), // SSH_FX_EOF：读完
        sftp_raw::test_support::status_body(0), // CLOSE
    ])
    .await;
    let (data, truncated) = raw_sftp_read_file(&mut client, "/up/caf\u{e9}.bin", 0, 4096)
        .await
        .unwrap();
    assert!(!truncated);
    assert_eq!(data, payload);
    let open = &sftp_raw::test_support::recorded_requests(&log)[1];
    assert_eq!(open[0], 3); // FXP_OPEN
    assert_eq!(&open[9..21], b"/up/caf\xe9.bin");
}

#[tokio::test]
async fn raw_sftp_upload_download_round_trip_closes_latin1_loop() {
    // M19 往返闭环：同一显示路径下，upload 的 OPEN 帧路径字节与
    // download 的 OPEN 帧路径字节完全一致（同一 latin1_encode_display
    // 逆变换），且上传载荷经下载侧逐字节回收——latin-1 域内显示 → 字节
    // → 显示精确闭环（duplex 桩先例，M18 同款）。
    let payload = b"mcp round trip \xE9 \xA9 payload".to_vec();
    let display_path = "/up/caf\u{e9}.bin";
    let raw_path = b"/up/caf\xe9.bin";

    // 前半程：upload（LSTAT 不存在 → OPEN → WRITE → CLOSE）。
    let (mut client, upload_log) = stub_raw_sftp(vec![
        sftp_raw::test_support::status_body(2),
        sftp_raw::test_support::handle_body(b"h1"),
        sftp_raw::test_support::status_body(0),
        sftp_raw::test_support::status_body(0),
    ])
    .await;
    let bytes = raw_sftp_write_bytes(&mut client, display_path, &payload, false)
        .await
        .unwrap();
    assert_eq!(bytes, payload.len());
    let upload_open = &sftp_raw::test_support::recorded_requests(&upload_log)[2];
    assert_eq!(upload_open[0], 3); // FXP_OPEN
    assert_eq!(&upload_open[9..9 + raw_path.len()], &raw_path[..]);

    // 后半程：download（OPEN → READ → EOF → CLOSE），同一显示路径。
    let (mut client, download_log) = stub_raw_sftp(vec![
        sftp_raw::test_support::handle_body(b"h1"),
        sftp_raw::test_support::data_body(&payload),
        sftp_raw::test_support::status_body(1), // SSH_FX_EOF：读完
        sftp_raw::test_support::status_body(0),
    ])
    .await;
    let (data, truncated) = raw_sftp_read_file(&mut client, display_path, 0, 4096)
        .await
        .unwrap();
    assert!(!truncated);
    assert_eq!(data, payload);
    let download_open = &sftp_raw::test_support::recorded_requests(&download_log)[1];
    assert_eq!(download_open[0], 3); // FXP_OPEN
                                     // 两侧 OPEN 帧路径字节一致：AI 把 upload 响应里的 remotePath 原样
                                     // 回传给 sftp_download 即命中同一组服务器字节。

    assert_eq!(
        &download_open[9..9 + raw_path.len()],
        &upload_open[9..9 + raw_path.len()]
    );
}
