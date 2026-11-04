//! Docker management panel backend (`docker/list` / `docker/logs` /
//! `docker/logs/start` / `docker/logs/stop` / `docker/action` /
//! `docker/terminal/*`): POSIX collector scripts executed through `sh -s`
//! heredocs plus pure parsers, in the same style as
//! [`crate::features::metrics`]. Zero new dependencies for the collector
//! path by design — the Docker CLI is driven over shell, never the daemon
//! API, and no docker client crate is pulled in. The interactive terminal
//! (Task 3) adds a PTY channel over the already-authenticated SSH transport
//! and `portable-pty` for the local plane.
//!
//! Safety model for actions: the verb comes from a fixed whitelist, the
//! container id must match `^[0-9a-f]{12,64}$`, read-only connections are
//! refused by the caller (`ensure_writable`), the intent is audited before
//! execution, and the sudo password never travels on the command line —
//! the fallback reuses the Quick Sudo pipeline (`exec_with_sudo`) which
//! pipes credentials over stdin.

use crate::features::security::audit as audit_log;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::sessions::ssh::exec;

mod action;
mod engine;
pub(crate) mod files;
mod local;
mod parse;
mod scripts;
mod shell;
mod shell_local;
mod stats_limits;
mod stream;
mod stream_batch;

#[cfg(test)]
mod shell_local_tests;
#[cfg(test)]
mod shell_tests;
#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use action::*;
#[allow(unused_imports)]
pub use engine::*;
#[allow(unused_imports)]
pub use files::*;
#[allow(unused_imports)]
pub use local::*;
#[allow(unused_imports)]
pub use parse::*;
#[allow(unused_imports)]
pub use scripts::*;
pub use shell::*;
pub use stats_limits::*;
pub use stream::*;
pub use stream_batch::*;

/// Wait cap for the container list collector (one `docker ps` round-trip).
pub const LIST_TIMEOUT: Duration = Duration::from_secs(15);
/// Wait cap for `docker logs --tail N`; tailing is bounded by `--tail`.
pub const LOGS_TIMEOUT: Duration = Duration::from_secs(30);
/// Wait cap for one `docker stats --no-stream` snapshot (one daemon round-trip
/// per poll; the monitor tab samples every few seconds).
pub const STATS_TIMEOUT: Duration = Duration::from_secs(10);
/// Wait cap for lifecycle actions; `stop`/`restart` default to a 10s
/// container shutdown timeout, so 60s covers the worst case with slack.
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Bounds for the `--tail` line count of `docker/logs`.
pub const TAIL_MIN: u64 = 10;
pub const TAIL_MAX: u64 = 1000;
pub const TAIL_DEFAULT: u64 = 200;

/// Test-only engine constructor: the same shape `parse_engine` produces, so
/// command-rendering tests do not have to build JSON params to get an
/// endpoint-carrying engine.
#[cfg(test)]
pub(crate) fn test_engine(cli: &str, endpoint: Option<&str>) -> EngineConfig {
    EngineConfig {
        cli: cli.to_string(),
        endpoint: endpoint.map(str::to_string),
    }
}

// —— Engine / endpoint configuration (Podman & custom daemons) ————————

/// Default container CLI when the caller passes no `cli` parameter.
pub const DEFAULT_CLI: &str = "docker";

/// The `docker ps --format` template shared by the remote LIST_SCRIPT and
/// the local direct-exec collector (7 tab-separated columns, docker's own
/// `\t` escapes). One test pins LIST_SCRIPT to this constant so the two
/// collection paths cannot drift apart.
pub const PS_FORMAT: &str =
    "{{.ID}}\\t{{.Names}}\\t{{.Image}}\\t{{.State}}\\t{{.Status}}\\t{{.Ports}}\\t{{.CreatedAt}}";

/// Pre-1.13 fallback template: `{{.State}}` entered `docker ps --format`
/// with Docker 1.13 (API 1.25); older CLIs (CentOS 7 extras ships 1.12)
/// fail the whole template with a parsing error whose stderr the collector
/// swallows — the panel then showed an empty list with no hint (issue
/// #168). The legacy set keeps the same column ORDER minus State; the
/// parser derives state from the Status text. It runs only as a second
/// in-script attempt when the modern format produced no rows.
pub const PS_FORMAT_LEGACY: &str =
    "{{.ID}}\\t{{.Names}}\\t{{.Image}}\\t{{.Status}}\\t{{.Ports}}\\t{{.CreatedAt}}";

/// The section marker emitted right before the legacy-format retry. The
/// parser splits the `DBXDOCKER_PS` section on it; absent means the modern
/// attempt produced rows (or the host is empty).
pub const PS_LEGACY_MARKER: &str = "DBXDOCKER_PS_LEGACY";

/// `docker stats` row template: one JSON object per sample carrying CPU%,
/// memory usage/limit, network/block IO and PIDs (Docker ≥17.09). Older
/// CLIs reject it; the payload then degrades to the stopped/denied shapes
/// the parser already distinguishes, never a crash.
pub const STATS_FORMAT: &str = "{{json .}}";

/// Marker line that splits the stats snapshot from shell noise.
pub const STATS_MARKER: &str = "DBXDOCKER_STATS";

/// PATH hardening prepended to every remote collector heredoc (same
/// well-known install dirs as the local CLI discovery below). Actions keep
/// the bare cli verb: the approval text is derived from that exact
/// rendering, and a list that succeeded proves the CLI resolves.
pub const PATH_PREFIX: &str = concat!(
    "PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin:/snap/bin",
    ":/Applications/Docker.app/Contents/Resources/bin:$HOME/.docker/bin",
    ":$HOME/.orbstack/bin:$HOME/bin:$HOME/.local/bin\"\n",
);
