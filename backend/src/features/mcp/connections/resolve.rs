//! Connection resolution and credential binding.

use super::*;

pub(in crate::features::mcp) fn connection_pool_id(arguments: &Value) -> String {
    let host = arguments.get("host").and_then(Value::as_str).unwrap_or("");
    let port = arg_port_lossy(arguments);
    let username = arguments
        .get("username")
        .and_then(Value::as_str)
        .unwrap_or("");
    format!("mcp-{username}@{host}:{port}")
}

/// Pool key for an arguments payload: an explicit `connectionId` (the DBX
/// bridge path) wins over inline credentials, keeping `ssh_close` on the same
/// pool key `connection()` used to register the handle.
pub(in crate::features::mcp) fn connection_pool_key(arguments: &Value) -> String {
    arguments
        .get("connectionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| connection_pool_id(arguments))
}

/// Tools whose execution targets a saved connection: in stdio mode an
/// unregistered `connectionId` is forwarded through the DBX app bridge (L1
/// fallback) instead of failing the local registry lookup. Local-only tools
/// are deliberately absent: `ssh_close` keeps local pool semantics,
/// known-hosts / quick-sudo / settings tools never target a connection, and
/// `ssh_list_connections` is the discovery surface itself. `ssh_test_connection`
/// forwards when a saved reference cannot be resolved locally (the app-side
/// sidecar holds the credentials) while inline-credential calls stay local.
/// `sftp_upload` / `sftp_download` forward too: bridge and stdio sidecar run
/// on the same machine, so `localPath` stays valid on the app side (its
/// sidecar re-applies the local transfer gates with the shared plugin settings).
pub(in crate::features::mcp) fn is_connection_bound_tool(name: &str) -> bool {
    matches!(
        name,
        "ssh_exec"
            | "ssh_exec_sudo"
            | "ssh_run_bg"
            | "ssh_task_status"
            | "sftp_list_dir"
            | "sftp_stat"
            | "sftp_read_file"
            | "sftp_write_file"
            | "sftp_upload"
            | "sftp_download"
    )
}

/// Registry entry as a list view: metadata plus credential PRESENCE flags
/// only (same discipline as the Quick Sudo profile views — never values).
pub(in crate::features::mcp) fn registry_connection_view(connection: &StoredConnection) -> Value {
    json!({
        "id": connection.id,
        "name": connection.name,
        "host": connection.host,
        "port": connection.port,
        "username": connection.username,
        "authentication": connection.authentication.method_name(),
        "readOnly": connection.read_only,
        "passwordSet": !connection.password.is_empty(),
    })
}

/// Merges the bridge connection list (preferred, when the app answered)
/// with the session registry, deduplicated by id (bridge data wins).
/// Returns the tool payload: `source` records the data origin, and the
/// degraded branch carries a note pointing at the app start/upgrade
/// requirement. Bridge entries are credential-free by contract; registry
/// views expose presence flags only.
pub(in crate::features::mcp) fn connection_list_result(
    bridge: Result<Vec<Value>, String>,
    registry: &[StoredConnection],
    scope: &ConnectionScope,
) -> Value {
    // §1.3: an active scope hides out-of-scope entries from the list view
    // (both bridge and registry sources), so discovery cannot enumerate
    // beyond the operator's allowlist. DenyAll intentionally produces an
    // empty list rather than reusing the persisted unrestricted empty array.
    fn in_scope(entry: &Value, scope: &ConnectionScope) -> bool {
        scope.allows(
            entry.get("id").and_then(Value::as_str).unwrap_or_default(),
            entry.get("name").and_then(Value::as_str),
            entry
                .get("host")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
    }
    match bridge {
        Ok(entries) => {
            let listed: std::collections::HashSet<String> = entries
                .iter()
                .filter_map(|entry| entry.get("id").and_then(Value::as_str).map(str::to_string))
                .collect();
            let mut merged: Vec<Value> = entries
                .into_iter()
                .filter(|entry| in_scope(entry, scope))
                .collect();
            for connection in registry
                .iter()
                .filter(|connection| !listed.contains(&connection.id))
                .filter(|connection| {
                    scope.allows(&connection.id, connection.name.as_deref(), &connection.host)
                })
            {
                merged.push(registry_connection_view(connection));
            }
            json!({ "connections": merged, "source": "dbx-app-bridge" })
        }
        Err(_) => json!({
            "connections": registry
                .iter()
                .filter(|connection| {
                    scope.allows(
                        &connection.id,
                        connection.name.as_deref(),
                        &connection.host,
                    )
                })
                .map(registry_connection_view)
                .collect::<Vec<_>>(),
            "source": "session-registry",
            "note": "The DBX app bridge did not answer the saved-connection list (the app is not running, or this DBX version predates the /list-plugin-connections route). Only connections registered in this session are shown; start or upgrade the DBX app to get the full saved-connection list.",
        }),
    }
}

/// Pure selector→id resolution over a bridge connection list: exact name
/// and/or complete endpoint selectors filter candidates, exactly one hit
/// yields its id, and ambiguity lists every candidate id + host.
pub(in crate::features::mcp) fn resolve_connection_in_bridge_list(
    entries: &[Value],
    arguments: &Value,
) -> Result<String, String> {
    let name = non_empty_argument(arguments, "connectionName");
    let endpoint = endpoint_selector(arguments);
    let mut hits: Vec<(&str, &str)> = Vec::new();
    for entry in entries {
        let entry_name = entry.get("name").and_then(Value::as_str).map(str::trim);
        let entry_host = entry.get("host").and_then(Value::as_str).unwrap_or("");
        let entry_port = entry
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(22);
        let entry_username = entry.get("username").and_then(Value::as_str).unwrap_or("");
        let name_matches = name.is_none_or(|wanted| entry_name == Some(wanted));
        let endpoint_matches = endpoint.is_none_or(|selector| {
            entry_host.trim().eq_ignore_ascii_case(selector.host)
                && entry_port == selector.port
                && entry_username == selector.username
        });
        if name_matches && endpoint_matches {
            if let Some(id) = entry.get("id").and_then(Value::as_str) {
                hits.push((id, entry_host));
            }
        }
    }
    let selector = name
        .map(|value| format!("connection name '{value}'"))
        .or_else(|| endpoint.as_ref().map(endpoint_selector_label))
        .unwrap_or_else(|| "connection selectors".to_string());
    match hits.len() {
        1 => Ok(hits[0].0.to_string()),
        0 => Err(format!("No saved connection matched {selector}")),
        _ => Err(format!(
            "{selector} is ambiguous: {}; pass connectionId instead",
            hits.iter()
                .map(|(id, host)| format!("{id} (host {host})"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Renders registry candidates for the ambiguous-connectionName error.
pub(in crate::features::mcp) fn render_connection_candidates(
    connections: &[&StoredConnection],
) -> String {
    connections
        .iter()
        .map(|connection| format!("{} (host {})", connection.id, connection.host))
        .collect::<Vec<_>>()
        .join(", ")
}
