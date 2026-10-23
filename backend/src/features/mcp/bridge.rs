//! MCP bridge operations.

use super::*;

impl McpState {
    /// L1 stdio bridge fallback decision: `Ok(Some((connection_id,
    /// arguments)))` when the call must be forwarded through the DBX app
    /// bridge, `Ok(None)` when the local path owns the call (registered
    /// reference, no connection reference, local-only tool, runInTerminal
    /// routing, or a name the bridge list cannot resolve — the inline path
    /// then raises its own guidance error), `Err` for an actionable
    /// disambiguation failure from the bridge list.
    pub(super) async fn bridge_forward_plan(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Result<Option<(String, Value)>, String> {
        if !self.bridge_fallback || !is_connection_bound_tool(name) {
            return Ok(None);
        }
        // runInTerminal routing already owns its own bridge forward
        // (`ssh_exec_app_bridge`), which never required a registry entry;
        // keep that path exclusive so a down bridge cannot stack two
        // sequential wake-and-wait attempts. A malformed value cannot reach
        // here (call_tool validated the arguments' port only; be lossy).
        if arg_bool(arguments, "runInTerminal").unwrap_or(None) == Some(true) {
            return Ok(None);
        }
        if let Some(id) = arguments
            .get("connectionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            let registered = self.dbx_connections.read().await.contains_key(id);
            // Registered: the local path runs unchanged (embedded-bridge
            // semantics). Unregistered: forward as-is — the app-side sidecar
            // holds the connection's credentials, read-only flag, and sudo
            // whitelist, and answers with its own MCP envelope.
            return Ok((!registered).then(|| (id.to_string(), arguments.clone())));
        }
        if arguments.get("connectionName").is_none() && endpoint_selector(arguments).is_none() {
            return Ok(None);
        }
        // A registry hit (unique or ambiguous) stays local — the same
        // resolver reports ambiguity with all candidates. Only a stdio
        // session with no registry entry needs the bridge list to resolve
        // the selector to an id before forwarding.
        match self.registered_connection_by_ref(arguments).await {
            Ok(Some(_)) | Err(_) => Ok(None),
            Ok(None) => match self.resolve_connection_via_bridge(arguments).await {
                Ok(id) => {
                    let mut forwarded = arguments.clone();
                    if let Some(map) = forwarded.as_object_mut() {
                        map.insert("connectionId".to_string(), json!(id.clone()));
                    }
                    Ok(Some((id, forwarded)))
                }
                // Bridge list unavailable or selector not found: fall
                // through so the inline path reports the normal guidance.
                Err(_) => Ok(None),
            },
        }
    }

    /// Forwards one connection-bound tool call through the DBX app bridge
    /// (L1). Same wake-and-verify contract as `ssh_exec_app_bridge`; the
    /// tool timeout follows the `timeoutSecs` argument with the same 5–300s
    /// clamp ssh_exec applies. The 200 body is the app's MCP content
    /// envelope and is returned verbatim.
    pub(super) async fn forward_tool_via_bridge(
        &self,
        name: &str,
        connection_id: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        app_bridge::ensure_app_bridge(app_bridge::DEFAULT_ENSURE_WAIT).await?;
        let timeout_secs = arg_u64(arguments, "timeoutSecs")
            .ok()
            .flatten()
            .map(|secs| secs.clamp(5, 300))
            .unwrap_or(300);
        app_bridge::call_plugin_tool(
            connection_id,
            name,
            arguments.clone(),
            Duration::from_secs(timeout_secs),
        )
        .await
    }
}
