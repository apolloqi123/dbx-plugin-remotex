//! MCP connections helpers.

use super::*;

impl McpState {
    /// True when the call must pass the read-only gates: either the DBX
    /// connection it references was registered read-only, or the operator
    /// flipped the process-wide `DBX_SSH_MCP_READ_ONLY` kill switch.
    pub(super) async fn connection_is_read_only(&self, arguments: &Value) -> bool {
        self.global_read_only || self.registered_connection_is_read_only(arguments).await
    }

    /// Resolves a saved connection reference without making the caller carry
    /// an opaque id. Exact `connectionId` wins, then an exact display name,
    /// then a unique endpoint (`host` + `username`, with port defaulting to
    /// 22). Endpoint fields narrow a name match when both are supplied. Any
    /// ambiguity is an error; a miss returns `None` so inline credentials and
    /// the stdio bridge fallback retain their existing behavior.
    pub(super) async fn registered_connection_by_ref(
        &self,
        arguments: &Value,
    ) -> Result<Option<StoredConnection>, String> {
        let id = non_empty_argument(arguments, "connectionId");
        let name = non_empty_argument(arguments, "connectionName");
        let endpoint = endpoint_selector(arguments);
        if id.is_none() && name.is_none() && endpoint.is_none() {
            return Ok(None);
        }
        let registry = self.dbx_connections.read().await;

        if let Some(id) = id {
            if let Some(connection) = registry.get(id) {
                if !connection_matches_selectors(connection, name, endpoint.as_ref()) {
                    return Err(format!(
                        "connectionId '{id}' does not match the supplied connectionName/host/port/username"
                    ));
                }
                return Ok(Some(connection.clone()));
            }
            // Preserve the existing self-heal behavior for a stale id when a
            // second selector can still identify the saved connection.
            if name.is_none() && endpoint.is_none() {
                return Ok(None);
            }
        }

        let candidates: Vec<&StoredConnection> = registry
            .values()
            .filter(|connection| connection_matches_selectors(connection, name, endpoint.as_ref()))
            .collect();
        match candidates.len() {
            0 => Ok(None),
            1 => Ok(Some(candidates[0].clone())),
            _ => {
                let selector = name
                    .map(|value| format!("connection name '{value}'"))
                    .or_else(|| endpoint.as_ref().map(endpoint_selector_label))
                    .unwrap_or_else(|| "connection selectors".to_string());
                Err(format!(
                    "{selector} is ambiguous: {}; pass connectionId instead",
                    render_connection_candidates(&candidates)
                ))
            }
        }
    }

    /// True when the arguments reference a DBX-registered connection that was
    /// registered as read-only through `mcp/call` lifecycle payloads.
    ///
    /// Without a resolvable reference the gate falls back to endpoint
    /// identity (host + port + username): keying read-only by connectionId
    /// alone would let a caller walk around a read-only registration by
    /// re-dialing the same host with inline credentials.
    pub(super) async fn registered_connection_is_read_only(&self, arguments: &Value) -> bool {
        match self.registered_connection_by_ref(arguments).await {
            Ok(Some(connection)) => connection.read_only,
            // Ambiguous connectionName: conservatively read-only; the
            // disambiguation error with every candidate surfaces from the
            // execution path.
            Err(_) => true,
            Ok(None) => self.inline_dial_is_registered_read_only(arguments).await,
        }
    }

    /// True when the inline dial arguments (host/port/username) identify the
    /// same endpoint as a DBX-registered read-only connection.
    pub(super) async fn inline_dial_is_registered_read_only(&self, arguments: &Value) -> bool {
        self.registered_connection_matching_inline(arguments)
            .await
            .is_some_and(|connection| connection.read_only)
    }

    /// Finds a lifecycle-registered connection whose endpoint identity
    /// (host + port + username) matches the inline dial arguments. The port
    /// defaults to 22 on both sides; host comparison is ASCII-case-insensitive.
    pub(super) async fn registered_connection_matching_inline(
        &self,
        arguments: &Value,
    ) -> Option<StoredConnection> {
        let host = arguments
            .get("host")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let port = arg_port_lossy(arguments);
        let username = arguments
            .get("username")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        self.dbx_connections
            .read()
            .await
            .values()
            .find(|connection| {
                connection.host.trim().eq_ignore_ascii_case(host)
                    && connection.port == port
                    && connection.username == username
            })
            .cloned()
    }

    /// Resolves the connection's sudoers-style allowlist: by `connectionId`
    /// (or `connectionName`) through the lifecycle registry, else by inline
    /// endpoint identity (the same fallback as the read-only gate, so
    /// re-dialing a whitelisted host with inline credentials cannot skip the
    /// list). Empty = not configured. An ambiguous connectionName is an
    /// error, not a silent gate-off.
    pub(super) async fn sudo_allowlist_for(
        &self,
        arguments: &Value,
    ) -> Result<Vec<Vec<String>>, String> {
        Ok(match self.registered_connection_by_ref(arguments).await? {
            Some(connection) => {
                if !connection.sudo_whitelist.is_empty() {
                    sudo_allowlist::entries_from_lines(&connection.sudo_whitelist)
                } else {
                    Default::default()
                }
            }
            None => self
                .registered_connection_matching_inline(arguments)
                .await
                .and_then(|connection| {
                    (!connection.sudo_whitelist.is_empty())
                        .then(|| sudo_allowlist::entries_from_lines(&connection.sudo_whitelist))
                })
                .unwrap_or_default(),
        })
    }

    /// stdio connection resolution: reads the bridge list (no app wake-up —
    /// a down bridge simply fails the resolution and the call degrades) and
    /// applies the same exact-name / unique-endpoint rules as the lifecycle
    /// registry.
    pub(super) async fn resolve_connection_via_bridge(
        &self,
        arguments: &Value,
    ) -> Result<String, String> {
        let entries = app_bridge::list_plugin_connections().await?;
        resolve_connection_in_bridge_list(&entries, arguments)
    }

    /// `ssh_list_connections`: saved-connection discovery for stdio agents.
    /// The bridge list comes first (metadata only — the app never returns
    /// credentials), merged with this session's lifecycle registry
    /// (deduplicated by id; registry entries may add credential-presence
    /// flags because they carry the secrets). A bridge that answers nothing
    /// (app down, or an older app without the /list-plugin-connections
    /// route) degrades to the registry alone plus an upgrade note instead of
    /// failing.
    pub(super) async fn ssh_list_connections_tool(&self) -> Result<Value, String> {
        let registry: Vec<StoredConnection> = self
            .dbx_connections
            .read()
            .await
            .values()
            .cloned()
            .collect();
        let bridge = app_bridge::list_plugin_connections().await;
        let scope = self.effective_scope();
        Ok(connection_list_result(bridge, &registry, &scope))
    }

    /// Terminal-routed `ssh_exec` / `ssh_exec_sudo`: resolves the
    /// connection's live workbench PTY, applies the §1 approval matrix
    /// (sudo and catastrophic mcp_safety hits are elevated), then types the
    /// command into the terminal and captures the output.
    /// Polls `session_id_for_connection` until the workbench PTY shows up or
    /// `wait` elapses, so a just-triggered workbench auto-open can win the
    /// race against the first terminal-routed command.
    pub(super) async fn wait_for_connection_session(
        &self,
        connection_id: &str,
        wait: Duration,
    ) -> Result<String, String> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match self.runtime.session_id_for_connection(connection_id).await {
                Ok(session_id) => return Ok(session_id),
                Err(error) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(error);
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    pub(super) async fn ssh_close(&self, arguments: &Value) -> Result<Value, String> {
        // A selector-less close would silently target the meaningless
        // "mcp-@:22" pool key; demand a reference instead of a confusing
        // "no cached connection" miss.
        if non_empty_argument(arguments, "connectionId").is_none()
            && non_empty_argument(arguments, "connectionName").is_none()
            && endpoint_selector(arguments).is_none()
        {
            return Err(
                "ssh_close needs a connection reference: pass connectionId (or \
                 connectionName, or a unique host+username endpoint, as listed by \
                 ssh_list_connections)"
                    .to_string(),
            );
        }
        let pool_id = connection_pool_key(arguments);
        let entry = self.connections.write().await.remove(&pool_id);
        match entry {
            Some(entry) => {
                let _ = entry
                    .handle
                    .disconnect(
                        russh::Disconnect::ByApplication,
                        "MCP connection closed",
                        "English",
                    )
                    .await;
                for jump in entry.jumps {
                    let _ = jump
                        .disconnect(
                            russh::Disconnect::ByApplication,
                            "MCP jump connection closed",
                            "English",
                        )
                        .await;
                }
                Ok(json!({ "connectionId": pool_id, "closed": true }))
            }
            None => Err(format!("No cached connection for {pool_id}")),
        }
    }

    pub(super) async fn drop_connection(&self, arguments: &Value) {
        let pool_id = connection_pool_key(arguments);
        self.connections.write().await.remove(&pool_id);
    }

    pub(super) async fn connection(
        &self,
        arguments: &Value,
    ) -> Result<Arc<Handle<SshClient>>, String> {
        // DBX bridge calls reference a saved connection by id (or name);
        // inline calls (standalone --mcp mode) derive the pool key from the
        // credentials. The unified lookup resolves connectionId first, then
        // connectionName, and fails on ambiguity with every candidate.
        let explicit_id = arguments
            .get("connectionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let explicit_name = arguments
            .get("connectionName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let registered = self.registered_connection_by_ref(arguments).await?;
        let (pool_id, connection) = match registered {
            // The real registry id is the pool key either way, so pooled
            // handles line up with the app-side and forwarded calls.
            Some(connection) => (connection.id.clone(), Some(connection)),
            None => {
                if let Some(id) = explicit_id {
                    // Unregistered id: a pooled handle may still exist (an
                    // earlier registration left one behind), so the pool hit
                    // below gets its chance before the guidance error fires.
                    (id.to_string(), None)
                } else if let Some(name) = explicit_name {
                    // Unregistered name: no pool key can be derived, so the
                    // self-heal error fires immediately (L0).
                    return Err(format!(
                        "No connection named '{name}' is registered with this plugin session and \
                         the DBX app bridge is unavailable. Start the DBX app, list saved \
                         connections with ssh_list_connections, or provide inline credentials \
                         (host/username plus password or privateKeyPath/privateKeyContent)."
                    ));
                } else {
                    (
                        connection_pool_id(arguments),
                        Some(stored_connection_from_arguments(arguments)?),
                    )
                }
            }
        };
        {
            let guard = self.connections.read().await;
            if let Some(entry) = guard.get(&pool_id) {
                return Ok(entry.handle.clone());
            }
        }
        // Unregistered connectionId with no pooled handle: with the L1
        // bridge fallback active this only fires when the DBX app bridge is
        // also unavailable, so the message carries the full self-heal path.
        let connection = match connection {
            Some(connection) => connection,
            None => {
                return Err(format!(
                    "Connection {pool_id} is not registered with this plugin session and the DBX \
                     app bridge is unavailable. Start the DBX app, list saved connections with \
                     ssh_list_connections, or provide inline credentials (host/username plus \
                     password or privateKeyPath/privateKeyContent)."
                ))
            }
        };
        let (handle, jumps) = self.runtime.connect_headless(&connection).await?;
        let mut guard = self.connections.write().await;
        drop(connection);
        let entry = guard.entry(pool_id).or_insert(McpConnection {
            handle: handle.clone(),
            jumps,
            sftp: None,
        });
        Ok(entry.handle.clone())
    }

    /// `mcp/call` entry used by the DBX MCP bridge (`dbx_call_plugin_tool`):
    /// registers the forwarded connection lifecycle payload, then dispatches
    /// the tool with a `connectionId` reference so credentials never travel
    /// inline with tool arguments. The emitter enables the agent terminal
    /// routing (`runInTerminal` / `agentTerminalMode`); stdio mode passes
    /// `None` and only leaves the hidden exec channel for `runInTerminal`
    /// calls, which forward to the DBX app's local TCP bridge.
    pub async fn call_dbx(&self, params: &Value, emitter: PluginEmitter) -> Result<Value, String> {
        self.call_dbx_with(params, Some(emitter)).await
    }

    /// Emitter-less variant; tests (and the stdio path semantics) use it to
    /// exercise lifecycle registration without a bridge emitter.
    pub(crate) async fn call_dbx_with(
        &self,
        params: &Value,
        emitter: Option<PluginEmitter>,
    ) -> Result<Value, String> {
        let tool = required_str(params, "tool")?.to_string();
        let mut arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        if !arguments.is_object() {
            return Err("arguments must be a JSON object".to_string());
        }
        if let Some(lifecycle) = params.get("lifecycle") {
            let connection = StoredConnection::from_lifecycle_params(lifecycle)?;
            self.dbx_connections
                .write()
                .await
                .insert(connection.id.clone(), connection.clone());
            if let Some(map) = arguments.as_object_mut() {
                map.insert("connectionId".to_string(), json!(connection.id));
            }
        }
        self.call_tool(&tool, &arguments, emitter.as_ref()).await
    }
}

mod builder;
mod resolve;

pub(super) use builder::*;
pub(super) use resolve::*;
