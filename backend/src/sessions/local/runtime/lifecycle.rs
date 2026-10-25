//! Spawn, pump and reap of one local shell.
//!
//! `LocalTerminalRuntime::start` resolves the shell, opens the PTY, starts the
//! reader/waiter threads and hands ownership to `spawn_pump`, which
//! multiplexes output, input, resize and close commands, drains the PTY tail
//! and publishes the exited state frame before dropping the session.

use super::*;

use crate::core::time::unix_now_secs;

use super::io::{observe_local_recording, publish_local_terminal};

impl LocalTerminalRuntime {
    pub async fn start(
        &self,
        request: LocalTerminalStartRequest,
        emitter: PluginEmitter,
    ) -> Result<Value, String> {
        let cols = request.cols.clamp(2, u16::MAX as u32) as u16;
        let rows = request.rows.clamp(2, u16::MAX as u32) as u16;
        let requested = request
            .shell
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let ds_shell = directory_services_shell();
        let shell_env = std::env::var_os("SHELL").map(|value| value.to_string_lossy().into_owned());
        let spec = pick_shell(
            requested,
            shell_env.as_deref(),
            ds_shell.as_deref(),
            current_platform(),
        );
        let integration = prepare_integration(spec.kind, request.shell_integration.unwrap_or(true));
        // The handshake reply is a cursor position, so it must land inside the
        // viewport: an out-of-range value would inherit a position ConPTY
        // cannot honour, and the shell's first output would be re-serialized
        // mid-banner (the orphan-glyph artifact this reply exists to prevent).
        let cursor = inherit_cursor(request.cursor_row, request.cursor_col, rows, cols);
        // cwd：显式参数须为现存目录，非法值静默回落家目录（前端传的是上次
        // 会话跟踪到的 cwd，目录可能已被删除）。
        let cwd = request
            .cwd
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .or_else(home_dir);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Failed to open a pseudo terminal: {error}"))?;

        let mut command = CommandBuilder::new(&spec.program);
        // Integration args replace the base login args wholesale (the bash
        // wrapper reproduces the login chain itself — bash cannot combine
        // `-l` with `--rcfile`). On Windows the UTF-8 console switch folds
        // into the same argv afterwards (issue #173 Windows side).
        let argv = apply_windows_utf8_console(current_platform(), spec.kind, integration.args);
        command.args(&argv);
        command.env("TERM", "xterm-256color");
        // 24-bit color for programs that probe COLORTERM (VS Code sets the
        // same), plus a detectable marker akin to TERM_PROGRAM=vscode.
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "dbx");
        // UTF-8 locale fallback for a shell that inherited none (issue #173):
        // a GUI-launched sidecar carries no LANG, so the shell line-edited in
        // the C locale and CJK garbled. Applied beneath the user env below —
        // an explicitly configured LANG/LC_* always wins.
        let user_env_pairs = sanitize_user_env(&request.env);
        if let Some(lang) = fallback_lang(&user_env_pairs, current_platform(), &|key| {
            std::env::var(key).ok()
        }) {
            command.env("LANG", lang);
        }
        // User env sits beneath integration env: injection owns its mechanics
        // (ZDOTDIR…) and the managed keys above are filtered out entirely, so
        // the shell's rendering contract holds no matter what was sent.
        for (key, value) in user_env_pairs {
            command.env(key, value);
        }
        for (key, value) in &integration.env {
            command.env(key, value);
        }
        if let Some(home) = cwd {
            command.cwd(home);
        }

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("Failed to start {}: {error}", spec.program))?;
        // The sidecar's copy of the slave must die immediately: with it
        // alive the master never sees EOF once the shell exits.
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| format!("Failed to read the pseudo terminal: {error}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| format!("Failed to write the pseudo terminal: {error}"))?;
        let killer = child.clone_killer();

        let session_id = uuid::Uuid::new_v4().to_string();
        let replay = Arc::new(AsyncMutex::new(ReplayBuffer::default()));
        let recorder: Arc<Mutex<Option<session_recording::SessionRecorder>>> =
            Arc::new(Mutex::new(None));
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        self.sessions.write().await.insert(
            session_id.clone(),
            Arc::new(LocalSession {
                workbench_id: request.workbench_id.clone(),
                shell: spec.program.clone(),
                created_at_secs: unix_now_secs(),
                terminal_tx: cmd_tx,
                replay: replay.clone(),
                recorder: recorder.clone(),
                cols,
                rows,
            }),
        );
        spawn_pump(
            session_id.clone(),
            request.workbench_id,
            rows,
            cols,
            cursor,
            pair.master,
            reader,
            writer,
            killer,
            child,
            cmd_rx,
            replay,
            recorder,
            emitter,
            self.sessions.clone(),
        );
        Ok(json!({
            "sessionId": session_id,
            "shell": spec.program,
            "shellIntegration": integration.injected,
        }))
    }
}

/// Reader → pump output queue depth. Each slot is one PTY read (8 KiB), so
/// the queue is bounded at ~2 MiB per session, matching the replay-buffer
/// budget. When the publisher falls behind the reader drops the newest chunk
/// (never blocks, never grows) and logs a rate-limited byte count.
const LOCAL_READER_OUTPUT_QUEUE: usize = 256;

/// Owns the PTY master for one session and multiplexes PTY output, workbench
/// commands and the child's exit status. Terminal condition is the child's
/// exit (or the bounded grace after a kill/EOF); the teardown publishes the
/// State frame, notifies the workbench and drops the session.
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_pump(
    session_id: String,
    workbench_id: String,
    pty_rows: u16,
    pty_cols: u16,
    // The terminal's cursor at spawn time — the value ConPTY's inherit-cursor
    // handshake must be answered with (see [`LocalTerminalStartRequest`]).
    cursor: (u16, u16),
    master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    mut killer: Box<dyn ChildKiller + Send + Sync>,
    child: Box<dyn Child + Send + Sync>,
    mut cmd_rx: mpsc::Receiver<LocalTerminalCommand>,
    replay: Arc<AsyncMutex<ReplayBuffer>>,
    recorder: Arc<Mutex<Option<session_recording::SessionRecorder>>>,
    emitter: PluginEmitter,
    sessions: Arc<RwLock<HashMap<String, Arc<LocalSession>>>>,
) {
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(LOCAL_READER_OUTPUT_QUEUE);
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buffer = [0u8; 8192];
        let mut dropped_bytes = 0_u64;
        let mut last_drop_log = std::time::Instant::now();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => match out_tx.try_send(buffer[..n].to_vec()) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(data)) => {
                        // Drop-newest overflow policy: the pump is the only
                        // producer and cannot lag behind a bounded queue
                        // without this counter growing, so memory stays flat.
                        dropped_bytes = dropped_bytes.saturating_add(data.len() as u64);
                        if last_drop_log.elapsed() >= Duration::from_secs(5) {
                            eprintln!(
                                "[dbx-plugin-sshx] local terminal output backlog; dropped {dropped_bytes} bytes (newest chunks)"
                            );
                            last_drop_log = std::time::Instant::now();
                        }
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                },
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        if dropped_bytes > 0 {
            eprintln!(
                "[dbx-plugin-sshx] local terminal output ended; dropped {dropped_bytes} bytes in total"
            );
        }
    });

    let (exit_tx, mut exit_rx) = mpsc::unbounded_channel::<Option<u32>>();
    std::thread::spawn(move || {
        let mut child = child;
        let status = child.wait();
        let _ = exit_tx.send(status.ok().map(|status| status.exit_code()));
    });

    let writer = Arc::new(Mutex::new(writer));
    tokio::spawn(async move {
        let mut master = Some(master);
        // Tracked so same-size client resizes can be dropped before ConPTY
        // (requested_pty_size) and so the CPR handshake answers with the live
        // viewport, not the spawn-time one.
        let mut pty_rows = pty_rows;
        let mut pty_cols = pty_cols;
        let mut closing = false;
        let mut exit_code: Option<Option<u32>> = None;
        let mut handshake_filter =
            ConptyHandshakeFilter::new(current_platform() == Platform::Windows);
        // Startup-window resize gate (Windows): host-driven geometry changes
        // racing the shell's opening output are merged here and flushed once
        // output stays quiet — see [`StartupResizeGate`]. The flush timer is
        // pinned far-future while nothing is pending and re-armed from
        // `next_wake` at the top of every loop turn.
        let mut resize_gate = StartupResizeGate::new(current_platform());
        let session_started_at = tokio::time::Instant::now();
        let mut resize_flush_timer = Box::pin(tokio::time::sleep_until(idle_flush_deadline()));
        loop {
            if closing {
                // Graceful-first teardown: with the master dropped the slave
                // sees EOF and the shell exits cleanly — zsh/bash flush their
                // command history only on a clean exit, an immediate SIGKILL
                // would silently drop it. Only a child that outlives the
                // grace window (hung job, `no hup` shell) gets the killer.
                match tokio::time::timeout(LOCAL_CLOSE_GRACE, exit_rx.recv()).await {
                    Ok(code) => {
                        exit_code = code;
                    }
                    Err(_) => {
                        let _ = killer.kill();
                        if let Ok(code) =
                            tokio::time::timeout(LOCAL_KILL_GRACE, exit_rx.recv()).await
                        {
                            exit_code = code;
                        }
                    }
                }
                break;
            }
            resize_flush_timer.as_mut().reset(
                resize_gate
                    .next_wake()
                    .map_or_else(idle_flush_deadline, |elapsed| session_started_at + elapsed),
            );
            tokio::select! {
                chunk = out_rx.recv() => match chunk {
                    Some(data) => {
                        resize_gate.on_output(session_started_at.elapsed());
                        let (forward, handshake) = handshake_filter.filter(&data);
                        if handshake {
                            let reply = conpty_cpr_reply(cursor.0, cursor.1);
                            let mut writer = writer.lock().unwrap_or_else(|poison| poison.into_inner());
                            let _ = writer.write_all(&reply).and_then(|_| writer.flush());
                        }
                        if !forward.is_empty() {
                            observe_local_recording(&recorder, &forward);
                            publish_local_terminal(&session_id, TerminalStream::Stdout, forward, &replay, &emitter).await;
                        }
                    }
                    None => closing = true,
                },
                command = cmd_rx.recv() => match command {
                    Some(LocalTerminalCommand::Input(data)) => {
                        let handle = writer.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            let mut writer =
                                handle.lock().unwrap_or_else(|poison| poison.into_inner());
                            writer.write_all(&data).and_then(|_| writer.flush())
                        })
                        .await
                        .unwrap_or_else(|_| {
                            Err(std::io::Error::other("local terminal write task panicked"))
                        });
                        if result.is_err() {
                            closing = true;
                        }
                    }
                    Some(LocalTerminalCommand::Resize { cols, rows }) => {
                        if let Some(size) = requested_pty_size(pty_rows, pty_cols, rows, cols) {
                            match resize_gate.resize(size, session_started_at.elapsed()) {
                                StartupResizeDecision::Apply => {
                                    // Track the size only after a successful resize:
                                    // a failed call would poison the same-size dedup
                                    // and the ConPTY CPR reply, which must answer
                                    // with the live viewport.
                                    match apply_pty_resize(master.as_deref(), size) {
                                        Some((rows, cols)) => {
                                            pty_rows = rows;
                                            pty_cols = cols;
                                        }
                                        None => eprintln!(
                                            "[dbx-plugin-sshx] local terminal resize to {}x{} failed",
                                            size.cols, size.rows
                                        ),
                                    }
                                }
                                // Deferred: the gate keeps the merged geometry
                                // and the flush timer (re-armed from next_wake at
                                // the loop top) applies it once the startup
                                // output has been quiet.
                                StartupResizeDecision::Defer => {}
                            }
                        }
                    }
                    Some(LocalTerminalCommand::Close) | None => {
                        // Drop the master BEFORE the grace window: the slave
                        // side sees EOF and interactive shells exit (and flush
                        // history) on their own; killer only after the grace.
                        drop(master.take());
                        closing = true;
                        // Late/blocked input senders must fail fast instead of
                        // piling up behind the close grace window.
                        cmd_rx.close();
                    }
                },
                _ = &mut resize_flush_timer => {
                    if let Some(size) = resize_gate.poll(session_started_at.elapsed()) {
                        if let Some((rows, cols)) = apply_pty_resize(master.as_deref(), size) {
                            pty_rows = rows;
                            pty_cols = cols;
                        }
                    }
                },
                code = exit_rx.recv() => {
                    exit_code = code;
                    break;
                }
            }
        }
        // The master may already be taken by a graceful Close; dropping the
        // remaining value here unblocks the reader thread even when a
        // background child still holds the slave.
        drop(master);
        // The exit signal races the reader thread: the child can be reaped
        // while the PTY's last bytes (final command output, logout message)
        // are still in flight, so the channel must be drained before the
        // exited frame — dropping it here would silently lose the tail the
        // user is waiting to read.
        while let Ok(Some(data)) = tokio::time::timeout(LOCAL_EXIT_DRAIN, out_rx.recv()).await {
            observe_local_recording(&recorder, &data);
            publish_local_terminal(&session_id, TerminalStream::Stdout, data, &replay, &emitter)
                .await;
        }
        // A handshake prefix held back at the very end of the stream is the
        // terminal's last visible bytes — never drop them.
        let residue = handshake_filter.flush();
        if !residue.is_empty() {
            observe_local_recording(&recorder, &residue);
            publish_local_terminal(
                &session_id,
                TerminalStream::Stdout,
                residue,
                &replay,
                &emitter,
            )
            .await;
        }
        publish_local_terminal(
            &session_id,
            TerminalStream::State,
            b"local-terminal-exited".to_vec(),
            &replay,
            &emitter,
        )
        .await;
        let _ = emitter.event(
            "local/session/state",
            json!({
                "sessionId": session_id,
                "workbenchId": workbench_id,
                "state": "exited",
                "exitCode": exit_code.flatten(),
            }),
        );
        sessions.write().await.remove(&session_id);
    });
}
