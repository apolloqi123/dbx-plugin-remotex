//! Task provider streaming layer.

use super::*;
use crate::sessions::ssh::exec::shell_quote as shell_single_quote;
use crate::sessions::ssh::open_session_channel;

pub(super) async fn wait_cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        // Sender dropped = the run was deregistered (finished); treat as a
        // cancel signal so a supervisor never outlives its registry entry.
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

/// Reaps the remote run: drops the channel and asks sshd to terminate the
/// command's process group. A plain channel close does not signal children of
/// non-PTY sessions, so the command (and its sshd session slot) would survive
/// a cancel/timeout. Dropping a russh `Channel` never sends
/// SSH_MSG_CHANNEL_CLOSE either — the explicit close is what makes
/// cancel/timeout a real teardown instead of a status flip.
pub(super) async fn close_channel(channel: &mut russh::Channel<russh::client::Msg>) {
    let _ = channel.signal(russh::Sig::TERM).await;
    let _ = channel.eof().await;
    let _ = channel.close().await;
}

/// How one streamed run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunEnd {
    Completed(Option<i32>),
    Cancelled,
    TimedOut,
}

/// Opens a session channel, runs `script`, streams stdout/stderr lines
/// through `on_log`, and honors cancel plus an optional deadline. On cancel
/// or timeout the channel is closed before returning, so the remote process
/// is reaped by sshd. Runs with `pty` merge stderr into stdout (protocol
/// semantics); `on_tick` fires on the heartbeat cadence and is only passed
/// by resident supervision (its return keeps the loop alive — never cancel).
pub(super) async fn run_streaming(
    handle: &Handle<SshClient>,
    script: &str,
    pty: bool,
    timeout: Option<Duration>,
    cancel: &mut watch::Receiver<bool>,
    mut on_log: impl FnMut(LogStream, String),
    mut on_tick: Option<&mut (dyn FnMut() + Send)>,
) -> Result<RunEnd, String> {
    // Uncancellable dial: a dropped in-flight open leaks the session the
    // server already allocated. Cancel is handled by the exec select below,
    // which closes the channel and reports Cancelled.
    let mut channel = open_session_channel(handle)
        .await
        .map_err(|error| format!("Failed to open exec channel: {error}"))?;
    if pty {
        // Best-effort: PTY-less servers still run the command.
        let _ = channel
            .request_pty(true, "xterm-256color", 24, 80, 0, 0, &[])
            .await;
    }
    tokio::select! {
        biased;
        _ = wait_cancelled(cancel) => {
            close_channel(&mut channel).await;
            return Ok(RunEnd::Cancelled);
        }
        started = channel.exec(true, script.as_bytes()) =>
            started.map_err(|error| format!("Failed to start command: {error}"))?,
    }
    let deadline = timeout.map(|t| tokio::time::Instant::now() + t);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(RESIDENT_HEARTBEAT_SECS));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut stdout = LineBuffer::default();
    let mut stderr = LineBuffer::default();
    let mut exit_code: Option<i32> = None;
    // Macro (not a closure): each expansion's `on_log` borrows are temporary,
    // so the collect loop below can keep using `on_log` between flushes.
    macro_rules! flush_buffers {
        () => {{
            stdout.flush(&mut |line| on_log(LogStream::Stdout, line));
            stderr.flush(&mut |line| on_log(LogStream::Stderr, line));
        }};
    }
    loop {
        let message = tokio::select! {
            biased;
            _ = wait_cancelled(cancel) => {
                flush_buffers!();
                close_channel(&mut channel).await;
                return Ok(RunEnd::Cancelled);
            }
            _ = heartbeat.tick() => {
                if let Some(on_tick) = on_tick.as_deref_mut() {
                    on_tick();
                }
                continue;
            }
            waited = async {
                match deadline {
                    Some(deadline) => tokio::time::timeout_at(deadline, channel.wait())
                        .await
                        .map_err(|_| ()),
                    None => Ok(channel.wait().await),
                }
            } => match waited {
                Ok(message) => message,
                Err(()) => {
                    flush_buffers!();
                    close_channel(&mut channel).await;
                    return Ok(RunEnd::TimedOut);
                }
            },
        };
        match message {
            Some(ChannelMsg::Data { ref data }) => {
                stdout.push(data, |line| on_log(LogStream::Stdout, line));
            }
            Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                stderr.push(data, |line| on_log(LogStream::Stderr, line));
            }
            Some(ChannelMsg::ExitStatus { exit_status }) => {
                exit_code = Some(i32::try_from(exit_status).unwrap_or(-1));
            }
            Some(_) => {}
            None => break,
        }
    }
    flush_buffers!();
    Ok(RunEnd::Completed(exit_code))
}

/// Local-shell counterpart of [`run_streaming`]: spawns `shell args` on the
/// machine running DBX (no PTY — the `pty` field is SSH-protocol specific
/// and ignored here), pipes stdout/stderr through the same line buffering
/// into `on_log`, and honors cancel plus an optional deadline. Cancel and
/// timeout kill the whole process group, not just the shell leader. The
/// heartbeat tick mirrors the SSH path so resident supervision keeps
/// advancing `heartbeatAt` while a quiet process holds the pipes open.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_local_streaming(
    shell: &str,
    args: &[String],
    working_directory: &str,
    environment: &[(String, String)],
    timeout: Option<Duration>,
    cancel: &mut watch::Receiver<bool>,
    mut on_log: impl FnMut(LogStream, String),
    mut on_tick: Option<&mut (dyn FnMut() + Send)>,
) -> Result<RunEnd, String> {
    let mut command = tokio::process::Command::new(shell);
    command
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Backstop: a dropped (abandoned) future must not leak the process.
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    if !working_directory.is_empty() {
        // A missing directory surfaces as the spawn error below with the OS
        // message — the local analogue of the SSH path's `exit 125`.
        command.current_dir(working_directory);
    }
    for (key, value) in environment {
        command.env(key, value);
    }
    if *cancel.borrow_and_update() {
        return Ok(RunEnd::Cancelled);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to spawn local shell {shell:?}: {error}"))?;
    // Pipe readers feed one channel; the single select loop below keeps
    // emission ordered and closure-owned (no shared mutable logger). Each
    // pump owns its reader task; both sides share the log channel.
    let (sender, mut receiver) = mpsc::unbounded_channel::<(LogStream, Vec<u8>)>();
    fn pump_stream<R>(
        pipe: Option<R>,
        stream: LogStream,
        sender: &mpsc::UnboundedSender<(LogStream, Vec<u8>)>,
    ) where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let Some(pipe) = pipe else {
            return;
        };
        let sender = sender.clone();
        let mut reader = tokio::io::BufReader::new(pipe);
        tokio::spawn(async move {
            let mut buffer = Vec::with_capacity(512);
            loop {
                buffer.clear();
                match reader.read_until(b'\n', &mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if sender.send((stream, buffer.clone())).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
    pump_stream(child.stdout.take(), LogStream::Stdout, &sender);
    pump_stream(child.stderr.take(), LogStream::Stderr, &sender);
    drop(sender);
    let deadline = timeout.map(|t| tokio::time::Instant::now() + t);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(RESIDENT_HEARTBEAT_SECS));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut stdout = LineBuffer::default();
    let mut stderr = LineBuffer::default();
    loop {
        let chunk = tokio::select! {
            biased;
            _ = wait_cancelled(cancel) => {
                terminate_local_process(&mut child).await;
                stdout.flush(&mut |line| on_log(LogStream::Stdout, line));
                stderr.flush(&mut |line| on_log(LogStream::Stderr, line));
                return Ok(RunEnd::Cancelled);
            }
            _ = heartbeat.tick() => {
                if let Some(on_tick) = on_tick.as_deref_mut() {
                    on_tick();
                }
                continue;
            }
            waited = async {
                match deadline {
                    Some(deadline) => tokio::time::timeout_at(deadline, receiver.recv())
                        .await
                        .map_err(|_| ()),
                    None => Ok(receiver.recv().await),
                }
            } => match waited {
                Ok(chunk) => chunk,
                Err(()) => {
                    terminate_local_process(&mut child).await;
                    stdout.flush(&mut |line| on_log(LogStream::Stdout, line));
                    stderr.flush(&mut |line| on_log(LogStream::Stderr, line));
                    return Ok(RunEnd::TimedOut);
                }
            },
        };
        let Some((stream, bytes)) = chunk else {
            // Both pipe readers finished: the child closed its streams.
            break;
        };
        match stream {
            LogStream::Stdout => stdout.push(&bytes, |line| on_log(LogStream::Stdout, line)),
            _ => stderr.push(&bytes, |line| on_log(LogStream::Stderr, line)),
        }
    }
    stdout.flush(&mut |line| on_log(LogStream::Stdout, line));
    stderr.flush(&mut |line| on_log(LogStream::Stderr, line));
    let status = tokio::select! {
        biased;
        _ = wait_cancelled(cancel) => {
            terminate_local_process(&mut child).await;
            child.wait().await
        }
        waited = child.wait() => waited,
    };
    let exit_code = status
        .ok()
        .and_then(|status| status.code())
        // Terminated by a signal (or a platform without exit codes): report
        // the SSH-path sentinel for "no exit status".
        .unwrap_or(-1);
    Ok(RunEnd::Completed(Some(exit_code)))
}

// ---------------------------------------------------------------------------
// SSH script-file runs (script_path → remote mktemp round-trip)
// ---------------------------------------------------------------------------

/// Reads the task's local script file. The plugin and the file share one
/// trust domain (the file was picked on this machine), so content is read
/// verbatim; size is capped at [`SCRIPT_UPLOAD_LIMIT`].
pub(super) fn read_script_file(script_path: &str) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(script_path)
        .map_err(|error| format!("script_path {script_path:?} is not readable: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("script_path {script_path:?} is not a regular file"));
    }
    if metadata.len() as usize > SCRIPT_UPLOAD_LIMIT {
        return Err(format!(
            "script_path {script_path:?} exceeds the {} MiB script limit",
            SCRIPT_UPLOAD_LIMIT / (1024 * 1024)
        ));
    }
    std::fs::read(script_path)
        .map_err(|error| format!("Failed to read script_path {script_path:?}: {error}"))
}

/// Uploads script bytes to a fresh remote `mktemp` file (mode 0600 by
/// mktemp semantics) over an SFTP session opened on the task transport's
/// own handle — no workbench session is involved. Returns the temp path.
pub(super) async fn ssh_upload_script(
    handle: &Handle<SshClient>,
    bytes: &[u8],
) -> Result<String, String> {
    let temp_path = exec_plain(handle, "mktemp", Duration::from_secs(30), &[])
        .await
        .map_err(|error| format!("Failed to create remote temp file: {error}"))
        .and_then(|outcome| {
            if outcome.exit_code == 0 {
                let path = outcome.output.trim().to_string();
                if path.is_empty() {
                    Err("Remote mktemp returned an empty path".to_string())
                } else {
                    Ok(path)
                }
            } else {
                Err(format!(
                    "Remote mktemp failed with exit code {}",
                    outcome.exit_code
                ))
            }
        })?;
    let channel = open_session_channel(handle)
        .await
        .map_err(|error| format!("Failed to open SFTP channel: {error}"))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| format!("Failed to start SFTP: {error}"))?;
    let sftp = SftpSession::new_with_config(channel.into_stream(), sftp_client_config(false))
        .await
        .map_err(|error| format!("Failed to initialize SFTP: {error}"))?;
    let upload = async {
        let mut file = sftp
            .create(&temp_path)
            .await
            .map_err(|error| format!("SFTP script upload failed: {error}"))?;
        file.write_all(bytes)
            .await
            .map_err(|error| format!("SFTP script upload failed: {error}"))?;
        file.flush()
            .await
            .map_err(|error| format!("SFTP script upload failed: {error}"))?;
        Ok::<(), String>(())
    }
    .await;
    let _ = sftp.close().await;
    upload?;
    Ok(temp_path)
}

/// Removes an uploaded script temp file. Best-effort by design: it runs on
/// every path after the script run (completed, cancelled, timed out, or
/// failed), and a failed cleanup must never mask the run's real outcome.
pub(super) async fn ssh_remove_script_file(handle: &Handle<SshClient>, temp_path: &str) {
    let command = format!("rm -f {}", shell_single_quote(temp_path));
    let _ = exec_plain(handle, &command, Duration::from_secs(15), &[]).await;
}

/// Whether the remote server has `bash` — the uploaded script runs under
/// bash when available, `sh` otherwise (Alpine-style images).
pub(super) async fn ssh_has_bash(handle: &Handle<SshClient>) -> bool {
    matches!(
        exec_plain(handle, "command -v bash", Duration::from_secs(15), &[]).await,
        Ok(outcome) if outcome.exit_code == 0 && !outcome.output.trim().is_empty()
    )
}
