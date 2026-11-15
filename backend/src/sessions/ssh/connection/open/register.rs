//! Session registration, keepalive and startup plumbing for a newly opened
//! SSH session.
//!
//! All of it runs once per `open_session`, after the PTY channel is up and
//! before (or right after) the read loop takes ownership of the channel.

use super::*;

use crate::core::time::unix_now_secs;

impl SshRuntime {
    /// Registers the freshly opened session and returns its id, the shared
    /// entry and the terminal command receiver the read loop consumes.
    ///
    /// A requested session id is honored only when it is well formed and free;
    /// a collision is resolved with a fresh UUID. The entry is built before the
    /// collision check, as before.
    pub(super) async fn register_session(
        &self,
        request: &SessionOpenRequest,
        connection: &StoredConnection,
        workbench_id: &str,
        handle: Arc<Handle<SshClient>>,
        transport_lease: Arc<SharedTransportLease>,
        inherited_orchestration: Option<SudoAuth>,
    ) -> (String, Arc<SessionEntry>, mpsc::Receiver<TerminalCommand>) {
        let mut session_id = request
            .requested_session_id
            .as_deref()
            .filter(|id| valid_requested_session_id(id))
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let (terminal_tx, terminal_rx) = mpsc::channel(256);
        let replay = Arc::new(AsyncMutex::new(ReplayBuffer::default()));
        let bound_profile = {
            let store = sudo_profiles::load_store(&self.data_dir);
            effective_sudo_profile(connection, &store)
        };
        let orchestration =
            Arc::new(RwLock::new(inherited_orchestration.unwrap_or_else(|| {
                resolved_sudo_auth(connection, bound_profile.as_ref())
            })));
        // In-terminal Quick Sudo: answers sudo password / 2FA prompts while
        // the user keeps typing normal commands (ported from tiny-rdm). The
        // watcher is attached here and re-synced on every runtime settings
        // update, so configuring Quick Sudo after connecting still arms it.
        let entry = Arc::new(SessionEntry {
            connection_id: connection.id.clone(),
            workbench_id: RwLock::new(workbench_id.to_string()),
            read_only: connection.read_only,
            connected: AtomicBool::new(true),
            files_only: false,
            created_at_secs: unix_now_secs(),
            created_seq: self.session_seq.fetch_add(1, Ordering::Relaxed),
            handle,
            transport_lease,
            orchestration: orchestration.clone(),
            auto_sudo: Arc::new(Mutex::new(None)),
            triggers: Arc::new(Mutex::new(None)),
            terminal_tx,
            replay,
            agent_recorder: Arc::new(Mutex::new(None)),
            session_recorder: Arc::new(Mutex::new(None)),
            agent_exec_lock: Arc::new(AsyncMutex::new(())),
        });
        Self::sync_auto_sudo(&entry, connection);
        Self::sync_triggers(&entry, connection);
        let mut sessions = self.sessions.write().await;
        if sessions.contains_key(&session_id) {
            session_id = Uuid::new_v4().to_string();
        }
        sessions.insert(session_id.clone(), entry.clone());

        (session_id, entry, terminal_rx)
    }

    /// Registers the remote-desktop file transfer transport: an authenticated
    /// SSH connection with no PTY, replay buffer or read loop. The SFTP
    /// helpers only need the handle and the shared transport lease, so this
    /// lets the transfer panel reuse the whole upload/download pipeline
    /// without opening a shell on the server.
    pub(super) async fn register_files_session(
        &self,
        workbench_id: &str,
        connection: &StoredConnection,
        handle: Arc<Handle<SshClient>>,
        transport_lease: Arc<SharedTransportLease>,
    ) -> String {
        let (terminal_tx, _terminal_rx) = mpsc::channel(1);
        let replay = Arc::new(AsyncMutex::new(ReplayBuffer::default()));
        let entry = Arc::new(SessionEntry {
            connection_id: connection.id.clone(),
            workbench_id: RwLock::new(workbench_id.to_string()),
            read_only: false,
            connected: AtomicBool::new(true),
            files_only: true,
            created_at_secs: unix_now_secs(),
            created_seq: self.session_seq.fetch_add(1, Ordering::Relaxed),
            handle,
            transport_lease,
            orchestration: Arc::new(RwLock::new(resolved_sudo_auth(connection, None))),
            auto_sudo: Arc::new(Mutex::new(None)),
            triggers: Arc::new(Mutex::new(None)),
            terminal_tx,
            replay,
            agent_recorder: Arc::new(Mutex::new(None)),
            session_recorder: Arc::new(Mutex::new(None)),
            agent_exec_lock: Arc::new(AsyncMutex::new(())),
        });
        let session_id = Uuid::new_v4().to_string();
        self.sessions
            .write()
            .await
            .insert(session_id.clone(), entry);
        session_id
    }

    /// Terminal activity keepalive: resets server-side idle policies that watch
    /// PTY input, which protocol keepalives don't satisfy. Holds only the
    /// command sender, so the loop ends as soon as the session's read loop
    /// drops its receiver.
    pub(super) fn spawn_terminal_keepalive(connection: &StoredConnection, entry: &SessionEntry) {
        if connection.terminal_keepalive_secs > 0 {
            let terminal_tx = entry.terminal_tx.clone();
            let interval_secs = connection.terminal_keepalive_secs;
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                // The interval's first tick fires immediately; the session
                // just opened, so nothing needs resetting yet.
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    if terminal_tx
                        .send(TerminalCommand::Input(TERMINAL_KEEPALIVE_INPUT.to_vec()))
                        .await
                        .is_err()
                    {
                        // Read loop gone — the session is closed.
                        break;
                    }
                }
            });
        }
    }

    /// Arms the per-prompt OSC 7 hook when the session really runs an
    /// interactive shell (spawn exec / remote_command replace the shell, so
    /// typing the hook line there would feed the command's stdin). Detection
    /// runs off the read loop — probing inside a select! arm can stall the loop
    /// for the detection timeout — and the resolved shell rides the command
    /// queue into the loop, FIFO with the startup-command input lines.
    pub(super) fn arm_prompt_hook(
        spawn_exec: Option<&str>,
        connection: &StoredConnection,
        entry: &SessionEntry,
    ) {
        if interactive_shell_session(spawn_exec, &connection.remote_command) {
            let hook_handle = entry.handle.clone();
            let hook_lease = entry.transport_lease.clone();
            let hook_tx = entry.terminal_tx.clone();
            tokio::spawn(async move {
                let shell = detect_remote_shell(&hook_handle, &hook_lease).await;
                let _ = hook_tx
                    .send(TerminalCommand::InstallPromptHook { shell })
                    .await;
            });
        }
    }

    /// Types the connection's startup command sequence (Tabby "Login scripts"
    /// parity, M7 P0-4) through the same input channel the keepalive uses. Only
    /// for real shell sessions; the event reports just the count and
    /// completion, since command contents can carry secrets.
    pub(super) fn inject_startup_commands(
        &self,
        spawn_exec: Option<&str>,
        connection: &StoredConnection,
        entry: &SessionEntry,
        session_id: &str,
        emitter: &PluginEmitter,
    ) {
        if spawn_exec.is_none() && startup_commands::executes_for(&connection.remote_command) {
            let plan = startup_commands::load_plan(&self.data_dir, &connection.id);
            if !plan.is_empty() {
                let terminal_tx = entry.terminal_tx.clone();
                let startup_emitter = emitter.clone();
                let startup_session_id = session_id.to_owned();
                let startup_count = plan.len();
                tokio::spawn(async move {
                    let completed = startup_commands::inject_sequence(&plan, |payload| {
                        let terminal_tx = terminal_tx.clone();
                        async move {
                            terminal_tx
                                .send(TerminalCommand::Input(payload))
                                .await
                                .is_ok()
                        }
                    })
                    .await;
                    let _ = startup_emitter.event(
                        "ssh/startup",
                        json!({
                            "sessionId": startup_session_id,
                            "count": startup_count,
                            "completed": completed,
                        }),
                    );
                });
            }
        }
    }

    /// Auto-record (M14): attaches a recorder to every new session when the
    /// preference is on. Read-only connections are not exempt (recording is
    /// passive output capture); a recording already in progress is skipped.
    /// Both outcomes are reported once through `ssh/recording/auto`, whose
    /// payload carries only ids, never content.
    pub(super) fn attach_auto_recording(
        &self,
        slot: &Arc<Mutex<Option<session_recording::SessionRecorder>>>,
        connection: &StoredConnection,
        session_id: &str,
        connection_id: &str,
        emitter: &PluginEmitter,
    ) {
        if session_recording::auto_record_enabled() {
            let mut slot = slot.lock().unwrap_or_else(|poison| poison.into_inner());
            if slot.is_some() {
                let _ = emitter.event(
                    "ssh/recording/auto",
                    json!({
                        "sessionId": session_id,
                        "skipped": true,
                    }),
                );
            } else {
                let recording_id = Uuid::new_v4().to_string();
                match session_recording::SessionRecorder::start(
                    &self.data_dir,
                    &recording_id,
                    connection_id,
                    &connection.host,
                    session_id,
                    80,
                    24,
                ) {
                    Ok(recorder) => {
                        *slot = Some(recorder);
                        let _ = emitter.event(
                            "ssh/recording/auto",
                            json!({
                                "sessionId": session_id,
                                "recordingId": recording_id,
                            }),
                        );
                    }
                    Err(error) => {
                        eprintln!("[ssh-trace] auto-record start failed: {error}");
                    }
                }
            }
        }
    }
}
