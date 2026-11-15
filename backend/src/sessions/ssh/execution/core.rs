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

    /// Runs a fixed read-only command on the remote-desktop transfer session.
    /// That session has no PTY and is absent from the host's connection
    /// registry, so it cannot use `ssh/exec`; the transfer feature only needs
    /// a fresh channel on the already authenticated transport.
    pub async fn files_session_exec(
        &self,
        session_id: &str,
        command: &str,
        timeout: Duration,
    ) -> Result<crate::sessions::ssh::exec::ExecOutcome, String> {
        let session = self.session(session_id).await?;
        if !session.files_only {
            return Err("Not a file transfer session".to_string());
        }
        crate::sessions::ssh::exec::exec_plain(&session.handle, command, timeout, &[]).await
    }

    /// Filesystem-family plain command channel: regular sessions go through
    /// [`SshRuntime::exec`] (env/timeout/registration preserved); files-only
    /// transfer sessions are absent from the host connection registry, so they
    /// reuse the already authenticated transport handle directly.
    pub(crate) async fn filesystem_exec(
        &self,
        session_id: &str,
        command: &str,
        timeout_secs: u64,
    ) -> Result<crate::sessions::ssh::exec::ExecOutcome, String> {
        let session = self.session(session_id).await?;
        if session.files_only {
            let timeout = Duration::from_secs(timeout_secs.clamp(5, 300));
            return crate::sessions::ssh::exec::exec_plain(&session.handle, command, timeout, &[])
                .await;
        }
        let response = self
            .exec(session_id, None, command, false, Some(timeout_secs))
            .await?;
        Ok(ExecOutcome {
            output: response
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            exit_code: response
                .get("exitCode")
                .and_then(Value::as_i64)
                .map(|code| code as i32)
                .unwrap_or(-1),
        })
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

    pub(in super::super) fn exec_response(&self, outcome: &ExecOutcome) -> Value {
        json!({
            "success": true,
            "output": outcome.output,
            "exitCode": outcome.exit_code,
        })
    }
}
