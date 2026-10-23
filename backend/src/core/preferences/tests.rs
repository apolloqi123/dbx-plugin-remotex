use super::*;

#[test]
fn local_shell_cwd_and_env_whitelist_and_roundtrip() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    // cwd roundtrip：裁剪存储；空串 = 清除（load 不再返回）；非字符串拒绝。
    save_preferences(
        data_dir.path(),
        &json!({ "localShellCwd": "  /tmp/work  " }),
    )
    .unwrap();
    assert_eq!(
        load_preferences(data_dir.path())["localShellCwd"],
        json!("/tmp/work")
    );
    save_preferences(data_dir.path(), &json!({ "localShellCwd": "" })).unwrap();
    assert!(load_preferences(data_dir.path())
        .get("localShellCwd")
        .is_none());
    assert!(save_preferences(data_dir.path(), &json!({ "localShellCwd": 3 })).is_err());

    // env roundtrip：缺 value 视为空值；坏条目（空名/含=/托管键/超长值）
    // 静默丢弃，好条目保留。
    save_preferences(
        data_dir.path(),
        &json!({ "localShellEnv": [
            {"name": "EMPTY_OK"},
            {"name": "DBX_WORK", "value": "1"},
            {"name": "", "value": "drop"},
            {"name": "A=B", "value": "drop"},
            {"name": "TERM", "value": "drop"},
            {"name": "TOO_LONG", "value": "x".repeat(4097)},
            {"value": "no name"},
        ]}),
    )
    .unwrap();
    assert_eq!(
        load_preferences(data_dir.path())["localShellEnv"],
        json!([
            { "name": "EMPTY_OK", "value": "" },
            { "name": "DBX_WORK", "value": "1" },
        ])
    );
    // 非数组 env 整批拒绝。
    assert!(save_preferences(data_dir.path(), &json!({ "localShellEnv": "x" })).is_err());
}

#[test]
fn wallpaper_roundtrip_validates_magic_and_size() {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine as _;

    let data_dir = tempfile::tempdir().expect("tempdir");
    // 空存储：get 返回空对象。
    assert_eq!(load_wallpaper(data_dir.path()), json!({}));
    // png 魔数（1x1 透明 png 文件头即可，不做像素解码）。
    let png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
    let saved = save_wallpaper(data_dir.path(), &json!({ "imageBase64": B64.encode(&png) }))
        .expect("save png");
    let data_url = saved["dataUrl"].as_str().unwrap();
    assert!(data_url.starts_with("data:image/png;base64,"));
    assert_eq!(load_wallpaper(data_dir.path()), saved);
    // jpeg / webp 魔数同样接受。
    save_wallpaper(
        data_dir.path(),
        &json!({ "imageBase64": B64.encode([0xFF, 0xD8, 0xFF, 0xE0]) }),
    )
    .expect("save jpeg");
    assert!(load_wallpaper(data_dir.path())["dataUrl"]
        .as_str()
        .unwrap()
        .starts_with("data:image/jpeg;base64,"));
    let webp: Vec<u8> = [b"RIFF".as_slice(), &[1, 2, 3, 4], b"WEBP"].concat();
    save_wallpaper(data_dir.path(), &json!({ "imageBase64": B64.encode(webp) }))
        .expect("save webp");
    assert!(load_wallpaper(data_dir.path())["dataUrl"]
        .as_str()
        .unwrap()
        .starts_with("data:image/webp;base64,"));
    // 非法魔数拒绝且不落盘污染（仍是上一张 webp）。
    let error = save_wallpaper(
        data_dir.path(),
        &json!({ "imageBase64": B64.encode(b"GIF89a....") }),
    )
    .expect_err("must reject non-image");
    assert!(error.contains("png, jpeg or webp"));
    // 非法 base64 拒绝；缺参/非字符串拒绝。
    assert!(save_wallpaper(data_dir.path(), &json!({ "imageBase64": "!!!" })).is_err());
    assert!(save_wallpaper(data_dir.path(), &json!({})).is_err());
    // 大小上限：>8MiB 的二进制拒绝（粗判在 base64 长度上先触发）。
    let oversized = vec![0u8; WALLPAPER_MAX_BYTES + 1];
    let error = save_wallpaper(
        data_dir.path(),
        &json!({ "imageBase64": B64.encode(&oversized) }),
    )
    .expect_err("must reject oversize");
    assert!(error.contains("limit"));
    // clear 幂等：清除后 get 回空对象，再次 clear 仍成功。
    assert_eq!(clear_wallpaper(data_dir.path()), json!({}));
    assert_eq!(load_wallpaper(data_dir.path()), json!({}));
    assert_eq!(clear_wallpaper(data_dir.path()), json!({}));
}

#[test]
fn wallpaper_prefs_roundtrip_with_clamp() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    // 空偏好：两键都不出现（前端按缺省处理）。
    let prefs = load_preferences(data_dir.path());
    assert!(prefs.get("wallpaper_enabled").is_none());
    assert!(prefs.get("wallpaper_opacity").is_none());
    save_preferences(
        data_dir.path(),
        &json!({ "wallpaper_enabled": true, "wallpaper_opacity": 999 }),
    )
    .expect("save");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(prefs["wallpaper_enabled"], true);
    assert_eq!(prefs["wallpaper_opacity"], 90);
    assert!(save_preferences(data_dir.path(), &json!({ "wallpaper_enabled": "yes" })).is_err());
}

#[test]
fn local_terminal_prefs_roundtrip_with_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    // 空偏好：两键都不出现（前端按缺省处理）。
    let prefs = load_preferences(dir.path());
    assert!(prefs.get("localShell").is_none());
    assert!(prefs.get("localShellIntegration").is_none());
    // 写入 + 读回；空串 shell 归一为空串（=自动探测）。
    save_preferences(
        dir.path(),
        &serde_json::json!({ "localShell": "  /opt/homebrew/bin/fish  ", "localShellIntegration": false }),
    )
    .expect("save");
    let prefs = load_preferences(dir.path());
    assert_eq!(prefs["localShell"], "/opt/homebrew/bin/fish");
    assert_eq!(prefs["localShellIntegration"], false);
    // 非法类型报错且不落盘污染。
    let error = save_preferences(dir.path(), &serde_json::json!({ "localShell": 42 }))
        .expect_err("must reject");
    assert!(error.contains("localShell"));
}

#[test]
fn roundtrip_merges_and_normalizes() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stored = save_preferences(
        data_dir.path(),
        &json!({ "downloadDir": "  /tmp/dl  ", "downloadUseDefaultDir": true, "evil": "dropped" }),
    )
    .expect("save");
    assert_eq!(stored["downloadDir"].as_str().unwrap(), "/tmp/dl");
    assert_eq!(stored["downloadUseDefaultDir"].as_bool(), Some(true));
    assert!(stored.get("evil").is_none());
    // 部分更新只改出现的键。
    let again = save_preferences(data_dir.path(), &json!({ "downloadUseDefaultDir": false }))
        .expect("save again");
    assert_eq!(again["downloadDir"].as_str().unwrap(), "/tmp/dl");
    assert_eq!(again["downloadUseDefaultDir"].as_bool(), Some(false));
}

#[test]
fn missing_or_corrupted_file_yields_empty() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    assert_eq!(load_preferences(data_dir.path()), json!({}));
    std::fs::write(store_path(data_dir.path()), "{not json").expect("write junk");
    assert_eq!(load_preferences(data_dir.path()), json!({}));
}

#[test]
fn rejects_wrong_types() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    assert!(save_preferences(data_dir.path(), &json!({ "downloadDir": 7 })).is_err());
    assert!(save_preferences(data_dir.path(), &json!({ "downloadUseDefaultDir": "yes" })).is_err());
}

#[cfg(unix)]
#[test]
fn preference_temp_file_is_created_with_0600_before_rename() {
    use std::os::unix::fs::PermissionsExt;

    let data_dir = tempfile::tempdir().expect("tempdir");
    let path = store_path(data_dir.path());
    write_preferences_atomically(&path, "{\"prefs\":{}}").expect("atomic preference write");
    assert_eq!(
        std::fs::metadata(path)
            .expect("preference metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600,
    );
}

#[test]
fn startup_commands_pref_roundtrip_with_shape_cleaning() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    // 空偏好：键不出现（前端按无配置处理）。
    assert!(load_preferences(data_dir.path())
        .get("startup_commands")
        .is_none());
    // 非对象整体报错且不落盘污染。
    let error = save_preferences(data_dir.path(), &json!({ "startup_commands": [] }))
        .expect_err("must reject non-object store");
    assert!(error.contains("startup_commands"));
    // 写入（部分行非法/禁用/超限）+ 读回：清洗语义在 startup_commands 模块
    // 单测覆盖，这里验证偏好链路（键名、嵌套布局、部分合并）。
    save_preferences(
        data_dir.path(),
        &json!({ "startup_commands": {
            "conn-1": { "enabled": true, "commands": [
                { "command": "echo one" },
                { "command": "echo two", "delayMs": 5, "enabled": false },
            ] },
        } }),
    )
    .expect("save");
    let prefs = load_preferences(data_dir.path());
    let store = prefs.get("startup_commands").expect("store kept");
    assert_eq!(store["conn-1"]["enabled"], true);
    assert_eq!(store["conn-1"]["commands"].as_array().unwrap().len(), 2);
    // 部分更新：只带别的键时 startup_commands 原样保留。
    save_preferences(data_dir.path(), &json!({ "downloadDir": "/tmp/x" })).expect("partial");
    let prefs = load_preferences(data_dir.path());
    assert!(prefs.get("startup_commands").is_some());
    assert_eq!(prefs["downloadDir"], "/tmp/x");
}

#[test]
fn rdp_experimental_gate_defaults_closed_and_only_accepts_true() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    assert!(!rdp_experimental_enabled(data_dir.path()));
    save_preferences(
        data_dir.path(),
        &json!({ "rdp_experimental_enabled": true }),
    )
    .expect("enable experimental RDP");
    assert!(rdp_experimental_enabled(data_dir.path()));
    save_preferences(
        data_dir.path(),
        &json!({ "rdp_experimental_enabled": false }),
    )
    .expect("disable experimental RDP");
    assert!(!rdp_experimental_enabled(data_dir.path()));
}

#[test]
fn rdp_security_prefs_whitelist_and_roundtrip() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    // 空偏好：两键都不出现（前端按缺省 use_nla=true/prompt 处理）。
    let prefs = load_preferences(data_dir.path());
    assert!(prefs.get("rdp_experimental_enabled").is_none());
    assert!(prefs.get("rdp_use_nla").is_none());
    assert!(prefs.get("rdp_certificate_policy").is_none());
    // 写入 + 读回。
    save_preferences(
        data_dir.path(),
        &json!({ "rdp_experimental_enabled": true, "rdp_use_nla": false, "rdp_certificate_policy": "strict" }),
    )
    .expect("save");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(prefs["rdp_experimental_enabled"], true);
    assert_eq!(prefs["rdp_use_nla"], false);
    assert_eq!(prefs["rdp_certificate_policy"], "strict");
    // 白名单外证书策略报错且不落盘污染（fail-closed：无「静默接受」项）。
    let error = save_preferences(
        data_dir.path(),
        &json!({ "rdp_certificate_policy": "accept-always" }),
    )
    .expect_err("must reject unknown policy");
    assert!(
        error.contains("prompt, strict or accept-temporarily"),
        "{error}"
    );
    // 非 bool 拒绝。
    assert!(save_preferences(
        data_dir.path(),
        &json!({ "rdp_experimental_enabled": "yes" })
    )
    .is_err());
    assert!(save_preferences(data_dir.path(), &json!({ "rdp_use_nla": "yes" })).is_err());
    // 部分更新只改出现的键。
    save_preferences(data_dir.path(), &json!({ "rdp_use_nla": true })).expect("partial");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(prefs["rdp_use_nla"], true);
    assert_eq!(prefs["rdp_certificate_policy"], "strict");
}

#[test]
fn sftp_pipeline_prefs_clamp_and_roundtrip() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    // 空偏好：三键都不出现（前端按缺省处理），读取器回默认值。
    assert!(load_preferences(data_dir.path())
        .get("transfer_max_active")
        .is_none());
    assert!(load_preferences(data_dir.path())
        .get("sftp_compat_mode")
        .is_none());
    assert!(load_preferences(data_dir.path())
        .get("sftp_name_encoding")
        .is_none());
    assert_eq!(
        transfer_max_active(data_dir.path()),
        TRANSFER_MAX_ACTIVE_DEFAULT
    );
    assert!(!sftp_compat_mode(data_dir.path()));
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), None),
        crate::filesystem::sftp::name::NameEncoding::Auto
    );
    // 写入 + 读回：深度超界钳制、非法编码拒绝（不落盘污染）。
    save_preferences(
        data_dir.path(),
        &json!({
            "transfer_max_active": 99,
            "sftp_compat_mode": true,
            "sftp_name_encoding": "latin-1",
        }),
    )
    .expect("save");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(prefs["transfer_max_active"], 8);
    assert_eq!(prefs["sftp_compat_mode"], true);
    assert_eq!(prefs["sftp_name_encoding"], "latin-1");
    assert_eq!(
        transfer_max_active(data_dir.path()),
        TRANSFER_MAX_ACTIVE_MAX
    );
    assert!(sftp_compat_mode(data_dir.path()));
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), None),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
    // 非法形状报错。
    assert!(save_preferences(data_dir.path(), &json!({ "sftp_compat_mode": "yes" })).is_err());
    assert!(save_preferences(data_dir.path(), &json!({ "sftp_name_encoding": "gbk" })).is_err());
    assert!(save_preferences(data_dir.path(), &json!({ "sftp_name_encoding": 7 })).is_err());
    // 部分更新只改出现的键。
    save_preferences(
        data_dir.path(),
        &json!({ "sftp_compat_mode": false, "sftp_name_encoding": "auto" }),
    )
    .expect("partial");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(prefs["transfer_max_active"], 8);
    assert_eq!(prefs["sftp_compat_mode"], false);
    assert_eq!(prefs["sftp_name_encoding"], "auto");
}

#[test]
fn transfer_download_limit_kib_clamps_and_defaults_to_unlimited() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    // 空偏好：键不出现（=不限速），读取器回 0。
    assert!(load_preferences(data_dir.path())
        .get("transfer_download_limit_kib")
        .is_none());
    assert_eq!(transfer_download_limit_kib(data_dir.path()), 0);
    // 写入 + 读回；超界钳制到 1 GiB/s，负数/字符串数字容错同既有数值键。
    save_preferences(
        data_dir.path(),
        &json!({ "transfer_download_limit_kib": 2_000_000 }),
    )
    .expect("save oversized");
    assert_eq!(
        transfer_download_limit_kib(data_dir.path()),
        TRANSFER_DOWNLOAD_LIMIT_KIB_MAX
    );
    save_preferences(
        data_dir.path(),
        &json!({ "transfer_download_limit_kib": 512 }),
    )
    .expect("save 512");
    assert_eq!(transfer_download_limit_kib(data_dir.path()), 512);
    // 回落 0 = 重新不限速。
    save_preferences(
        data_dir.path(),
        &json!({ "transfer_download_limit_kib": 0 }),
    )
    .expect("save 0");
    assert_eq!(transfer_download_limit_kib(data_dir.path()), 0);
    // 纯函数直测：非法形状（非数值）回落 0，边界值夹紧。
    assert_eq!(sanitize_transfer_download_limit_kib(&json!("abc")), 0);
    assert_eq!(sanitize_transfer_download_limit_kib(&json!(-5)), 0);
    assert_eq!(sanitize_transfer_download_limit_kib(&json!(true)), 0);
    assert_eq!(sanitize_transfer_download_limit_kib(&json!(1)), 1);
    assert_eq!(
        sanitize_transfer_download_limit_kib(&json!(TRANSFER_DOWNLOAD_LIMIT_KIB_MAX + 1)),
        TRANSFER_DOWNLOAD_LIMIT_KIB_MAX
    );
}

#[test]
fn transfer_compress_prefs_default_auto_with_clamped_threshold() {
    use crate::features::transfer::compress::CompressPolicy;
    let data_dir = tempfile::tempdir().expect("tempdir");
    // 缺省：策略 auto、阈值 64 MiB。
    assert_eq!(
        transfer_compress_mode(data_dir.path()),
        CompressPolicy::Auto
    );
    assert_eq!(
        transfer_compress_threshold_mib(data_dir.path()),
        crate::features::transfer::compress::THRESHOLD_MIB_DEFAULT
    );
    // 写入读回：off + 阈值 0（=不限下限）；on 亦合法。
    save_preferences(
        data_dir.path(),
        &json!({ "transfer_compress_mode": "off", "transfer_compress_threshold_mib": 0 }),
    )
    .expect("save compress prefs");
    assert_eq!(transfer_compress_mode(data_dir.path()), CompressPolicy::Off);
    assert_eq!(transfer_compress_threshold_mib(data_dir.path()), 0);
    save_preferences(data_dir.path(), &json!({ "transfer_compress_mode": "on" })).expect("save on");
    assert_eq!(transfer_compress_mode(data_dir.path()), CompressPolicy::On);
    // 超界钳制到上限；策略非法值整键报错（同 sftp_name_encoding 严格向）。
    save_preferences(
        data_dir.path(),
        &json!({ "transfer_compress_threshold_mib": 999_999 }),
    )
    .expect("save oversized threshold");
    assert_eq!(
        transfer_compress_threshold_mib(data_dir.path()),
        crate::features::transfer::compress::THRESHOLD_MIB_MAX
    );
    assert!(
        save_preferences(data_dir.path(), &json!({ "transfer_compress_mode": "yes" }),).is_err()
    );
    // 纯函数直测：非法形状（非数值/负数）回落默认值，0 是合法下限。
    assert_eq!(
        sanitize_transfer_compress_threshold_mib(&json!("abc")),
        crate::features::transfer::compress::THRESHOLD_MIB_DEFAULT
    );
    assert_eq!(
        sanitize_transfer_compress_threshold_mib(&json!(-1)),
        crate::features::transfer::compress::THRESHOLD_MIB_DEFAULT
    );
    assert_eq!(sanitize_transfer_compress_threshold_mib(&json!(0)), 0);
}

#[test]
fn sftp_encoding_override_resolves_priority_three_states() {
    // 三态：连接覆盖 > 全局 > 缺省 auto（resolve 纯函数直测）。
    assert_eq!(
        resolve_sftp_name_encoding(Some("latin-1"), Some("auto")),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
    assert_eq!(
        resolve_sftp_name_encoding(None, Some("latin-1")),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
    assert_eq!(
        resolve_sftp_name_encoding(None, None),
        crate::filesystem::sftp::name::NameEncoding::Auto
    );
    // 白名单回退：覆盖值非法（gbk）时回退全局；全局也非法回缺省。
    assert_eq!(
        resolve_sftp_name_encoding(Some("gbk"), Some("latin-1")),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
    assert_eq!(
        resolve_sftp_name_encoding(Some("gbk"), None),
        crate::filesystem::sftp::name::NameEncoding::Auto
    );
    // 大小写/空白容错与全局读取器一致（parse 语义共用）。
    assert_eq!(
        resolve_sftp_name_encoding(Some(" LATIN-1 "), None),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
}

#[test]
fn sftp_encoding_override_pref_roundtrip_and_priority() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    // 空偏好：覆盖键不出现；无连接上下文与带连接上下文都回全局缺省 auto。
    assert!(load_preferences(data_dir.path())
        .get("sftp_name_encoding_overrides")
        .is_none());
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), None),
        crate::filesystem::sftp::name::NameEncoding::Auto
    );
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), Some("conn-1")),
        crate::filesystem::sftp::name::NameEncoding::Auto
    );
    // 全局 latin-1：未覆盖连接跟随全局。
    save_preferences(data_dir.path(), &json!({ "sftp_name_encoding": "latin-1" }))
        .expect("save global");
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), Some("conn-1")),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), None),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
    // 连接覆盖优先于全局；未覆盖连接仍跟随全局。
    save_preferences(
        data_dir.path(),
        &json!({ "sftp_name_encoding_overrides": {
            "conn-1": "auto",
            "conn-2": "latin-1",
            "conn-3": "gbk",
            "conn-4": 7,
            "": "auto",
        } }),
    )
    .expect("save overrides");
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), Some("conn-1")),
        crate::filesystem::sftp::name::NameEncoding::Auto
    );
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), Some("conn-2")),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), Some("conn-9")),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
    // 非法桶（gbk / 非字符串 / 空 connectionId）清洗丢弃，不报错。
    let prefs = load_preferences(data_dir.path());
    let store = prefs
        .get("sftp_name_encoding_overrides")
        .and_then(Value::as_object)
        .expect("store kept");
    assert_eq!(store.len(), 2);
    assert!(store.contains_key("conn-1") && store.contains_key("conn-2"));
    // 整体非对象报错且不落盘污染。
    let error = save_preferences(
        data_dir.path(),
        &json!({ "sftp_name_encoding_overrides": [] }),
    )
    .expect_err("must reject non-object store");
    assert!(error.contains("sftp_name_encoding_overrides"));
    // 部分更新：只带别的键时覆盖表原样保留。
    save_preferences(data_dir.path(), &json!({ "sftp_compat_mode": true })).expect("partial");
    assert!(load_preferences(data_dir.path())
        .get("sftp_name_encoding_overrides")
        .is_some());
    // 「跟随全局」= 删除本连接桶：整表替换后回退全局。
    save_preferences(
        data_dir.path(),
        &json!({ "sftp_name_encoding_overrides": { "conn-2": "latin-1" } }),
    )
    .expect("remove conn-1 bucket");
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), Some("conn-1")),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
    assert_eq!(
        sftp_name_encoding_for(data_dir.path(), Some("conn-2")),
        crate::filesystem::sftp::name::NameEncoding::Latin1
    );
}

#[test]
fn transfer_and_suggestion_prefs_clamp_and_roundtrip() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    save_preferences(
        data_dir.path(),
        &json!({
            "transfer_concurrency": 99,
            "transfer_duplicate_policy": "ask",
            "history_suggestions_enabled": false,
            "history_suggestion_min_chars": 0,
            "history_suggestion_max_chars": 999,
        }),
    )
    .expect("save");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(prefs["transfer_concurrency"], 10);
    assert_eq!(prefs["transfer_duplicate_policy"], "ask");
    assert_eq!(prefs["history_suggestions_enabled"], false);
    assert_eq!(prefs["history_suggestion_min_chars"], 1);
    assert_eq!(prefs["history_suggestion_max_chars"], 512);
    // 非法策略名报错；缺省键不出现（前端按默认处理）。
    let error = save_preferences(
        data_dir.path(),
        &json!({ "transfer_duplicate_policy": "clobber" }),
    )
    .expect_err("must reject");
    assert!(error.contains("transfer_duplicate_policy"));
    let fresh = tempfile::tempdir().expect("tempdir");
    assert_eq!(load_preferences(fresh.path()), json!({}));
}

#[test]
fn terminal_feature_prefs_roundtrip_with_defaults() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    // 空偏好：五键都不出现（前端按缺省 = 功能全关处理）。
    let prefs = load_preferences(data_dir.path());
    assert!(prefs.get("action_links_enabled").is_none());
    assert!(prefs.get("action_links_matchers").is_none());
    assert!(prefs.get("terminal_show_line_numbers").is_none());
    assert!(prefs.get("terminal_show_timestamps").is_none());
    assert!(prefs.get("terminal_timestamp_format").is_none());
    // 写入 + 读回：matchers 部分字段缺省为 true；格式串原样保留。
    save_preferences(
        data_dir.path(),
        &serde_json::json!({
            "action_links_enabled": true,
            "action_links_matchers": { "ipv4": false },
            "terminal_show_line_numbers": true,
            "terminal_show_timestamps": true,
            "terminal_timestamp_format": "[HH:mm]",
        }),
    )
    .expect("save");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(prefs["action_links_enabled"], true);
    assert_eq!(prefs["action_links_matchers"]["ipv4"], false);
    assert_eq!(prefs["action_links_matchers"]["host_port"], true);
    assert_eq!(prefs["action_links_matchers"]["archive"], true);
    assert_eq!(prefs["terminal_show_line_numbers"], true);
    assert_eq!(prefs["terminal_show_timestamps"], true);
    assert_eq!(prefs["terminal_timestamp_format"], "[HH:mm]");
    // 非法形状报错且不落盘污染；matchers 非对象拒绝。
    assert!(save_preferences(
        data_dir.path(),
        &serde_json::json!({ "action_links_enabled": 1 })
    )
    .is_err());
    assert!(save_preferences(
        data_dir.path(),
        &serde_json::json!({ "action_links_matchers": "all" })
    )
    .is_err());
    assert!(save_preferences(
        data_dir.path(),
        &serde_json::json!({ "terminal_show_timestamps": "yes" })
    )
    .is_err());
    // 在线搜索引擎表：trim + 超长截断；非字符串拒绝。
    assert!(save_preferences(
        data_dir.path(),
        &serde_json::json!({ "ctx_search_engines": 7 })
    )
    .is_err());
    save_preferences(
        data_dir.path(),
        &serde_json::json!({ "ctx_search_engines": "  Google|https://www.google.com/search?q=%s  " }),
    )
    .expect("save engines");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(
        prefs["ctx_search_engines"],
        "Google|https://www.google.com/search?q=%s"
    );
    let long = "x".repeat(MAX_CTX_SEARCH_ENGINES_LEN + 10);
    save_preferences(
        data_dir.path(),
        &serde_json::json!({ "ctx_search_engines": long }),
    )
    .expect("save long engines");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(
        prefs["ctx_search_engines"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        MAX_CTX_SEARCH_ENGINES_LEN
    );
    // 格式串清洗：危险字符剔除、超长截断、清洗后为空回退默认。
    save_preferences(
        data_dir.path(),
        &serde_json::json!({ "terminal_timestamp_format": "  <>  " }),
    )
    .expect("save fallback");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(prefs["terminal_timestamp_format"], "[HH:mm:ss]");
    let long = "Y".repeat(100);
    save_preferences(
        data_dir.path(),
        &serde_json::json!({ "terminal_timestamp_format": long }),
    )
    .expect("save long");
    let prefs = load_preferences(data_dir.path());
    assert_eq!(
        prefs["terminal_timestamp_format"].as_str().unwrap().len(),
        64
    );
}
