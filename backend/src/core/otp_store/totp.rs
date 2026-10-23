//! Metrics support.

use super::*;

/// 绑定解析结果：生成当前窗口码所需的全部材料。
#[derive(Debug, Clone, PartialEq)]
pub struct BoundTotp {
    pub entry_id: String,
    pub key: Vec<u8>,
    pub algorithm: OtpAlgorithm,
    pub digits: u8,
    pub period: u64,
}

/// 按连接取绑定的 TOTP 条目密钥（显式 data_dir 变体，测试友好）。
/// 未绑定 / 绑定的是 hotp / 密钥不可用 → None。
pub fn bound_totp_in(data_dir: &Path, connection_id: &str) -> Option<BoundTotp> {
    let store = load_store(data_dir);
    let entry_id = store.bindings.get(connection_id)?;
    let entry = store
        .entries
        .iter()
        .find(|entry| &entry.id == entry_id)
        .filter(|entry| entry.otp_type == "totp")?;
    let vault = vault_for(data_dir);
    let key = get_decrypted_secret(&vault, entry)?;
    Some(BoundTotp {
        entry_id: entry.id.clone(),
        key,
        algorithm: entry.algorithm,
        digits: entry.digits,
        period: entry.period,
    })
}

/// 自动应答回落入口：进程级 data_dir 上的 [`bound_totp_in`]。
/// 无 data_dir 参数变体（历史签名）；自动应答现走 take_connection_totp_key。
#[allow(dead_code)]
pub fn bound_totp(connection_id: &str) -> Option<BoundTotp> {
    bound_totp_in(&data_dir()?, connection_id)
}

/// 共享防重放缓存：`entry_id -> (code, time_step, period)`。`otp/generate`
/// 与自动应答回落共用——同一窗口的码只发一次。
/// 共享防重放缓存：entry_id -> (最近发出的码, time_step, period)。
pub(crate) type WindowCache = HashMap<String, (String, u64, u64)>;

pub(crate) static WINDOW_CODES: OnceLock<Mutex<WindowCache>> = OnceLock::new();

pub(crate) fn window_codes() -> &'static Mutex<HashMap<String, (String, u64, u64)>> {
    WINDOW_CODES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 当前窗口的码已被发出（`otp/generate` 或自动应答任一路径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowUsed {
    pub remaining_secs: u64,
}

/// 一次窗口取码成功的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCode {
    pub code: String,
    pub remaining_secs: u64,
}

/// 取 `entry` 当前窗口的 TOTP 码并记入共享防重放缓存：同 `entry_id` 同
/// time_step 已取过的码返回 [`WindowUsed`]（调用方等待 `remaining_secs` 到
/// 下一周期）。HOTP 不走这里（没有时间窗口可言）。
pub fn take_window_code(
    entry_id: &str,
    key: &[u8],
    algorithm: OtpAlgorithm,
    digits: u8,
    period: u64,
    now_secs: u64,
) -> Result<WindowCode, WindowUsed> {
    let period = period.max(1);
    let step = now_secs / period;
    let (code, remaining) = otp::totp_at(algorithm, key, period, now_secs, digits);
    let mut cache = window_codes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // 清掉已过期的窗口记录，缓存不随条目数量无界增长。
    cache.retain(|_, (_, cached_step, cached_period)| {
        *cached_step * *cached_period + *cached_period > now_secs
    });
    if let Some((cached_code, cached_step, _)) = cache.get(entry_id) {
        if *cached_step == step && *cached_code == code {
            return Err(WindowUsed {
                remaining_secs: remaining,
            });
        }
    }
    cache.insert(entry_id.to_string(), (code.clone(), step, period));
    Ok(WindowCode {
        code,
        remaining_secs: remaining,
    })
}

/// 连接绑定的 TOTP 条目 + 共享防重放缓存取码，一步到位（自动应答回落用）。
/// 返回 `Some` 时本窗口的码已预留；`None` = 未绑定 / hotp / 密钥不可用 /
/// 本窗口已发出（保持现状链，等待下一周期）。
pub fn take_connection_totp_key(data_dir: &Path, connection_id: &str) -> Option<BoundTotp> {
    let bound = bound_totp_in(data_dir, connection_id)?;
    let now = unix_now_secs();
    take_window_code(
        &bound.entry_id,
        &bound.key,
        bound.algorithm,
        bound.digits,
        bound.period,
        now,
    )
    .ok()?;
    Some(bound)
}
