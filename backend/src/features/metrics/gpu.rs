//! GPU / Ascend NPU accelerator overview sections for the metrics document
//! (IMPL_PLAN Task P1-4). The shape mirrors [`crate::features::metrics`]: one POSIX
//! shell collector per accelerator family whose raw output is wrapped in
//! `BEGIN`/`END` marker lines, plus pure parser functions so hand-written
//! Linux fixtures drive the unit tests without a server.
//!
//! * NVIDIA: `nvidia-smi --query-gpu` CSV (quote-aware: model names may
//!   contain commas) + `--query-compute-apps` processes joined onto their
//!   GPU by uuid (MIG `GPU-uuid/…` instance rows included). `N/A`,
//!   `[Not Supported]` and empty fields become JSON null.
//! * Ascend: `npu-smi info` found on `PATH`, `/usr/local/bin` or the CANN
//!   driver tools directory; the `|`-separated table is parsed into
//!   per-chip devices keyed `ascend:{npu_index}:{chip_id}` (one card with
//!   several dies yields several devices), HBM preferred over Memory when
//!   the header says so, and a missing process table degrades to empty.

use russh::client::Handle;
use serde::Serialize;

use crate::core::exec::exec_plain;
use crate::sessions::ssh::SshClient;

/// MiB per byte-count unit: `nvidia-smi --format=csv,nounits` memory columns
/// and npu-smi `*-Usage(MB)` columns are MiB; the JSON payload exposes byte
/// counts so the frontend can reuse the shared byte formatter directly.
const MIB: u64 = 1024 * 1024;

/// Read-only collector: `command -v` availability probe first (`GPU_AVAILABLE
/// 0/1`), then the two CSV queries wrapped in marker lines so a driver that
/// half-works cannot corrupt the framing.
pub fn gpu_overview_script() -> String {
    concat!(
        "if command -v nvidia-smi >/dev/null 2>&1; then ",
        "echo 'GPU_AVAILABLE 1'; ",
        "echo GPU_CSV_BEGIN; ",
        "nvidia-smi --query-gpu=index,uuid,name,driver_version,temperature.gpu,utilization.gpu,utilization.memory,memory.total,memory.used,memory.free,power.draw,power.limit,fan.speed,pstate --format=csv,noheader,nounits 2>/dev/null; ",
        "echo GPU_CSV_END; ",
        "echo GPU_PROCESS_CSV_BEGIN; ",
        "nvidia-smi --query-compute-apps=gpu_uuid,pid,used_gpu_memory,process_name --format=csv,noheader,nounits 2>/dev/null; ",
        "echo GPU_PROCESS_CSV_END; ",
        "else echo 'GPU_AVAILABLE 0'; fi",
    )
    .to_string()
}

/// Read-only collector: npu-smi probed on `PATH`, then the two well-known
/// absolute locations (the driver tools directory glob needs `-x` checks
/// because unmatched globs stay literal), then the CANN toolkit version from
/// the Ascend install-info files (toolkit, nnae, nnrt — first hit wins).
pub fn npu_overview_script() -> String {
    concat!(
        "npu_bin=''; ",
        "if command -v npu-smi >/dev/null 2>&1; then npu_bin='npu-smi'; ",
        "elif [ -x /usr/local/bin/npu-smi ]; then npu_bin='/usr/local/bin/npu-smi'; ",
        "else for p in /usr/local/Ascend/driver/tools/*/npu-smi; do ",
        "if [ -x \"$p\" ]; then npu_bin=\"$p\"; break; fi; done; fi; ",
        "if [ -n \"$npu_bin\" ]; then ",
        "echo 'NPU_AVAILABLE 1'; ",
        "echo NPU_SMI_BEGIN; ",
        "\"$npu_bin\" info 2>/dev/null; ",
        "echo NPU_SMI_END; ",
        "else echo 'NPU_AVAILABLE 0'; fi; ",
        "for f in /usr/local/Ascend/ascend_toolkit_install.info /usr/local/Ascend/ascend_nnae_install.info /usr/local/Ascend/ascend_nnrt_install.info; do ",
        "if [ -r \"$f\" ]; then ",
        "cann=$(awk -F= '/^version=/{print $2; exit}' \"$f\" 2>/dev/null); ",
        "if [ -z \"$cann\" ]; then cann=$(head -n 1 \"$f\" 2>/dev/null); fi; ",
        "if [ -n \"$cann\" ]; then echo \"NPU_CANN $cann\"; break; fi; fi; done",
    )
    .to_string()
}

// `gpu.rs` is loaded through `#[path = "gpu.rs"]`, so rustc resolves child
// modules against `metrics/`; the explicit paths keep them in `gpu/`.
#[path = "gpu/parse_gpu.rs"]
mod parse_gpu;
#[path = "gpu/parse_npu.rs"]
mod parse_npu;
#[path = "gpu/parse_shared.rs"]
mod parse_shared;
#[path = "gpu/types.rs"]
mod types;

#[cfg(test)]
#[path = "gpu/tests.rs"]
mod tests;

use parse_shared::*;

pub use parse_gpu::*;
pub use parse_npu::*;
pub use types::*;
