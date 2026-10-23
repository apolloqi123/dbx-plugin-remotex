//! Production misoperation guards for the MCP exec tools.
//!
//! Every `ssh_exec` / `ssh_exec_sudo` command is classified before it is
//! executed:
//!
//! - [`CommandRisk::ReadOnly`] — provably read-only (inspection verbs such as
//!   `ls`, `df`, `systemctl status`). The only class allowed through the
//!   read-only connection gate.
//! - [`CommandRisk::Destructive`] — matches a known catastrophic pattern
//!   (disk formatting, recursive deletes of system roots, shutdown, raw
//!   device writes, ...). Refused outright on read-only connections and
//!   requires `confirmDestructive: true` everywhere else.
//! - [`CommandRisk::Unknown`] — everything else. Allowed on normal
//!   connections, refused on read-only ones (whitelist, not blacklist:
//!   unrecognized means unproven).
//!
//! The classifier is deliberately conservative: quoted strings are not
//! parsed (a `;` inside quotes still splits segments, which can only
//! downgrade `ReadOnly` to `Unknown`, never upgrade), command substitution
//! and output redirection force `Unknown`, and privileged/wrapper prefixes
//! are unwrapped before the inner command is assessed.

mod assess;
mod segment;
mod tables;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use assess::*;
#[allow(unused_imports)]
pub use segment::*;
#[allow(unused_imports)]
pub use tables::*;
