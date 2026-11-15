//! Plugin preferences layer.

use super::*;

/// Bounded so a broken renderer cannot grow the file without limit.
pub(crate) const MAX_DOWNLOAD_DIR_LEN: usize = 512;

pub(crate) const MAX_TIMESTAMP_FORMAT_LEN: usize = 64;

/// 右键「在线搜索」引擎表原始文本（每行 name|url 模板）上限：12 行内短串足够，
/// 更大的输入按坏输入截断（前端解析器同样有行数/长度上限）。
pub(crate) const MAX_CTX_SEARCH_ENGINES_LEN: usize = 2048;

pub(crate) fn sanitize_download_dir(value: &Value) -> Option<String> {
    let dir = value.as_str()?.trim();
    Some(dir.chars().take(MAX_DOWNLOAD_DIR_LEN).collect())
}

/// 本地终端 shell 偏好：程序路径或可解析名，空串=跟随自动探测。
pub(crate) fn sanitize_local_shell(value: &Value) -> Option<String> {
    let shell = value.as_str()?.trim();
    Some(shell.chars().take(200).collect())
}

/// 本地 Shell 启动目录：仅裁剪与限长（≤512 字符），不校验存在性——
/// `local/terminal/start` 时再对现存目录校验并回落家目录。空串 = 未设置。
pub(crate) fn sanitize_local_shell_cwd(value: &Value) -> Option<String> {
    let dir = value.as_str()?.trim();
    Some(dir.chars().take(512).collect())
}

/// 本地 Shell 环境变量（`[{name, value}]`）：复用
/// `local_terminal::sanitize_env_pair` 的同一条规则集（≤32 对、名 ≤128 字节
/// 且不含 `=`/控制字符、值 ≤4096 字节、托管键忽略）。坏条目静默丢弃而非整
/// 批拒绝，与 sidecar 侧"坏 env 不能挡住提示符"的 fail-safe 契约一致。
pub(crate) fn sanitize_local_shell_env(value: &Value) -> Option<Value> {
    let entries = value.as_array()?;
    let mut pairs = Vec::new();
    for entry in entries {
        if pairs.len() >= super::env::USER_ENV_MAX_PAIRS {
            break;
        }
        let name = match entry.get("name").and_then(Value::as_str) {
            Some(name) => name,
            None => continue,
        };
        let text = entry.get("value").and_then(Value::as_str).unwrap_or("");
        if let Some((name, text)) = super::env::sanitize_env_pair(name, text) {
            pairs.push(json!({ "name": name, "value": text }));
        }
    }
    Some(Value::Array(pairs))
}

/// 冲突策略白名单：自动重命名（默认）/ 询问我 / 覆盖已有文件。
pub(crate) fn sanitize_conflict_policy(value: &Value) -> Option<&'static str> {
    match value.as_str()? {
        "rename" => Some("rename"),
        "ask" => Some("ask"),
        "overwrite" => Some("overwrite"),
        _ => None,
    }
}

/// RDP 证书策略白名单（评审定案，RDP_CREDSSP_REVIEW_CHECKLIST §3-C）：
/// prompt（默认，120s 确认窗）/ strict / accept-temporarily。不存在
/// 「静默接受任意证书」的选项——白名单之外一律拒绝写入。
pub(crate) fn sanitize_rdp_certificate_policy(value: &Value) -> Option<&'static str> {
    match value.as_str()? {
        "prompt" => Some("prompt"),
        "strict" => Some("strict"),
        "accept-temporarily" => Some("accept-temporarily"),
        _ => None,
    }
}

/// 会话级传输并发深度（M14-B）：1..=8，缺省 3（与历史硬编码一致）。
pub const TRANSFER_MAX_ACTIVE_MIN: u64 = 1;

pub const TRANSFER_MAX_ACTIVE_MAX: u64 = 8;

pub const TRANSFER_MAX_ACTIVE_DEFAULT: u64 = 3;

/// 传输并发深度：超界钳制、非法回落默认；键不存在返回 None（上层用默认）。
pub fn sanitize_transfer_max_active(value: &Value) -> u64 {
    sanitize_u64_clamped(
        value,
        TRANSFER_MAX_ACTIVE_MIN,
        TRANSFER_MAX_ACTIVE_MAX,
        TRANSFER_MAX_ACTIVE_DEFAULT,
    )
}

/// 下载限速（issue #66）：0=不限速（缺省），上限 1 GiB/s。超界钳制、
/// 非法（负数/非数值/超出 u64）回落 0（=不限速），语义与既有数值键同向。
pub const TRANSFER_DOWNLOAD_LIMIT_KIB_MAX: u64 = 1_048_576;

pub fn sanitize_transfer_download_limit_kib(value: &Value) -> u64 {
    sanitize_u64_clamped(value, 0, TRANSFER_DOWNLOAD_LIMIT_KIB_MAX, 0)
}

/// 读取限速偏好（缺省 0 = 不限速）。下载任务 start 时快照一次现值，
/// 之后整个任务沿用——改动对下一个下载任务生效，进行中的任务不受
/// 中途修改影响（避免节奏漂移）。
pub fn transfer_download_limit_kib(data_dir: &Path) -> u64 {
    load_preferences(data_dir)
        .get("transfer_download_limit_kib")
        .map(sanitize_transfer_download_limit_kib)
        .unwrap_or(0)
}

/// 读取并发深度（缺省回默认值）。sidecar 每次任务启动时现读现用——
/// 改动即时生效，进行中的任务按原深度自然完成。
pub fn transfer_max_active(data_dir: &Path) -> u64 {
    let prefs = load_preferences(data_dir);
    prefs
        .get("transfer_max_active")
        .map(sanitize_transfer_max_active)
        .unwrap_or(TRANSFER_MAX_ACTIVE_DEFAULT)
}

/// 压缩传输策略（gzip 混合方案）：`auto`（默认，智能综合判断，含弱 CPU
/// 门槛）/`on`（始终尝试，豁免 CPU 门槛）/`off`（关闭）。任务 start 时现读
/// 现决——改动对下一个任务生效，进行中任务按原定通道完成。
pub fn transfer_compress_mode(
    data_dir: &Path,
) -> crate::core::preferences::transfer::CompressPolicy {
    load_preferences(data_dir)
        .get("transfer_compress_mode")
        .and_then(Value::as_str)
        .and_then(crate::core::preferences::transfer::CompressPolicy::parse)
        .unwrap_or(crate::core::preferences::transfer::CompressPolicy::Auto)
}

/// 压缩传输生效阈值（MiB）：0 = 不设下限（冒烟/显式全量场景），
/// 1..=65536；非法回落默认 64。
pub fn sanitize_transfer_compress_threshold_mib(value: &Value) -> u64 {
    sanitize_u64_clamped(
        value,
        0,
        crate::core::preferences::transfer::THRESHOLD_MIB_MAX,
        crate::core::preferences::transfer::THRESHOLD_MIB_DEFAULT,
    )
}

pub fn transfer_compress_threshold_mib(data_dir: &Path) -> u64 {
    load_preferences(data_dir)
        .get("transfer_compress_threshold_mib")
        .map(sanitize_transfer_compress_threshold_mib)
        .unwrap_or(crate::core::preferences::transfer::THRESHOLD_MIB_DEFAULT)
}

/// 老旧服务器兼容模式（M14-B）：缺省关。开启后 SFTP 会话不做流水线并发、
/// 传输深度强制 1，并避开非标准扩展请求。
pub fn sftp_compat_mode(data_dir: &Path) -> bool {
    load_preferences(data_dir)
        .get("sftp_compat_mode")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// 连接级文件名编码覆盖（M16）：`{ <connectionId>: "auto"|"latin-1" }`。
/// 桶上限与 `startup_commands` 的连接数上限同向，防手改文件无限膨胀。
pub const SFTP_NAME_ENCODING_OVERRIDES_MAX_CONNECTIONS: usize = 512;

/// 覆盖桶清洗：键为 connectionId、值走编码白名单；非法桶/非法值静默丢弃
/// （手改文件不得卡死工作台，与 `startup_commands::sanitize_store` 同向），
/// 桶数超限截断。
pub fn sanitize_sftp_encoding_overrides(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let mut out = Map::new();
    for (connection_id, entry) in object {
        if out.len() >= SFTP_NAME_ENCODING_OVERRIDES_MAX_CONNECTIONS {
            break;
        }
        if connection_id.trim().is_empty() {
            continue;
        }
        if let Some(encoding) = entry
            .as_str()
            .and_then(crate::core::model::NameEncoding::parse)
        {
            out.insert(
                connection_id.clone(),
                Value::String(encoding.as_str().to_string()),
            );
        }
    }
    Some(Value::Object(out))
}

/// 优先级解析（M16 纯函数）：连接覆盖 > 全局偏好 > 缺省 auto。
/// 白名单回退语义：覆盖值/全局值非法（含非白名单串）一律视为「未设置」，
/// 由下一级兜底，绝不因手改文件报错。
pub fn resolve_sftp_name_encoding(
    override_value: Option<&str>,
    global_value: Option<&str>,
) -> crate::core::model::NameEncoding {
    let parse = |raw: Option<&str>| raw.and_then(crate::core::model::NameEncoding::parse);
    parse(override_value)
        .or_else(|| parse(global_value))
        .unwrap_or(crate::core::model::NameEncoding::Auto)
}

/// 连接级编码判定：`connection_id` 命中覆盖桶（且值合法）时覆盖全局偏好，
/// 否则跟随全局，再缺省 auto。单次读盘，判定点每次调用现读现用。
pub fn sftp_name_encoding_for(
    data_dir: &Path,
    connection_id: Option<&str>,
) -> crate::core::model::NameEncoding {
    let prefs = load_preferences(data_dir);
    let override_value = connection_id.and_then(|id| {
        prefs
            .get("sftp_name_encoding_overrides")
            .and_then(|store| store.get(id))
            .and_then(Value::as_str)
    });
    resolve_sftp_name_encoding(
        override_value,
        prefs.get("sftp_name_encoding").and_then(Value::as_str),
    )
}

/// 数值偏好钳制：非负整数夹进 [min, max]，超界取边界、非法取 fallback。
pub(crate) fn sanitize_u64_clamped(value: &Value, min: u64, max: u64, fallback: u64) -> u64 {
    let raw = match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    };
    match raw {
        Some(value) => value.clamp(min, max),
        None => fallback,
    }
}

/// 动作链接三类匹配器开关（ipv4/host_port/archive）。逐字段收紧、缺省 true；
/// 整键缺失或形状非法由调用方跳过（前端按缺省处理）。
pub(crate) fn sanitize_action_links_matchers(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let flag = |key: &str| object.get(key).and_then(Value::as_bool).unwrap_or(true);
    Some(json!({
        "ipv4": flag("ipv4"),
        "host_port": flag("host_port"),
        "archive": flag("archive"),
    }))
}

/// 终端时间戳格式：白名单字符（字母数字与 []:-./, 空格）+ 64 字符截断；
/// 清洗后为空回退默认 "[HH:mm:ss]"，与前端 sanitizeGutterSettings 同向。
pub(crate) fn sanitize_timestamp_format(value: &Value) -> Option<String> {
    let raw = value.as_str()?;
    let cleaned: String = raw
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '[' | ']' | ':' | '-' | '.' | '/' | ',' | ' ')
        })
        .take(MAX_TIMESTAMP_FORMAT_LEN)
        .collect();
    let trimmed = cleaned.trim();
    Some(if trimmed.is_empty() {
        "[HH:mm:ss]".to_string()
    } else {
        cleaned
    })
}

/// 在线搜索引擎表原始文本：仅裁首尾空白并截断到上限；行级校验在前端解析器。
pub(crate) fn sanitize_ctx_search_engines(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    Some(text.chars().take(MAX_CTX_SEARCH_ENGINES_LEN).collect())
}
