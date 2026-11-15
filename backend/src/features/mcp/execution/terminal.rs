//! MCP execution helpers.

use super::*;

impl McpState {
    pub(crate) async fn ssh_exec_terminal_tool(
        &self,
        name: &str,
        command: &str,
        connection_id: &str,
        timeout_secs: Option<Duration>,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let mode = self.runtime.agent_terminal_mode(connection_id);
        let risk = if name == "ssh_exec_sudo" {
            // Sudo runs through the user's terminal where the auto-sudo
            // state machine or a human answers the password prompt.
            agent_terminal::CommandRisk::Elevated
        } else if mcp_safety::runs_under_sudo(command) {
            // An inline `sudo …` is privilege escalation too, even when the
            // inner verb is harmless: teaching mode must approve it.
            agent_terminal::CommandRisk::Elevated
        } else {
            match mcp_safety::assess_command(command) {
                mcp_safety::CommandRisk::Destructive(_) => agent_terminal::CommandRisk::Elevated,
                mcp_safety::CommandRisk::ReadOnly => agent_terminal::CommandRisk::ReadOnly,
                mcp_safety::CommandRisk::Unknown => agent_terminal::CommandRisk::Mutating,
            }
        };
        let timeout = timeout_secs.map(|duration| duration.as_secs());
        // `ssh_exec_sudo` must type its `sudo` prefix into the terminal or
        // the escalation silently disappears (this path has no separate sudo
        // orchestration — the terminal's auto-sudo state machine answers the
        // prompt the prefixed command raises). Idempotent, so a prefix kept
        // through the approval dialog round-trips unchanged; an inline
        // `sudo …` from `ssh_exec` already carries it and stays untouched.
        // Shadowing here also puts the prefixed text on the approval prompt,
        // so what the user approves is exactly what gets typed.
        let command = if name == "ssh_exec_sudo" {
            agent_terminal::sudo_command_text(command)
        } else {
            command.to_string()
        };
        // Both routing outcomes need the connection's live workbench PTY.
        // The DBX app bridge opens the workbench tab right before forwarding,
        // so the PTY session may take a moment to appear: poll briefly before
        // falling back to the guidance error.
        let session_id = self
            .wait_for_connection_session(connection_id, Duration::from_secs(20))
            .await
            .map_err(|_| NO_TERMINAL_SESSION_MESSAGE.to_string())?;
        // Serialize concurrent agent commands on the same session: the guard
        // is intentionally held across the approval wait and the whole run —
        // that IS the serialization, so a second command queues behind an
        // in-flight approval instead of interleaving keystrokes with it on
        // the shared PTY. The lock is per-session, so different connections
        // still execute in parallel (no global lock here on purpose).
        let _exec_guard = self.runtime.agent_exec_guard(&session_id).await?;
        // Remembered approvals (IMPL_PLAN D1): a command on the connection's
        // remembered list downgrades `Prompt` to `Run`, so "approve +
        // remember" actually skips later prompts. Matching mirrors the
        // settings view (sudoers-style token rules on the exact typed
        // command); the disaster gate stays last — it lives inside
        // `exec_in_terminal` and is never bypassed by memory (D2).
        let remembered = agent_terminal::remembered_bypasses_prompt(
            &command,
            agent_approvals::matches(
                &agent_approvals::load_store(&self.runtime.data_dir()),
                connection_id,
                &command,
            ),
        );
        match agent_terminal::decide_with_memory(mode, risk, remembered) {
            agent_terminal::RoutingDecision::Run => {
                self.runtime
                    .exec_in_terminal(&session_id, name, &command, risk, timeout, emitter)
                    .await
            }
            agent_terminal::RoutingDecision::Prompt => {
                let approved = self
                    .runtime
                    .request_agent_approval(&session_id, name, &command, risk, None, emitter)
                    .await?;
                self.runtime
                    .exec_in_terminal(&session_id, name, &approved, risk, timeout, emitter)
                    .await
            }
            agent_terminal::RoutingDecision::Deny(reason) => Err(reason.to_string()),
        }
    }

    /// `ssh_terminal_input` (IMPL_PLAN_NETCATTY A2-T6): injects raw input
    /// into the connection's LIVE terminal session (interactive answers /
    /// Ctrl-C semantics). Output is deliberately not collected — the
    /// terminal itself shows it; for captured output the caller uses
    /// `ssh_exec` with `runInTerminal: true`. No session means a guidance
    /// error, matching the agent terminal "visible before it runs" rule.
    pub(crate) async fn ssh_terminal_input_tool(&self, arguments: &Value) -> Result<Value, String> {
        let input = required_str(arguments, "input")?;
        let append_newline = arg_bool(arguments, "appendNewline")?.unwrap_or(false);
        let normalized = mcp_safety::normalize_terminal_input(input);
        // Terminal sessions are keyed by saved connection id; an endpoint
        // (host/user) selector has no lifecycle registration and therefore
        // no workbench PTY either — both shapes fall into the same guidance.
        let connection_id = match self.registered_connection_by_ref(arguments).await? {
            Some(stored) => stored.id,
            None => {
                return Err(format!(
                    "{NO_TERMINAL_SESSION_MESSAGE}; for captured output use \
                     ssh_exec with runInTerminal: true"
                ));
            }
        };
        let session_id = self
            .runtime
            .session_id_for_connection(&connection_id)
            .await
            .map_err(|_| {
                format!(
                    "{NO_TERMINAL_SESSION_MESSAGE}; for captured output use \
                     ssh_exec with runInTerminal: true"
                )
            })?;
        // batch_terminal_input reports per-target outcomes (it cannot fail
        // as a call); a full/closed input queue surfaces in the row error.
        let outcome = self
            .runtime
            .batch_terminal_input(
                std::slice::from_ref(&session_id),
                &normalized,
                append_newline,
            )
            .await;
        let row = outcome
            .get("results")
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .ok_or("Terminal input outcome is missing its result row")?;
        if row.get("success").and_then(Value::as_bool) != Some(true) {
            return Err(row
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Terminal input could not be delivered")
                .to_string());
        }
        Ok(json!({ "sent": true, "sessionId": session_id }))
    }
}

/// Tools that mutate remote state and must be rejected on read-only
/// connections (mirrors the workbench `ensure_writable` / sudo-exec gates).
/// Plain `ssh_exec` is not listed: on read-only connections it is gated by
/// the read-only command whitelist in `call_tool` instead, so inspection
/// commands (`df`, `systemctl status`, ...) stay available. Like the
/// workbench read-only terminal, `sudo` execution is always refused.
/// `sftp_download` stays allowed too: it only reads the remote side (its
/// local write target is guarded by `is_sensitive_local_path`, and the
/// remote path by the sensitive-path denylist on read-only connections).
/// Terminal-input gate (IMPL_PLAN_NETCATTY §1.2, A2-T5). `normalized` is
/// already through `normalize_terminal_input`. Ordered cheapest-first:
/// ① read-only connections only accept control-only input (Enter / Ctrl+C /
/// escape runs — anything shell-visible could mutate state); ② any `\r`
/// line that assesses destructive needs `confirmDestructive` (refused
/// outright on read-only, mirroring `ssh_exec`); ③ `sudo …` lines go
/// through the connection's sudo allowlist when it declares one. ④ the
/// confirm permission-mode hook (§1.3 `execPermissionMode`) is a
/// deliberate留位 — the mode setting itself is not implemented yet.
pub(crate) fn terminal_input_gate(
    normalized: &str,
    read_only: bool,
    confirm_destructive: bool,
    allowlist: &[Vec<String>],
) -> Result<(), String> {
    if read_only && !mcp_safety::is_control_only_input(normalized) {
        return Err(
            "Connection is read-only; terminal input may only be control sequences \
             (Enter, Ctrl+C, escape runs) with no visible text"
                .to_string(),
        );
    }
    // Destructive check via the aggregate line assessor: the worst
    // `\r`-delimited line wins, so one catastrophic line is enough to gate.
    if let CommandRisk::Destructive(reason) = mcp_safety::assess_terminal_input(normalized) {
        if read_only {
            return Err(format!(
                "Refused on read-only connection ({reason}): {normalized}"
            ));
        }
        if !confirm_destructive {
            return Err(format!(
                "Terminal input looks destructive ({reason}): {normalized}. \
                 Retry with confirmDestructive: true if this is intended."
            ));
        }
    }
    for line in normalized.split('\r') {
        if mcp_safety::runs_under_sudo(line)
            && !allowlist.is_empty()
            && !sudo_allowlist::is_allowed(allowlist, line)
        {
            return Err(format!(
                "sudo command is not allowed by this connection's whitelist. \
                 Allowed patterns: {}",
                sudo_allowlist::render_entries(allowlist)
            ));
        }
    }
    Ok(())
}
