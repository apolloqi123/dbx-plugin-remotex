//! MCP dispatch operations.

use super::*;
use crate::core::plugin_ids::PLUGIN_ID;

impl McpState {
    pub(super) async fn dispatch(&self, request: Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        if method.starts_with("notifications/") {
            return None;
        }
        // Envelope validation (reliability round 5): a malformed request
        // envelope gets a structured -32600 instead of being silently
        // tolerated (wrong protocol version) or crashing on weird id
        // shapes. Notifications above stay silent regardless of envelope.
        if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(Self::invalid_request_envelope(
                id,
                "Invalid request: jsonrpc must be exactly \"2.0\"".to_string(),
            ));
        }
        if let Some(id) = &id {
            if !id.is_string() && !id.is_number() {
                return Some(Self::invalid_request_envelope(
                    None,
                    format!("Invalid request: id must be a string or number, got {id}"),
                ));
            }
        }
        if method.is_empty() {
            return Some(Self::invalid_request_envelope(
                id,
                "Invalid request: method must be a non-empty string".to_string(),
            ));
        }
        // JSON-RPC error tiering (family-wide with ldap/kafka/files): the
        // transport layer uses the standard codes — parse -32700, unknown
        // method -32601, invalid request -32600 — while tool-level errors
        // stay -32000 across all four plugins.
        let result: Result<Value, (i64, String)> = match method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": {
                    // 同族基线：完整插件 id（files/ldap/kafka 同款）。
                    "name": PLUGIN_ID,
                    "version": env!("CARGO_PKG_VERSION"),
                },
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_definitions() })),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                // stdio mode has no event emitter: `runInTerminal` routing
                // goes through the DBX app's local TCP bridge inside
                // `ssh_exec_tool` (the None-emitter arm).
                self.call_tool(&name, &arguments, None)
                    .await
                    .map_err(|message| (-32000, message))
            }
            other => Err((-32601, format!("Method not found: {other}"))),
        };
        Some(match (id, result) {
            (Some(id), Ok(result)) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            (Some(id), Err((code, message))) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": code, "message": message },
            }),
            // A request without an id is invalid JSON-RPC; reply with a null id.
            (None, result) => Self::invalid_request_envelope(
                None,
                result
                    .err()
                    .map(|(_, message)| message)
                    .unwrap_or_else(|| "Invalid request".to_string()),
            ),
        })
    }

    /// Structured -32600 for a malformed request envelope: the id is echoed
    /// only when it is itself protocol-valid (string/number), otherwise
    /// nulled per JSON-RPC 2.0's invalid-request framing.
    pub(super) fn invalid_request_envelope(id: Option<Value>, message: String) -> Value {
        let id = match id {
            Some(id) if id.is_string() || id.is_number() => id,
            _ => Value::Null,
        };
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32600, "message": message },
        })
    }

    /// Execution-plane audit (IMPL_PLAN §2): every gated tool call lands
    /// one ledger row with its verdict, exit code, and duration — the row
    /// the workbench audit view and `ssh/audit/list` replay. Gate refusals
    /// that return before the inner dispatch record `Pass` gates with the
    /// refusal error text (the verdict detail lives in `error`); approval
    /// lifecycle rows are separate (`audit_approval` in ssh.rs).
    pub(super) async fn call_tool(
        &self,
        name: &str,
        arguments: &Value,
        emitter: Option<&PluginEmitter>,
    ) -> Result<Value, String> {
        if !is_confirm_gated_tool(name) {
            return self.call_tool_inner(name, arguments, emitter).await;
        }
        let started = std::time::Instant::now();
        let result = self.call_tool_inner(name, arguments, emitter).await;
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default();
        let entry = audit_log::AuditEntry {
            ts_ms,
            tool: name.to_string(),
            connection_id: arguments
                .get("connectionId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            gate: audit_log::GateOutcome::Pass,
            approval: audit_log::ApprovalTrail::None,
            outcome: if result.is_ok() {
                audit_log::EntryOutcome::Ok
            } else {
                audit_log::EntryOutcome::Error
            },
            exit_code: result
                .as_ref()
                .ok()
                .and_then(|value| value.get("exitCode"))
                .and_then(Value::as_i64),
            duration_ms: started.elapsed().as_millis() as u64,
            mode: if emitter.is_some() {
                audit_log::ExecMode::Embedded
            } else {
                audit_log::ExecMode::Stdio
            },
            // The exec family carries `command` in and returns `output` —
            // record both (clamped in audit_log::append) so the ledger replays
            // what ran and what it printed. Other gated tools simply carry no
            // such fields and stay null.
            command: arguments
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_string),
            output: result
                .as_ref()
                .ok()
                .and_then(|value| value.get("output"))
                .and_then(Value::as_str)
                .map(str::to_string),
            error: result
                .as_ref()
                .err()
                .map(|error| error.chars().take(256).collect()),
        };
        if let Err(error) = audit_log::append(&self.runtime.data_dir(), &entry) {
            eprintln!("[mcp] audit append failed: {error}");
        }
        result
    }

    pub(super) async fn call_tool_inner(
        &self,
        name: &str,
        arguments: &Value,
        emitter: Option<&PluginEmitter>,
    ) -> Result<Value, String> {
        // Cheapest local checks first: an unregistered tool name and a
        // malformed port must fail with an actionable error before any gate,
        // registry lookup, or dial. A malformed port must never silently
        // dial the default 22 instead of the requested one.
        if !is_known_tool(name) {
            return Err(unknown_tool_message(name));
        }
        arg_port(arguments)?;
        // Normalize a saved-connection reference before any gate or tool sees
        // it. This makes connectionName and unique host/port/username matches
        // equivalent to an explicit connectionId, including pool keys and
        // visible-terminal routing, while still rejecting ambiguity. The
        // resolved entry is kept alongside for the §1.3 scope gate.
        let resolved_ref = self.registered_connection_by_ref(arguments).await?;
        let normalized_arguments = resolved_ref.as_ref().map(|connection| {
            let mut normalized = arguments.clone();
            if let Some(map) = normalized.as_object_mut() {
                map.insert("connectionId".to_string(), json!(connection.id));
            }
            normalized
        });
        let arguments = normalized_arguments
            .as_ref()
            .map_or(arguments, |normalized| normalized as &Value);
        // §1.3 scope gate: while `connectionScope` is non-empty, every
        // connection-class tool must resolve to an in-scope saved entry.
        // Inline credentials (endpoint selectors) have no registry identity
        // and are refused outright — fail closed, so a pinned scope cannot
        // be widened by dialing around the registry.
        {
            let scope = self.effective_scope();
            if scope.is_active() && is_connection_scoped_tool(name) {
                match resolved_ref.as_ref() {
                    Some(connection) => {
                        if !scope.allows(
                            &connection.id,
                            connection.name.as_deref(),
                            &connection.host,
                        ) {
                            return Err(format!(
                                "Connection '{}' ({} / {}) is outside this MCP server's \
                                 connectionScope; ask the operator to widen the scope or \
                                 pick an in-scope connection",
                                connection.id, connection.host, connection.username
                            ));
                        }
                    }
                    None => {
                        return Err(
                            "connectionScope is active on this MCP server: inline-credential \
                             calls are refused. Pass a saved connectionId/connectionName that \
                             is inside the scope (see ssh_list_connections)"
                                .to_string(),
                        );
                    }
                }
            }
        }
        // Safety gates, ordered cheapest-first and all evaluated before any
        // network I/O:
        // 1. Read-only gate: write-class tools are rejected when the DBX
        //    connection they reference was opened read-only (mirroring the
        //    workbench `ensure_writable` gate) or the operator forced the
        //    whole server read-only via DBX_SSH_MCP_READ_ONLY.
        // 2. Read-only command whitelist: `ssh_exec` stays available on
        //    read-only connections, but only for provably read-only
        //    commands (ls, df, systemctl status, ...).
        // 3. Sensitive-path denylist: on read-only connections even
        //    whitelisted inspection tools may not touch credential/key paths
        //    (~/.ssh, /etc/shadow, .env, *.pem, ...) — the exfiltration
        //    channel the command whitelist cannot see for SFTP tools.
        // 4. Destructive-command confirmation: recognized catastrophic
        //    patterns require an explicit confirmDestructive: true on every
        //    connection (and are refused outright on read-only ones).
        let read_only = self.connection_is_read_only(arguments).await;
        if is_write_tool(name) && read_only {
            return Err(format!(
                "Tool {name} is a write operation and the connection is read-only"
            ));
        }
        if read_only {
            if let Some(path) = sensitive_read_path(name, arguments) {
                return Err(format!(
                    "Tool {name} reads the sensitive path {path}; refused on read-only \
                     connections"
                ));
            }
        }
        // Command-text gates (sudo allowlist, destructive assessment, read-only
        // whitelist, terminal-input gate) — run here on the original text, and
        // again after a human-edited approval below. Editing during confirm
        // must never widen what these gates accepted.
        let command_text = if name == "ssh_terminal_input" {
            required_str(arguments, "input")?.to_string()
        } else if matches!(name, "ssh_exec" | "ssh_exec_sudo" | "ssh_run_bg") {
            required_str(arguments, "command")?.to_string()
        } else {
            String::new()
        };
        if !command_text.is_empty() {
            self.enforce_command_text_gates(name, &command_text, read_only, arguments)
                .await?;
        }
        // §1.3 confirm permission mode: after every existing gate, before
        // any execution path (the L1 bridge forward included), a gated tool
        // must carry a human approval. Fail-closed without an emitter: a
        // stdio standalone session has no approval channel, so the call
        // errors immediately instead of hanging on the 120s timeout.
        let mut confirmed_arguments: Option<Value> = None;
        if self.exec_confirm_required(name) {
            let Some(emitter) = emitter else {
                return Err(format!(
                    "Tool {name} requires approval under execPermissionMode=confirm, but \
                     this MCP session has no approval channel (stdio standalone mode). \
                     Switch the mode back to autonomous (mcp/settings/set) or run the \
                     server inside the DBX workbench session."
                ));
            };
            let command_key = if name == "ssh_terminal_input" {
                "input"
            } else {
                "command"
            };
            let command: String = required_str(arguments, command_key)?.to_string();
            let connection_id = arguments.get("connectionId").and_then(Value::as_str);
            let approved = self
                .runtime
                .request_mcp_confirm(name, &command, connection_id, emitter)
                .await?;
            // Command-based tools retain the existing editable approval
            // contract: the approved text replaces the original execution
            // input.
            if approved != command {
                let mut rewritten = arguments.clone();
                if let Some(map) = rewritten.as_object_mut() {
                    map.insert(command_key.to_string(), json!(approved));
                }
                confirmed_arguments = Some(rewritten);
            }
        }
        let arguments = confirmed_arguments.as_ref().unwrap_or(arguments);
        if let Some(rewritten) = confirmed_arguments.as_ref() {
            // 人工在确认框里编辑过的命令文本必须重过同一组闸：批准的语义是
            // "执行这条改写后的文本"，不是"豁免安全闸"——改写可以把无害
            // 命令换成破坏性命令、把白名单外的 sudo 塞进来。
            let edited = rewritten
                .get(if name == "ssh_terminal_input" {
                    "input"
                } else {
                    "command"
                })
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !edited.is_empty() {
                self.enforce_command_text_gates(name, edited, read_only, rewritten)
                    .await?;
            }
        }
        // L1 stdio bridge fallback: a call referencing a `connectionId` that
        // is NOT in this session's lifecycle registry (the normal state of a
        // standalone `--mcp` session) is forwarded to the running DBX app's
        // own sidecar through the local TCP bridge, so saved connections work
        // without inline credentials and credentials never travel in tool
        // arguments. Gate semantics across the forward: the local gates above
        // stay in front of it (the DBX_SSH_MCP_READ_ONLY kill switch and the
        // command-text checks are arguments-only and must not be dodged),
        // while the registry-metadata gates (per-connection read-only flag,
        // sudo whitelist) cannot resolve here by definition — the app-side
        // sidecar enforces them against its own lifecycle registration.
        if emitter.is_none() {
            if let Some((connection_id, forwarded)) =
                self.bridge_forward_plan(name, arguments).await?
            {
                if let Ok(result) = self
                    .forward_tool_via_bridge(name, &connection_id, &forwarded)
                    .await
                {
                    return Ok(result);
                }
                // Bridge unreachable or refused (app down, unwakeable, older
                // app): fall through to the inline path so the original
                // guidance error — now carrying the self-heal hints — is
                // what the caller sees.
            }
        }
        let text = self.run_tool(name, arguments, emitter).await?;
        // The app-bridge forward returns the app's MCP content envelope
        // verbatim (`app_bridge::call_plugin_tool`); wrapping again would
        // bury the app's answer one JSON level deeper, so an already
        // enveloped result passes through untouched.
        if text.get("content").is_some() && text.get("isError").is_some() {
            return Ok(text);
        }
        Ok(json!({
            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&text).unwrap_or_default() }],
            "isError": false,
        }))
    }
}
