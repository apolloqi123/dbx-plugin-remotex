//! TaskProvider orchestration methods.

use super::*;

impl TaskProvider {
    pub(crate) fn new() -> Self {
        Self {
            runs: Mutex::new(HashMap::new()),
            seqs: Mutex::new(HashMap::new()),
            residents: Mutex::new(HashMap::new()),
            resident_history: Mutex::new(VecDeque::new()),
        }
    }

    pub(super) fn next_seq(&self, run_id: &str) -> u64 {
        let mut seqs = self
            .seqs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let next = seqs.entry(run_id.to_string()).or_insert(0);
        *next += 1;
        *next
    }

    pub(super) fn drop_seq(&self, run_id: &str) {
        self.seqs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(run_id);
    }

    pub(super) fn register_run(&self, run_id: &str) -> watch::Receiver<bool> {
        let (sender, receiver) = watch::channel(false);
        self.runs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(run_id.to_string(), sender);
        receiver
    }

    pub(super) fn finish_run(&self, run_id: &str) {
        self.runs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(run_id);
        self.drop_seq(run_id);
    }

    pub(super) fn cancel_run(&self, run_id: &str) -> bool {
        self.runs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(run_id)
            .map(|sender| sender.send(true).is_ok())
            .unwrap_or(false)
    }

    pub(super) fn residents_lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, ResidentEntry>> {
        self.residents
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(super) fn emit_state(&self, emitter: &PluginEmitter, request: &TaskRequest, state: &str) {
        let _ = emitter.event(
            "task/state",
            json!({
                "event": "task/state",
                "taskId": request.task_id,
                "runId": request.run_id,
                "state": state,
            }),
        );
    }

    pub(super) fn emit_progress(
        &self,
        emitter: &PluginEmitter,
        request: &TaskRequest,
        percent: f64,
    ) {
        let _ = emitter.event(
            "task/progress",
            json!({
                "event": "task/progress",
                "taskId": request.task_id,
                "runId": request.run_id,
                "percent": percent,
            }),
        );
    }

    pub(super) fn emit_resident_state(
        &self,
        emitter: &PluginEmitter,
        task_id: &str,
        run_id: &str,
        session_id: &str,
        state: ResidentState,
    ) {
        let _ = emitter.event(
            "task/state",
            json!({
                "event": "task/state",
                "taskId": task_id,
                "runId": run_id,
                "sessionId": session_id,
                "state": state.as_str(),
            }),
        );
    }

    #[allow(clippy::too_many_arguments)] // flat event fields mirror the frozen payload shape
    pub(super) fn emit_log(
        &self,
        emitter: &PluginEmitter,
        task_id: &str,
        run_id: &str,
        seq: u64,
        stream: LogStream,
        level: &str,
        message: &str,
    ) {
        let _ = emitter.event(
            "task/log",
            json!({
                "event": "task/log",
                "taskId": task_id,
                "runId": run_id,
                "seq": seq,
                "stream": stream.as_str(),
                "level": level,
                "message": message,
                "timestamp": rfc3339_now(),
            }),
        );
    }

    // -- task/validate ------------------------------------------------------

    /// `task/validate`: pure config/trigger inspection — no dial, no secret
    /// access (ADR §5.2: validation must not touch secret plaintext).
    pub(crate) fn validate_request(params: &Value) -> Result<Value, String> {
        let task = params
            .get("task")
            .ok_or_else(|| task_error(CODE_INVALID_CONFIG, "Missing task envelope"))?;
        let trigger_id = task
            .get("triggerId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let config = task.get("config").cloned().unwrap_or_else(|| json!({}));
        let (mut errors, warnings) = validate_task_config(trigger_id, &config);
        if task
            .get("connectionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .is_empty()
        {
            // A missing connection makes the task unrunnable but the
            // definition itself may still be edited — a warning keeps the
            // form usable (health model: connection missing = warning).
            errors.push("connectionId is required".to_string());
        }
        Ok(json!({
            "valid": errors.is_empty(),
            "errors": errors,
            "warnings": warnings,
        }))
    }

    // -- task/execute -------------------------------------------------------

    /// `task/execute`: run the command once and resolve with the final
    /// result. Timeout/cancel return typed RPC errors after the remote
    /// process has actually been torn down.
    /// Redaction source for a connection: `Some` when the stored (hydrated)
    /// connection exists; `None` when only a live session does (secrets were
    /// never pushed this process run). Errors with the contract's
    /// `connection_missing` code when neither exists.
    pub(super) async fn resolve_redactor(
        ssh: &SshRuntime,
        connection_id: &str,
    ) -> Result<Redactor, String> {
        match ssh.stored_connection(connection_id).await {
            Ok(connection) => Ok(Redactor::from_connection(&connection)),
            Err(error) => {
                if ssh.has_live_session(connection_id).await {
                    Ok(Redactor::default())
                } else {
                    Err(task_error(CODE_CONNECTION_MISSING, error))
                }
            }
        }
    }

    pub(crate) async fn execute(
        &self,
        ssh: &SshRuntime,
        params: &Value,
        emitter: PluginEmitter,
    ) -> Result<Value, String> {
        let request = TaskRequest::parse(params)?;
        if request.trigger_id != "execute" {
            return Err(task_error(
                CODE_INVALID_CONFIG,
                format!("trigger {:?} does not support execute", request.trigger_id),
            ));
        }
        let config = TaskConfig::parse(&request.trigger_id, &request.config)?;
        // Resolve the bound connection's provider marker first: a local
        // shell target skips the SSH credential pipeline entirely (there is
        // no secret material to redact on one). Everything else keeps the
        // fail-fast `connection_missing` behavior — a scheduled run against
        // a dropped connection must not masquerade as a transport failure
        // (ADR §5.4: connection_missing is non-retryable).
        let local_target = ssh.local_target(&request.connection_id).await;
        let redactor = if local_target.is_some() {
            Redactor::default()
        } else {
            match Self::resolve_redactor(ssh, &request.connection_id).await {
                Ok(redactor) => redactor,
                Err(error) => return Err(error),
            }
        };
        let mut cancel = self.register_run(&request.run_id);
        self.emit_state(&emitter, &request, "running");
        self.emit_progress(&emitter, &request, 0.0);
        self.emit_log(
            &emitter,
            &request.task_id,
            &request.run_id,
            self.next_seq(&request.run_id),
            LogStream::System,
            "info",
            &redactor.scrub(&format!("Starting command on {}", request.connection_id)),
        );
        let outcome = if let Some(target) = local_target {
            self.run_local(&target, &request, &config, &mut cancel, &emitter)
                .await
        } else {
            self.run_command(ssh, &request, &config, &redactor, &mut cancel, &emitter)
                .await
        };
        self.finish_run(&request.run_id);
        match outcome {
            Ok(RunEnd::Completed(Some(0))) => {
                self.emit_state(&emitter, &request, "success");
                self.emit_progress(&emitter, &request, 100.0);
                Ok(json!({
                    "success": true,
                    "exitCode": 0,
                    "message": "Command completed",
                    "artifacts": [],
                }))
            }
            Ok(RunEnd::Completed(exit_code)) => {
                self.emit_state(&emitter, &request, "failed");
                let exit_code = exit_code.unwrap_or(-1);
                self.emit_log(
                    &emitter,
                    &request.task_id,
                    &request.run_id,
                    self.next_seq(&request.run_id),
                    LogStream::System,
                    "error",
                    &format!("Command exited with code {exit_code}"),
                );
                Ok(json!({
                    "success": false,
                    "exitCode": exit_code,
                    "message": format!("Command exited with code {exit_code}"),
                    "artifacts": [],
                }))
            }
            Ok(RunEnd::TimedOut) => {
                self.emit_state(&emitter, &request, "timeout");
                Err(task_error(
                    CODE_TIMEOUT,
                    format!(
                        "command exceeded its {}s limit and was stopped",
                        config.timeout_seconds.unwrap_or_default()
                    ),
                ))
            }
            Ok(RunEnd::Cancelled) => {
                self.emit_state(&emitter, &request, "cancelled");
                Err(task_error(CODE_CANCELLED, "command was cancelled"))
            }
            Err(error) => {
                self.emit_state(&emitter, &request, "failed");
                Err(redactor.scrub(&error))
            }
        }
    }

    pub(super) async fn run_command(
        &self,
        ssh: &SshRuntime,
        request: &TaskRequest,
        config: &TaskConfig,
        redactor: &Redactor,
        cancel: &mut watch::Receiver<bool>,
        emitter: &PluginEmitter,
    ) -> Result<RunEnd, String> {
        // Script mode reads the local file before dialing: a bad
        // `script_path` must fail the run without burning a transport dial.
        let script_bytes = if config.script_path.is_empty() {
            None
        } else {
            Some(read_script_file(&config.script_path)?)
        };
        let transport = ssh
            .task_transport(
                &request.connection_id,
                &format!("task-{}", request.run_id),
                Some(emitter.clone()),
            )
            .await
            .map_err(|error| redactor.scrub(&error))?;
        // Script runs upload the file once and execute it through the
        // existing framing (env exports, guarded cd, interpreter call as the
        // trailing statement so the exit status is the script's own). The
        // temp file is removed on every path below — including cancel,
        // timeout and exec errors — so a crashed run never litters /tmp.
        let (script, uploaded_temp) = match script_bytes {
            None => (build_script(config), None),
            Some(bytes) => {
                let temp_path = ssh_upload_script(transport.handle(), &bytes)
                    .await
                    .map_err(|error| redactor.scrub(&error))?;
                let has_bash = ssh_has_bash(transport.handle()).await;
                let line = script_command_line(has_bash, &temp_path);
                (build_script_with(config, &line), Some(temp_path))
            }
        };
        let outcome = run_streaming(
            transport.handle(),
            &script,
            config.pty,
            config.timeout_seconds.map(Duration::from_secs),
            cancel,
            |stream, message| {
                let message = redactor.scrub(&message);
                self.emit_log(
                    emitter,
                    &request.task_id,
                    &request.run_id,
                    self.next_seq(&request.run_id),
                    stream,
                    stream.default_level(),
                    &message,
                );
            },
            None,
        )
        .await;
        if let Some(temp_path) = uploaded_temp {
            ssh_remove_script_file(transport.handle(), &temp_path).await;
        }
        transport.release().await;
        outcome
    }

    /// Local-shell run (`io.dbx.remotex.local` target): spawn the connection's
    /// shell on this machine with process-level env/cwd and the same
    /// streaming/cancel/timeout contract as the SSH path. Script mode
    /// verifies the file here (before spawn) so a stale path fails the run
    /// with a precise message instead of a shell usage error.
    pub(super) async fn run_local(
        &self,
        target: &LocalTarget,
        request: &TaskRequest,
        config: &TaskConfig,
        cancel: &mut watch::Receiver<bool>,
        emitter: &PluginEmitter,
    ) -> Result<RunEnd, String> {
        if !config.script_path.is_empty() {
            let path = std::path::Path::new(&config.script_path);
            if !path.is_file() {
                return Err(format!(
                    "script_path {:?} does not exist or is not a file",
                    config.script_path
                ));
            }
        }
        let shell = target.shell_or(&crate::sessions::local::resolve_default_shell(
            &crate::platform::paths::plugin_data_dir(),
        ));
        let args = local_spawn_args(&shell, config);
        run_local_streaming(
            &shell,
            &args,
            &config.working_directory,
            &config.environment,
            config.timeout_seconds.map(Duration::from_secs),
            cancel,
            |stream, message| {
                self.emit_log(
                    emitter,
                    &request.task_id,
                    &request.run_id,
                    self.next_seq(&request.run_id),
                    stream,
                    stream.default_level(),
                    &message,
                );
            },
            None,
        )
        .await
    }

    // -- task/stop ----------------------------------------------------------

    /// `task/stop`: cancel a run-mode execution (by runId) or stop a
    /// resident session (by sessionId). Idempotent: unknown ids answer `{}`.
    pub(crate) async fn stop(&self, params: &Value) -> Result<Value, String> {
        // Resident first: sessionId is the precise target.
        if let Some(session_id) = params
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            let cancel = {
                let mut residents = self.residents_lock();
                match residents.get_mut(session_id) {
                    Some(entry) => {
                        if matches!(
                            entry.state,
                            ResidentState::Running
                                | ResidentState::Starting
                                | ResidentState::Crashed
                        ) {
                            entry.state = ResidentState::Stopping;
                        }
                        entry.cancel.clone()
                    }
                    None => return Ok(json!({})),
                }
            };
            let _ = cancel.send(true);
            // Wait (bounded) so the response reflects a real teardown, not
            // just an accepted signal.
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                let state = self
                    .residents_lock()
                    .get(session_id)
                    .map(|entry| entry.state)
                    .unwrap_or(ResidentState::Stopped);
                if matches!(state, ResidentState::Stopped | ResidentState::Degraded) {
                    return Ok(json!({}));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            return Ok(json!({}));
        }
        if let Some(run_id) = params
            .get("runId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            self.cancel_run(run_id);
        }
        Ok(json!({}))
    }

    // -- task/status --------------------------------------------------------

    /// `task/status`: reconcile view for the host supervisor. Unknown
    /// session ids report `stopped` so a sidecar restart (which loses the
    /// in-memory registry) converges instead of wedging recovery.
    pub(crate) fn status(&self, params: &Value) -> Result<Value, String> {
        let session_id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let task_id = params
            .get("taskId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        // Snapshot under the lock, then release it: retired-session lookup
        // takes the history lock and we never hold both at once.
        let live = {
            let residents = self.residents_lock();
            match (session_id, task_id) {
                (Some(session_id), _) => residents
                    .get(session_id)
                    .map(|entry| (entry.state, entry.heartbeat_at, entry.restart_count)),
                (None, Some(task_id)) => residents
                    .values()
                    .find(|entry| entry.task_id == task_id)
                    .map(|entry| (entry.state, entry.heartbeat_at, entry.restart_count)),
                (None, None) => {
                    return Err(task_error(
                        CODE_INVALID_CONFIG,
                        "sessionId or taskId is required",
                    ))
                }
            }
        };
        // A session that reached a terminal state is no longer live, but its
        // final state (`degraded` — restart budget exhausted, `stopped`) is
        // exactly what the host reconciles against. Reporting the blanket
        // "unknown id" answer here would claim success for failed sessions.
        let status = live.or_else(|| {
            self.resident_summary(session_id, task_id)
                .map(|summary| (summary.state, summary.heartbeat_at, summary.restart_count))
        });
        Ok(match status {
            Some((state, heartbeat_at, restart_count)) => json!({
                "state": state.as_str(),
                "heartbeatAt": heartbeat_at.map(|at| {
                    rfc3339(at.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
                }),
                "restartCount": restart_count,
            }),
            None => json!({
                "state": "stopped",
                "heartbeatAt": Value::Null,
                "restartCount": 0,
            }),
        })
    }
}
