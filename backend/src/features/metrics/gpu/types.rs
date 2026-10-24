//! GPU / NPU metric payload types.

use super::*;

/// Top-level NVIDIA overview: `available` mirrors the `GPU_AVAILABLE` probe
/// line; `gpus` carries one entry per physical GPU with its compute
/// processes attached.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuOverview {
    pub available: bool,
    pub gpus: Vec<GpuInfo>,
}

/// One physical GPU. Device identity is the uuid (stable across driver
/// reloads and unique under MIG); memory fields are byte counts.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub index: u64,
    pub uuid: String,
    pub name: String,
    pub driver: Option<String>,
    /// GPU temperature in degrees Celsius.
    pub temperature: Option<f64>,
    /// GPU utilization percent.
    pub utilization: Option<f64>,
    /// Memory-controller utilization percent (`utilization.memory`).
    pub mem_util: Option<f64>,
    pub total_mem: Option<u64>,
    pub used_mem: Option<u64>,
    pub free_mem: Option<u64>,
    /// Current board power draw in watts.
    pub power_draw: Option<f64>,
    /// Enforced power limit in watts (`[Not Supported]` on many boards).
    pub power_limit: Option<f64>,
    /// Fan speed percent (`[N/A]` on passively cooled boards).
    pub fan: Option<f64>,
    /// Performance state such as `P0` (max) or `P8` (idle).
    pub pstate: Option<String>,
    pub processes: Vec<GpuProcess>,
}

/// One compute process on a GPU, joined onto its card by uuid.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuProcess {
    pub uuid: String,
    pub pid: u64,
    /// GPU memory used by the process, in bytes.
    pub mem: Option<u64>,
    pub name: String,
}

/// Top-level Ascend overview. `cann` is the toolkit/nnae/nnrt version string
/// when an Ascend install-info file was readable.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NpuOverview {
    pub available: bool,
    pub cann: Option<String>,
    pub devices: Vec<NpuDevice>,
}

/// One NPU chip. Ascend exposes one-card-many-chip cards as repeated rows
/// under the same NPU index, so the device key pairs the NPU index with the
/// chip id (`ascend:{npu_index}:{chip_id}`). Memory fields are byte counts.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NpuDevice {
    pub device_key: String,
    pub npu_index: u64,
    pub chip_id: u64,
    /// Card model such as `910B4` or `310P3`.
    pub name: String,
    /// `OK` / `Warning` / `Error` as reported by npu-smi.
    pub health: Option<String>,
    /// Board power draw in watts.
    pub power: Option<f64>,
    /// Chip temperature in degrees Celsius.
    pub temperature: Option<f64>,
    /// AI Core utilization percent.
    pub aicore: Option<f64>,
    /// `hbm` when the header says HBM-Usage, `memory` otherwise.
    pub memory_label: Option<String>,
    pub used_mem: Option<u64>,
    pub total_mem: Option<u64>,
    pub processes: Vec<NpuProcess>,
}

/// One compute process on an NPU chip (joined by NPU index; the process
/// table's Device ID column carries the NPU index).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NpuProcess {
    pub pid: u64,
    pub name: String,
    pub npu_index: u64,
    /// Device memory used by the process, in bytes.
    pub mem: Option<u64>,
}
