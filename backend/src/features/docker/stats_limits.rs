//! CPU totals for the container monitor: the host core count and the
//! container's CPU cap (`NanoCpus`, or `CpuQuota`/`CpuPeriod`). This is a
//! one-shot lookup per monitor open — the 3s stats poll must not also run
//! `info`/`inspect` every time.
use super::*;

pub const CPU_HOST_MARKER: &str = "DBXDOCKER_CPU_HOST";
pub const CPU_LIMIT_MARKER: &str = "DBXDOCKER_CPU_LIMIT";

/// Collects the daemon host's core count and the container's CPU cap.
///
/// Host cores come from `docker info .NCPU` (Podman: `Host.CPUs`), which
/// reflects the daemon host even when the socket points at another machine;
/// `nproc`/`getconf`/`/proc/cpuinfo` are fallbacks for old engines. The cap
/// uses `NanoCpus` first (what `--cpus` writes), then quota/period (what
/// `--cpu-quota`/`--cpu-period` write). CPU shares are deliberately ignored:
/// they are a relative weight, not a ceiling.
pub fn stats_limits_script(engine: &EngineConfig, container_id: &str) -> Result<String, String> {
    validate_container_id(container_id)?;
    Ok(format!(
        "sh -s <<'DBXDOCKER'\n\
         {PATH_PREFIX}{export}\
         echo {CPU_HOST_MARKER}\n\
         ({cli} info --format '{{{{.NCPU}}}}' 2>/dev/null || {cli} info --format '{{{{.Host.CPUs}}}}' 2>/dev/null || nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null || grep -c '^processor' /proc/cpuinfo 2>/dev/null || echo 0)\n\
         echo {CPU_LIMIT_MARKER}\n\
         {cli} inspect --format '{{{{.HostConfig.NanoCpus}}}}|{{{{.HostConfig.CpuQuota}}}}|{{{{.HostConfig.CpuPeriod}}}}' {container_id} 2>&1\n\
         DBXDOCKER\n",
        export = engine.env_prefix(),
        cli = engine.cli,
    ))
}

/// Parses the marker text into `{hostCpus, cpuLimitCores}`. `cpuLimitCores`
/// is `null` when the container is unlimited (or the engine predates the
/// fields); the frontend then falls back to the host total.
pub fn stats_limits_payload(output: &str) -> Value {
    json!({
        "hostCpus": parse_host_cpus(output),
        "cpuLimitCores": parse_cpu_limit(output),
    })
}

fn section<'a>(output: &'a str, marker: &str, next: Option<&str>) -> &'a str {
    let Some(start) = output.find(marker) else {
        return "";
    };
    let body = &output[start + marker.len()..];
    match next.and_then(|next| body.find(next)) {
        Some(end) => &body[..end],
        None => body,
    }
}

fn parse_host_cpus(output: &str) -> u64 {
    // The fallback chain can print warnings before the number; the last
    // purely numeric line is the authoritative count.
    section(output, CPU_HOST_MARKER, Some(CPU_LIMIT_MARKER))
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .last()
        .unwrap_or(0)
}

fn parse_cpu_limit(output: &str) -> Value {
    for line in section(output, CPU_LIMIT_MARKER, None).lines() {
        let parts: Vec<&str> = line.trim().split('|').collect();
        if parts.len() != 3 {
            continue;
        }
        let parsed: Option<Vec<i64>> = parts
            .iter()
            .map(|part| part.trim().parse::<i64>().ok())
            .collect();
        let Some([nano, quota, period]) = parsed.as_deref() else {
            continue;
        };
        let cores = if *nano > 0 {
            *nano as f64 / 1_000_000_000.0
        } else if *quota > 0 && *period > 0 {
            *quota as f64 / *period as f64
        } else {
            return Value::Null;
        };
        if cores <= 0.0 {
            return Value::Null;
        }
        // Keep sub-millicore precision out of the payload; the UI shows at
        // most two decimals of a core.
        return json!((cores * 1_000.0).round() / 1_000.0);
    }
    Value::Null
}

/// Local daemon: the sidecar's own machine is the host. The CLI is still
/// asked for the cap (argv only, endpoint via env, never a shell).
pub async fn collect_stats_limits_local(
    engine: &EngineConfig,
    container_id: &str,
) -> Result<Value, String> {
    validate_container_id(container_id)?;
    let host_cpus = std::thread::available_parallelism()
        .map(|count| count.get() as u64)
        .unwrap_or(0);
    let cli = find_local_cli(&engine.cli)
        .ok_or_else(|| format!("{} CLI not found on this machine", engine.cli))?;
    let (_, output) = run_local(
        &cli,
        &[
            "inspect",
            "--format",
            "{{.HostConfig.NanoCpus}}|{{.HostConfig.CpuQuota}}|{{.HostConfig.CpuPeriod}}",
            container_id,
        ],
        STATS_TIMEOUT,
        engine.endpoint.as_deref(),
    )
    .await?;
    Ok(stats_limits_payload(&format!(
        "{CPU_HOST_MARKER}\n{host_cpus}\n{CPU_LIMIT_MARKER}\n{output}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_cores_and_an_unlimited_container() {
        let payload = stats_limits_payload("DBXDOCKER_CPU_HOST\n16\nDBXDOCKER_CPU_LIMIT\n0|0|0\n");
        assert_eq!(payload["hostCpus"], 16);
        assert!(payload["cpuLimitCores"].is_null());
    }

    #[test]
    fn parses_nanocpus_and_quota_period_limits() {
        assert_eq!(
            stats_limits_payload("DBXDOCKER_CPU_HOST\n16\nDBXDOCKER_CPU_LIMIT\n2000000000|0|0\n")
                ["cpuLimitCores"],
            2.0
        );
        assert_eq!(
            stats_limits_payload("DBXDOCKER_CPU_HOST\n16\nDBXDOCKER_CPU_LIMIT\n0|50000|100000\n")
                ["cpuLimitCores"],
            0.5
        );
    }

    #[test]
    fn host_parser_ignores_warnings_and_missing_markers() {
        let payload = stats_limits_payload(
            "DBXDOCKER_CPU_HOST\nCannot connect to the Docker daemon\n8\nDBXDOCKER_CPU_LIMIT\n0|0|0\n",
        );
        assert_eq!(payload["hostCpus"], 8);
        assert_eq!(stats_limits_payload("nothing")["hostCpus"], 0);
        assert!(stats_limits_payload("nothing")["cpuLimitCores"].is_null());
        // An inspect error line must not be mistaken for a cap.
        assert!(stats_limits_payload(
            "DBXDOCKER_CPU_HOST\n8\nDBXDOCKER_CPU_LIMIT\nError: no such container\n"
        )["cpuLimitCores"]
            .is_null());
    }

    #[test]
    fn script_carries_markers_and_engine_specific_host_probes() {
        let engine = crate::features::docker::test_engine("docker", None);
        let script = stats_limits_script(&engine, "d4a7c9f1e2b3").expect("script");
        assert!(script.starts_with("sh -s <<'DBXDOCKER'"));
        assert!(script.contains("DBXDOCKER_CPU_HOST"));
        assert!(script.contains("DBXDOCKER_CPU_LIMIT"));
        assert!(script.contains("docker info --format '{{.NCPU}}'"));
        assert!(script.contains("docker info --format '{{.Host.CPUs}}'"));
        assert!(script.contains("nproc"));
        assert!(script.contains("inspect --format"));
        assert!(script.contains("d4a7c9f1e2b3"));
        assert!(stats_limits_script(&engine, "web-nginx").is_err());
    }
}
