//! 传输偏好（压缩策略与阈值）：偏好层与压缩管线共用的纯类型。

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
