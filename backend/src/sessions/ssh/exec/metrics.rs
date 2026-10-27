//! Exec metrics layer.

use super::*;

/// Server metrics collected with read-only commands: /proc readers on Linux
/// with sysctl/`vm_stat`/`iostat` fallbacks on macOS, `df -kP` for mounts,
/// plus the network-rate and top-process extensions. Thin delegation to
/// [`crate::metrics`], which owns the extended collector and parsers.
pub async fn collect_metrics(handle: &Handle<SshClient>) -> Result<serde_json::Value, String> {
    crate::features::metrics::collect_metrics(handle).await
}

/// Parses the output of the collector script (`METRICS_SCRIPT` in metrics.rs)
/// into a metrics JSON object.
/// Pure so it can be unit-tested without a server.
pub fn parse_metrics_output(output: &str) -> serde_json::Value {
    use serde_json::json;

    let mut hostname = serde_json::Value::Null;
    let mut kernel = serde_json::Value::Null;
    let mut loadavg = String::new();
    let mut uptime = String::new();
    let mut nproc: serde_json::Value = serde_json::Value::Null;
    let mut mem_kib: HashMap<&str, u64> = HashMap::new();
    let mut cpu_samples: Vec<Vec<u64>> = Vec::new();
    // One map per aggregate sample; `cpuN` rows attach to the latest map.
    let mut cpu_core_samples: Vec<HashMap<u32, Vec<u64>>> = Vec::new();
    let mut disks = Vec::new();
    let mut section = "";

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if is_section_marker(line) {
            section = line;
            continue;
        }
        if let Some(rest) = line.strip_prefix("hostname=") {
            hostname = json!(rest);
        } else if let Some(rest) = line.strip_prefix("kernel=") {
            kernel = json!(rest);
        } else if let Some(rest) = line.strip_prefix("loadavg=") {
            loadavg = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("uptime=") {
            uptime = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("nproc=") {
            nproc = rest
                .parse::<u32>()
                .map(|v| json!(v))
                .unwrap_or(serde_json::Value::Null);
        } else if section == "--mem--" {
            // "MemTotal:       16384 kB"
            if let Some((key, value)) = rest_after_colon(line) {
                if let Some(kib) = value
                    .split_whitespace()
                    .next()
                    .and_then(|n| n.parse::<u64>().ok())
                {
                    mem_kib.insert(key, kib);
                }
            }
        } else if section == "--cpu--" {
            if let Some(rest) = line.strip_prefix("cpu ") {
                let values = rest
                    .split_whitespace()
                    .filter_map(|n| n.parse::<u64>().ok())
                    .collect::<Vec<_>>();
                if values.len() >= 4 {
                    cpu_samples.push(values);
                    cpu_core_samples.push(HashMap::new());
                }
            } else if let Some(rest) = line.strip_prefix("cpu") {
                // `cpuN user nice system idle iowait …`; the aggregate line is
                // consumed above and section markers never reach this branch.
                if let Some((index, values)) = rest.split_once(' ') {
                    if let (Ok(index), Some(sample)) =
                        (index.parse::<u32>(), cpu_core_samples.last_mut())
                    {
                        let values = values
                            .split_whitespace()
                            .filter_map(|n| n.parse::<u64>().ok())
                            .collect::<Vec<_>>();
                        if values.len() >= 4 {
                            sample.insert(index, values);
                        }
                    }
                }
            }
        } else if section == "--df--" {
            // filesystem total used avail pct mount
            let fields = line.split_whitespace().collect::<Vec<_>>();
            // 严格行形校验：容量三列必须是数字、百分比列必须以 % 结尾、挂载点
            // 必须是绝对路径。段切换认所有 `--name--` 标记（is_section_marker），
            // `--os--` 之后的 os-release 行根本到不了这里；行形校验只兜底
            // 非标记形状的杂散行。
            //
            // 列从右往左锚定（mount 恒为最后一列）：文件系统名可能含空格——
            // Windows 本地采集经 Git-Bash sh，MSYS df 把 Git 安装根报成
            // `C:/Program Files/Git`，左锚定会把系统盘整行拒掉。
            if fields.len() >= 6 && fields[fields.len() - 1].starts_with('/') {
                if let (Ok(total_kib), Ok(used_kib), Ok(available_kib)) = (
                    fields[fields.len() - 5].parse::<u64>(),
                    fields[fields.len() - 4].parse::<u64>(),
                    fields[fields.len() - 3].parse::<u64>(),
                ) {
                    if let Some(percent) = fields[fields.len() - 2]
                        .strip_suffix('%')
                        .and_then(|p| p.parse::<f64>().ok())
                    {
                        disks.push(json!({
                            "filesystem": fields[..fields.len() - 5].join(" "),
                            "mount": fields[fields.len() - 1],
                            // 伪造采集源的容量值可逼近 u64 上限：饱和避免
                            // release 回绕 / debug panic。
                            "totalBytes": total_kib.saturating_mul(1024),
                            "usedBytes": used_kib.saturating_mul(1024),
                            "availableBytes": available_kib.saturating_mul(1024),
                            "percentUsed": percent,
                        }));
                    }
                }
            }
        }
    }

    let mut cpu_percent = serde_json::Value::Null;
    if let [first, second] = cpu_samples.as_slice() {
        if let Some(percent) = cpu_busy_percent(first, second) {
            cpu_percent = json!(percent);
        }
    }

    // Per-core usage is additive: old sidecars omit `cpuN` rows entirely and
    // macOS keeps the aggregate-only fallback, so the list simply stays empty.
    let mut per_core: Vec<serde_json::Value> = Vec::new();
    if let [first, second] = cpu_core_samples.as_slice() {
        let mut indices = second
            .keys()
            .filter(|index| first.contains_key(index))
            .copied()
            .collect::<Vec<_>>();
        indices.sort_unstable();
        for index in indices {
            if let Some(percent) = cpu_busy_percent(&first[&index], &second[&index]) {
                per_core.push(json!({ "index": index, "percent": percent }));
            }
        }
    }

    let loads: Vec<f64> = loadavg
        .split_whitespace()
        .take(3)
        .filter_map(|n| n.parse::<f64>().ok())
        .collect();
    let (load1, load5, load15) = match loads.as_slice() {
        [one, five, fifteen] => (json!(one), json!(five), json!(fifteen)),
        _ => (
            serde_json::Value::Null,
            serde_json::Value::Null,
            serde_json::Value::Null,
        ),
    };
    let mem_total = mem_kib.get("MemTotal").copied().unwrap_or(0) * 1024;
    // Linux ≥3.14 reports MemAvailable directly; pre-3.14 kernels and the
    // MSYS /proc shim the local collector hits on Windows (Git-Bash sh) only
    // carry MemFree — fall back to the classic free+buffers+cached estimate,
    // otherwise `used` degenerates to the full total and the dock band pins
    // MEM at 100%.
    let mem_available = match mem_kib.get("MemAvailable") {
        Some(available) => available.saturating_mul(1024),
        None => mem_kib
            .get("MemFree")
            .copied()
            .unwrap_or(0)
            .saturating_add(mem_kib.get("Buffers").copied().unwrap_or(0))
            .saturating_add(mem_kib.get("Cached").copied().unwrap_or(0))
            .saturating_mul(1024),
    };
    // 异常数据源可能让回退估计超过 MemTotal（used 饱和到 0，MEM 带倒挂）：
    // classic free 同样把 available 夹紧在 total 以内。
    let mem_available = mem_available.min(mem_total);
    let mem_used = mem_total.saturating_sub(mem_available);
    // Cache = buffers + page cache (the "free -w" cache column) and a subset of
    // `usedBytes`, which keeps its historical total-available semantics for the
    // dock band. Clamping prevents a quirky /proc shim from overshooting.
    let mem_cache = mem_kib
        .get("Buffers")
        .copied()
        .unwrap_or(0)
        .saturating_add(mem_kib.get("Cached").copied().unwrap_or(0))
        .saturating_mul(1024)
        .min(mem_used);
    // Remainder of total after used; the donut then draws
    // (used - cache) + cache + free == total.
    let mem_free = mem_total.saturating_sub(mem_used);
    let swap_total = mem_kib.get("SwapTotal").copied().unwrap_or(0) * 1024;
    let swap_free = mem_kib.get("SwapFree").copied().unwrap_or(0) * 1024;
    let uptime_seconds = uptime
        .split_whitespace()
        .next()
        .and_then(|n| n.parse::<f64>().ok())
        .map(|v| json!(v as u64))
        .unwrap_or(serde_json::Value::Null);

    json!({
        "hostname": hostname,
        "kernel": kernel,
        "uptimeSeconds": uptime_seconds,
        "cpu": {
            "cores": nproc,
            "percent": cpu_percent,
            "load1": load1,
            "load5": load5,
            "load15": load15,
            "perCore": per_core,
        },
        "memory": {
            "totalBytes": mem_total,
            "availableBytes": mem_available,
            "usedBytes": mem_used,
            "cacheBytes": mem_cache,
            "freeBytes": mem_free,
            "swapTotalBytes": swap_total,
            "swapUsedBytes": swap_total.saturating_sub(swap_free),
        },
        "disks": disks,
    })
}

/// Busy percentage between two `/proc/stat` tick rows (user nice system idle
/// iowait …). Idle time is idle+iowait; `None` when the counter wrapped or the
/// sample did not advance, so callers can leave the field unset.
pub(super) fn cpu_busy_percent(first: &[u64], second: &[u64]) -> Option<f64> {
    let delta = second
        .iter()
        .zip(first.iter())
        .map(|(next, prev)| *next as i64 - *prev as i64)
        .collect::<Vec<_>>();
    let total: i64 = delta.iter().sum();
    let idle = delta.get(3).copied().unwrap_or(0) + delta.get(4).copied().unwrap_or(0);
    if total <= 0 {
        return None;
    }
    Some(((total - idle) as f64 * 100.0 / total as f64 * 10.0).round() / 10.0)
}

pub(super) fn rest_after_colon(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    Some((key.trim(), value.trim()))
}

/// `--name--` 形状的段标记（内部只允许小写字母/数字/连字符）。解析器只消费
/// --mem--/--cpu--/--df-- 三段，但必须识别**所有**标记：只认三个会让未来新增
/// 段（如 --os-- 的 os-release）的内容漏进当前段——落在 df 段里就是幽灵磁盘行。
pub(super) fn is_section_marker(line: &str) -> bool {
    let Some(inner) = line
        .strip_prefix("--")
        .and_then(|rest| rest.strip_suffix("--"))
    else {
        return false;
    };
    !inner.is_empty()
        && inner
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Parses `df -kP <path>` output (header stripped) into a usage object.
pub fn parse_disk_usage(df_output: &str) -> Option<serde_json::Value> {
    use serde_json::json;
    // `df -kP <path>` prints one data line for the filesystem holding the
    // path, but that line does not necessarily start with '/': overlay and
    // tmpfs mounts (containers!) name their filesystem `overlay`/`tmpfs`.
    // Prefer a device-style line, then fall back to any 6-field line whose
    // block counts parse as numbers (the POSIX header says "1024-blocks",
    // so it never passes the numeric check).
    let candidate = |line: &str| -> Option<serde_json::Value> {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 6 {
            return None;
        }
        Some(json!({
            "filesystem": fields[0],
            "mount": fields[5],
            "totalBytes": fields[1].parse::<u64>().ok()? * 1024,
            "usedBytes": fields[2].parse::<u64>().ok()? * 1024,
            "availableBytes": fields[3].parse::<u64>().ok()? * 1024,
            "percentUsed": fields[4].trim_end_matches('%').parse::<f64>().ok()?,
        }))
    };
    df_output
        .lines()
        .rev()
        .find_map(|line| {
            line.trim()
                .starts_with('/')
                .then(|| candidate(line))
                .flatten()
        })
        .or_else(|| df_output.lines().rev().find_map(candidate))
}
