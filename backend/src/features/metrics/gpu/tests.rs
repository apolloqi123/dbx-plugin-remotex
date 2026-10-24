use super::*;
use serde_json::json;

/// nvidia-smi CSV fixture: the 3090's model name is quoted (contains a
/// comma), the A100 reports `N/A` / `[Not Supported]` on several
/// counters, and one compute process points at a uuid no listed GPU owns.
const GPU_FIXTURE: &str = "\
GPU_AVAILABLE 1
GPU_CSV_BEGIN
0, GPU-5a1b2c3d-1111-2222-3333-444455556666, \"NVIDIA GeForce RTX 3090, 24GB\", 535.129.03, 45, 12, 30, 24576, 8192, 16384, 65.02, 350.00, 60, P2
1, GPU-9f8e7d6c-aaaa-bbbb-cccc-ddddeeeeffff, NVIDIA A100-SXM4-40GB, 535.129.03, [N/A], 0, 0, 40960, 1024, 39936, N/A, [Not Supported], [N/A], P0
GPU_CSV_END
GPU_PROCESS_CSV_BEGIN
GPU-5a1b2c3d-1111-2222-3333-444455556666, 4321, 4096, python3
GPU-5a1b2c3d-1111-2222-3333-444455556666, 4322, 512, /usr/local/bin/trainer --epochs 10
GPU-deadbeef-0000-0000-0000-000000000000, 999, 100, mystery
GPU_PROCESS_CSV_END
";

/// Collector output when nvidia-smi is not installed.
const GPU_UNAVAILABLE_FIXTURE: &str = "GPU_AVAILABLE 0\n";

/// npu-smi info fixture: header rows plus data rows in Ascend's
/// `|`-separated layout, npu 0 carries two chips (one card, two dies),
/// HBM column present, process table with Device-ID association and a
/// CANN toolkit version line.
const NPU_FIXTURE: &str = "\
NPU_AVAILABLE 1
NPU_SMI_BEGIN
+-------------------------------------------------------------------------------------------+
| npu-smi 23.0.0                            Version: 23.0.0                                 |
+---------------------------+----------------+----------------------------------------------+
| NPU     Name              | Health         | Power(W)     Temp(C)         Huge-Flash(T)  |
| Chip    Device            | Bus-Id         | Aicore(%)    ASICL(%)        HBM-Usage(MB)  |
+===========================+================+==============================================+
| 0       910B4             | OK             | 63.6         42               0 / 0         |
| 0       0                 | 0000:C1:00.0   | 0            0                8192 / 32509  |
| 0       1                 | 0000:C1:00.1   | 12           0                10240 / 32509 |
+===========================+================+==============================================+
| 1       310P3             | OK             | 43.5         40               0 / 0         |
| 1       0                 | 0000:C2:00.0   | 0            0                512 / 21504   |
+===========================+================+==============================================+
+-------------------------------------------------------------------------------------------+
| Process ID                 Process name                    | Device ID | Memory(MB)        |
+-------------------------------------------------------------------------------------------+
| 123456    python3                                           | 0         | 4096              |
| 123457    mindspore                                         | 1         | 2048              |
+-------------------------------------------------------------------------------------------+
NPU_SMI_END
NPU_CANN 8.0.RC1
";

/// npu-smi info without an HBM column (older DDR-based cards) and with
/// the process table missing entirely.
const NPU_DDR_NO_PROCS_FIXTURE: &str = "\
NPU_AVAILABLE 1
NPU_SMI_BEGIN
+---------------------------+----------------+----------------------------------------------+
| NPU     Name              | Health         | Power(W)     Temp(C)         Huge-Flash(T)  |
| Chip    Device            | Bus-Id         | Aicore(%)    ASICL(%)        DDR-Usage(MB)  |
+===========================+================+==============================================+
| 0       310P3             | OK             | 43.5         40               0 / 0         |
| 0       0                 | 0000:C1:00.0   | 0            0                512 / 21504   |
+===========================+================+==============================================+
NPU_SMI_END
";

const NPU_UNAVAILABLE_FIXTURE: &str = "NPU_AVAILABLE 0\n";

// —— GPU collector script ———————————————————————

#[test]
fn gpu_script_probes_and_wraps_both_queries() {
    let script = gpu_overview_script();
    assert!(script.contains("command -v nvidia-smi"), "{script}");
    assert!(script.contains("GPU_AVAILABLE"), "{script}");
    assert!(
        script.contains("GPU_CSV_BEGIN") && script.contains("GPU_CSV_END"),
        "{script}"
    );
    assert!(
        script.contains("GPU_PROCESS_CSV_BEGIN") && script.contains("GPU_PROCESS_CSV_END"),
        "{script}"
    );
    assert!(
        script.contains(
            "--query-gpu=index,uuid,name,driver_version,temperature.gpu,utilization.gpu,\
             utilization.memory,memory.total,memory.used,memory.free,power.draw,power.limit,\
             fan.speed,pstate"
        ),
        "{script}"
    );
    assert!(
        script.contains("--query-compute-apps=gpu_uuid,pid,used_gpu_memory,process_name"),
        "{script}"
    );
    assert!(script.contains("--format=csv,noheader,nounits"), "{script}");
}

#[test]
fn npu_script_probes_all_paths_and_reads_cann() {
    let script = npu_overview_script();
    assert!(script.contains("command -v npu-smi"), "{script}");
    assert!(script.contains("/usr/local/bin/npu-smi"), "{script}");
    assert!(
        script.contains("/usr/local/Ascend/driver/tools/*/npu-smi"),
        "{script}"
    );
    assert!(script.contains("NPU_AVAILABLE"), "{script}");
    assert!(
        script.contains("NPU_SMI_BEGIN") && script.contains("NPU_SMI_END"),
        "{script}"
    );
    assert!(script.contains("NPU_CANN"), "{script}");
    assert!(
        script.contains("/usr/local/Ascend/ascend_toolkit_install.info"),
        "{script}"
    );
    // nnae / nnrt toolkit variants are probed as CANN fallbacks.
    assert!(script.contains("ascend_nnae_install.info"), "{script}");
    assert!(script.contains("ascend_nnrt_install.info"), "{script}");
}

// —— GPU parsing ——————————————————————————————————

#[test]
fn parses_gpu_csv_with_quoted_names_and_na_fields() {
    let parsed = parse_gpu_overview_output(GPU_FIXTURE);
    assert_eq!(parsed["available"], json!(true));
    let gpus = parsed["gpus"].as_array().unwrap();
    assert_eq!(gpus.len(), 2);

    let rtx = &gpus[0];
    assert_eq!(rtx["index"], json!(0));
    assert_eq!(rtx["uuid"], "GPU-5a1b2c3d-1111-2222-3333-444455556666");
    // The quoted comma inside the model name stays one field.
    assert_eq!(rtx["name"], "NVIDIA GeForce RTX 3090, 24GB");
    assert_eq!(rtx["driver"], "535.129.03");
    assert_eq!(rtx["temperature"], json!(45.0));
    assert_eq!(rtx["utilization"], json!(12.0));
    assert_eq!(rtx["memUtil"], json!(30.0));
    // MiB CSV columns are exposed as byte counts.
    assert_eq!(rtx["totalMem"], json!(24_576_u64 * 1024 * 1024));
    assert_eq!(rtx["usedMem"], json!(8_192_u64 * 1024 * 1024));
    assert_eq!(rtx["freeMem"], json!(16_384_u64 * 1024 * 1024));
    assert_eq!(rtx["powerDraw"], json!(65.02));
    assert_eq!(rtx["powerLimit"], json!(350.0));
    assert_eq!(rtx["fan"], json!(60.0));
    assert_eq!(rtx["pstate"], "P2");

    let a100 = &gpus[1];
    // N/A / [Not Supported] collapse to null.
    assert_eq!(a100["temperature"], serde_json::Value::Null);
    assert_eq!(a100["powerDraw"], serde_json::Value::Null);
    assert_eq!(a100["powerLimit"], serde_json::Value::Null);
    assert_eq!(a100["fan"], serde_json::Value::Null);
    assert_eq!(a100["pstate"], "P0");
}

#[test]
fn gpu_processes_join_by_uuid_and_unknowns_drop() {
    let parsed = parse_gpu_overview_output(GPU_FIXTURE);
    let gpus = parsed["gpus"].as_array().unwrap();
    let rtx_processes = gpus[0]["processes"].as_array().unwrap();
    assert_eq!(rtx_processes.len(), 2);
    assert_eq!(rtx_processes[0]["pid"], json!(4321));
    assert_eq!(rtx_processes[0]["name"], "python3");
    assert_eq!(
        rtx_processes[0]["uuid"],
        "GPU-5a1b2c3d-1111-2222-3333-444455556666"
    );
    assert_eq!(rtx_processes[0]["mem"], json!(4_096_u64 * 1024 * 1024));
    // Process names may contain spaces; they stay one field.
    assert_eq!(
        rtx_processes[1]["name"],
        "/usr/local/bin/trainer --epochs 10"
    );
    // The A100 has no compute processes; the orphan process is dropped.
    assert!(gpus[1]["processes"].as_array().unwrap().is_empty());
}

#[test]
fn gpu_unavailable_yields_false_with_empty_list() {
    let parsed = parse_gpu_overview_output(GPU_UNAVAILABLE_FIXTURE);
    assert_eq!(parsed["available"], json!(false));
    assert!(parsed["gpus"].as_array().unwrap().is_empty());
}

#[test]
fn gpu_malformed_csv_rows_are_skipped() {
    let parsed = parse_gpu_overview_output(
        "GPU_AVAILABLE 1\nGPU_CSV_BEGIN\nnot,a,csv,row,at,all,,,,,,,x\n\
         0, GPU-ok, Short GPU, 1.0, 40, 5, 5, 8192, 100, 8092, 10, 100, 50\nGPU_CSV_END\n",
    );
    // The short row (13 fields) is skipped; nothing crashes.
    assert_eq!(parsed["gpus"].as_array().unwrap().len(), 0);
}

#[test]
fn gpu_escaped_quotes_inside_names_survive() {
    let parsed = parse_gpu_overview_output(
        "GPU_AVAILABLE 1\nGPU_CSV_BEGIN\n\
         0, GPU-qq, \"Model \"\"Pro\"\", 24GB\", 1.0, 40, 5, 5, 8192, 100, 8092, 10, 100, 50, P0\n\
         GPU_CSV_END\nGPU_PROCESS_CSV_BEGIN\nGPU_PROCESS_CSV_END\n",
    );
    assert_eq!(parsed["gpus"][0]["name"], "Model \"Pro\", 24GB");
}

// —— NPU parsing ——————————————————————————————————

#[test]
fn parses_npu_devices_with_hbm_and_multi_chip_card() {
    let parsed = parse_npu_overview_output(NPU_FIXTURE);
    assert_eq!(parsed["available"], json!(true));
    assert_eq!(parsed["cann"], "8.0.RC1");
    let devices = parsed["devices"].as_array().unwrap();
    // npu 0 owns two chips, npu 1 one chip.
    assert_eq!(devices.len(), 3);

    let first = &devices[0];
    assert_eq!(first["deviceKey"], "ascend:0:0");
    assert_eq!(first["npuIndex"], json!(0));
    assert_eq!(first["chipId"], json!(0));
    assert_eq!(first["name"], "910B4");
    assert_eq!(first["health"], "OK");
    assert_eq!(first["power"], json!(63.6));
    assert_eq!(first["temperature"], json!(42.0));
    assert_eq!(first["aicore"], json!(0.0));
    // HBM preferred: the chip header says HBM-Usage.
    assert_eq!(first["memoryLabel"], "hbm");
    assert_eq!(first["usedMem"], json!(8_192_u64 * 1024 * 1024));
    assert_eq!(first["totalMem"], json!(32_509_u64 * 1024 * 1024));

    let second = &devices[1];
    assert_eq!(second["deviceKey"], "ascend:0:1");
    assert_eq!(second["chipId"], json!(1));
    assert_eq!(second["aicore"], json!(12.0));
    assert_eq!(second["usedMem"], json!(10_240_u64 * 1024 * 1024));

    let third = &devices[2];
    assert_eq!(third["deviceKey"], "ascend:1:0");
    assert_eq!(third["name"], "310P3");
    assert_eq!(third["usedMem"], json!(512_u64 * 1024 * 1024));
}

#[test]
fn npu_processes_attach_to_matching_npu_index() {
    let parsed = parse_npu_overview_output(NPU_FIXTURE);
    let devices = parsed["devices"].as_array().unwrap();
    let first_processes = devices[0]["processes"].as_array().unwrap();
    assert_eq!(first_processes.len(), 1);
    assert_eq!(first_processes[0]["pid"], json!(123456));
    assert_eq!(first_processes[0]["name"], "python3");
    assert_eq!(first_processes[0]["npuIndex"], json!(0));
    assert_eq!(first_processes[0]["mem"], json!(4_096_u64 * 1024 * 1024));
    // npu 1's process lands on the matching device; npu 0 chip 1 has none.
    assert_eq!(devices[1]["processes"].as_array().unwrap().len(), 0);
    assert_eq!(devices[2]["processes"].as_array().unwrap().len(), 1);
    assert_eq!(devices[2]["processes"][0]["pid"], json!(123457));
}

#[test]
fn npu_without_hbm_uses_memory_label_and_tolerates_missing_process_table() {
    let parsed = parse_npu_overview_output(NPU_DDR_NO_PROCS_FIXTURE);
    assert_eq!(parsed["available"], json!(true));
    assert_eq!(parsed["cann"], serde_json::Value::Null);
    let devices = parsed["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["memoryLabel"], "memory");
    assert!(devices[0]["processes"].as_array().unwrap().is_empty());
}

#[test]
fn npu_unavailable_yields_false_with_empty_list() {
    let parsed = parse_npu_overview_output(NPU_UNAVAILABLE_FIXTURE);
    assert_eq!(parsed["available"], json!(false));
    assert!(parsed["devices"].as_array().unwrap().is_empty());
    assert_eq!(parsed["cann"], serde_json::Value::Null);
}

#[test]
fn npu_empty_or_absent_sections_degrade_cleanly() {
    // A probe that found the binary but the info call failed.
    let parsed = parse_npu_overview_output("NPU_AVAILABLE 1\nNPU_SMI_BEGIN\nNPU_SMI_END\n");
    assert_eq!(parsed["available"], json!(true));
    assert!(parsed["devices"].as_array().unwrap().is_empty());
    // No output at all (exec failed on a weird shell): everything empty.
    let empty = parse_npu_overview_output("");
    assert_eq!(empty["available"], json!(false));
    assert!(empty["devices"].as_array().unwrap().is_empty());
}
