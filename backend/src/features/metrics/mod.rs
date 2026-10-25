//! Extended server metrics: per-interface network rates and top processes,
//! appended to the base CPU/memory/disk sample from [`crate::sessions::ssh::exec`].
//!
//! The collector stays a single POSIX shell command in the same style as the
//! base script: two network counter snapshots separated by `sleep 1`
//! (`/proc/net/dev` on Linux, `netstat -ibn` on macOS/BSD) and one `ps`
//! invocation sorted by CPU. CPU/memory/load/uptime prefer the Linux `/proc`
//! readers and fall back to sysctl/`vm_stat`/`iostat` on macOS; the fallback
//! branches re-emit the same line shapes so the shared parsers stay
//! unchanged. All parsing lives in pure functions so Linux and macOS sample
//! fixtures can be unit-tested without a server.

pub mod history;

use std::collections::BTreeMap;

use russh::client::Handle;
use serde::Serialize;

// Lives in its own file but stays nested under `crate::features::metrics` so main.rs's
// flat `mod` list (integrator-owned) is untouched.
#[path = "gpu.rs"]
pub mod metrics_gpu;

use crate::sessions::ssh::exec::exec_plain;
use crate::sessions::ssh::SshClient;

mod collect;
mod network;
mod parse;
mod process;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use collect::*;
#[allow(unused_imports)]
pub(crate) use network::*;
#[allow(unused_imports)]
pub(crate) use parse::*;
#[allow(unused_imports)]
pub(crate) use process::*;
