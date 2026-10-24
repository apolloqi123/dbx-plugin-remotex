//! Ascend NPU collection and parsing.

use super::*;

/// Parses `npu-smi info` collector output into the `npu` metrics section.
/// Pure so it can be unit-tested without a server.
pub fn parse_npu_overview_output(output: &str) -> serde_json::Value {
    let available = output.lines().any(|line| line.trim() == "NPU_AVAILABLE 1");
    let cann = output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("NPU_CANN ")
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(str::to_string)
    });
    let mut devices: Vec<NpuDevice> = Vec::new();
    let mut card: Option<CardState> = None;
    let mut hbm = false;
    let mut in_process_table = false;
    for line in marker_section(output, "NPU_SMI_BEGIN", "NPU_SMI_END").lines() {
        let trimmed = line.trim();
        // Separator rows (`+---+---+`, `+===+===+`) carry no `|` cell body.
        if trimmed.is_empty() || !trimmed.contains('|') {
            continue;
        }
        let cells = pipe_cells(trimmed);
        if cells.is_empty() {
            continue;
        }
        if is_chip_header(&cells) {
            // HBM preferred: only 910-class boards expose an HBM column.
            hbm = cells
                .iter()
                .any(|cell| cell.to_ascii_uppercase().contains("HBM"));
            continue;
        }
        if is_card_header(&cells) {
            card = None;
            in_process_table = false;
            continue;
        }
        if cells
            .iter()
            .any(|cell| cell.to_ascii_lowercase().contains("process"))
        {
            in_process_table = true;
            continue;
        }
        if in_process_table {
            if let Some((pid, name, npu_index, mem)) = parse_npu_process_row(&cells) {
                if let Some(device) = devices
                    .iter_mut()
                    .find(|device| device.npu_index == npu_index)
                {
                    device.processes.push(NpuProcess {
                        pid,
                        name,
                        npu_index,
                        mem,
                    });
                }
            }
            continue;
        }
        let head: Vec<&str> = cells[0].split_whitespace().collect();
        let Some(first) = head.first().and_then(|token| token.parse::<u64>().ok()) else {
            continue;
        };
        if head.len() < 2 {
            continue;
        }
        if head[1].parse::<u64>().is_err() {
            // Card row: `NPU Name | Health | Power(W) Temp(C) Huge-Flash(T)`.
            card = Some(CardState {
                name: head[1..].join(" "),
                health: optional_string(&cells[1]),
                power: detail_tokens(&cells)
                    .first()
                    .and_then(|token| token.parse::<f64>().ok()),
                temperature: detail_tokens(&cells)
                    .get(1)
                    .and_then(|token| token.parse::<f64>().ok()),
            });
        } else {
            // Chip row. npu-smi repeats the NPU index in the first column
            // and distinguishes the dies in the second (`Chip Device`):
            // `| 0 0 |` and `| 0 1 |` are the two dies of one card.
            let Ok(chip_id) = head[1].parse::<u64>() else {
                continue;
            };
            let state = card.as_ref();
            let detail = detail_tokens(&cells);
            let memory = slash_pair(&detail);
            devices.push(NpuDevice {
                device_key: format!("ascend:{}:{}", first, chip_id),
                npu_index: first,
                chip_id,
                name: state.map_or_else(String::new, |state| state.name.clone()),
                health: state.and_then(|state| state.health.clone()),
                power: state.and_then(|state| state.power),
                temperature: state.and_then(|state| state.temperature),
                aicore: detail.first().and_then(|token| token.parse::<f64>().ok()),
                memory_label: Some(if hbm { "hbm" } else { "memory" }.to_string()),
                used_mem: memory.and_then(|(used, _)| used).map(mib_bytes),
                total_mem: memory.and_then(|(_, total)| total).map(mib_bytes),
                processes: Vec::new(),
            });
        }
    }
    serde_json::to_value(NpuOverview {
        available,
        cann,
        devices,
    })
    .unwrap_or_default()
}
