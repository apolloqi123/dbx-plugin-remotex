//! Scheduler Task Provider (`io.dbx.remotex.tasks`) — the plugin side of the
//! frozen Scheduler / Task Center contract
//! (`integration/docs/adr/scheduler-task-contract.md` §6).
//!
//! Division of labor: the host Scheduler owns trigger / queue / retry /
//! timeout policy / cancel bookkeeping / run state persistence; this module
//! owns the SSH protocol side — dialing through the existing connection
//! lifecycle, streaming stdout/stderr, honoring cancel with a real remote
//! teardown, resident supervision with a bounded restart policy, and
//! heartbeats.
//!
//! Hard rules enforced here:
//! - Only the five fixed RPCs exist (`task/validate|execute|start|stop|
//!   status`); there is deliberately no `ssh/runScheduledCommand`-style
//!   provider-specific surface.
//! - Credentials never enter task config, params, logs or events: task
//!   config carries a `connectionId`, the command text and env only, and
//!   every emitted line goes through [`Redactor`] with the connection's
//!   secret material before it leaves the process.
//! - Cancel/timeout actually stop the remote process: the channel is
//!   closed (`EOF` + `CLOSE`) so sshd reaps the command instead of leaking
//!   it — same invariant as `exec::exec_plain_cancellable`.
//! - Resident restarts are bounded (`max_restarts` within
//!   `restart_window_seconds`); exceeding the bound degrades the session
//!   instead of crash-looping forever.
//!
//! Run-mode executes are synchronous on purpose: the request future resolves
//! with the final `{success, exitCode, message, artifacts}` payload, and the
//! host's own execution-policy timeout plus `task/stop` remain the external
//! cancel paths.
//!
//! Bound-connection providers: besides SSH connections the provider accepts
//! `io.dbx.remotex.local` targets (`src/local_target.rs`) so scheduled commands
//! can run on the machine running DBX itself. The local-vs-SSH decision is
//! the stored connection's provider marker (the host rehydrates local
//! targets through `connection/connect`, the plugin registers them in
//! `SshRuntime::local_targets`); on a local target the run spawns the
//! connection's shell through `tokio::process::Command` — no TCP, no
//! credential pipeline — with the same streaming, cancel (process-group
//! kill), timeout, env and working-directory semantics as the SSH path.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::sessions::ssh::exec::exec_plain;

#[path = "task_config.rs"]
mod task_config;
#[path = "task_provider_impl.rs"]
mod task_provider_impl;
#[path = "task_redact.rs"]
mod task_redact;
#[path = "task_resident.rs"]
mod task_resident;
#[path = "task_streaming.rs"]
mod task_streaming;

#[cfg(test)]
#[path = "task_provider_tests.rs"]
mod task_provider_tests;
use crate::sessions::local::target::LocalTarget;
use dbx_plugin_sdk::PluginEmitter;
use russh::client::Handle;
use russh::ChannelMsg;
use russh_sftp::client::SftpSession;
use serde_json::{json, Value};
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio::sync::watch;

use crate::sessions::ssh::{sftp_client_config, SshClient, SshRuntime};

/// Machine-readable error codes prefixed onto RPC error strings. The host
/// maps these onto `TaskError` kinds / reserved `error_code` vocabulary
/// (ADR §5.4/§8.4): `invalid_config` / `connection_missing` /
/// `permission_denied` are non-retryable, `timeout` / `cancelled` map to
/// their run states, everything unprefixed stays a plain transport failure.
pub(crate) const CODE_INVALID_CONFIG: &str = "invalid_config";
pub(crate) const CODE_CONNECTION_MISSING: &str = "connection_missing";
pub(crate) const CODE_TIMEOUT: &str = "timeout";
pub(crate) const CODE_CANCELLED: &str = "cancelled";

#[allow(unused_imports)]
use task_config::*;
#[allow(unused_imports)]
use task_provider_impl::*;
#[allow(unused_imports)]
use task_redact::*;
#[allow(unused_imports)]
use task_resident::*;
#[allow(unused_imports)]
use task_streaming::*;

const TRIGGERS: [&str; 2] = ["execute", "resident"];

/// Per-stream partial-line flush cap: a silent `tail -F` writing one giant
/// line must still produce visible log events instead of buffering forever.
const PARTIAL_LINE_LIMIT: usize = 8192;
/// Resident heartbeat cadence while the channel is alive: a quiet `tail -F`
/// produces no output chunks, so the ticker is what advances `heartbeatAt`
/// for the host supervisor.
const RESIDENT_HEARTBEAT_SECS: u64 = 20;
/// Config-independent restart ceiling: even a misconfigured task cannot ask
/// the supervisor for an unbounded loop.
const HARD_MAX_RESTARTS: u32 = 50;
/// Backoff floor: an immediate restart storm on a dead endpoint is worse
/// than a slightly slow one.
const MIN_RESTART_BACKOFF_SECS: u64 = 1;
/// Local script uploads (and their SSH temp-file round-trip) are small by
/// nature — a scheduler task bootstrap script, not a data transfer. The cap
/// keeps one fat `script_path` from stalling the channel for minutes; real
/// file movement belongs on the SFTP surface.
const SCRIPT_UPLOAD_LIMIT: usize = 4 * 1024 * 1024;

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 5,
            backoff_seconds: 10,
            window_seconds: None,
        }
    }
}

impl RestartPolicy {
    fn parse(value: &Value, errors: &mut Vec<String>) -> Self {
        let mut policy = Self::default();
        if let Some(raw) = value.get("max_restarts") {
            match raw.as_u64().map(|n| u32::try_from(n).ok()) {
                Some(Some(n)) if n <= HARD_MAX_RESTARTS => policy.max_restarts = n,
                _ => errors.push(format!(
                    "max_restarts must be an integer between 0 and {HARD_MAX_RESTARTS}"
                )),
            }
        }
        if let Some(raw) = value.get("restart_backoff_seconds") {
            match raw.as_u64() {
                Some(n) if (MIN_RESTART_BACKOFF_SECS..=3600).contains(&n) => {
                    policy.backoff_seconds = n;
                }
                _ => errors.push(format!(
                    "restart_backoff_seconds must be between {MIN_RESTART_BACKOFF_SECS} and 3600"
                )),
            }
        }
        if let Some(raw) = value.get("restart_window_seconds") {
            match raw.as_u64() {
                Some(n) if (1..=86_400).contains(&n) => policy.window_seconds = Some(n),
                _ => errors.push("restart_window_seconds must be between 1 and 86400".to_string()),
            }
        }
        policy
    }

    /// Pure restart-budget decision: `history` holds the instants of past
    /// restarts. A restart is allowed only while the (window-pruned) history
    /// still has budget left; otherwise the caller degrades the session.
    fn restart_allowed(&self, history: &[Instant], now: Instant) -> bool {
        let recent = match self.window_seconds {
            Some(window) => {
                let cutoff = now.checked_sub(Duration::from_secs(window));
                history
                    .iter()
                    .copied()
                    .filter(|instant| cutoff.is_none_or(|cutoff| *instant >= cutoff))
                    .count()
            }
            None => history.len(),
        };
        recent < self.max_restarts as usize
    }
}

/// Provider state: one instance per sidecar process, shared by the fixed
/// `task/*` RPC arms. Nothing here persists — the host owns persistence and
/// reconciles resident sessions through `task/status` after a sidecar
/// restart (unknown session ids report `stopped`).
pub(crate) struct TaskProvider {
    /// run-mode cancel signals keyed by runId (`task/stop` target).
    runs: Mutex<HashMap<String, watch::Sender<bool>>>,
    /// Per-run monotonic log sequence (contract: starts at 1).
    seqs: Mutex<HashMap<String, u64>>,
    /// Live resident sessions keyed by sessionId. Entries leave this map the
    /// moment they reach a terminal state, so repeated start/stop cycles cannot
    /// grow it without bound.
    residents: Mutex<HashMap<String, ResidentEntry>>,
    /// Bounded tail of terminal resident summaries, newest last. Kept only so
    /// `task/status` can still report the real final state (`degraded`,
    /// `crashed`) of a session that already left `residents`, instead of the
    /// blanket `stopped` used for genuinely unknown ids.
    resident_history: Mutex<VecDeque<ResidentSummary>>,
}

/// Upper bound on retained terminal resident summaries. The host keeps its own
/// run history; this window only serves in-process reconciliation.
pub(super) const RESIDENT_HISTORY_CAP: usize = 64;

impl TaskProvider {}

// ---------------------------------------------------------------------------
// Tests (pure logic; live end-to-end coverage lives in
// scripts/smoke/smoke_task_provider.py against the dbx-ssh-test container)
// ---------------------------------------------------------------------------
