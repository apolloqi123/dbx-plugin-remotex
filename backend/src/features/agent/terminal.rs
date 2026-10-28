//! AI terminal synchronous execution (agent terminal mode): pure routing,
//! sanitizing, and output-capture logic for commands an AI/MCP client runs
//! inside the user's interactive workbench terminal. No SSH I/O lives here —
//! everything is unit-testable without a connection (see the M1/M2 sections
//! of `docs/architecture/PROTOCOL.zh-CN.md`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::sessions::ssh::exec;

/// Connection-level agent terminal mode (`agentTerminalMode`). Persisted per
/// connection in `<data_dir>/agent-modes.json` so the toggle in the terminal
/// toolbar survives sidecar/app restarts; unknown connections read `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentTerminalMode {
    #[default]
    Off,
    Auto,
    Strict,
}

impl AgentTerminalMode {
    /// Lenient parse for stored/forwarded values: anything unknown degrades
    /// to `Off` instead of failing the connection.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "strict" => Self::Strict,
            _ => Self::Off,
        }
    }

    /// Strict parse for `ssh/settings/set`: an unknown value must be refused
    /// so a typo cannot silently disable the approval machinery. Built on
    /// [`Self::parse`] with a canonical-name round-trip check.
    pub fn parse_exact(value: &str) -> Result<Self, String> {
        let parsed = Self::parse(value);
        if parsed.name() == value.trim().to_ascii_lowercase() {
            Ok(parsed)
        } else {
            Err(format!(
                "agentTerminalMode must be \"off\", \"auto\", or \"strict\"; got '{value}'"
            ))
        }
    }

    /// Canonical lowercase name used in settings and event payloads.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Auto => "auto",
            Self::Strict => "strict",
        }
    }
}

/// Risk of one MCP command as seen by the terminal routing. Read-only views
/// auto-run in graded mode; anything that can change remote state prompts;
/// sudo/catastrophic commands are elevated (also prompt, distinct badge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRisk {
    ReadOnly,
    Mutating,
    Elevated,
}

impl CommandRisk {
    /// Canonical name carried in `ssh/agent/prompt` / `ssh/agent/notice`.
    pub fn name(self) -> &'static str {
        match self {
            Self::ReadOnly => "readonly",
            Self::Mutating => "mutating",
            Self::Elevated => "elevated",
        }
    }
}

mod capture;
mod policy;
mod recorder;

#[cfg(test)]
mod tests;

pub use capture::*;
pub use policy::*;
pub use recorder::*;
