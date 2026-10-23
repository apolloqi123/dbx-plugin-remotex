use super::*;

#[test]
fn frames_carry_big_endian_length() {
    assert_eq!(frame_packet(&[1, 2, 3]), vec![0, 0, 0, 3, 1, 2, 3]);
    assert_eq!(frame_packet(&[]), vec![0, 0, 0, 0]);
}

#[test]
fn init_requests_version_three() {
    let frame = build_init();
    assert_eq!(&frame[..4], &[0, 0, 0, 5]);
    assert_eq!(&frame[4..], &[FXP_INIT, 0, 0, 0, 3]);
}

#[test]
fn string_requests_carry_type_id_and_path_bytes() {
    let frame = build_string_request(FXP_STAT, 7, b"/tmp");
    // type + id + len + path
    assert_eq!(frame.len(), 4 + 1 + 4 + 4 + 4);
    assert_eq!(frame[4], FXP_STAT);
    assert_eq!(&frame[5..9], &7_u32.to_be_bytes());
    assert_eq!(&frame[9..13], &4_u32.to_be_bytes());
    assert_eq!(&frame[13..], b"/tmp");
    let frame = build_close(1, b"h");
    assert_eq!(frame[4], FXP_CLOSE);
    let frame = build_readdir(2, b"h");
    assert_eq!(frame[4], FXP_READDIR);
    let frame = build_read(3, b"h", 9, 100);
    assert_eq!(frame[4], FXP_READ);
    // type + id + handle len + handle 之后才是 offset/len。
    assert_eq!(&frame[14..22], &9_u64.to_be_bytes());
    assert_eq!(&frame[22..26], &100_u32.to_be_bytes());
}

/// 请求类型码对 draft-02 字面值逐一对表：防"常量与校验器共用同一错值"
/// 的自洽回归盲区（READDIR 曾误写 16=REALPATH，离线桩测不出，真机
/// sftp-server fatal 即通道 EOF）。
#[test]
fn request_type_codes_match_draft02_literals() {
    assert_eq!(FXP_INIT, 1_u8);
    assert_eq!(FXP_VERSION, 2_u8);
    assert_eq!(FXP_OPEN, 3_u8);
    assert_eq!(FXP_CLOSE, 4_u8);
    assert_eq!(FXP_READ, 5_u8);
    assert_eq!(FXP_WRITE, 6_u8);
    assert_eq!(FXP_LSTAT, 7_u8);
    assert_eq!(FXP_SETSTAT, 9_u8);
    assert_eq!(FXP_OPENDIR, 11_u8);
    assert_eq!(FXP_READDIR, 12_u8);
    assert_eq!(FXP_REMOVE, 13_u8);
    assert_eq!(FXP_MKDIR, 14_u8);
    assert_eq!(FXP_RMDIR, 15_u8);
    // 16 = REALPATH：客户端不发起该请求，无对应常量（曾误把 16 当 READDIR）。
    assert_eq!(FXP_STAT, 17_u8);
    assert_eq!(FXP_RENAME, 18_u8);
    assert_eq!(FXP_READLINK, 19_u8);
    assert_eq!(FXP_SYMLINK, 20_u8);
    assert_eq!(FXP_STATUS, 101_u8);
    assert_eq!(FXP_HANDLE, 102_u8);
    assert_eq!(FXP_DATA, 103_u8);
    assert_eq!(FXP_NAME, 104_u8);
    assert_eq!(FXP_ATTRS, 105_u8);
    // 读/列目录两大关键帧用字面值再钉一次（回归直接读帧字节）。
    assert_eq!(build_readdir(2, b"h")[4], 12_u8);
    assert_eq!(build_string_request(FXP_OPENDIR, 3, b"h")[4], 11_u8);
}

#[test]
fn response_header_splits_type_and_id() {
    let payload = [FXP_HANDLE, 0, 0, 0, 9, 1, b'h'];
    let (kind, id, body) = parse_response_header(&payload).unwrap();
    assert_eq!(kind, FXP_HANDLE);
    assert_eq!(id, Some(9));
    assert_eq!(body, &[1, b'h']);
    // VERSION 无 id。
    let payload = [FXP_VERSION, 0, 0, 0, 3];
    let (kind, id, body) = parse_response_header(&payload).unwrap();
    assert_eq!(kind, FXP_VERSION);
    assert_eq!(id, None);
    assert_eq!(body, &[0, 0, 0, 3]);
    assert!(parse_response_header(&[]).is_err());
    assert!(parse_response_header(&[FXP_STATUS, 0, 0]).is_err());
}

#[test]
fn attrs_decode_v3_layouts() {
    // flags=0：空 attrs。
    let (attrs, used) = parse_attrs(&0_u32.to_be_bytes()).unwrap();
    assert_eq!(attrs, RawAttrs::default());
    assert_eq!(used, 4);
    // size + permissions + acmodtime + 扩展跳过。
    let mut buf = Vec::new();
    let flags = ATTR_SIZE | ATTR_PERMISSIONS | ATTR_ACMODTIME | ATTR_EXTENDED;
    buf.extend_from_slice(&flags.to_be_bytes());
    buf.extend_from_slice(&4096_u64.to_be_bytes());
    buf.extend_from_slice(&0o100644_u32.to_be_bytes());
    buf.extend_from_slice(&100_u32.to_be_bytes()); // atime
    buf.extend_from_slice(&200_u32.to_be_bytes()); // mtime
    buf.extend_from_slice(&1_u32.to_be_bytes()); // 扩展数
    buf.extend_from_slice(&2_u32.to_be_bytes());
    buf.extend_from_slice(b"ex");
    buf.extend_from_slice(&1_u32.to_be_bytes());
    buf.extend_from_slice(b"v");
    let (attrs, used) = parse_attrs(&buf).unwrap();
    assert_eq!(attrs.size, Some(4096));
    assert_eq!(attrs.permissions, Some(0o100644));
    assert_eq!(attrs.mtime, Some(200));
    assert_eq!(used, buf.len());
    // uid/gid 跳过不影响后续字段（v3 字段顺序固定：size 先于 uid/gid）。
    let mut buf = Vec::new();
    buf.extend_from_slice(&(ATTR_UIDGID | ATTR_SIZE).to_be_bytes());
    buf.extend_from_slice(&7_u64.to_be_bytes());
    buf.extend_from_slice(&1000_u32.to_be_bytes());
    buf.extend_from_slice(&1000_u32.to_be_bytes());
    let (attrs, _) = parse_attrs(&buf).unwrap();
    assert_eq!(attrs.size, Some(7));
    // 截断拒绝。
    assert!(parse_attrs(&(ATTR_SIZE | ATTR_UIDGID).to_be_bytes()).is_err());
}

#[test]
fn name_entries_decode_raw_filename_bytes() {
    // 两个条目：latin-1 "caf\xe9" 与 UTF-8 "生产"。
    let mut body = Vec::new();
    body.extend_from_slice(&2_u32.to_be_bytes());
    body.extend_from_slice(&4_u32.to_be_bytes());
    body.extend_from_slice(b"caf\xe9");
    body.extend_from_slice(&0_u32.to_be_bytes()); // longname 空
    body.extend_from_slice(&ATTR_SIZE.to_be_bytes());
    body.extend_from_slice(&12_u64.to_be_bytes());
    body.extend_from_slice(&6_u32.to_be_bytes());
    body.extend_from_slice("生产".as_bytes());
    body.extend_from_slice(&0_u32.to_be_bytes());
    body.extend_from_slice(&ATTR_SIZE.to_be_bytes());
    body.extend_from_slice(&24_u64.to_be_bytes());
    let entries = parse_name_entries(&body).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, b"caf\xe9".to_vec());
    assert_eq!(entries[0].attrs.size, Some(12));
    assert_eq!(entries[1].name, "生产".as_bytes().to_vec());
    assert_eq!(entries[1].attrs.size, Some(24));
    // 截断拒绝。
    assert!(parse_name_entries(&2_u32.to_be_bytes()).is_err());
}

#[test]
fn status_handle_data_packets_parse() {
    assert_eq!(
        parse_status_code(&SSH_FX_EOF.to_be_bytes()).unwrap(),
        SSH_FX_EOF
    );
    assert_eq!(parse_handle(&[0, 0, 0, 1, b'h']).unwrap(), b"h".to_vec());
    let mut data = 3_u32.to_be_bytes().to_vec();
    data.extend_from_slice(b"abc");
    assert_eq!(parse_data(&data).unwrap(), b"abc".to_vec());
}

#[test]
fn rename_requests_carry_both_path_strings() {
    let frame = build_rename(9, b"/old\xE9", b"/new");
    // type + id + (len + path) * 2
    assert_eq!(frame.len(), 4 + 1 + 4 + 4 + 5 + 4 + 4);
    assert_eq!(frame[4], FXP_RENAME);
    assert_eq!(&frame[5..9], &9_u32.to_be_bytes());
    assert_eq!(&frame[9..13], &5_u32.to_be_bytes());
    assert_eq!(&frame[13..18], b"/old\xE9");
    assert_eq!(&frame[18..22], &4_u32.to_be_bytes());
    assert_eq!(&frame[22..], b"/new");
}

#[test]
fn encode_attrs_round_trips_the_parsed_subset() {
    // 空 attrs。
    assert_eq!(encode_attrs(&RawAttrs::default()), 0_u32.to_be_bytes());
    // 权限保留（上传提交）：只编码 permissions。
    let attrs = RawAttrs {
        permissions: Some(0o644 & 0o7777),
        ..RawAttrs::default()
    };
    let mut expected = (ATTR_PERMISSIONS).to_be_bytes().to_vec();
    expected.extend_from_slice(&0o644_u32.to_be_bytes());
    assert_eq!(encode_attrs(&attrs), expected);
    // touch 的 utimes 语义：atime/mtime 成对编码。
    let attrs = RawAttrs {
        atime: Some(100),
        mtime: Some(200),
        ..RawAttrs::default()
    };
    let mut expected = ATTR_ACMODTIME.to_be_bytes().to_vec();
    expected.extend_from_slice(&100_u32.to_be_bytes());
    expected.extend_from_slice(&200_u32.to_be_bytes());
    assert_eq!(encode_attrs(&attrs), expected);
    // 解析↔编码在全部字段上闭环。
    let mut buf = Vec::new();
    let flags = ATTR_SIZE | ATTR_PERMISSIONS | ATTR_ACMODTIME;
    buf.extend_from_slice(&flags.to_be_bytes());
    buf.extend_from_slice(&9_u64.to_be_bytes());
    buf.extend_from_slice(&0o100755_u32.to_be_bytes());
    buf.extend_from_slice(&11_u32.to_be_bytes());
    buf.extend_from_slice(&22_u32.to_be_bytes());
    let (attrs, _) = parse_attrs(&buf).unwrap();
    assert_eq!(attrs.size, Some(9));
    assert_eq!(attrs.permissions, Some(0o100755));
    assert_eq!(attrs.atime, Some(11));
    assert_eq!(attrs.mtime, Some(22));
    assert_eq!(encode_attrs(&attrs), buf);
}

#[test]
fn write_side_packet_builders_carry_expected_layouts() {
    // OPEN：type + id + len+path + pflags + attrs(flags=0)。
    let frame = build_open(
        5,
        b"/up/a\xE9.bin",
        PFLAGS_WRITE | PFLAGS_CREAT | PFLAGS_TRUNC,
        &RawAttrs::default(),
    );
    assert_eq!(frame[4], FXP_OPEN);
    assert_eq!(&frame[5..9], &5_u32.to_be_bytes());
    let path_len = 10_u32.to_be_bytes();
    assert_eq!(&frame[9..13], &path_len);
    assert_eq!(&frame[13..23], b"/up/a\xE9.bin");
    let pflags = (PFLAGS_WRITE | PFLAGS_CREAT | PFLAGS_TRUNC).to_be_bytes();
    assert_eq!(&frame[23..27], &pflags);
    assert_eq!(&frame[27..], &0_u32.to_be_bytes()); // 空 attrs
                                                    // WRITE：type + id + len+handle + offset + len+data。
    let frame = build_write(6, b"h1", 4096, b"abc");
    assert_eq!(frame[4], FXP_WRITE);
    assert_eq!(&frame[9..13], &2_u32.to_be_bytes());
    assert_eq!(&frame[13..15], b"h1");
    assert_eq!(&frame[15..23], &4096_u64.to_be_bytes());
    assert_eq!(&frame[23..27], &3_u32.to_be_bytes());
    assert_eq!(&frame[27..], b"abc");
    // SETSTAT：type + id + len+path + attrs。
    let frame = build_setstat(
        7,
        b"/tmp/f",
        &RawAttrs {
            permissions: Some(0o600),
            ..RawAttrs::default()
        },
    );
    assert_eq!(frame[4], FXP_SETSTAT);
    assert_eq!(&frame[9..13], &6_u32.to_be_bytes());
    assert_eq!(&frame[19..23], &ATTR_PERMISSIONS.to_be_bytes());
    assert_eq!(&frame[23..], &0o600_u32.to_be_bytes());
    // READLINK 复用字符串请求骨架。
    let frame = build_readlink(8, b"/lnk");
    assert_eq!(frame[4], FXP_READLINK);
    // SYMLINK：OpenSSH 次序，第一个字符串是 target。
    let frame = build_symlink(9, b"tgt\xE9", b"/lnk");
    assert_eq!(frame[4], FXP_SYMLINK);
    assert_eq!(&frame[9..13], &4_u32.to_be_bytes());
    assert_eq!(&frame[13..17], b"tgt\xE9");
    assert_eq!(&frame[17..21], &4_u32.to_be_bytes());
    assert_eq!(&frame[21..], b"/lnk");
}

/// 内存双工流的桩服务器：INIT 回 VERSION，随后每个请求按脚本回预置
/// 包体（自动回填请求帧里的 id）。纯内存往返，不连 SSH。
/// 脚本包体格式：`[type, id 占位 4 字节, 剩余包体]`。
async fn scripted_server(stream: tokio::io::DuplexStream, replies: Vec<Vec<u8>>) {
    test_support::scripted_server(stream, replies, None).await
}

fn status_body(code: u32) -> Vec<u8> {
    test_support::status_body(code)
}

fn attrs_body(size: u64, permissions: u32) -> Vec<u8> {
    test_support::attrs_body(size, permissions)
}

fn handle_body(value: &[u8]) -> Vec<u8> {
    test_support::handle_body(value)
}

fn data_body(data: &[u8]) -> Vec<u8> {
    test_support::data_body(data)
}

/// NAME 单条目回包（READLINK 用）：name + 空 longname + 空 attrs。
fn name_body_one(name: &[u8]) -> Vec<u8> {
    test_support::name_body_one(name)
}

#[tokio::test]
async fn write_ops_round_trip_through_scripted_replies() {
    let (client_side, server_side) = tokio::io::duplex(4096);
    tokio::spawn(scripted_server(
        server_side,
        vec![
            handle_body(b"h1"),        // open_write → HANDLE
            status_body(0),            // write → ok
            status_body(0),            // close → ok
            status_body(0),            // setstat → ok
            name_body_one(b"tgt\xE9"), // readlink → NAME
            status_body(2),            // symlink → 失败
        ],
    ));
    let mut client = RawSftp::init(client_side).await.unwrap();
    let handle = client.open_write(b"/up/a\xE9.bin").await.unwrap();
    assert_eq!(handle, b"h1".to_vec());
    client.write_chunk(&handle, 0, b"data").await.unwrap();
    client.close(&handle).await.unwrap();
    client
        .setstat(
            b"/up/a\xE9.bin",
            &RawAttrs {
                permissions: Some(0o600),
                ..RawAttrs::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(client.readlink(b"/lnk").await.unwrap(), b"tgt\xE9".to_vec());
    let error = client.symlink(b"tgt", b"/lnk2").await.unwrap_err();
    assert!(error.contains("status 2"), "{error}");
    // WRITE 单包超限在客户端侧直接拒绝（不发出请求）。
    let oversized = vec![0_u8; MAX_WRITE_CHUNK + 1];
    assert!(client.write_chunk(b"h", 0, &oversized).await.is_err());
}

#[tokio::test]
async fn write_ops_pair_requests_with_scripted_replies() {
    let (client_side, server_side) = tokio::io::duplex(4096);
    tokio::spawn(scripted_server(
        server_side,
        vec![
            status_body(0),          // mkdir ok
            status_body(11),         // remove failed (SSH_FX_FAILURE-ish)
            attrs_body(7, 0o040755), // lstat → 目录
            status_body(0),          // rmdir ok
            status_body(0),          // rename ok
        ],
    ));
    let mut client = RawSftp::init(client_side).await.unwrap();

    client.mkdir(b"/a/b").await.unwrap();
    let error = client.remove(b"/a/b/\xE9.txt").await.unwrap_err();
    assert!(error.contains("status 11"), "{error}");
    let attrs = client.lstat(b"/a/b").await.unwrap();
    assert_eq!(attrs.size, Some(7));
    assert_eq!(attrs.permissions, Some(0o040755));
    client.rmdir(b"/a/b").await.unwrap();
    client.rename(b"/old\xE9", b"/new").await.unwrap();
}

#[tokio::test]
async fn unexpected_replies_surface_as_errors() {
    let (client_side, server_side) = tokio::io::duplex(4096);
    tokio::spawn(scripted_server(
        server_side,
        vec![attrs_body(1, 0o100644)], // remove 收到 ATTRS → 异常回包
    ));
    let mut client = RawSftp::init(client_side).await.unwrap();
    let error = client.remove(b"/a").await.unwrap_err();
    assert!(error.contains("unexpected reply"), "{error}");
}

#[tokio::test]
async fn read_file_loops_reads_until_cap_and_sends_raw_path_bytes() {
    let requests = test_support::request_log();
    let (client_side, server_side) = tokio::io::duplex(4096);
    tokio::spawn(test_support::scripted_server(
        server_side,
        vec![
            handle_body(b"h1"), // OPEN(READ)
            data_body(b"ab"),   // READ#1
            data_body(b"cde"),  // READ#2 → 累计达到 cap
            status_body(0),     // CLOSE
        ],
        Some(requests.clone()),
    ));
    let mut client = RawSftp::init(client_side).await.unwrap();
    let data = client.read_file(b"/d/caf\xE9.txt", 0, 5).await.unwrap();
    assert_eq!(data, b"abcde".to_vec());
    // INIT 之后的 OPEN 帧携带原始路径字节（latin-1 往返的字节级证据）：
    // payload = type + id + path_len + path + pflags + attrs。
    let open = &test_support::recorded_requests(&requests)[1];
    assert_eq!(open[0], FXP_OPEN);
    let path = b"/d/caf\xE9.txt";
    assert_eq!(&open[9..9 + path.len()], &path[..]);
    // READ 帧 offset 从 0 推进。
    let read = &test_support::recorded_requests(&requests)[2];
    assert_eq!(read[0], FXP_READ);
    assert_eq!(&read[11..19], &0_u64.to_be_bytes());
}

#[tokio::test]
async fn read_file_stops_on_eof_and_honors_offset() {
    let requests = test_support::request_log();
    let (client_side, server_side) = tokio::io::duplex(4096);
    tokio::spawn(test_support::scripted_server(
        server_side,
        vec![
            handle_body(b"h1"),      // OPEN(READ)
            data_body(b"xy"),        // READ#1（短读但未到 cap）
            status_body(SSH_FX_EOF), // READ#2 → EOF 收尾
            status_body(0),          // CLOSE
        ],
        Some(requests.clone()),
    ));
    let mut client = RawSftp::init(client_side).await.unwrap();
    let data = client.read_file(b"/f", 100, 10).await.unwrap();
    assert_eq!(data, b"xy".to_vec());
    // READ#1 的 offset 显式携带调用方给的起点。
    let read = &test_support::recorded_requests(&requests)[2];
    assert_eq!(&read[11..19], &100_u64.to_be_bytes());
}

#[test]
fn error_status_extracts_codes_from_error_strings() {
    assert_eq!(
        error_status("SFTP raw lstat failed with status 2"),
        Some(SSH_FX_NO_SUCH_FILE)
    );
    assert_eq!(
        error_status("SFTP raw read failed with status 11"),
        Some(11)
    );
    assert_eq!(error_status("SFTP raw read failed: boom"), None);
    assert_eq!(error_status(""), None);
}

/// CI run 36146663371 根因回归：MKDIR 帧必须携带 ATTRS 字段
/// （draft-ietf-secsh-filexfer-02 §5.2）。修复前的帧（path 后即截断）
/// 让 OpenSSH sftp-server 的 `decode_attrib` fatal 退出。此处逐字节
/// 锁定修复后的帧布局。
#[test]
fn mkdir_request_carries_attrs_field() {
    let frame = build_mkdir(4, b"/d/n\xE9", &RawAttrs::default());
    // len = type(1) + id(4) + path_len(4) + path(5) + attrs flags(4) = 18。
    assert_eq!(
        frame,
        vec![
            0, 0, 0, 18, // 帧长
            FXP_MKDIR, 0, 0, 0, 4, // type + id
            0, 0, 0, 5, b'/', b'd', b'/', b'n', 0xE9, // len + path 原始字节
            0, 0, 0, 0, // ATTRS flags=0（空 attrs）
        ]
    );
    // 修复前的错误形态（build_string_request 装的 MKDIR，缺 attrs）必须
    // 被严格桩拒绝——CI "early eof" 的离线复现锚点。
    let malformed = build_string_request(FXP_MKDIR, 4, b"/d/n\xE9");
    assert_ne!(frame, malformed);
    assert!(test_support::validate_request(&malformed[4..]).is_err());
    assert!(test_support::validate_request(&frame[4..]).is_ok());
}

/// 严格桩一致性回归：客户端所有操作构造器产出的帧都必须恰好通过
/// draft-02 布局校验；典型畸形帧（截断/拖尾/缺字段）必须被拒绝。
#[test]
fn strict_validator_accepts_all_client_frames_and_rejects_malformed() {
    let attrs = RawAttrs {
        permissions: Some(0o644),
        ..RawAttrs::default()
    };
    let frames = vec![
        build_init(),
        build_open(1, b"/a", PFLAGS_READ, &RawAttrs::default()),
        build_close(2, b"h"),
        build_read(3, b"h", 5, 10),
        build_write(4, b"h", 5, b"abc"),
        build_string_request(FXP_LSTAT, 5, b"/a"),
        build_setstat(6, b"/a", &attrs),
        build_string_request(FXP_OPENDIR, 7, b"/a"),
        build_string_request(FXP_REMOVE, 8, b"/a"),
        build_mkdir(9, b"/a", &RawAttrs::default()),
        build_string_request(FXP_RMDIR, 10, b"/a"),
        build_readdir(11, b"h"),
        build_string_request(FXP_STAT, 12, b"/a"),
        build_rename(13, b"/a", b"/b"),
        build_readlink(14, b"/a"),
        build_symlink(15, b"t", b"/l"),
    ];
    for frame in &frames {
        let payload = &frame[4..];
        if let Err(error) = test_support::validate_request(payload) {
            panic!(
                "client frame rejected by strict stub ({error}); hexdump: {}",
                test_support::hexdump(payload)
            );
        }
    }
    // 截断（id 都不完整）/ INIT 拖尾 / STAT 拖尾 / READ 缺 offset / MKDIR
    // 缺 attrs。
    let v = test_support::validate_request;
    assert!(v(&[FXP_STAT, 0, 0]).is_err());
    assert!(v(&[FXP_INIT, 0, 0, 0, 3, 0]).is_err());
    assert!(v(
        &build_string_request(FXP_STAT, 1, b"/a")[4..] // 拖尾一个字节
            .iter()
            .copied()
            .chain([0_u8])
            .collect::<Vec<_>>()
    )
    .is_err());
    let mut short_read = build_read(1, b"h", 0, 8)[4..].to_vec();
    short_read.truncate(short_read.len() - 8); // 掐掉 offset+len
    assert!(v(&short_read).is_err());
    assert!(v(&build_string_request(FXP_MKDIR, 1, b"/a")[4..]).is_err());
}

#[tokio::test]
async fn mkdir_round_trips_through_strict_stub() {
    let (client_side, server_side) = tokio::io::duplex(4096);
    tokio::spawn(scripted_server(server_side, vec![status_body(0)]));
    let mut client = RawSftp::init(client_side).await.unwrap();
    client.mkdir(b"/d/n\xE9").await.unwrap();
}

/// CI run 36146663371 的离线复现：OpenSSH 语义桩（解析失败即 fatal
/// 断流）收到修复前的畸形 MKDIR（path 后无 attrs）时，客户端读响应
/// 得到 "early eof"——与真机 smoke 的错误串逐字一致。
#[tokio::test]
async fn attrless_mkdir_reproduces_ci_early_eof_against_openssh_fatal_stub() {
    let (client_side, server_side) = tokio::io::duplex(4096);
    tokio::spawn(test_support::openssh_fatal_server(server_side, vec![]));
    let mut client = RawSftp::init(client_side).await.unwrap();
    // 修复前的错误帧形态：MKDIR 用 build_string_request 装（缺 ATTRS）。
    client
        .send(&build_string_request(FXP_MKDIR, 99, b"/d/n\xE9"))
        .await
        .unwrap();
    let error = client.recv().await.unwrap_err();
    assert!(error.contains("early eof"), "{error}");
}
