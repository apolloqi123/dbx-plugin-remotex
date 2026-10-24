//! NVIDIA GPU collection and parsing.

use super::*;

/// Collects the NVIDIA GPU overview over a new exec channel (read-only).
pub async fn collect_gpu_overview(handle: &Handle<SshClient>) -> Result<serde_json::Value, String> {
    let outcome = exec_plain(
        handle,
        &gpu_overview_script(),
        std::time::Duration::from_secs(15),
        &[],
    )
    .await?;
    Ok(parse_gpu_overview_output(&outcome.output))
}

/// Collects the Ascend NPU overview over a new exec channel (read-only).
pub async fn collect_npu_overview(handle: &Handle<SshClient>) -> Result<serde_json::Value, String> {
    let outcome = exec_plain(
        handle,
        &npu_overview_script(),
        std::time::Duration::from_secs(15),
        &[],
    )
    .await?;
    Ok(parse_npu_overview_output(&outcome.output))
}

/// Parses `nvidia-smi --query-gpu` + `--query-compute-apps` collector output
/// into the `gpu` metrics section. Pure so it can be unit-tested without a
/// server.
pub fn parse_gpu_overview_output(output: &str) -> serde_json::Value {
    let available = output.lines().any(|line| line.trim() == "GPU_AVAILABLE 1");
    let mut gpus: Vec<GpuInfo> = Vec::new();
    for row in marker_section(output, "GPU_CSV_BEGIN", "GPU_CSV_END").lines() {
        let fields = split_csv_row(row);
        if fields.len() < 14 {
            continue;
        }
        let Ok(index) = fields[0].parse::<u64>() else {
            continue;
        };
        let uuid = fields[1].clone();
        if uuid.is_empty() {
            continue;
        }
        gpus.push(GpuInfo {
            index,
            uuid,
            name: fields[2].clone(),
            driver: optional_string(&fields[3]),
            temperature: parse_optional_f64(&fields[4]),
            utilization: parse_optional_f64(&fields[5]),
            mem_util: parse_optional_f64(&fields[6]),
            total_mem: parse_optional_u64(&fields[7]).map(mib_bytes),
            used_mem: parse_optional_u64(&fields[8]).map(mib_bytes),
            free_mem: parse_optional_u64(&fields[9]).map(mib_bytes),
            power_draw: parse_optional_f64(&fields[10]),
            power_limit: parse_optional_f64(&fields[11]),
            fan: parse_optional_f64(&fields[12]),
            pstate: optional_string(&fields[13]),
            processes: Vec::new(),
        });
    }
    for row in marker_section(output, "GPU_PROCESS_CSV_BEGIN", "GPU_PROCESS_CSV_END").lines() {
        let fields = split_csv_row(row);
        if fields.len() < 4 {
            continue;
        }
        let uuid = fields[0].clone();
        let Ok(pid) = fields[1].parse::<u64>() else {
            continue;
        };
        // `query-compute-apps` reports the physical GPU uuid, or a
        // `GPU-uuid/instance` path under MIG — match both.
        let target = gpus.iter_mut().find(|gpu| {
            gpu.uuid == uuid
                || uuid
                    .strip_prefix(gpu.uuid.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        });
        if let Some(gpu) = target {
            gpu.processes.push(GpuProcess {
                uuid,
                pid,
                mem: parse_optional_u64(&fields[2]).map(mib_bytes),
                name: fields[3..].join(", "),
            });
        }
    }
    serde_json::to_value(GpuOverview { available, gpus }).unwrap_or_default()
}
