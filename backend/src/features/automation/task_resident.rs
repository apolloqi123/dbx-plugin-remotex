//! Task provider resident layer.

use super::*;

// ---------------------------------------------------------------------------
// TaskProvider
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResidentState {
    Starting,
    Running,
    Stopping,
    Stopped,
    Crashed,
    Degraded,
}

impl ResidentState {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Crashed => "crashed",
            Self::Degraded => "degraded",
        }
    }
}

/// Registry entry for one resident session, keyed by `session_id` (the map
/// key is the session id, so it is not duplicated into the struct).
pub(super) struct ResidentEntry {
    pub(super) task_id: String,
    pub(super) state: ResidentState,
    pub(super) heartbeat_at: Option<SystemTime>,
    pub(super) restart_count: u32,
    pub(super) cancel: watch::Sender<bool>,
}

impl TaskProvider {
    // -- task/start (resident) ----------------------------------------------

    /// `task/start`: spawn a supervised resident session. Per the contract's
    /// concurrency default (Replace) an existing session for the same task
    /// is stopped first; each TaskDefinition has at most one live session.
    pub(crate) async fn start_resident(
        self: &Arc<Self>,
        ssh: Arc<SshRuntime>,
        params: Value,
        emitter: PluginEmitter,
    ) -> Result<Value, String> {
        let request = TaskRequest::parse(&params)?;
        if request.trigger_id != "resident" {
            return Err(task_error(
                CODE_INVALID_CONFIG,
                format!(
                    "trigger {:?} does not support resident start",
                    request.trigger_id
                ),
            ));
        }
        let config = TaskConfig::parse(&request.trigger_id, &request.config)?;
        // Fail fast before a supervisor is spawned: a missing connection
        // would otherwise burn its restart budget crashing on dial errors.
        // Local shell targets have no credential pipeline to verify —
        // presence in the local-target registry was the connectionMissing
        // check (an unknown id means the host never connected it).
        if ssh.local_target(&request.connection_id).await.is_none() {
            Self::resolve_redactor(&ssh, &request.connection_id).await?;
        }
        // Replace: signal every live session of this task; their supervisor
        // loops observe the cancel and land on `stopped` on their own.
        {
            let mut residents = self.residents_lock();
            for entry in residents.values_mut() {
                if entry.task_id == request.task_id
                    && !matches!(
                        entry.state,
                        ResidentState::Stopped | ResidentState::Degraded
                    )
                {
                    entry.state = ResidentState::Stopping;
                    let _ = entry.cancel.send(true);
                }
            }
        }
        let session_id = format!("resident-{}", uuid::Uuid::new_v4().simple());
        let (cancel, cancel_rx) = watch::channel(false);
        self.residents_lock().insert(
            session_id.clone(),
            ResidentEntry {
                task_id: request.task_id.clone(),
                state: ResidentState::Starting,
                heartbeat_at: Some(SystemTime::now()),
                restart_count: 0,
                cancel,
            },
        );
        self.emit_resident_state(
            &emitter,
            &request.task_id,
            &request.run_id,
            &session_id,
            ResidentState::Starting,
        );
        let provider = Arc::clone(self);
        let supervise_session = session_id.clone();
        tokio::spawn(async move {
            provider
                .resident_supervise(ssh, request, config, supervise_session, cancel_rx, emitter)
                .await;
        });
        Ok(json!({ "sessionId": session_id, "state": "starting" }))
    }

    /// Resident supervision loop: dial → run → on crash obey the bounded
    /// restart policy → degrade when the budget is exhausted. Cancel lands
    /// at every await point and always ends in a real channel teardown.
    pub(super) async fn resident_supervise(
        self: Arc<Self>,
        ssh: Arc<SshRuntime>,
        request: TaskRequest,
        config: TaskConfig,
        session_id: String,
        mut cancel: watch::Receiver<bool>,
        emitter: PluginEmitter,
    ) {
        // start_resident already verified presence; an empty redactor only
        // means the secrets were never pushed this process run (or the
        // target is a local shell with no credentials at all).
        let redactor = Self::resolve_redactor(&ssh, &request.connection_id)
            .await
            .unwrap_or_default();
        // A local target spawns on this machine; SSH targets keep dialing a
        // transport per restart attempt. The decision re-reads the registry
        // each iteration so a mid-life disconnect+reconnect (possibly as a
        // different provider binding) is honored like any other dial.
        let local_target = ssh.local_target(&request.connection_id).await;
        let script = build_script(&config);
        let mut restarts: Vec<Instant> = Vec::new();
        loop {
            let session_for_tick = session_id.clone();
            let provider_for_tick = Arc::clone(&self);
            let mut on_log = |stream: LogStream, message: String| {
                let message = redactor.scrub(&message);
                provider_for_tick.emit_log(
                    &emitter,
                    &request.task_id,
                    &request.run_id,
                    provider_for_tick.next_seq(&request.run_id),
                    stream,
                    stream.default_level(),
                    &message,
                );
            };
            let mut on_tick = || {
                provider_for_tick.resident_heartbeat(&session_for_tick);
            };
            let outcome = if let Some(target) = local_target.as_ref() {
                // Local shell target: nothing to dial — running starts at the
                // spawn, so the state flip happens right before it.
                self.resident_set_state(&session_id, ResidentState::Running);
                self.emit_resident_state(
                    &emitter,
                    &request.task_id,
                    &request.run_id,
                    &session_id,
                    ResidentState::Running,
                );
                let shell = target.shell_or(&crate::sessions::local::resolve_default_shell(
                    &crate::platform::paths::plugin_data_dir(),
                ));
                let args = local_spawn_args(&shell, &config);
                run_local_streaming(
                    &shell,
                    &args,
                    &config.working_directory,
                    &config.environment,
                    // Contract: timeout_seconds governs run executions and
                    // the resident start phase only — the resident loop
                    // itself is supervised by the restart policy, not a
                    // wall clock.
                    None,
                    &mut cancel,
                    &mut on_log,
                    Some(&mut on_tick),
                )
                .await
            } else {
                // Dial (live session handle or dedicated headless transport).
                let transport = match ssh
                    .task_transport(
                        &request.connection_id,
                        &format!("task-{}", request.run_id),
                        Some(emitter.clone()),
                    )
                    .await
                {
                    Ok(transport) => transport,
                    Err(error) => {
                        let degraded = self
                            .resident_crash_step(
                                &request,
                                &config,
                                &session_id,
                                &emitter,
                                &mut restarts,
                                &mut cancel,
                                &redactor.scrub(&format!("connect failed: {error}")),
                            )
                            .await;
                        if degraded {
                            return;
                        }
                        continue;
                    }
                };
                self.resident_set_state(&session_id, ResidentState::Running);
                self.emit_resident_state(
                    &emitter,
                    &request.task_id,
                    &request.run_id,
                    &session_id,
                    ResidentState::Running,
                );
                let outcome = run_streaming(
                    transport.handle(),
                    &script,
                    config.pty,
                    // Contract: timeout_seconds governs run executions and the
                    // resident start phase only — the resident loop itself is
                    // supervised by the restart policy, not a wall clock.
                    None,
                    &mut cancel,
                    &mut on_log,
                    Some(&mut on_tick),
                )
                .await;
                transport.release().await;
                outcome
            };
            match outcome {
                Ok(RunEnd::Cancelled) => {
                    self.resident_set_state(&session_id, ResidentState::Stopped);
                    self.emit_resident_state(
                        &emitter,
                        &request.task_id,
                        &request.run_id,
                        &session_id,
                        ResidentState::Stopped,
                    );
                    self.emit_log(
                        &emitter,
                        &request.task_id,
                        &request.run_id,
                        self.next_seq(&request.run_id),
                        LogStream::System,
                        "info",
                        "Resident command stopped",
                    );
                    return;
                }
                Ok(RunEnd::TimedOut) => {
                    // Unreachable for residents (no wall-clock deadline), kept
                    // total so the match stays exhaustive.
                    let degraded = self
                        .resident_crash_step(
                            &request,
                            &config,
                            &session_id,
                            &emitter,
                            &mut restarts,
                            &mut cancel,
                            "Resident start phase exceeded its deadline",
                        )
                        .await;
                    if degraded {
                        return;
                    }
                }
                Ok(RunEnd::Completed(Some(0))) => {
                    // A clean exit is a completed session, not a crash: a
                    // resident command that finishes successfully settles the
                    // session as `stopped`, so the host's probe marks the run
                    // successful instead of burning the restart budget on
                    // short commands.
                    self.resident_set_state(&session_id, ResidentState::Stopped);
                    self.emit_resident_state(
                        &emitter,
                        &request.task_id,
                        &request.run_id,
                        &session_id,
                        ResidentState::Stopped,
                    );
                    self.emit_log(
                        &emitter,
                        &request.task_id,
                        &request.run_id,
                        self.next_seq(&request.run_id),
                        LogStream::System,
                        "info",
                        "Resident command completed",
                    );
                    return;
                }
                Ok(RunEnd::Completed(exit_code)) => {
                    let detail = match exit_code {
                        Some(code) => format!("exited with code {code}"),
                        None => "channel closed without an exit status".to_string(),
                    };
                    let degraded = self
                        .resident_crash_step(
                            &request,
                            &config,
                            &session_id,
                            &emitter,
                            &mut restarts,
                            &mut cancel,
                            &format!("Resident command {detail}"),
                        )
                        .await;
                    if degraded {
                        return;
                    }
                }
                Err(error) => {
                    let degraded = self
                        .resident_crash_step(
                            &request,
                            &config,
                            &session_id,
                            &emitter,
                            &mut restarts,
                            &mut cancel,
                            &redactor.scrub(&error),
                        )
                        .await;
                    if degraded {
                        return;
                    }
                }
            }
        }
    }

    /// One crash iteration: record it, then restart (bounded) or degrade.
    /// Returns `true` when the session is finished (degraded or — on cancel
    /// during backoff — stopped).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn resident_crash_step(
        &self,
        request: &TaskRequest,
        config: &TaskConfig,
        session_id: &str,
        emitter: &PluginEmitter,
        restarts: &mut Vec<Instant>,
        cancel: &mut watch::Receiver<bool>,
        detail: &str,
    ) -> bool {
        let now = Instant::now();
        restarts.push(now);
        self.resident_set_state(session_id, ResidentState::Crashed);
        self.emit_resident_state(
            emitter,
            &request.task_id,
            &request.run_id,
            session_id,
            ResidentState::Crashed,
        );
        let policy = &config.restart;
        if !policy.restart_allowed(restarts, now) {
            // Surface the exhausted budget through restartCount too: the
            // denied restart still happened as a crash the supervisor
            // handled, and the reconcile view should show it.
            {
                let mut residents = self.residents_lock();
                if let Some(entry) = residents.get_mut(session_id) {
                    entry.restart_count = restarts.len() as u32;
                }
            }
            self.resident_set_state(session_id, ResidentState::Degraded);
            self.emit_resident_state(
                emitter,
                &request.task_id,
                &request.run_id,
                session_id,
                ResidentState::Degraded,
            );
            self.emit_log(
                emitter,
                &request.task_id,
                &request.run_id,
                self.next_seq(&request.run_id),
                LogStream::System,
                "error",
                &format!(
                    "{detail}; restart budget exhausted ({} restarts within {}), session degraded",
                    restarts.len(),
                    policy
                        .window_seconds
                        .map(|window| format!("{window}s window"))
                        .unwrap_or_else(|| "the process lifetime".to_string()),
                ),
            );
            return true;
        }
        let attempt = restarts.len() as u32;
        {
            let mut residents = self.residents_lock();
            if let Some(entry) = residents.get_mut(session_id) {
                entry.restart_count = attempt;
            }
        }
        self.emit_log(
            emitter,
            &request.task_id,
            &request.run_id,
            self.next_seq(&request.run_id),
            LogStream::System,
            "warn",
            &format!(
                "{detail}; restarting ({attempt}/{}) in {}s",
                policy.max_restarts, policy.backoff_seconds
            ),
        );
        let deadline = Instant::now() + Duration::from_secs(policy.backoff_seconds);
        while Instant::now() < deadline {
            if *cancel.borrow_and_update() {
                self.resident_set_state(session_id, ResidentState::Stopped);
                self.emit_resident_state(
                    emitter,
                    &request.task_id,
                    &request.run_id,
                    session_id,
                    ResidentState::Stopped,
                );
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    pub(super) fn resident_set_state(&self, session_id: &str, state: ResidentState) {
        let mut residents = self.residents_lock();
        if let Some(entry) = residents.get_mut(session_id) {
            entry.state = state;
            if state == ResidentState::Running || state == ResidentState::Stopped {
                entry.heartbeat_at = Some(SystemTime::now());
            }
        }
    }

    pub(super) fn resident_heartbeat(&self, session_id: &str) {
        let mut residents = self.residents_lock();
        if let Some(entry) = residents.get_mut(session_id) {
            entry.heartbeat_at = Some(SystemTime::now());
        }
    }
}
