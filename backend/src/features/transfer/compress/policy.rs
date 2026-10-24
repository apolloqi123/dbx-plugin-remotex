/// 压缩传输生效的模式。序列化进 RPC 响应（`compression` 字段）与进度事件。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompressionMode {
    /// 普通传输（现状管线，协议零变化）。
    None,
    /// gzip 压缩通道：本地 flate2 + 远端 gzip/gunzip（树为 tar.gz）。
    Gzip,
}

impl CompressionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CompressionMode::None => "none",
            CompressionMode::Gzip => "gzip",
        }
    }
}

/// 压缩策略（偏好 `transfer_compress_mode`，设置→传输）：auto=智能综合
/// 判断（默认，含弱 CPU 门槛）；on=始终尝试（跳过 CPU 门槛，其余回退条件
/// 与运行时吞吐守卫仍然生效）；off=关闭。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompressPolicy {
    Auto,
    On,
    Off,
}

impl CompressPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(CompressPolicy::Auto),
            "on" => Some(CompressPolicy::On),
            "off" => Some(CompressPolicy::Off),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CompressPolicy::Auto => "auto",
            CompressPolicy::On => "on",
            CompressPolicy::Off => "off",
        }
    }
}

/// 默认生效阈值：64 MiB 以下不压缩（小文件压缩收益盖不住 CPU 与固定开销）。
pub const THRESHOLD_MIB_DEFAULT: u64 = 64;
/// 阈值上限（MiB）：设置面钳制，防呆不防饿。
pub const THRESHOLD_MIB_MAX: u64 = 65536;
/// 比率守卫：压缩后体积 ≥ 原始体积的 95% 即判定不划算，回退普通传输。
/// （已压缩数据偶尔缩 1-2%，网络节省盖不住两端 CPU 与额外磁盘写。）
pub const WORTHWHILE_PERCENT: u128 = 95;
/// 弱 CPU 门槛（auto 策略）：本机并行度 ≤2 核直接跳过压缩——压缩吃满单核
/// 会拖垮整机，收益在慢网低配机上是负的。`on` 策略跳过本门槛。
pub const WEAK_CPU_PARALLELISM: u64 = 2;
/// 运行时吞吐守卫（上传预压，所有策略生效）：处理满 8 MiB 且耗时满 1s 时
/// 做一次速率检查，压缩速率低于该下限（MiB/s）即中止回退普通推送——
/// 比核数更真实的「CPU 不行」信号，弱单核/被限流的容器都能兜住。
pub const RATE_GUARD_MIN_BYTES: u64 = 8 * 1024 * 1024;
pub const RATE_GUARD_MIN_ELAPSED: std::time::Duration = std::time::Duration::from_secs(1);
pub const RATE_GUARD_FLOOR_MIB_S: u64 = 20;

/// 已压缩/不可压缩类型：再压一遍几乎不缩，纯浪费两端 CPU。大小写不敏感，
/// 只看末段扩展名。
const INCOMPRESSIBLE_EXTENSIONS: &[&str] = &[
    "7z", "avi", "apk", "bz2", "deb", "dll", "dmg", "exe", "flac", "gif", "gz", "iso", "jpeg",
    "jpg", "m4a", "m4v", "mkv", "mov", "mp3", "mp4", "mpeg", "mpg", "ogg", "png", "rar", "rpm",
    "svgz", "tbz", "tgz", "webm", "webp", "xz", "zip", "zst",
];

/// 压缩流的分块读写缓冲（与 TRANSFER_CHUNK_SIZE 同口径，回调节流由调用方做）。
pub(super) const STREAM_BUFFER_SIZE: usize = 256 * 1024;

/// 判定一次传输是否走压缩通道。`name` 传文件末段名（树模式传 `None`，跳过
/// 扩展名黑名单——混合内容树里总有文本成员）；`local_parallelism` 传本机
/// 可用并行度（`available_parallelism`，0=未知则不启用 CPU 门槛）。所有
/// 回退条件都收敛在这里，调用方（ssh.rs）只补两件本模块不知道的事：远端
/// 工具探测与会话属性。上传方向的运行时吞吐守卫独立于本决策（compress_
/// file_to 内，所有策略生效）。
#[allow(clippy::too_many_arguments)]
pub fn decide(
    policy: CompressPolicy,
    size: u64,
    threshold_mib: u64,
    name: Option<&str>,
    tool_available: bool,
    read_only: bool,
    latin1: bool,
    local_parallelism: u64,
) -> CompressionMode {
    if policy == CompressPolicy::Off || !tool_available || read_only || latin1 {
        return CompressionMode::None;
    }
    // 弱 CPU 门槛（仅 auto）：核数不足不压。`on` 是用户对门槛的显式豁免
    // （慢网上快双核仍可能受益）；运行时吞吐守卫仍然兜底。
    if policy == CompressPolicy::Auto
        && local_parallelism > 0
        && local_parallelism <= WEAK_CPU_PARALLELISM
    {
        return CompressionMode::None;
    }
    // 0 = 不设下限（冒烟/显式全量场景），否则按 MiB 阈值。
    if threshold_mib > 0 && size < threshold_mib.saturating_mul(1024 * 1024) {
        return CompressionMode::None;
    }
    if let Some(name) = name {
        if !is_compressible_name(name) {
            return CompressionMode::None;
        }
    }
    CompressionMode::Gzip
}

/// 本机可用并行度（0 = 探测失败，决策按「不启用 CPU 门槛」处理）。
pub fn local_parallelism() -> u64 {
    std::thread::available_parallelism()
        .map(|value| value.get() as u64)
        .unwrap_or(0)
}

/// 运行时吞吐守卫的错误标记串：调用方据此把中止映射为「不划算回退」而非失败。
pub const RATE_GUARD_ERROR: &str = "compression-rate-guard";

pub fn is_rate_guard_error(error: &std::io::Error) -> bool {
    error.to_string().contains(RATE_GUARD_ERROR)
}

/// 扩展名黑名单判定：大小写不敏感，取最后一个 `.` 之后的段。无扩展名视为
/// 可压缩（数据库 dump、日志、裸文本是大头）。
pub fn is_compressible_name(file_name: &str) -> bool {
    let lowered = file_name.to_lowercase();
    match lowered.rsplit_once('.') {
        Some((_, extension)) => !INCOMPRESSIBLE_EXTENSIONS.contains(&extension),
        None => true,
    }
}

/// 比率守卫：`compressed * 100 < original * 95`（u128 防大文件溢出）。
/// original 为 0 视为不值得（空文件没有传输量可省）。
pub fn worthwhile(original: u64, compressed: u64) -> bool {
    if original == 0 {
        return false;
    }
    u128::from(compressed) * 100 < u128::from(original) * WORTHWHILE_PERCENT
}
