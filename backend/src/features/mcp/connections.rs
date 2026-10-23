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

pub(super) fn connection_pool_id(arguments: &Value) -> String {
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
pub(super) fn connection_pool_key(arguments: &Value) -> String {
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
pub(super) fn is_connection_bound_tool(name: &str) -> bool {
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
pub(super) fn registry_connection_view(connection: &StoredConnection) -> Value {
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
pub(super) fn connection_list_result(
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
pub(super) fn resolve_connection_in_bridge_list(
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
pub(super) fn render_connection_candidates(connections: &[&StoredConnection]) -> String {
    connections
        .iter()
        .map(|connection| format!("{} (host {})", connection.id, connection.host))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn stored_connection_from_arguments(
    arguments: &Value,
) -> Result<StoredConnection, String> {
    let jump_hosts = parse_jump_hosts(arguments)?;
    let host = required_str(arguments, "host")?;
    let username = required_str(arguments, "username")?;
    let port = arg_port(arguments)?;
    let password = arguments
        .get("password")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let private_key_path = arguments
        .get("privateKeyPath")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let private_key_content = arguments
        .get("privateKeyContent")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let has_key = !private_key_path.is_empty() || !private_key_content.is_empty();
    let authentication = match arguments
        .get("authentication")
        .and_then(Value::as_str)
        .unwrap_or("password")
    {
        "private-key" if has_key => AuthenticationMethod::PrivateKey,
        "private-key-password" if has_key => AuthenticationMethod::PrivateKeyPassword,
        "agent" => AuthenticationMethod::Agent,
        _ => {
            if has_key {
                AuthenticationMethod::PrivateKey
            } else {
                AuthenticationMethod::Password
            }
        }
    };
    if matches!(authentication, AuthenticationMethod::Password) && password.is_empty() {
        let password_command = arguments
            .get("passwordCommand")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if password_command.is_empty() {
            return Err("Password authentication requires a password".to_string());
        }
    }
    if matches!(
        authentication,
        AuthenticationMethod::PrivateKey | AuthenticationMethod::PrivateKeyPassword
    ) && !has_key
    {
        return Err(
            "Private-key authentication requires privateKeyPath or privateKeyContent".to_string(),
        );
    }
    // Expect 式触发器（JSON 字符串形态，同 §2.1 schema）与外部密码管理器
    // 命令：解析/校验与存储路径同一套代码（triggers::parse_triggers），非法
    // 即拨号报错。内联拨号没有 secret binding，sendSecretKey 引用的槽位
    // 无法填充，引用即报错。
    let triggers = crate::features::automation::triggers::parse_triggers(
        arguments.get("triggers"),
        &|_key| None,
    )?;
    let password_command = arguments
        .get("passwordCommand")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let passphrase_command = arguments
        .get("passphraseCommand")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let connect_timeout = arg_u64(arguments, "connectTimeoutSecs")?;
    Ok(StoredConnection {
        sudo_whitelist: Vec::new(),
        id: connection_pool_id(arguments),
        // Inline MCP dials carry no display name.
        name: None,
        host: host.to_string(),
        port,
        runtime_host: host.to_string(),
        runtime_port: port,
        username: username.to_string(),
        protocol: "ssh".to_string(),
        password,
        authentication,
        private_key_path,
        private_key: private_key_content,
        private_key_passphrase: arguments
            .get("privateKeyPassphrase")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        agent_socket: arguments
            .get("agentSocket")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        connect_timeout_secs: connect_timeout.unwrap_or(30).max(1),
        connect_timeout_explicit: connect_timeout.is_some(),
        keepalive_interval_secs: 30,
        // MCP drives exec channels, never the user's interactive PTY —
        // activity injection doesn't apply here.
        terminal_keepalive_secs: 0,
        read_only: false,
        sudo_password: arguments
            .get("sudoPassword")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        totp_secret: arguments
            .get("totpSecret")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        // MCP keeps its own quickSudoProfile resolution (explicit arguments
        // win), so the stored connection carries the plain on/off source.
        sudo_source: if arg_bool(arguments, "quickSudo")?.unwrap_or(true) {
            SudoSource::Custom
        } else {
            SudoSource::Off
        },
        sudo_profile_ref: String::new(),
        sudo_use_pty: false,
        password_prompt_hint: arguments
            .get("passwordPromptHint")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        totp_prompt_hint: arguments
            .get("totpPromptHint")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        auth_flow_mode: arguments
            .get("authFlowMode")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        // MCP tool calls carry their own env/command semantics; connection
        // SetEnv / RemoteCommand are workbench connection-form features.
        set_env: Vec::new(),
        remote_command: String::new(),
        triggers_enabled: triggers.is_some(),
        triggers,
        password_command,
        passphrase_command,
        jump_hosts,
    })
}

/// Parses the optional `jumpHosts` array (snake_case fields, same shape as
/// `external_config.jump_hosts`) into the ProxyJump chain.
pub(super) fn parse_jump_hosts(arguments: &Value) -> Result<Vec<JumpHost>, String> {
    let Some(list) = arguments.get("jumpHosts") else {
        return Ok(Vec::new());
    };
    let Some(list) = list.as_array() else {
        return Err("jumpHosts must be an array of jump host objects".to_string());
    };
    if list.len() > 3 {
        return Err("At most 3 jump hosts are supported".to_string());
    }
    let mut hosts = Vec::with_capacity(list.len());
    for (position, value) in list.iter().enumerate() {
        let host = JumpHost::from_json(value)
            .map_err(|error| format!("jumpHosts[{position}]: {error}"))?;
        host.validate(position)
            .map_err(|error| format!("jumpHosts[{position}]: {error}"))?;
        hosts.push(host);
    }
    Ok(hosts)
}
