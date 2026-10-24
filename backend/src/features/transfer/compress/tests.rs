use super::*;

fn no_progress(_: u64) {}

#[test]
fn decide_respects_every_fallback_condition() {
    use CompressPolicy::{Auto, Off};
    let big = 200 * 1024 * 1024;
    let cores = 8_u64; // 正常机器并行度
                       // 全绿 → 压缩。
    assert_eq!(
        decide(Auto, big, 64, Some("dump.sql"), true, false, false, cores),
        CompressionMode::Gzip
    );
    // 逐条回退：策略关 / 低于阈值 / 黑名单类型 / 远端缺工具 / 只读 / latin-1。
    assert_eq!(
        decide(Off, big, 64, Some("dump.sql"), true, false, false, cores),
        CompressionMode::None
    );
    assert_eq!(
        decide(
            Auto,
            64 * 1024 * 1024 - 1,
            64,
            Some("dump.sql"),
            true,
            false,
            false,
            cores
        ),
        CompressionMode::None
    );
    assert_eq!(
        decide(Auto, big, 64, Some("clip.mp4"), true, false, false, cores),
        CompressionMode::None
    );
    assert_eq!(
        decide(Auto, big, 64, Some("dump.sql"), false, false, false, cores),
        CompressionMode::None
    );
    assert_eq!(
        decide(Auto, big, 64, Some("dump.sql"), true, true, false, cores),
        CompressionMode::None
    );
    assert_eq!(
        decide(Auto, big, 64, Some("dump.sql"), true, false, true, cores),
        CompressionMode::None
    );
    // 阈值 0 = 不设下限；树模式 name=None 跳过黑名单。
    assert_eq!(
        decide(Auto, 1, 0, Some("a.txt"), true, false, false, cores),
        CompressionMode::Gzip
    );
    assert_eq!(
        decide(Auto, big, 64, None, true, false, false, cores),
        CompressionMode::Gzip
    );
    // 大小写不敏感的扩展名判定。
    assert_eq!(
        decide(Auto, big, 0, Some("Photo.JPG"), true, false, false, cores),
        CompressionMode::None
    );
}

#[test]
fn weak_cpu_gate_applies_to_auto_only() {
    use CompressPolicy::{Auto, On};
    let big = 200 * 1024 * 1024;
    // auto + ≤2 核：跳过压缩（弱 CPU 综合判断）。
    assert_eq!(
        decide(Auto, big, 0, Some("dump.sql"), true, false, false, 2),
        CompressionMode::None
    );
    assert_eq!(
        decide(Auto, big, 0, Some("dump.sql"), true, false, false, 1),
        CompressionMode::None
    );
    // 3 核放行；并行度未知（0）不启用门槛。
    assert_eq!(
        decide(Auto, big, 0, Some("dump.sql"), true, false, false, 3),
        CompressionMode::Gzip
    );
    assert_eq!(
        decide(Auto, big, 0, Some("dump.sql"), true, false, false, 0),
        CompressionMode::Gzip
    );
    // on 策略显式豁免 CPU 门槛（慢网快双核场景）。
    assert_eq!(
        decide(On, big, 0, Some("dump.sql"), true, false, false, 1),
        CompressionMode::Gzip
    );
}

#[test]
fn compress_policy_parses_wire_strings() {
    assert_eq!(CompressPolicy::parse("auto"), Some(CompressPolicy::Auto));
    assert_eq!(CompressPolicy::parse("on"), Some(CompressPolicy::On));
    assert_eq!(CompressPolicy::parse("off"), Some(CompressPolicy::Off));
    assert_eq!(CompressPolicy::parse("yes"), None);
    assert_eq!(CompressPolicy::Auto.as_str(), "auto");
    assert_eq!(CompressPolicy::On.as_str(), "on");
    assert_eq!(CompressPolicy::Off.as_str(), "off");
}

#[test]
fn rate_guard_error_is_recognizable() {
    let error = std::io::Error::other(RATE_GUARD_ERROR);
    assert!(is_rate_guard_error(&error));
    let other = std::io::Error::other("gzip boom");
    assert!(!is_rate_guard_error(&other));
    // 20 MiB/s 下限的换算口径：守卫检查本身在 compress_file_to 内闭环,
    // 这里只锁错误标记串的稳定性。
    assert_eq!(RATE_GUARD_ERROR, "compression-rate-guard");
}

#[test]
fn compressible_name_checks_last_extension_only() {
    assert!(is_compressible_name("backup.sql"));
    assert!(is_compressible_name("no-extension"));
    assert!(is_compressible_name("archive.data")); // 未知扩展名不拦
                                                   // 双扩展名以末段为准：.tar.gz / .sql.gz 都是压缩件。
    assert!(!is_compressible_name("dump.sql.gz"));
    assert!(!is_compressible_name("snapshot.tar.gz"));
    assert!(!is_compressible_name("MOVIE.MP4"));
    assert!(!is_compressible_name("lib.so.dll"));
}

#[test]
fn worthwhile_ratio_and_zero_original() {
    assert!(worthwhile(1000, 949));
    assert!(!worthwhile(1000, 950));
    assert!(!worthwhile(1000, 1000));
    // 4 GiB 级不溢出。
    assert!(worthwhile(4 * 1024 * 1024 * 1024, 1024 * 1024 * 1024));
    assert!(!worthwhile(0, 0));
}

#[test]
fn probe_and_remote_commands_quote_paths() {
    assert!(probe_command(false).contains("command -v gunzip"));
    assert!(probe_command(true).contains("command -v tar"));
    let quoted = remote_gzip_command("/data/my file's.sql", "/data/.dbx-x.gz");
    // 单引号转义成 '\''，路径整体在引号内。
    assert!(quoted.contains("'\\''"));
    assert!(quoted.starts_with("gzip -c '"));
    assert!(remote_gunzip_command("/a.gz", "/b").starts_with("gunzip -c '"));
    let tar = remote_tar_command("/data/my dir's", "/data/.dbx-t.tgz");
    assert!(tar.starts_with("tar -czf '"));
    assert!(tar.ends_with("-C '/data/my dir'\\''s' ."));
}

#[test]
fn shell_quote_wraps_and_escapes_single_quotes() {
    assert_eq!(shell_quote("plain"), "'plain'");
    assert_eq!(shell_quote("it's a test"), r"'it'\''s a test'");
    assert_eq!(shell_quote("back\\slash"), r"'back\slash'");
}

#[test]
fn gzip_roundtrip_preserves_bytes_and_reports_progress() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.bin");
    let compressed = dir.path().join("source.gz");
    let restored = dir.path().join("restored.bin");
    // 高冗余数据（压缩明显）+ 尾部唯一字节（往返保真断言）。
    let mut payload = Vec::new();
    for round in 0..4096 {
        payload
            .extend_from_slice(format!("line {round} of highly repetitive log data\n").as_bytes());
    }
    payload.extend_from_slice(b"tail-marker-0123456789");
    std::fs::write(&source, &payload).unwrap();

    let consumed = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter = consumed.clone();
    let compressed_size = compress_file_to(
        &source,
        &compressed,
        &AtomicBool::new(false),
        &move |bytes| counter.store(bytes, Ordering::Release),
        None,
    )
    .unwrap();
    assert!(
        compressed_size < payload.len() as u64 / 2,
        "repetitive data should halve"
    );
    assert_eq!(consumed.load(Ordering::Acquire), payload.len() as u64);

    let decompressed_size = decompress_file_to(
        &compressed,
        &restored,
        &AtomicBool::new(false),
        &no_progress,
    )
    .unwrap();
    assert_eq!(decompressed_size, payload.len() as u64);
    assert_eq!(std::fs::read(&restored).unwrap(), payload);
}

#[test]
fn compress_abort_cleans_partial_target() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.bin");
    let compressed = dir.path().join("source.gz");
    std::fs::write(&source, vec![b'x'; 1024]).unwrap();
    let cancelled = AtomicBool::new(true);
    let error = compress_file_to(&source, &compressed, &cancelled, &no_progress, None)
        .expect_err("cancelled run must fail");
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert!(!compressed.exists(), "partial target must be removed");
}

#[test]
fn decompress_corrupt_input_fails() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("bad.gz");
    let target = dir.path().join("bad.out");
    std::fs::write(&source, b"not gzip at all").unwrap();
    assert!(decompress_file_to(&source, &target, &AtomicBool::new(false), &no_progress).is_err());
}

#[test]
fn compression_mode_strings_are_wire_stable() {
    assert_eq!(CompressionMode::None.as_str(), "none");
    assert_eq!(CompressionMode::Gzip.as_str(), "gzip");
}

// 多成员 gz（MultiGzDecoder 语义）：解压侧对 `gzip` 命令流的多段输出
// 保持兼容，将来改坏 read 路径时这条测试兜底。
#[test]
fn multi_member_gzip_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let compressed = dir.path().join("multi.gz");
    let restored = dir.path().join("multi.out");
    let raw = std::fs::File::create(&compressed).unwrap();
    let mut encoder = flate2::write::GzEncoder::new(raw, flate2::Compression::default());
    encoder.write_all(b"member-one").unwrap();
    let raw = encoder.finish().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(raw, flate2::Compression::default());
    encoder.write_all(b"member-two").unwrap();
    encoder.finish().unwrap();
    decompress_file_to(
        &compressed,
        &restored,
        &AtomicBool::new(false),
        &no_progress,
    )
    .unwrap();
    assert_eq!(std::fs::read(&restored).unwrap(), b"member-onemember-two");
}

// tar.gz 解包：普通文件/目录落位、符号链接跳过（与普通树管线「不跟
// 随 symlink」语义对齐）、进度累计只算普通文件字节。
#[test]
fn extract_tar_gz_unpacks_regular_files_and_skips_links() {
    let dir = tempfile::tempdir().unwrap();
    let archive_path = dir.path().join("tree.tgz");
    let root = dir.path().join("staging");
    let file = std::fs::File::create(&archive_path).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);

    let log_data = b"2026-09-29 compressed tree entry\n";
    let mut header = tar::Header::new_gnu();
    header.set_size(log_data.len() as u64);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    builder
        .append_data(&mut header, "./logs/app.log", &log_data[..])
        .unwrap();

    let mut header = tar::Header::new_gnu();
    header.set_size(0);
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_cksum();
    builder
        .append_link(&mut header, "./link-to-log", "logs/app.log")
        .unwrap();

    let encoder = builder.into_inner().unwrap();
    encoder.finish().unwrap();

    let seen = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter = seen.clone();
    let extracted = extract_tar_gz(
        &archive_path,
        &root,
        &AtomicBool::new(false),
        &move |bytes| counter.store(bytes, Ordering::Release),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(root.join("logs/app.log")).unwrap(),
        b"2026-09-29 compressed tree entry\n"
    );
    // 符号链接成员被跳过，不落盘。
    assert!(root.join("link-to-log").symlink_metadata().is_err());
    assert_eq!(extracted, log_data.len() as u64);
    assert_eq!(seen.load(Ordering::Acquire), log_data.len() as u64);
}

// 路径穿越防护：`../` 成员条目必须被拒绝，不得逃出解包根。tar crate
// 在 append/set_path 层就拦 `..`，所以这里手搓原始 ustar 头字节构造
// 恶意归档，验证 unpack_in 的解包侧防线。
#[test]
fn extract_tar_gz_rejects_traversal_entries() {
    fn ustar_header(name: &str, size: u64) -> Vec<u8> {
        let mut header = vec![0_u8; 512];
        let name_bytes = name.as_bytes();
        header[..name_bytes.len()].copy_from_slice(name_bytes);
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        for byte in header[148..156].iter_mut() {
            *byte = b' ';
        }
        let sum: u32 = header.iter().map(|&byte| byte as u32).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        header
    }
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside.txt");
    let archive_path = dir.path().join("evil.tgz");
    let root = dir.path().join("staging");
    let raw = tempfile::tempdir().unwrap();
    let tar_path = raw.path().join("evil.tar");
    let mut tar_file = std::fs::File::create(&tar_path).unwrap();
    let data = b"escaped";
    tar_file
        .write_all(&ustar_header("../outside.txt", data.len() as u64))
        .unwrap();
    tar_file.write_all(data).unwrap();
    tar_file.write_all(&vec![0_u8; 512 - data.len()]).unwrap();
    tar_file.write_all(&[0_u8; 1024]).unwrap();
    drop(tar_file);
    let mut encoder = flate2::write::GzEncoder::new(
        std::fs::File::create(&archive_path).unwrap(),
        flate2::Compression::default(),
    );
    std::io::copy(&mut std::fs::File::open(&tar_path).unwrap(), &mut encoder).unwrap();
    encoder.finish().unwrap();
    // unpack_in 对 `..` 条目是静默跳过（Ok）而非报错——安全性质是
    // 「不得逃逸」，被跳过成员在树管线里会作为缺失文件入账。
    let extracted =
        extract_tar_gz(&archive_path, &root, &AtomicBool::new(false), &no_progress).unwrap();
    assert_eq!(extracted, 0, "skipped entry contributes no bytes");
    assert!(
        !outside.exists(),
        "traversal entry must not escape the root"
    );
    assert!(!root.join("outside.txt").exists());
}
