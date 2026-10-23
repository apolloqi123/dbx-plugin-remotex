//! Plugin preferences layer.

use super::*;

pub(crate) const STORAGE_VERSION: u64 = 1;

pub(crate) const FILE_NAME: &str = "preferences.json";

pub fn store_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(FILE_NAME)
}

/// Reads the raw preferences map; a missing or corrupted file yields an empty
/// map so a bad file can never break the workbench (same policy as
/// quick-commands).
pub(crate) fn load_map(data_dir: &Path) -> Map<String, Value> {
    let text = std::fs::read_to_string(store_path(data_dir)).unwrap_or_default();
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| value.get("prefs")?.as_object().cloned())
        .unwrap_or_default()
}

/// Public view: only allowlisted keys, normalized.
pub fn load_preferences(data_dir: &Path) -> Value {
    let map = load_map(data_dir);
    let mut prefs = Map::new();
    if let Some(dir) = map.get("downloadDir").and_then(sanitize_download_dir) {
        prefs.insert("downloadDir".to_string(), Value::String(dir));
    }
    if let Some(use_default) = map.get("downloadUseDefaultDir").and_then(Value::as_bool) {
        prefs.insert(
            "downloadUseDefaultDir".to_string(),
            Value::Bool(use_default),
        );
    }
    if let Some(policy) = map
        .get("downloadConflictPolicy")
        .and_then(sanitize_conflict_policy)
    {
        prefs.insert(
            "downloadConflictPolicy".to_string(),
            Value::String(policy.to_string()),
        );
    }
    if let Some(shell) = map.get("localShell").and_then(sanitize_local_shell) {
        prefs.insert("localShell".to_string(), Value::String(shell));
    }
    if let Some(integration) = map.get("localShellIntegration").and_then(Value::as_bool) {
        prefs.insert(
            "localShellIntegration".to_string(),
            Value::Bool(integration),
        );
    }
    // 本地 Shell 启动目录/环境变量：cwd 空串视为未设置不入返回；env 空数组
    // 照常返回（前端按无变量处理）。
    if let Some(dir) = map.get("localShellCwd").and_then(sanitize_local_shell_cwd) {
        if !dir.is_empty() {
            prefs.insert("localShellCwd".to_string(), Value::String(dir));
        }
    }
    if let Some(env) = map.get("localShellEnv").and_then(sanitize_local_shell_env) {
        prefs.insert("localShellEnv".to_string(), env);
    }
    // 上传并发（1..=10，默认 3）与重复目标策略（P1-5）。
    if map.contains_key("transfer_concurrency") {
        let concurrency = sanitize_u64_clamped(&map["transfer_concurrency"], 1, 10, 3);
        prefs.insert("transfer_concurrency".to_string(), Value::from(concurrency));
    }
    if let Some(policy) = map
        .get("transfer_duplicate_policy")
        .and_then(sanitize_conflict_policy)
    {
        prefs.insert(
            "transfer_duplicate_policy".to_string(),
            Value::String(policy.to_string()),
        );
    }
    // 会话级传输并发深度（M14-B）与老旧服务器兼容模式、文件名编码偏好。
    if map.contains_key("transfer_max_active") {
        prefs.insert(
            "transfer_max_active".to_string(),
            Value::from(sanitize_transfer_max_active(&map["transfer_max_active"])),
        );
    }
    // 下载限速（issue #66）：0=不限速（缺省），1..=1048576 KiB/s。
    if map.contains_key("transfer_download_limit_kib") {
        prefs.insert(
            "transfer_download_limit_kib".to_string(),
            Value::from(sanitize_transfer_download_limit_kib(
                &map["transfer_download_limit_kib"],
            )),
        );
    }
    // 压缩传输（gzip 混合方案）：策略三态（auto/on/off）+ 阈值 MiB（0=不限下限）。
    if let Some(mode) = map
        .get("transfer_compress_mode")
        .and_then(Value::as_str)
        .and_then(crate::features::transfer::compress::CompressPolicy::parse)
    {
        prefs.insert(
            "transfer_compress_mode".to_string(),
            Value::String(mode.as_str().to_string()),
        );
    }
    if map.contains_key("transfer_compress_threshold_mib") {
        prefs.insert(
            "transfer_compress_threshold_mib".to_string(),
            Value::from(sanitize_transfer_compress_threshold_mib(
                &map["transfer_compress_threshold_mib"],
            )),
        );
    }
    if let Some(enabled) = map.get("sftp_compat_mode").and_then(Value::as_bool) {
        prefs.insert("sftp_compat_mode".to_string(), Value::Bool(enabled));
    }
    if let Some(encoding) = map
        .get("sftp_name_encoding")
        .and_then(Value::as_str)
        .and_then(crate::filesystem::sftp::name::NameEncoding::parse)
    {
        prefs.insert(
            "sftp_name_encoding".to_string(),
            Value::String(encoding.as_str().to_string()),
        );
    }
    // 连接级文件名编码覆盖（M16）：按 connectionId 分桶，形状清洗在
    // sanitize_sftp_encoding_overrides（单测覆盖）；键不存在时不出现。
    if let Some(store) = map
        .get("sftp_name_encoding_overrides")
        .and_then(sanitize_sftp_encoding_overrides)
    {
        prefs.insert("sftp_name_encoding_overrides".to_string(), store);
    }
    // 命令输入建议（P1-1）：开关（默认开）与查询长度上下限。
    if let Some(enabled) = map
        .get("history_suggestions_enabled")
        .and_then(Value::as_bool)
    {
        prefs.insert(
            "history_suggestions_enabled".to_string(),
            Value::Bool(enabled),
        );
    }
    if map.contains_key("history_suggestion_min_chars") {
        let min_chars = sanitize_u64_clamped(&map["history_suggestion_min_chars"], 1, 16, 1);
        prefs.insert(
            "history_suggestion_min_chars".to_string(),
            Value::from(min_chars),
        );
    }
    if map.contains_key("history_suggestion_max_chars") {
        let max_chars = sanitize_u64_clamped(&map["history_suggestion_max_chars"], 8, 512, 64);
        prefs.insert(
            "history_suggestion_max_chars".to_string(),
            Value::from(max_chars),
        );
    }
    if let Some(enabled) = map.get("action_links_enabled").and_then(Value::as_bool) {
        prefs.insert("action_links_enabled".to_string(), Value::Bool(enabled));
    }
    if let Some(matchers) = map
        .get("action_links_matchers")
        .and_then(sanitize_action_links_matchers)
    {
        prefs.insert("action_links_matchers".to_string(), matchers);
    }
    if let Some(line_numbers) = map
        .get("terminal_show_line_numbers")
        .and_then(Value::as_bool)
    {
        prefs.insert(
            "terminal_show_line_numbers".to_string(),
            Value::Bool(line_numbers),
        );
    }
    if let Some(timestamps) = map.get("terminal_show_timestamps").and_then(Value::as_bool) {
        prefs.insert(
            "terminal_show_timestamps".to_string(),
            Value::Bool(timestamps),
        );
    }
    if let Some(format) = map
        .get("terminal_timestamp_format")
        .and_then(sanitize_timestamp_format)
    {
        prefs.insert(
            "terminal_timestamp_format".to_string(),
            Value::String(format),
        );
    }
    if let Some(engines) = map
        .get("ctx_search_engines")
        .and_then(sanitize_ctx_search_engines)
    {
        prefs.insert("ctx_search_engines".to_string(), Value::String(engines));
    }
    // 背景图开关与透明度（百分比 10..=90，缺省 45；前端展示时 /100）。
    if let Some(enabled) = map.get("wallpaper_enabled").and_then(Value::as_bool) {
        prefs.insert("wallpaper_enabled".to_string(), Value::Bool(enabled));
    }
    if map.contains_key("wallpaper_opacity") {
        let opacity = sanitize_u64_clamped(&map["wallpaper_opacity"], 10, 90, 45);
        prefs.insert("wallpaper_opacity".to_string(), Value::from(opacity));
    }
    // 连接级启动命令（P0-4，Tabby Login scripts 对标）：按 connectionId 分桶，
    // 形状清洗在 startup_commands 模块（单测覆盖）。
    if let Some(store) = map
        .get("startup_commands")
        .and_then(crate::features::terminal::startup_commands::sanitize_store)
    {
        prefs.insert("startup_commands".to_string(), store);
    }
    // RDP 安全设置（RDP-2）：use_nla 缺省 true、证书策略缺省 prompt；
    // 键不存在时不出现（前端按缺省处理）。
    if let Some(enabled) = map.get("rdp_experimental_enabled").and_then(Value::as_bool) {
        prefs.insert("rdp_experimental_enabled".to_string(), Value::Bool(enabled));
    }
    if let Some(use_nla) = map.get("rdp_use_nla").and_then(Value::as_bool) {
        prefs.insert("rdp_use_nla".to_string(), Value::Bool(use_nla));
    }
    if let Some(policy) = map
        .get("rdp_certificate_policy")
        .and_then(sanitize_rdp_certificate_policy)
    {
        prefs.insert(
            "rdp_certificate_policy".to_string(),
            Value::String(policy.to_string()),
        );
    }

    Value::Object(prefs)
}

/// RDP is intentionally opt-in until the real-server validation matrix is complete.
/// Missing, malformed, or false values fail closed.
pub fn rdp_experimental_enabled(data_dir: &Path) -> bool {
    load_preferences(data_dir)
        .get("rdp_experimental_enabled")
        .and_then(Value::as_bool)
        == Some(true)
}

/// Merges the allowlisted keys present in `params` into the store and persists
/// atomically (tmp + rename). Returns the stored preferences.
pub fn save_preferences(data_dir: &Path, params: &Value) -> Result<Value, String> {
    let mut map = load_map(data_dir);
    if let Some(value) = params.get("downloadDir") {
        let dir = sanitize_download_dir(value)
            .ok_or_else(|| "downloadDir must be a string".to_string())?;
        map.insert("downloadDir".to_string(), Value::String(dir));
    }
    if let Some(value) = params.get("downloadUseDefaultDir") {
        let use_default = value
            .as_bool()
            .ok_or_else(|| "downloadUseDefaultDir must be a boolean".to_string())?;
        map.insert(
            "downloadUseDefaultDir".to_string(),
            Value::Bool(use_default),
        );
    }
    if let Some(value) = params.get("downloadConflictPolicy") {
        let policy = sanitize_conflict_policy(value)
            .ok_or_else(|| "downloadConflictPolicy must be rename, ask or overwrite".to_string())?;
        map.insert(
            "downloadConflictPolicy".to_string(),
            Value::String(policy.to_string()),
        );
    }
    if let Some(value) = params.get("localShell") {
        let shell =
            sanitize_local_shell(value).ok_or_else(|| "localShell must be a string".to_string())?;
        map.insert("localShell".to_string(), Value::String(shell));
    }
    if let Some(value) = params.get("localShellIntegration") {
        let integration = value
            .as_bool()
            .ok_or_else(|| "localShellIntegration must be a boolean".to_string())?;
        map.insert(
            "localShellIntegration".to_string(),
            Value::Bool(integration),
        );
    }
    // 本地 Shell 启动目录/环境变量：cwd 允许空串（= 清除）；env 非数组报错，
    // 数组内坏条目由净化器静默丢弃。
    if let Some(value) = params.get("localShellCwd") {
        let dir = sanitize_local_shell_cwd(value)
            .ok_or_else(|| "localShellCwd must be a string".to_string())?;
        map.insert("localShellCwd".to_string(), Value::String(dir));
    }
    if let Some(value) = params.get("localShellEnv") {
        let env = sanitize_local_shell_env(value)
            .ok_or_else(|| "localShellEnv must be an array of {name, value}".to_string())?;
        map.insert("localShellEnv".to_string(), env);
    }
    // 上传并发/重复策略与命令建议键：数值一律钳制到合法区间（非法回落默认），
    // 不报错，保证旧前端/手改文件不会把偏好写入卡死。
    if params.get("transfer_concurrency").is_some() {
        map.insert(
            "transfer_concurrency".to_string(),
            Value::from(sanitize_u64_clamped(
                &params["transfer_concurrency"],
                1,
                10,
                3,
            )),
        );
    }
    if let Some(value) = params.get("x11_forwarding") {
        let enabled = value
            .as_bool()
            .ok_or_else(|| "x11_forwarding must be a boolean".to_string())?;
        map.insert("x11_forwarding".to_string(), Value::Bool(enabled));
    }
    // 会话自动录制（M14）：默认关；只读连接不禁用（录制是被动输出捕获，
    // 不向远端发送任何内容）。
    if let Some(value) = params.get("auto_record") {
        let enabled = value
            .as_bool()
            .ok_or_else(|| "auto_record must be a boolean".to_string())?;
        map.insert("auto_record".to_string(), Value::Bool(enabled));
    }
    if let Some(value) = params.get("transfer_duplicate_policy") {
        let policy = sanitize_conflict_policy(value).ok_or_else(|| {
            "transfer_duplicate_policy must be rename, ask or overwrite".to_string()
        })?;
        map.insert(
            "transfer_duplicate_policy".to_string(),
            Value::String(policy.to_string()),
        );
    }
    // M14-B 三键：并发深度钳制到 1..=8；兼容模式布尔；编码走白名单。
    if params.get("transfer_max_active").is_some() {
        map.insert(
            "transfer_max_active".to_string(),
            Value::from(sanitize_transfer_max_active(&params["transfer_max_active"])),
        );
    }
    // 下载限速（issue #66）：数值一律钳制到 0..=1048576 KiB/s（0=不限速），
    // 非法回落 0，不报错——与 transfer_concurrency 等数值键同向。
    if params.get("transfer_download_limit_kib").is_some() {
        map.insert(
            "transfer_download_limit_kib".to_string(),
            Value::from(sanitize_transfer_download_limit_kib(
                &params["transfer_download_limit_kib"],
            )),
        );
    }
    // 压缩传输（gzip 混合方案）：策略走白名单（非法报错，同 sftp_name_encoding
    // 的严格向），阈值数值钳制到 0..=65536 MiB（0=不限下限），非法回落默认。
    if let Some(value) = params.get("transfer_compress_mode") {
        let mode = crate::features::transfer::compress::CompressPolicy::parse(
            value
                .as_str()
                .ok_or_else(|| "transfer_compress_mode must be auto, on or off".to_string())?,
        )
        .ok_or_else(|| "transfer_compress_mode must be auto, on or off".to_string())?;
        map.insert(
            "transfer_compress_mode".to_string(),
            Value::String(mode.as_str().to_string()),
        );
    }
    if params.get("transfer_compress_threshold_mib").is_some() {
        map.insert(
            "transfer_compress_threshold_mib".to_string(),
            Value::from(sanitize_transfer_compress_threshold_mib(
                &params["transfer_compress_threshold_mib"],
            )),
        );
    }
    if let Some(value) = params.get("sftp_compat_mode") {
        let enabled = value
            .as_bool()
            .ok_or_else(|| "sftp_compat_mode must be a boolean".to_string())?;
        map.insert("sftp_compat_mode".to_string(), Value::Bool(enabled));
    }
    if let Some(value) = params.get("sftp_name_encoding") {
        let encoding = crate::filesystem::sftp::name::NameEncoding::parse(
            value
                .as_str()
                .ok_or_else(|| "sftp_name_encoding must be auto or latin-1".to_string())?,
        )
        .ok_or_else(|| "sftp_name_encoding must be auto or latin-1".to_string())?;
        map.insert(
            "sftp_name_encoding".to_string(),
            Value::String(encoding.as_str().to_string()),
        );
    }
    // 连接级文件名编码覆盖（M16）：整表替换（对象按 connectionId 分桶，
    // 前端读改写合并自己的桶）；整体非对象报错，桶内非法值清洗丢弃。
    if let Some(value) = params.get("sftp_name_encoding_overrides") {
        let store = sanitize_sftp_encoding_overrides(value).ok_or_else(|| {
            "sftp_name_encoding_overrides must be an object keyed by connectionId".to_string()
        })?;
        map.insert("sftp_name_encoding_overrides".to_string(), store);
    }
    if let Some(value) = params.get("history_suggestions_enabled") {
        let enabled = value
            .as_bool()
            .ok_or_else(|| "history_suggestions_enabled must be a boolean".to_string())?;
        map.insert(
            "history_suggestions_enabled".to_string(),
            Value::Bool(enabled),
        );
    }
    if params.get("history_suggestion_min_chars").is_some() {
        map.insert(
            "history_suggestion_min_chars".to_string(),
            Value::from(sanitize_u64_clamped(
                &params["history_suggestion_min_chars"],
                1,
                16,
                2,
            )),
        );
    }
    if params.get("history_suggestion_max_chars").is_some() {
        map.insert(
            "history_suggestion_max_chars".to_string(),
            Value::from(sanitize_u64_clamped(
                &params["history_suggestion_max_chars"],
                8,
                512,
                64,
            )),
        );
    }
    if let Some(value) = params.get("action_links_enabled") {
        let enabled = value
            .as_bool()
            .ok_or_else(|| "action_links_enabled must be a boolean".to_string())?;
        map.insert("action_links_enabled".to_string(), Value::Bool(enabled));
    }
    if let Some(value) = params.get("action_links_matchers") {
        let matchers = sanitize_action_links_matchers(value)
            .ok_or_else(|| "action_links_matchers must be an object".to_string())?;
        map.insert("action_links_matchers".to_string(), matchers);
    }
    if let Some(value) = params.get("terminal_show_line_numbers") {
        let line_numbers = value
            .as_bool()
            .ok_or_else(|| "terminal_show_line_numbers must be a boolean".to_string())?;
        map.insert(
            "terminal_show_line_numbers".to_string(),
            Value::Bool(line_numbers),
        );
    }
    if let Some(value) = params.get("terminal_show_timestamps") {
        let timestamps = value
            .as_bool()
            .ok_or_else(|| "terminal_show_timestamps must be a boolean".to_string())?;
        map.insert(
            "terminal_show_timestamps".to_string(),
            Value::Bool(timestamps),
        );
    }
    if let Some(value) = params.get("terminal_timestamp_format") {
        let format = sanitize_timestamp_format(value)
            .ok_or_else(|| "terminal_timestamp_format must be a string".to_string())?;
        map.insert(
            "terminal_timestamp_format".to_string(),
            Value::String(format),
        );
    }
    if let Some(value) = params.get("ctx_search_engines") {
        let engines = sanitize_ctx_search_engines(value)
            .ok_or_else(|| "ctx_search_engines must be a string".to_string())?;
        map.insert("ctx_search_engines".to_string(), Value::String(engines));
    }
    if let Some(value) = params.get("wallpaper_enabled") {
        let enabled = value
            .as_bool()
            .ok_or_else(|| "wallpaper_enabled must be a boolean".to_string())?;
        map.insert("wallpaper_enabled".to_string(), Value::Bool(enabled));
    }
    if params.get("wallpaper_opacity").is_some() {
        map.insert(
            "wallpaper_opacity".to_string(),
            Value::from(sanitize_u64_clamped(
                &params["wallpaper_opacity"],
                10,
                90,
                45,
            )),
        );
    }
    // 连接级启动命令（P0-4）：整表替换（对象按 connectionId 分桶，前端
    // 读改写合并自己的桶），形状清洗在 startup_commands 模块（单测覆盖）。
    if let Some(value) = params.get("startup_commands") {
        let store = crate::features::terminal::startup_commands::sanitize_store(value).ok_or_else(
            || "startup_commands must be an object keyed by connectionId".to_string(),
        )?;
        map.insert("startup_commands".to_string(), store);
    }
    // RDP 安全设置（RDP-2）：布尔直存；证书策略走白名单（白名单外报错，
    // 不落盘污染），缺省语义由 rdp::security::resolve_security 兜底
    // （use_nla=true、certificate_policy=prompt）。
    // RDP 仍处实验阶段：仅显式 true 才显示/允许入口，普通用户默认不可用。
    if let Some(value) = params.get("rdp_experimental_enabled") {
        let enabled = value
            .as_bool()
            .ok_or_else(|| "rdp_experimental_enabled must be a boolean".to_string())?;
        map.insert("rdp_experimental_enabled".to_string(), Value::Bool(enabled));
    }
    if let Some(value) = params.get("rdp_use_nla") {
        let use_nla = value
            .as_bool()
            .ok_or_else(|| "rdp_use_nla must be a boolean".to_string())?;
        map.insert("rdp_use_nla".to_string(), Value::Bool(use_nla));
    }
    if let Some(value) = params.get("rdp_certificate_policy") {
        let policy = sanitize_rdp_certificate_policy(value).ok_or_else(|| {
            "rdp_certificate_policy must be prompt, strict or accept-temporarily".to_string()
        })?;
        map.insert(
            "rdp_certificate_policy".to_string(),
            Value::String(policy.to_string()),
        );
    }

    let path = store_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text = serde_json::to_string_pretty(&json!({
        "version": STORAGE_VERSION,
        "prefs": Value::Object(map),
    }))
    .map_err(|error| format!("Failed to encode preferences: {error}"))?;
    write_preferences_atomically(&path, &text)?;
    Ok(load_preferences(data_dir))
}

/// Writes preferences through a sibling temporary file then renames it into
/// place. On Unix the temporary file is created as 0600 before its first byte
/// is written, so startup-command text never exists with inherited umask
/// permissions.
pub(crate) fn write_preferences_atomically(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;

    let tmp = path.with_extension("json.tmp");
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
    };
    #[cfg(not(unix))]
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp);
    let mut file = file.map_err(|error| format!("Failed to write {}: {error}", tmp.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("Failed to write {}: {error}", tmp.display()))?;
    file.sync_all()
        .map_err(|error| format!("Failed to sync {}: {error}", tmp.display()))?;
    drop(file);
    std::fs::rename(&tmp, path)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))
}
