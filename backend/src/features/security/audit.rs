//! Append-only JSONL execution audit trail (`<data_dir>/audit-log.jsonl`) for
//! the MCP/AI execution plane. The audited surface is exactly the one the sudo
//! allowlist governs: every gated tool call (gate verdict, approval-lifecycle
//! outcome, exit code, duration, execution mode) plus approval lifecycle
//! events (prompt / approved / denied / timeout / remembered). Manual
//! workbench operations — interactive terminal typing, SFTP browsing, the
//! sudo file panel — are deliberately NOT recorded (IMPL_PLAN §0 non-goals).
//!
//! Cross-process semantics (decision D5): the embedded sidecar and the stdio
//! `--mcp` process share one data directory, so each entry is written with a
//! fresh open-append-close cycle and a single `write_all` of `json + "\n"` —
//! one `O_APPEND` write per line keeps concurrent inter-process appends
//! line-atomic. A process-local `Mutex` serializes the size-check + rotation
//! within a single process; inter-process races fall back to the O_APPEND
//! guarantee.
//!
//! Rotation: before each append, when the file has reached [`ROTATE_BYTES`]
//! (5 MiB) it is renamed to `audit-log.jsonl.1` (one retained generation; a
//! failed rename is ignored so writing never blocks). Reading (`tail`) spans
//! both generations — newest first, backfilling from the rotated predecessor —
//! so a rotation never hides previously visible entries. Malformed lines are
//! skipped, so a torn trailing line from a crash can never break replay.
//! `error` text is clamped before writing to keep worst-case lines small (long
//! command text is already clamped upstream).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Rotation threshold: a single audit file caps at 5 MiB before rolling to
/// `audit-log.jsonl.1`.
pub const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

/// Hard cap applied to `tail`'s `limit` regardless of caller input (the
/// protocol `ssh/audit/list` contract is limit ∈ [1, 500], enforced there;
/// this clamps defensively). One slot above the protocol maximum is allowed so
/// `ssh/audit/list` can fetch a single probe entry past the requested page and
/// still detect truncation at `limit = 500` (without it, a page of 500 could
/// never report that older entries remain).
const TAIL_LIMIT_CAP: usize = 501;

/// Cap of the `error` field, in characters, applied before writing.
const MAX_ERROR_CHARS: usize = 1024;

/// Cap of the `command` field, in characters, applied before writing.
const MAX_COMMAND_CHARS: usize = 512;

/// Cap of the `output` field, in characters, applied before writing. The
/// audit ledger is a replay of *what ran*, not a full transcript: a tail of
/// the output is enough to see what a command did.
const MAX_OUTPUT_CHARS: usize = 1024;

/// Which branch of the existing exec gate sequence produced this entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome {
    /// All gates passed; the command ran.
    Pass,
    /// Write tool refused (read-only / write-disabled policy).
    WriteDenied,
    /// Command rejected by the sudo allowlist match gate.
    WhitelistDenied,
    /// Command touched a sensitive path.
    SensitivePath,
    /// Destructive command without a confirmed token.
    DestructiveUnconfirmed,
    /// Sudo command rejected by the sudo allowlist gate.
    SudoAllowlistDenied,
    /// Server advertises read-only.
    ReadOnlyServer,
}

/// Approval lifecycle trail for the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalTrail {
    /// No approval interaction happened (auto-run, or refused by a gate).
    None,
    /// An approval prompt was raised (final decision carried elsewhere).
    Prompt,
    /// Approved by the human.
    Approved,
    /// Denied by the human.
    Denied,
    /// Prompt unanswered until timeout (treated as refusal).
    Timeout,
    /// Admitted by the remembered-commands allowlist (no prompt).
    Remembered,
}

/// Terminal result of the audited action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryOutcome {
    Ok,
    Error,
}

/// Execution channel the action used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecMode {
    /// Hidden stdio channel (plain ssh_exec).
    Stdio,
    /// Embedded sidecar MCP process.
    Embedded,
    /// Injected into the user's workbench terminal.
    Terminal,
}

/// One audit record. Serde shape is the `ssh/audit/list` entry contract:
/// camelCase keys, enum string values exactly as in PROTOCOL, and `None`
/// serialized as `null` (no `skip_serializing_if`) so every line keeps the
/// same shape. `command`/`output` carry the audited command text and its
/// (truncated) result — clamped at both the write sites and in [`append`].
#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    pub ts_ms: u64,
    pub tool: String,
    pub connection_id: String,
    pub gate: GateOutcome,
    pub approval: ApprovalTrail,
    pub outcome: EntryOutcome,
    pub exit_code: Option<i64>,
    pub duration_ms: u64,
    pub mode: ExecMode,
    pub command: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
}

mod codec;
mod ledger;

#[cfg(test)]
mod tests;

pub use ledger::*;
