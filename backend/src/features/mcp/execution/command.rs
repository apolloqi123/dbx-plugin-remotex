//! MCP execution helpers.

use super::*;

impl McpState {
    pub(crate) async fn ssh_exec_tool(
        &self,
        name: &str,
        arguments: &Value,
        emitter: Option<&PluginEmitter>,
    ) -> Result<Value, String> {
        let command = required_str(arguments, "command")?;
        let timeout_secs =
            arg_u64(arguments, "timeoutSecs")?.map(|secs| Duration::from_secs(secs.clamp(5, 300)));
        // A quickSudoProfile reference is resolved (and validated) before any
        // connection I/O so unknown ids/names fail fast.
        let sudo_profile = if name == "ssh_exec_sudo" {
            let store = sudo_profiles::load_store(&self.runtime.data_dir());
            resolve_profile_reference(&store, arguments)?
        } else {
            None
        };
        // Agent terminal routing (agent terminal mode plan §1): only the DBX
        // embedded bridge carries an emitter plus a lifecycle connectionId;
        // every other caller stays on the hidden exec channel below.
        let run_in_terminal = arg_bool(arguments, "runInTerminal")?;
        let connection_id = arguments
            .get("connectionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        match (emitter, connection_id) {
            (Some(emitter), Some(connection_id)) => {
                let mode = self.runtime.agent_terminal_mode(connection_id);
                // An explicit runInTerminal wins; the connection mode decides
                // when it is absent. Off keeps the existing path untouched.
                let route = run_in_terminal.unwrap_or(mode != AgentTerminalMode::Off);
                if route {
                    // Whether the agent explicitly opted into the visible
                    // terminal decides the `off` + elevated matrix cell
                    // (approve with explicit opt-in, deny without it).
                    let explicit = run_in_terminal == Some(true);
                    return self
                        .ssh_exec_terminal_tool(
                            name,
                            command,
                            connection_id,
                            explicit,
                            timeout_secs,
                            emitter,
                        )
                        .await;
                }
            }
            (None, _) if run_in_terminal == Some(true) => {
                // stdio mode forwards through the DBX app's local TCP bridge:
                // the app opens the connection's workbench tab and runs the
                // tool on its own sidecar, so the command lands in the app's
                // visible terminal.
                let Some(connection_id) = connection_id else {
                    return Err(
                        "runInTerminal needs a saved DBX connection: pass connectionId (the connection must exist in the DBX app) so the command can run in the app's visible terminal. Saved connection ids can be listed with ssh_list_connections."
                            .to_string(),
                    );
                };
                return self
                    .ssh_exec_app_bridge(name, connection_id, arguments, timeout_secs)
                    .await;
            }
            (Some(_), None) if run_in_terminal == Some(true) => {
                return Err(
                    "runInTerminal requires a lifecycle connectionId from the DBX embedded bridge. Saved connection ids can be listed with ssh_list_connections."
                        .to_string(),
                );
            }
            _ => {}
        }
        // Sudo allowlist rebuild for the hidden exec channel (mirrors the
        // early gate): when the connection declares a whitelist and this is a
        // sudo run, the command that actually executes is the token-quoted
        // rebuild — assignments stripped, metacharacters quarantined — so a
        // passed match can never regain shell semantics under the root
        // `sh -c`. Visible-terminal routes above keep the raw text: the real
        // sudo binary's env_reset and the shell's own command separators
        // bound the elevation there.
        let allowlist = self.sudo_allowlist_for(arguments).await?;
        let command: String = if !allowlist.is_empty()
            && (name == "ssh_exec_sudo" || mcp_safety::runs_under_sudo(command))
        {
            sudo_allowlist::allowed_rebuilt_command(&allowlist, command).ok_or_else(|| {
                format!(
                    "sudo command is not allowed by this connection's whitelist. \
                         Allowed patterns: {}",
                    sudo_allowlist::render_entries(&allowlist)
                )
            })?
        } else {
            command.to_string()
        };
        // Saved DBX connections declare their Quick Sudo source (form field
        // sudo_source); the hidden exec channel honors it exactly like the
        // workbench instead of requiring inline credentials on every call.
        // The unified lookup also honors connectionName and endpoint
        // selectors; an ambiguous reference is rejected before dialing.
        let stored = self
            .registered_connection_by_ref(arguments)
            .await
            .ok()
            .flatten();
        // The hidden exec channel carries the saved connection's setEnv too,
        // matching the workbench exec path.
        let set_env = stored
            .as_ref()
            .map(|connection| connection.set_env.clone())
            .unwrap_or_default();
        if name == "ssh_exec_sudo" {
            let auth = self
                .resolve_sudo_auth(arguments, sudo_profile, stored)
                .await?;
            // The hidden exec channel stays TTY-less on purpose: with a PTY,
            // each PAM factor read flushes typed-ahead input, so piped
            // credentials race an unknowable per-host timing; without one,
            // stdin is a plain pipe and `exec_with_sudo` queues password and
            // OTP deterministically.
            let connection = self.connection(arguments).await?;
            exec::exec_with_sudo(
                &connection,
                &auth,
                &command,
                timeout_secs.unwrap_or(exec::SUDO_EXEC_TIMEOUT),
                false,
                &set_env,
            )
            .await
            .map(|outcome| json!({ "output": outcome.output, "exitCode": outcome.exit_code }))
        } else {
            let connection = self.connection(arguments).await?;
            exec::exec_plain(
                &connection,
                &command,
                timeout_secs.unwrap_or(exec::PLAIN_EXEC_TIMEOUT),
                &set_env,
            )
            .await
            .map(|outcome| json!({ "output": outcome.output, "exitCode": outcome.exit_code }))
        }
    }

    /// `ssh_exec_sudo` credential resolution for the hidden exec channel:
    /// explicit per-call arguments always win; otherwise a saved DBX
    /// connection's declared sudo source applies (Global: its form profile
    /// reference / workbench binding, Custom: the connection's secret-bound
    /// sudo configuration, Off: refuse unless the caller passed explicit
    /// credentials), mirroring the workbench exec gate; the per-call
    /// `quickSudoProfile` reference replaces the connection's declared
    /// global profile.
    pub(crate) async fn resolve_sudo_auth(
        &self,
        arguments: &Value,
        explicit_profile: Option<sudo_profiles::SudoProfile>,
        stored: Option<StoredConnection>,
    ) -> Result<SudoAuth, String> {
        let has_explicit_credentials = ["sudoPassword", "totpSecret"].iter().any(|key| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        });
        if let Some(stored) = &stored {
            if stored.sudo_source == SudoSource::Off && !has_explicit_credentials {
                return Err("Quick Sudo is disabled for this connection".to_string());
            }
        }
        let store = sudo_profiles::load_store(&self.runtime.data_dir());
        let profile = match explicit_profile {
            Some(profile) => Some(profile),
            None => stored
                .as_ref()
                .and_then(|stored| crate::sessions::ssh::effective_sudo_profile(stored, &store)),
        };
        let mut auth = match (&stored, &profile) {
            (Some(stored), profile) => {
                crate::sessions::ssh::resolved_sudo_auth(stored, profile.as_ref())
            }
            (None, Some(profile)) => {
                let mut auth = sudo_auth(arguments);
                apply_profile_fallbacks(&mut auth, profile);
                auth
            }
            (None, None) => sudo_auth(arguments),
        };
        apply_explicit_argument_overrides(&mut auth, arguments);
        Ok(auth)
    }

    /// stdio-mode terminal forwarding: relays the exec tool call to the DBX
    /// app's local TCP bridge, which opens the connection's workbench tab and
    /// runs the tool on the app's own sidecar — the same process as the
    /// visible terminal. The 200 body is already MCP-content wrapped and is
    /// returned verbatim (`call_tool` passes it through untouched); failures
    /// keep the bridge error and add reconnect guidance.
    pub(crate) async fn ssh_exec_app_bridge(
        &self,
        name: &str,
        connection_id: &str,
        arguments: &Value,
        timeout_secs: Option<Duration>,
    ) -> Result<Value, String> {
        app_bridge::ensure_app_bridge(app_bridge::DEFAULT_ENSURE_WAIT).await?;
        let timeout = timeout_secs.unwrap_or(Duration::from_secs(300));
        app_bridge::call_plugin_tool(connection_id, name, arguments.clone(), timeout)
            .await
            .map_err(|error| format!("{error}. Open the connection in DBX and retry"))
    }

    /// Command-text gates shared by the first pass (the caller-submitted text)
    /// and the post-approval re-check (a human-edited confirmation text): the
    /// per-connection sudo allowlist, the destructive assessment, the
    /// read-only command whitelist, and the terminal-input gate. `command` is
    /// the raw text for exec tools and the typed payload for
    /// `ssh_terminal_input`.
    pub(crate) async fn enforce_command_text_gates(
        &self,
        name: &str,
        command: &str,
        read_only: bool,
        arguments: &Value,
    ) -> Result<(), String> {
        if matches!(name, "ssh_exec" | "ssh_exec_sudo" | "ssh_run_bg") {
            // Per-connection sudoers-style allowlist: privileged commands
            // must match a `sudo_whitelist` entry when the connection
            // declares one. Covers `ssh_exec_sudo` plus inline `sudo …` in
            // `ssh_exec`/`ssh_run_bg` (NOPASSWD / cached-stamp bypasses).
            if name == "ssh_exec_sudo" || mcp_safety::runs_under_sudo(command) {
                let allowlist = self.sudo_allowlist_for(arguments).await?;
                if !allowlist.is_empty() && !sudo_allowlist::is_allowed(&allowlist, command) {
                    return Err(format!(
                        "sudo command is not allowed by this connection's whitelist. \
                         Allowed patterns: {}",
                        sudo_allowlist::render_entries(&allowlist)
                    ));
                }
            }
            match mcp_safety::assess_command(command) {
                CommandRisk::Destructive(reason) if read_only => {
                    return Err(format!(
                        "Refused on read-only connection ({reason}): {command}"
                    ));
                }
                CommandRisk::Destructive(reason) => {
                    let confirmed = arg_bool(arguments, "confirmDestructive")?.unwrap_or(false);
                    if !confirmed {
                        return Err(format!(
                            "Command looks destructive ({reason}): {command}. \
                             Retry with confirmDestructive: true if this is intended."
                        ));
                    }
                }
                CommandRisk::Unknown if name == "ssh_exec" && read_only => {
                    return Err(format!(
                        "Connection is read-only and the command is not recognized \
                         as read-only: {command}. Only inspection commands (ls, cat, \
                         df, ps, systemctl status, journalctl, docker ps, ...) pass."
                    ));
                }
                _ => {}
            }
        }
        if name == "ssh_terminal_input" {
            // §1.2 gates run on the normalized text (the same text that will
            // be typed), so normalization happens here once and the handler
            // reuses the normalized form by re-deriving it from `input`.
            let normalized = mcp_safety::normalize_terminal_input(command);
            let allowlist = self.sudo_allowlist_for(arguments).await?;
            terminal_input_gate(
                &normalized,
                read_only,
                arg_bool(arguments, "confirmDestructive")?.unwrap_or(false),
                &allowlist,
            )?;
        }
        Ok(())
    }
}
