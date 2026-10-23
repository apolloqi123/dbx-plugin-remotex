//! MCP scope layer.

use super::*;

/// Effective connection-scope policy. Persisted empty arrays retain their
/// original "unrestricted" UI meaning, while an explicitly empty environment
/// override is fail-closed and therefore must use a distinct representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConnectionScope {
    Unrestricted,
    DenyAll,
    AllowList(Vec<String>),
}

impl ConnectionScope {
    pub(crate) fn from_persisted(entries: Vec<String>) -> Self {
        if entries.is_empty() {
            Self::Unrestricted
        } else {
            Self::AllowList(entries)
        }
    }

    pub(crate) fn allows(&self, id: &str, name: Option<&str>, host: &str) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::DenyAll => false,
            Self::AllowList(entries) => scope_allows(entries, id, name, host),
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        !matches!(self, Self::Unrestricted)
    }

    pub(crate) fn entries(&self) -> &[String] {
        match self {
            Self::AllowList(entries) => entries,
            Self::Unrestricted | Self::DenyAll => &[],
        }
    }
}

/// Parses `DBX_SSH_MCP_CONNECTION_SCOPE` (comma-separated entries). An env
/// var that is set but parses to zero entries still counts as an override —
/// an operator pinning an empty list means "no connections", not "unset".
pub(crate) fn scope_from_env(
    lookup: impl FnOnce(&str) -> Option<String>,
) -> Option<ConnectionScope> {
    let value = lookup("DBX_SSH_MCP_CONNECTION_SCOPE")?;
    let entries: Vec<String> = value
        .split(',')
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .take(CONNECTION_SCOPE_MAX_ENTRIES)
        .collect();
    Some(if entries.is_empty() {
        ConnectionScope::DenyAll
    } else {
        ConnectionScope::AllowList(entries)
    })
}

/// Scope match rule (§1.3): an entry allows a connection when it equals the
/// connection id, equals the connection name, or equals the host ASCII
/// case-insensitively. Empty entry lists are only used by callers that have
/// already selected the unrestricted policy.
pub fn scope_allows(scope: &[String], id: &str, name: Option<&str>, host: &str) -> bool {
    scope.iter().any(|entry| {
        entry == id
            || name.map(|name| entry == name).unwrap_or(false)
            || entry.eq_ignore_ascii_case(host)
    })
}

/// Confirm-mode gate set (§1.3): write tools plus the exec family.
/// Read-only tools and `ssh_close` are never intercepted.
pub(crate) fn is_confirm_gated_tool(name: &str) -> bool {
    is_write_tool(name) || matches!(name, "ssh_exec" | "ssh_multi_exec" | "ssh_terminal_input")
}

/// Tools whose resolved target must sit inside a non-empty
/// `connectionScope` (§1.3). Local tools (known-hosts management, the list
/// tool itself) stay outside.
pub(crate) fn is_connection_scoped_tool(name: &str) -> bool {
    if is_confirm_gated_tool(name) {
        return true;
    }
    matches!(
        name,
        "ssh_task_status"
            | "ssh_close"
            | "sftp_list_dir"
            | "sftp_stat"
            | "sftp_read_file"
            | "sftp_write_file"
            | "sftp_upload"
            | "sftp_download"
    )
}
