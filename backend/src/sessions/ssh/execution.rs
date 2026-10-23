//! Exec operations for [`SshRuntime`].

use super::*;

impl SshRuntime {
    /// Completion-host read-only gate (FIG wave-1 lane B, decision D4):
    /// `completion/execute` must refuse SSH targets on read-only
    /// connections. Minimal read-only query only — no other behavior of
    /// this module changes.
    pub async fn completion_session_read_only(&self, session_id: &str) -> Result<bool, String> {
        Ok(self.session(session_id).await?.read_only)
    }

    /// Connection-level sudoers-style allowlist for privileged commands
    /// (`sudo_whitelist` in the connection's external config). Empty config
    /// = gate off; otherwise the command (minus a leading `sudo` token) must
    /// match one entry. Mirrors the MCP gate on the workbench exec path;
    /// structured `sudo_fs` operations are user-driven and stay exempt.
    ///
    /// Returns the command that may actually execute: with the gate on, this
    /// is the allowlist's token-quoted rebuild (`allowed_rebuilt_command`) —
    /// assignments stripped, metacharacters quarantined — so a passed match
    /// can never regain shell semantics under the root `sh -c`.
    pub async fn ensure_sudo_allowed(
        &self,
        session_id: &str,
        command: &str,
    ) -> Result<String, String> {
        let session = self.session(session_id).await?;
        let connection = self
            .connections
            .read()
            .map_err(|_| "Connection registry is poisoned".to_string())?
            .get(&session.connection_id)
            .cloned()
            .ok_or("Connection is not active; reopen it from DBX".to_string())?;
        if connection.sudo_whitelist.is_empty() {
            return Ok(command.to_string());
        }
        let entries = crate::features::security::sudo::allowlist::entries_from_lines(
            &connection.sudo_whitelist,
        );
        if let Some(rebuilt) =
            crate::features::security::sudo::allowlist::allowed_rebuilt_command(&entries, command)
        {
            return Ok(rebuilt);
        }
        Err(format!(
            "sudo command is not allowed by this connection's whitelist. \
             Allowed patterns: {}",
            crate::features::security::sudo::allowlist::render_entries(&entries)
        ))
    }

    /// `ssh/agent/mode/get`: connection-scoped agent terminal mode probe for
    /// the MCP bridge — the DBX app asks this before forwarding a stdio tool
    /// call so it can decide whether the workbench tab must exist for a
    /// terminal-routed exec. No sessionId by design: the caller holds only
    /// the connection id, and unknown ids degrade to `off` instead of
    /// erroring (an old/stale reference must never break the silent path).
    pub async fn agent_mode_get(&self, connection_id: &str) -> Result<Value, String> {
        let has_terminal_session = self.session_id_for_connection(connection_id).await.is_ok();
        Ok(json!({
            "agentTerminalMode": self.agent_terminal_mode(connection_id).name(),
            "hasTerminalSession": has_terminal_session,
        }))
    }

    /// Runs a command on the session's connection, optionally with Quick Sudo
    /// orchestration (password injection plus automatic 2FA/TOTP answers).
    pub async fn exec(
        &self,
        session_id: &str,
        exec_id: Option<&str>,
        command: &str,
        sudo: bool,
        timeout_secs: Option<u64>,
    ) -> Result<Value, String> {
        let session = self.session(session_id).await?;
        if sudo && session.read_only {
            return Err(
                "Sudo execution is disabled by the read-only connection setting".to_string(),
            );
        }
        let timeout = Duration::from_secs(
            timeout_secs
                .unwrap_or(if sudo {
                    SUDO_EXEC_TIMEOUT.as_secs()
                } else {
                    PLAIN_EXEC_TIMEOUT.as_secs()
                })
                .clamp(5, 300),
        );
        // Connection config is needed on both paths now: Quick Sudo
        // orchestration for sudo, and the client-specified setEnv either way.
        let connection = self
            .connections
            .read()
            .map_err(|_| "Connection registry is poisoned".to_string())?
            .get(&session.connection_id)
            .cloned()
            .ok_or("Connection is not active; reopen it from DBX".to_string())?;
        if sudo && !connection.sudo_enabled() {
            return Err("Quick Sudo is disabled for this connection".to_string());
        }
        let orchestration = session
            .orchestration
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let keepalive_handle = session.handle.clone();
        let connection_id = session.connection_id.clone();
        let keepalive_session_id = session_id.to_string();
        let handle = session.handle.clone();
        let command = command.to_string();

        let use_pty = if sudo {
            let store = sudo_profiles::load_store(&self.data_dir);
            sudo_profiles::effective_use_pty(
                connection.sudo_use_pty,
                effective_sudo_profile(&connection, &store).as_ref(),
            )
        } else {
            false
        };
        // cancel_exec 经 watch 发信号：exec 在下一个 await 点退出并经既有
        // Err 路径优雅关闭 channel。裸 abort 会在 await 点丢弃 russh
        // Channel——drop 不发 SSH_MSG_CHANNEL_CLOSE，远端进程残留，sudo 还
        // 会持有 timestamp 锁死锁后续 sudo（exec.rs abort_exec_channel 的
        // 自述场景）。
        let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
        let run = async move {
            let outcome = if sudo {
                exec::exec_with_sudo_cancellable(
                    &handle,
                    &orchestration,
                    &command,
                    timeout,
                    use_pty,
                    &connection.set_env,
                    &mut cancel_rx,
                )
                .await?
            } else {
                exec::exec_plain_cancellable(
                    &handle,
                    &command,
                    timeout,
                    &connection.set_env,
                    &mut cancel_rx,
                )
                .await?
            };
            Ok(outcome)
        };

        let outcome = match exec_id.filter(|id| !id.is_empty()) {
            Some(exec_id) => {
                let exec_id = exec_id.to_string();
                let task = tokio::spawn(run);
                if let Ok(mut tasks) = self.exec_tasks.lock() {
                    tasks.insert(exec_id.clone(), cancel_tx);
                }
                let result = task
                    .await
                    .map_err(|join_error| {
                        if join_error.is_cancelled() {
                            "Remote command was cancelled".to_string()
                        } else {
                            "Remote command task failed".to_string()
                        }
                    })
                    .and_then(|inner| inner);
                if let Ok(mut tasks) = self.exec_tasks.lock() {
                    tasks.remove(&exec_id);
                }
                result?
            }
            None => run.await?,
        };
        if sudo {
            self.register_sudo_keepalive(&connection_id, &keepalive_session_id, keepalive_handle);
        }
        Ok(self.exec_response(&outcome))
    }

    /// Aborts an in-flight `ssh/exec`; ported from tiny-rdm's AbortCommand.
    /// Sends the cancellation signal instead of aborting the task: the exec
    /// future exits at its next await point and closes the channel through
    /// the regular error path (see [`exec::EXEC_CANCELLED_MESSAGE`]), so the
    /// remote process — and any sudo timestamp lock it holds — is reaped by
    /// sshd instead of leaking with the dropped future.
    pub fn cancel_exec(&self, exec_id: &str) -> Result<(), String> {
        let task = self
            .exec_tasks
            .lock()
            .map_err(|_| "Exec registry is poisoned".to_string())?
            .remove(exec_id);
        match task {
            // Send fails only when the receiver is gone (task already
            // finished; registry removal raced the finish) — nothing to do.
            Some(cancel) => {
                let _ = cancel.send(true);
                Ok(())
            }
            None => Err("Remote command was not found or already finished".to_string()),
        }
    }

    pub(super) fn exec_response(&self, outcome: &ExecOutcome) -> Value {
        json!({
            "success": true,
            "output": outcome.output,
            "exitCode": outcome.exit_code,
        })
    }

    /// Per-connection AI terminal mode; unknown connections read `off`.
    pub fn agent_terminal_mode(&self, connection_id: &str) -> AgentTerminalMode {
        self.agent_modes
            .lock()
            .ok()
            .and_then(|modes| modes.get(connection_id).copied())
            .unwrap_or(AgentTerminalMode::Off)
    }

    pub(super) fn set_agent_terminal_mode(
        &self,
        connection_id: &str,
        mode: AgentTerminalMode,
    ) -> Result<(), String> {
        let snapshot = {
            let mut modes = self
                .agent_modes
                .lock()
                .map_err(|_| "Agent mode registry is poisoned".to_string())?;
            if mode == AgentTerminalMode::Off {
                // Persisted off is the default: dropping the entry keeps the
                // file bounded by connections the user actually opted in.
                modes.remove(connection_id);
            } else {
                modes.insert(connection_id.to_string(), mode);
            }
            modes.clone()
        };
        agent_terminal::save_modes(&self.data_dir, &snapshot)
    }

    /// Runs an AI/MCP command inside the session's interactive PTY: the
    /// command is typed into the user's terminal (visible, interruptible),
    /// the output is captured by a session-level recorder, and the captured
    /// text is returned once a fresh prompt settles. On timeout the partial
    /// output is returned with `incomplete: true` and the command keeps
    /// running in the terminal for the user to take over.
    pub async fn exec_in_terminal(
        &self,
        session_id: &str,
        tool: &str,
        command: &str,
        risk: CommandRisk,
        timeout_secs: Option<u64>,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let session = self
            .sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or(NO_TERMINAL_SESSION_MESSAGE.to_string())?;
        // Strip control characters so the injected text cannot drive the
        // terminal or the auto-sudo state machine (same trust domain as the
        // quick-command bar: raw keyboard input, no shell quoting surface).
        let command = agent_terminal::sanitize_command(command)?;
        let timeout = Duration::from_secs(
            timeout_secs
                .unwrap_or(PLAIN_EXEC_TIMEOUT.as_secs())
                .clamp(5, 300),
        );

        // Install the recorder before injecting so no output is lost. The
        // first line of the command gates settling: without its echo the
        // recorder would settle on the login-banner prompt before the shell
        // even processed the injection (boot-restore race).
        let echo_fragment = command
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        {
            let mut slot = session
                .agent_recorder
                .lock()
                .map_err(|_| "Terminal recorder is poisoned".to_string())?;
            let mut recorder = TerminalRecorder::default();
            recorder.arm(&echo_fragment);
            *slot = Some(recorder);
        }
        let _ = emitter.event(
            "ssh/agent/notice",
            json!({
                "sessionId": session_id,
                "tool": tool,
                "command": command,
                "risk": risk.name(),
            }),
        );

        // Inject like the quick-command bar: raw text plus a carriage
        // return on the session's PTY input queue. The send is bounded so a
        // zombie session (dead read loop, full queue) cannot park the agent
        // call forever.
        let payload = format!("{command}\r").into_bytes();
        let injected = tokio::time::timeout(
            Duration::from_secs(5),
            session.terminal_tx.send(TerminalCommand::Input(payload)),
        )
        .await;
        match injected {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                if let Ok(mut slot) = session.agent_recorder.lock() {
                    *slot = None;
                }
                return Err(format!("Terminal is closed: {error}"));
            }
            Err(_) => {
                if let Ok(mut slot) = session.agent_recorder.lock() {
                    *slot = None;
                }
                return Err("Terminal is not accepting input (session unresponsive)".to_string());
            }
        }

        // Poll the recorder: prompt-seen plus 300ms of silence, or deadline.
        // A closed session (workbench tab closed mid-run) ends the wait with
        // a dedicated error instead of masquerading as a timeout.
        let deadline = tokio::time::Instant::now() + timeout;
        let mut poll_cycles = 0usize;
        let timed_out = loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            poll_cycles += 1;
            let settled = session
                .agent_recorder
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().map(TerminalRecorder::is_settled))
                .unwrap_or(false);
            if settled {
                break false;
            }
            if poll_cycles.is_multiple_of(20)
                && !self.sessions.read().await.contains_key(session_id)
            {
                if let Ok(mut slot) = session.agent_recorder.lock() {
                    slot.take();
                }
                let _ = emitter.event(
                    "ssh/agent/finish",
                    json!({ "sessionId": session_id, "status": "timeout" }),
                );
                return Err("Terminal session was closed while the command was running".to_string());
            }
            if tokio::time::Instant::now() >= deadline {
                break true;
            }
        };

        // Always remove the recorder; the read loop skips the empty slot.
        let recorder = session
            .agent_recorder
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        let output = match recorder {
            Some(mut recorder) => {
                if timed_out {
                    // Force the capture closed; the command keeps running in
                    // the terminal and the AI gets the partial output.
                    recorder.finish();
                }
                recorder.take_output(&command)
            }
            None => String::new(),
        };

        let _ = emitter.event(
            "ssh/agent/finish",
            json!({
                "sessionId": session_id,
                "status": if timed_out { "timeout" } else { "done" },
            }),
        );

        // exitCode stays null without shell integration: the AI judges from
        // the output text. `interrupted` is reserved for the workbench
        // interrupt button and stays false on this path.
        Ok(json!({
            "output": output,
            "exitCode": null,
            "mode": "terminal",
            "incomplete": timed_out,
            "interrupted": false,
        }))
    }

    /// Acquires the session's agent-execution lock for the caller. The owned
    /// guard is returned so `ssh_exec_terminal_tool` can hold it across the
    /// approval wait and the whole run — holding across `await` is the
    /// deliberate serialization (a queued command must wait out an in-flight
    /// approval, not race it). Per-session by design: different connections
    /// keep running in parallel.
    pub(crate) async fn agent_exec_guard(
        &self,
        session_id: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
        let session = self.session(session_id).await?;
        let lock = Arc::clone(&session.agent_exec_lock);
        Ok(lock.lock_owned().await)
    }

    /// Pure builder for the MCP confirm challenge payload (IMPL_PLAN §1.3):
    /// the shared `ssh/agent/prompt` event with `kind: "mcp-confirm"` and
    /// `source: "mcp"` (legacy terminal approvals never carry `source`, so
    /// the workbench can tell the two apart; older frontends fall back to
    /// their existing rendering). `risk` is mandatory for every
    /// `ssh/agent/prompt` event: the workbench decoder drops prompts without
    /// it, which silently denied MCP confirms after the 120s wait. MCP
    /// confirms are always the elevated tier. Unit-tested shape.
    pub(crate) fn mcp_confirm_challenge_payload(
        challenge_id: &str,
        tool: &str,
        command: &str,
        connection_id: Option<&str>,
        timeout_secs: u64,
    ) -> Value {
        let mut payload = json!({
            "challengeId": challenge_id,
            "kind": "mcp-confirm",
            "source": "mcp",
            "risk": CommandRisk::Elevated.name(),
            "tool": tool,
            "command": command,
            "requestedAt": unix_now_secs(),
            "timeoutSecs": timeout_secs,
        });
        if let Some(id) = connection_id.filter(|id| !id.is_empty()) {
            payload["connectionId"] = json!(id);
        }
        payload
    }

    /// MCP confirm permission gate (IMPL_PLAN §1.3): raises a
    /// `ssh/agent/prompt` approval challenge (kind `mcp-confirm`,
    /// source `mcp`) on the workbench and waits for `ssh/agent/resolve`.
    /// Returns the (possibly user-edited) command on approval; denial or
    /// the 120s timeout returns `Err`. One-shot challenge semantics are
    /// shared with the terminal approval path (`resolve_agent_challenge`).
    /// There is deliberately no session binding and no terminal follow-up
    /// events — an MCP confirm never types into a terminal.
    pub(crate) async fn request_mcp_confirm(
        &self,
        tool: &str,
        command: &str,
        connection_id: Option<&str>,
        emitter: &PluginEmitter,
    ) -> Result<String, String> {
        let wait = Duration::from_secs(MCP_CONFIRM_TIMEOUT_SECS);
        let challenge_id = Uuid::new_v4().to_string();
        let connection_id = connection_id.unwrap_or_default().to_string();
        let raised_at_ms = unix_now_ms();
        let (sender, receiver) = oneshot::channel();
        if let Ok(mut challenges) = self.agent_challenges.lock() {
            challenges.insert(
                challenge_id.clone(),
                PendingChallenge {
                    sender,
                    connection_id: connection_id.clone(),
                    tool: tool.to_string(),
                    command: command.to_string(),
                    raised_at_ms,
                },
            );
        }
        let raised = emitter.event(
            "ssh/agent/prompt",
            Self::mcp_confirm_challenge_payload(
                &challenge_id,
                tool,
                command,
                Some(&connection_id),
                wait.as_secs(),
            ),
        );
        if let Err(error) = raised {
            // Nobody can approve without the prompt; fail fast instead of
            // letting the challenge run into its timeout.
            if let Ok(mut challenges) = self.agent_challenges.lock() {
                challenges.remove(&challenge_id);
            }
            return Err(format!(
                "Failed to raise the approval prompt: {}",
                error.message
            ));
        }
        let decision = match tokio::time::timeout(wait, receiver).await {
            Ok(Ok(decision)) => Some(decision),
            // Timeout or dropped sender both mean "no user decision"; the
            // challenge is expired so a late resolve reports not found.
            Ok(Err(_)) | Err(_) => {
                if let Ok(mut challenges) = self.agent_challenges.lock() {
                    challenges.remove(&challenge_id);
                }
                None
            }
        };
        match decision {
            Some(AgentDecision::Approve { command: edited }) => {
                let edited = edited.filter(|text| !text.trim().is_empty());
                Ok(edited.unwrap_or_else(|| command.to_string()))
            }
            Some(AgentDecision::Deny) => {
                Err("Command not run: user denied the execution".to_string())
            }
            None => Err("Command not run: approval timed out waiting for the user".to_string()),
        }
    }

    /// Raises an approval challenge for a terminal-routed agent command:
    /// emits `ssh/agent/prompt`, then waits for `ssh/agent/resolve`.
    /// Returns the (possibly user-edited) command on approval; denial or
    /// timeout returns `Err` and emits `ssh/agent/finish{status:"denied"}`.
    pub(crate) async fn request_agent_approval(
        &self,
        session_id: &str,
        tool: &str,
        command: &str,
        risk: CommandRisk,
        timeout_secs: Option<u64>,
        emitter: &PluginEmitter,
    ) -> Result<String, String> {
        let wait = Duration::from_secs(
            timeout_secs
                .unwrap_or(AGENT_APPROVAL_DEFAULT_SECS)
                .clamp(AGENT_APPROVAL_MIN_SECS, AGENT_APPROVAL_MAX_SECS),
        );
        let challenge_id = Uuid::new_v4().to_string();
        let session = self.session(session_id).await?;
        let connection_id = session.connection_id.clone();
        let raised_at_ms = unix_now_ms();
        let (sender, receiver) = oneshot::channel();
        if let Ok(mut challenges) = self.agent_challenges.lock() {
            challenges.insert(
                challenge_id.clone(),
                PendingChallenge {
                    sender,
                    connection_id,
                    tool: tool.to_string(),
                    command: command.to_string(),
                    raised_at_ms,
                },
            );
        }
        let raised = emitter.event(
            "ssh/agent/prompt",
            json!({
                "challengeId": challenge_id,
                "sessionId": session_id,
                "tool": tool,
                "command": command,
                "risk": risk.name(),
                "requestedAt": unix_now_secs(),
                "timeoutSecs": wait.as_secs(),
            }),
        );
        if let Err(error) = raised {
            // Nobody can approve without the prompt; fail fast instead of
            // letting the challenge run into its timeout.
            if let Ok(mut challenges) = self.agent_challenges.lock() {
                challenges.remove(&challenge_id);
            }
            return Err(format!(
                "Failed to raise the approval prompt: {}",
                error.message
            ));
        }
        let decision = match tokio::time::timeout(wait, receiver).await {
            Ok(Ok(decision)) => Some(decision),
            // Timeout or dropped sender both mean "no user decision".
            Ok(Err(_)) | Err(_) => {
                // Expire the challenge so a late resolve reports not found.
                if let Some(pending) = self
                    .agent_challenges
                    .lock()
                    .ok()
                    .and_then(|mut challenges| challenges.remove(&challenge_id))
                {
                    self.audit_approval(
                        &pending.tool,
                        &pending.connection_id,
                        Some(&pending.command),
                        audit_log::ApprovalTrail::Timeout,
                        audit_log::EntryOutcome::Error,
                        unix_now_ms().saturating_sub(pending.raised_at_ms),
                    );
                }
                None
            }
        };
        // Approved: the follow-up notice/finish pair comes from
        // exec_in_terminal — emitting "denied" here would flash the workbench
        // banner state for a command that is about to run.
        if let Some(AgentDecision::Approve { command: edited }) = decision {
            let edited = edited.filter(|text| !text.trim().is_empty());
            return Ok(edited.unwrap_or_else(|| command.to_string()));
        }
        let _ = emitter.event(
            "ssh/agent/finish",
            json!({ "sessionId": session_id, "status": "denied" }),
        );
        match decision {
            Some(AgentDecision::Deny) => {
                Err("Command not run: user denied the terminal execution".to_string())
            }
            None => Err("Command not run: approval timed out waiting for the user".to_string()),
            Some(AgentDecision::Approve { .. }) => unreachable!("handled above"),
        }
    }

    /// Resolves a pending agent approval challenge (`ssh/agent/resolve`).
    /// One-shot: the challenge is removed from the registry before the
    /// decision is delivered, so a repeat resolve reports not found.
    /// `decision` is "approve" or "deny"; `command` carries the edited
    /// command text from the approval dialog (approve only); `remember`
    /// (approve only) persists the approved command on the connection's
    /// remembered-approval list so later identical matches skip the prompt.
    pub fn resolve_agent_challenge(
        &self,
        challenge_id: &str,
        decision: &str,
        command: Option<&str>,
        remember: bool,
    ) -> Result<(), String> {
        let approved = match decision {
            "approve" => true,
            "deny" => false,
            other => {
                return Err(format!(
                    "agent decision must be \"approve\" or \"deny\"; got '{other}'"
                ))
            }
        };
        let pending = self
            .agent_challenges
            .lock()
            .map_err(|_| "Agent challenge registry is poisoned".to_string())?
            .remove(challenge_id)
            .ok_or("Agent challenge was not found or already resolved")?;
        let decision = if approved {
            AgentDecision::Approve {
                command: command.map(str::to_string),
            }
        } else {
            AgentDecision::Deny
        };
        let mut remembered = false;
        if approved && remember {
            // The remembered text is what the dialog showed the user — the
            // edited command when one was sent, otherwise the original.
            let text = command
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .unwrap_or(&pending.command);
            // A destructive command is never rememberable (IMPL_PLAN D2):
            // the flag is silently dropped and the approval stays one-shot.
            let mut store = agent_approvals::load_store(&self.data_dir);
            match agent_approvals::remember(&mut store, &pending.connection_id, text) {
                Ok(newly_added) if newly_added => {
                    match agent_approvals::save_store(&self.data_dir, &store) {
                        Ok(()) => remembered = true,
                        Err(error) => {
                            eprintln!("[ssh] failed to persist remembered approval: {error}")
                        }
                    }
                }
                Ok(_) => remembered = true,
                Err(error) => eprintln!("[ssh] approval not remembered: {error}"),
            }
        }
        self.audit_approval(
            &pending.tool,
            &pending.connection_id,
            Some(&pending.command),
            match (approved, remembered) {
                (true, true) => audit_log::ApprovalTrail::Remembered,
                (true, false) => audit_log::ApprovalTrail::Approved,
                (false, _) => audit_log::ApprovalTrail::Denied,
            },
            if approved {
                audit_log::EntryOutcome::Ok
            } else {
                audit_log::EntryOutcome::Error
            },
            unix_now_ms().saturating_sub(pending.raised_at_ms),
        );
        pending
            .sender
            .send(decision)
            .map_err(|_| "Agent challenge is no longer waiting".to_string())
    }

    /// Appends one approval-lifecycle line to the execution audit ledger.
    /// Execution itself is audited by the MCP layer; these lines carry the
    /// human-decision trail (`approval` field) for terminal-routed commands.
    pub(super) fn audit_approval(
        &self,
        tool: &str,
        connection_id: &str,
        command: Option<&str>,
        approval: audit_log::ApprovalTrail,
        outcome: audit_log::EntryOutcome,
        duration_ms: u64,
    ) {
        let entry = audit_log::AuditEntry {
            ts_ms: unix_now_ms(),
            tool: tool.to_string(),
            connection_id: connection_id.to_string(),
            gate: audit_log::GateOutcome::Pass,
            approval,
            outcome,
            exit_code: None,
            duration_ms,
            mode: audit_log::ExecMode::Terminal,
            command: command.map(str::to_string),
            output: None,
            error: None,
        };
        if let Err(error) = audit_log::append(&self.data_dir, &entry) {
            eprintln!("[ssh] audit append failed: {error}");
        }
    }

    /// Starts a per-connection `sudo -nv` refresh loop after a successful
    /// sudo execution. The loop runs every [`exec::SUDO_KEEPALIVE_INTERVAL`]
    /// (4 minutes), stops itself after
    /// [`exec::SUDO_KEEPALIVE_MAX_FAILURES`] consecutive validation failures,
    /// and is aborted deterministically on disconnect / session cleanup via
    /// [`Self::stop_sudo_keepalive`]. Per connection single-instance: a
    /// repeat registration while one is live is a no-op.
    /// Registration decision for `register_sudo_keepalive` (pure for tests):
    /// an entry bound to the same live session is reused as-is; an entry bound
    /// to another (typically dead) session is replaced — the old early-return
    /// on any existing entry would silently drop the new session's keepalive
    /// after a reconnect (#202).
    pub(super) fn keepalive_registration_plan(
        existing_session: Option<&str>,
        incoming_session: &str,
    ) -> KeepaliveRegistration {
        match existing_session {
            Some(existing) if existing == incoming_session => KeepaliveRegistration::ReuseExisting,
            Some(_) => KeepaliveRegistration::ReplaceStale,
            None => KeepaliveRegistration::InsertFresh,
        }
    }

    pub(super) fn register_sudo_keepalive(
        &self,
        connection_id: &str,
        session_id: &str,
        handle: Arc<Handle<SshClient>>,
    ) {
        let mut keepalive = match self.sudo_keepalive.lock() {
            Ok(keepalive) => keepalive,
            Err(_) => return,
        };
        let existing_session = keepalive
            .get(connection_id)
            .map(|entry| entry.session_id.as_str());
        if matches!(
            Self::keepalive_registration_plan(existing_session, session_id),
            KeepaliveRegistration::ReuseExisting
        ) {
            return;
        }
        // Drop the stale entry and stop its loop before arming the new one.
        if let Some(existing) = keepalive.remove(connection_id) {
            existing.task.abort();
        }
        let interval = exec::SUDO_KEEPALIVE_INTERVAL;
        let registry = self.sudo_keepalive.clone();
        let id = connection_id.to_string();
        let task_handle = handle.clone();
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut failures = 0_u32;
            loop {
                ticker.tick().await;
                let succeeded = exec::validate_sudo_timestamp(&task_handle).await.is_ok();
                match exec::keepalive_failure_step(failures, succeeded) {
                    Some(next) => {
                        if next != 0 {
                            eprintln!(
                                "[ssh] sudo keepalive for {id}: timestamp validation failed ({next}/{})",
                                exec::SUDO_KEEPALIVE_MAX_FAILURES
                            );
                        }
                        failures = next;
                    }
                    None => {
                        eprintln!(
                            "[ssh] sudo keepalive for {id}: {} consecutive validation failures, stopping (sudo timestamp expired)",
                            exec::SUDO_KEEPALIVE_MAX_FAILURES
                        );
                        if let Ok(mut keepalive) = registry.lock() {
                            keepalive.remove(&id);
                        }
                        break;
                    }
                }
            }
        });
        keepalive.insert(
            connection_id.to_string(),
            SudoKeepalive {
                session_id: session_id.to_string(),
                task,
            },
        );
    }

    pub(super) async fn stop_sudo_keepalive(&self, connection_id: &str) {
        let entry = self
            .sudo_keepalive
            .lock()
            .ok()
            .and_then(|mut keepalive| keepalive.remove(connection_id));
        if let Some(entry) = entry {
            entry.task.abort();
        }
    }
}
