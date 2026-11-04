//! Exec run layer.

use super::*;

/// A never-firing cancel receiver for call sites that have no cancellation
/// surface (metrics collectors, docker listings, the keepalive probe): they
/// keep the plain signatures while the internals stay cancellation-aware.
pub fn never_cancels() -> watch::Receiver<bool> {
    watch::channel(false).1
}

/// Resolves once a cancellation is requested. A dropped sender (the exec
/// registry entry was removed because the task already finished) counts as
/// "never" so the select arms fall through to normal completion.
pub(super) async fn wait_cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            // Sender dropped = nobody can cancel anymore: the future must
            // stay PENDING forever. Completing here would make the biased
            // select arms take the cancel branch and instantly fail the exec
            // — `never_cancels()` drops its sender immediately, so every
            // plain/internal exec (metrics, processes, docker, completion)
            // would return "cancelled" without ever dialing the remote.
            std::future::pending::<()>().await;
        }
    }
}

/// Result of a remote command execution.
pub struct ExecOutcome {
    pub output: String,
    pub exit_code: i32,
}

/// One CHANNEL_REQUEST "env" entry with its failure policy. Built-in
/// defaults stay best-effort: default sshd configs only `AcceptEnv LANG`
/// and `LC_*`, so e.g. the internal `SUDO_ASKPASS` clear is commonly
/// refused and the caller must not care. Client-specified `setEnv` entries
/// are strict — a silently dropped variable changes what the remote command
/// ends up seeing.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ChannelEnv {
    pub(super) key: String,
    pub(super) value: String,
    pub(super) strict: bool,
}

/// Merges built-in channel env defaults with the connection's client
/// `setEnv` entries: user entries win on duplicate keys and every variable
/// ends up requested exactly once — dedup is decided locally here instead
/// of relying on server-side ordering of duplicate env requests.
pub(super) fn merge_channel_env(
    defaults: &[(&str, &str)],
    user_env: &[(String, String)],
) -> Vec<ChannelEnv> {
    let mut merged: Vec<ChannelEnv> = defaults
        .iter()
        .map(|(key, value)| ChannelEnv {
            key: (*key).to_string(),
            value: (*value).to_string(),
            strict: false,
        })
        .collect();
    for (key, value) in user_env {
        match merged.iter_mut().find(|entry| entry.key == *key) {
            Some(slot) => {
                slot.value = value.clone();
                slot.strict = true;
            }
            None => merged.push(ChannelEnv {
                key: key.clone(),
                value: value.clone(),
                strict: true,
            }),
        }
    }
    merged
}

/// Requests the merged environment on a session channel before its
/// shell/exec request. Strict (user-configured) entries fail the whole
/// channel setup with the variable named; best-effort defaults keep the
/// historical swallow-and-continue behavior. russh's `set_env` is
/// fire-and-forget, so "failure" here means a broken channel/transport —
/// a server that drops a variable for lack of `AcceptEnv` stays silent by
/// protocol design, exactly like the ssh(1) client.
pub(super) async fn apply_channel_env(
    channel: &mut russh::Channel<russh::client::Msg>,
    env: &[ChannelEnv],
) -> Result<(), String> {
    for entry in env {
        if let Err(error) = channel.set_env(true, &entry.key, &entry.value).await {
            if entry.strict {
                return Err(format!(
                    "Failed to set remote environment variable '{}': {error}",
                    entry.key
                ));
            }
        }
    }
    Ok(())
}

/// Requests a connection's client-specified `setEnv` entries on a bare
/// channel (interactive session path; no built-in defaults to merge).
/// Semantics are the client-specified half of ssh's SetEnv/SendEnv: only
/// entries from this connection's config are transmitted, never the local
/// process environment.
pub(crate) async fn apply_connection_env(
    channel: &mut russh::Channel<russh::client::Msg>,
    set_env: &[(String, String)],
) -> Result<(), String> {
    apply_channel_env(channel, &merge_channel_env(&[], set_env)).await
}

/// Runs a command without privilege escalation on a new channel.
pub async fn exec_plain(
    handle: &Handle<SshClient>,
    command: &str,
    timeout: Duration,
    set_env: &[(String, String)],
) -> Result<ExecOutcome, String> {
    exec_plain_cancellable(handle, command, timeout, set_env, &mut never_cancels()).await
}

/// [`exec_plain`] with a cancellation surface: the signal is honored at the
/// next await point (channel open, env/exec start, output collection), and a
/// cancelled run always closes the channel through the regular error path —
/// dropping a russh `Channel` never sends SSH_MSG_CHANNEL_CLOSE, so the
/// remote process would survive the task (a mid-authentication `sudo` keeps
/// the timestamp lock and deadlocks every later sudo on the connection).
pub async fn exec_plain_cancellable(
    handle: &Handle<SshClient>,
    command: &str,
    timeout: Duration,
    set_env: &[(String, String)],
    cancel: &mut watch::Receiver<bool>,
) -> Result<ExecOutcome, String> {
    // The dial is deliberately uncancellable: dropping a channel-open future
    // after sshd allocated the channel leaks that session (it is never closed)
    // and burns one `MaxSessions` slot for the connection. Cancellation is
    // honoured immediately after the open, through the abort path below.
    let mut channel = open_session_channel_labeled(handle, "exec")
        .await
        .map_err(|error| format!("Failed to open exec channel: {error}"))?;
    // env/exec start: the command may have been accepted by the time the
    // cancel lands, so this arm closes the channel before returning.
    tokio::select! {
        biased;
        _ = wait_cancelled(cancel) => {
            return Err(
                abort_exec_channel(&mut channel, EXEC_CANCELLED_MESSAGE.to_string()).await
            );
        }
        started = async {
            apply_connection_env(&mut channel, set_env).await?;
            channel
                .exec(true, command.as_bytes())
                .await
                .map_err(|error| format!("Failed to start command: {error}"))
        } => {
            // A refused env/exec request leaves the channel open; close it
            // instead of dropping it (drop never sends CHANNEL_CLOSE).
            if let Err(error) = started {
                return Err(abort_exec_channel(&mut channel, error).await);
            }
        }
    }
    match run_to_completion(&mut channel, Some(timeout), None, cancel, None).await {
        Ok(outcome) => Ok(outcome),
        Err(error) => Err(abort_exec_channel(&mut channel, error).await),
    }
}

/// Runs a command with Quick Sudo: pipes the password to `sudo -S` stdin and,
/// while the command is still authenticating, watches the prompt stream for
/// 2FA/TOTP follow-up prompts and answers them automatically. When no
/// password is configured it falls back to non-interactive `sudo -n`.
pub async fn exec_with_sudo(
    handle: &Handle<SshClient>,
    auth: &SudoAuth,
    command: &str,
    timeout: Duration,
    use_pty: bool,
    set_env: &[(String, String)],
) -> Result<ExecOutcome, String> {
    exec_with_sudo_cancellable(
        handle,
        auth,
        command,
        timeout,
        use_pty,
        set_env,
        &mut never_cancels(),
    )
    .await
}

/// [`exec_with_sudo`] with a cancellation surface (same semantics as
/// [`exec_plain_cancellable`]: cancel lands at the next await point and the
/// channel is always closed, so an interrupted `sudo -S` releases the
/// timestamp lock).
pub async fn exec_with_sudo_cancellable(
    handle: &Handle<SshClient>,
    auth: &SudoAuth,
    command: &str,
    timeout: Duration,
    use_pty: bool,
    set_env: &[(String, String)],
    cancel: &mut watch::Receiver<bool>,
) -> Result<ExecOutcome, String> {
    super::streaming::exec_with_sudo_inner(
        handle,
        auth,
        command,
        use_pty,
        set_env,
        Some(timeout),
        cancel,
        None,
        false,
    )
    .await
}

/// Closes a channel whose setup failed before a command was handed over
/// (refused PTY/env/exec/subsystem request). The channel is already open on
/// the server, so returning without a close would keep one session allocated
/// against `MaxSessions` for the life of the connection.
pub(crate) async fn close_setup_channel(channel: &mut russh::Channel<russh::client::Msg>) {
    let _ = channel.eof().await;
    let _ = channel.close().await;
    channel_ledger::closed(channel.id().number());
}

/// Aborts an exec channel after a failed/aborted remote run. Dropping a
/// russh `Channel` does NOT send SSH_MSG_CHANNEL_CLOSE, so the remote
/// process would keep waiting on stdin forever - a mid-authentication
/// `sudo` holds the sudo timestamp lock and deadlocks every later sudo on
/// the connection. Close the channel so sshd reaps the process.
///
/// Closing the channel alone is not enough for non-PTY sessions: sshd only
/// signals PTY child process groups, so a blocked `docker logs -f` would
/// survive the close and keep its session allocated (one `MaxSessions` slot
/// per cancelled stream). The SSH `signal` request kills the command's
/// process group first, which is also the part that reaps it on servers that
/// do not deliver EOF promptly.
pub(super) async fn abort_exec_channel(
    channel: &mut russh::Channel<russh::client::Msg>,
    error: String,
) -> String {
    signal_and_close(channel, error, Duration::ZERO).await
}

/// Stream variant of [`abort_exec_channel`]. The remote `docker logs -f`
/// supervisor cleans up its watcher children from a HUP/TERM trap, and a KILL
/// cannot be caught: sending HUP and KILL back-to-back lets the KILL land
/// before the trap runs, orphaning the watchdog for its full TTL (observed as
/// a stray supervisor/`sleep` process in `ps` after a container switch). A
/// short settle window between the signals gives the trap a chance to kill
/// the whole process group before the uncatchable backstop.
pub(super) async fn abort_stream_channel(
    channel: &mut russh::Channel<russh::client::Msg>,
    error: String,
) -> String {
    signal_and_close(channel, error, STREAM_SIGNAL_SETTLE).await
}

/// Settle window between HUP and TERM/KILL on stream channels.
pub(super) const STREAM_SIGNAL_SETTLE: Duration = Duration::from_millis(100);

async fn signal_and_close(
    channel: &mut russh::Channel<russh::client::Msg>,
    error: String,
    settle: Duration,
) -> String {
    // HUP first: interactive shells (notably bash inside a container behind
    // `docker exec`) ignore SIGTERM, so TERM alone can leave the remote client
    // alive and its sshd session/channel allocated. KILL is the backstop for
    // wrappers that install their own TERM/HUP handlers.
    let _ = channel.signal(russh::Sig::HUP).await;
    if !settle.is_zero() {
        tokio::time::sleep(settle).await;
    }
    let _ = channel.signal(russh::Sig::TERM).await;
    if !settle.is_zero() {
        tokio::time::sleep(settle).await;
    }
    let _ = channel.signal(russh::Sig::KILL).await;
    let _ = channel.eof().await;
    let _ = channel.close().await;
    channel_ledger::closed(channel.id().number());
    error
}

/// True when the failure happened before the remote command could have
/// started (dead pooled transport, refused channel open): retrying the tool
/// call cannot double-execute the command. Anything after `channel.exec()`
/// was accepted is NOT retry-safe (the command may have run server-side), so
/// those errors stay terminal and the caller decides whether rerunning is
/// safe.
pub fn is_pre_exec_transport_error(error: &str) -> bool {
    const PRE_EXEC_MARKERS: [&str; 4] = [
        "Failed to open exec channel",
        "Failed to open sudo channel",
        "Failed to start command",
        "Failed to start sudo command",
    ];
    PRE_EXEC_MARKERS.iter().any(|marker| error.contains(marker))
}

/// `(auth, use_pty, otp_piped)` — the third flag records that the OTP code
/// was already piped to stdin in phase 1, so the watcher must treat the
/// prompt that consumed it as answered instead of burning the next
/// secret's code on the same prompt.
pub(super) type PromptContext<'a> = (&'a SudoAuth, bool, bool);

pub(super) fn append_capped(buffer: &mut Vec<u8>, data: &[u8]) {
    if buffer.len() >= MAX_EXEC_OUTPUT_BYTES {
        return;
    }
    let remaining = MAX_EXEC_OUTPUT_BYTES - buffer.len();
    let take = data.len().min(remaining);
    buffer.extend_from_slice(&data[..take]);
}

/// Per-stream retained tail while a sink is streaming: every fragment has
/// already been forwarded downstream, so only the most recent bytes are kept
/// for the non-zero-exit / error report. 32 KiB per stream (64 KiB total for
/// stdout + stderr) instead of [`MAX_EXEC_OUTPUT_BYTES`] per stream, which
/// would let four concurrent log follows retain ~128 MiB of already-delivered
/// output.
pub(super) const MAX_STREAM_TAIL_BYTES: usize = 32 * 1024;

/// Appends `data` to a rolling tail of at most [`MAX_STREAM_TAIL_BYTES`],
/// dropping the oldest bytes first so error text at the end of the stream is
/// what survives.
pub(super) fn append_tail_capped(buffer: &mut Vec<u8>, data: &[u8]) {
    if data.len() >= MAX_STREAM_TAIL_BYTES {
        buffer.clear();
        buffer.extend_from_slice(&data[data.len() - MAX_STREAM_TAIL_BYTES..]);
        return;
    }
    let excess = (buffer.len() + data.len()).saturating_sub(MAX_STREAM_TAIL_BYTES);
    if excess > 0 {
        buffer.drain(..excess);
    }
    buffer.extend_from_slice(data);
}

/// Retains an output fragment for the exit/error report: a rolling tail while
/// streaming through a sink, the historical 16 MiB head cap otherwise.
fn retain_output(buffer: &mut Vec<u8>, data: &[u8], streaming: bool) {
    if streaming {
        append_tail_capped(buffer, data);
    } else {
        append_capped(buffer, data);
    }
}

/// Message for a streaming sink whose receiver was dropped. Deliberately
/// transport-neutral: the Docker layer adds its own context.
pub(super) const STREAM_SINK_CLOSED_MESSAGE: &str = "Command output stream receiver closed";

/// Forwards one output fragment to the streaming sink, racing the (possibly
/// back-pressured) bounded send against cancellation. Cancel wins so a
/// stalled consumer cannot delay teardown of the remote channel.
pub(super) async fn send_or_cancel(
    sink: &tokio::sync::mpsc::Sender<Vec<u8>>,
    data: &[u8],
    cancel: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    tokio::select! {
        biased;
        _ = wait_cancelled(cancel) => Err(EXEC_CANCELLED_MESSAGE.to_string()),
        sent = sink.send(data.to_vec()) =>
            sent.map_err(|_| STREAM_SINK_CLOSED_MESSAGE.to_string()),
    }
}

pub(super) async fn run_to_completion(
    channel: &mut russh::Channel<russh::client::Msg>,
    timeout: Option<Duration>,
    prompt_context: Option<PromptContext<'_>>,
    cancel: &mut watch::Receiver<bool>,
    sink: Option<&tokio::sync::mpsc::Sender<Vec<u8>>>,
) -> Result<ExecOutcome, String> {
    let deadline = timeout.map(|timeout| tokio::time::Instant::now() + timeout);
    let streaming = sink.is_some();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code: Option<i32> = None;
    let mut closed = false;
    let mut auth_rounds = 0_u32;
    let mut password_answered = prompt_context.is_some();
    let mut otp_answered = prompt_context
        .as_ref()
        .map(|(_, _, otp_piped)| *otp_piped)
        .unwrap_or(false);

    while !closed {
        let wait_message = async {
            match deadline {
                Some(deadline) => tokio::time::timeout_at(deadline, channel.wait())
                    .await
                    .map_err(|_| SUDO_WAIT_TIMEOUT_MESSAGE.to_string()),
                None => Ok(channel.wait().await),
            }
        };
        let message = tokio::select! {
            biased;
            // Cancel lands here even mid-collection: returning Err routes
            // through the caller's abort_exec_channel, which closes the
            // channel so sshd reaps the remote process.
            _ = wait_cancelled(cancel) => return Err(EXEC_CANCELLED_MESSAGE.to_string()),
            waited = wait_message => waited?,
        };
        let message = match message {
            Some(message) => message,
            None => break,
        };
        match message {
            ChannelMsg::Data { ref data } => {
                retain_output(&mut stdout, data, streaming);
                if let Some((auth, use_pty, _)) = prompt_context.as_ref() {
                    if *use_pty {
                        if let Some(error) = maybe_answer_prompt(
                            channel,
                            auth,
                            &String::from_utf8_lossy(data),
                            &mut auth_rounds,
                            &mut password_answered,
                            &mut otp_answered,
                        )
                        .await
                        {
                            return Err(error);
                        }
                    }
                }
                if let Some(sink) = sink {
                    send_or_cancel(sink, data, cancel).await?;
                }
            }
            ChannelMsg::ExtendedData { ref data, .. } => {
                retain_output(&mut stderr, data, streaming);
                if let Some((auth, _, _)) = prompt_context.as_ref() {
                    if let Some(error) = maybe_answer_prompt(
                        channel,
                        auth,
                        &String::from_utf8_lossy(data),
                        &mut auth_rounds,
                        &mut password_answered,
                        &mut otp_answered,
                    )
                    .await
                    {
                        return Err(error);
                    }
                }
                if let Some(sink) = sink {
                    send_or_cancel(sink, data, cancel).await?;
                }
            }
            ChannelMsg::ExitStatus { exit_status } => {
                exit_code = Some(exit_status as i32);
            }
            // EOF only means the peer closed its output stream; servers send
            // exit-status after it and Close last (RFC 4254). Treating EOF as
            // terminal dropped the exit code and turned failures into
            // `unwrap_or(0)` successes. The outer deadline still bounds a
            // server that never closes.
            ChannelMsg::Close => {
                closed = true;
            }
            _ => {}
        }
    }

    // Normal completion (peer Close / EOF + Close): release the ledger entry.
    channel_ledger::closed(channel.id().number());
    let mut output = String::from_utf8_lossy(&stdout)
        .trim_end_matches('\n')
        .to_string();
    let stderr_text = String::from_utf8_lossy(&stderr).trim().to_string();
    if output.is_empty() {
        if !stderr_text.is_empty() {
            output = stderr_text;
        }
    } else if !stderr_text.is_empty() && exit_code.unwrap_or(0) != 0 {
        output.push('\n');
        output.push_str(&stderr_text);
    }
    Ok(ExecOutcome {
        output,
        exit_code: exit_code.unwrap_or(0),
    })
}

/// Detects auth prompts in a freshly received chunk and answers them on the
/// channel stdin. Returns `Err` when sudo reports an authentication failure.
pub(super) async fn maybe_answer_prompt(
    channel: &mut russh::Channel<russh::client::Msg>,
    auth: &SudoAuth,
    chunk: &str,
    auth_rounds: &mut u32,
    password_answered: &mut bool,
    otp_answered: &mut bool,
) -> Option<String> {
    let normalized = normalize_auth_prompt_text(chunk).to_lowercase();
    if normalized.is_empty() {
        return None;
    }
    if AUTH_FAILURE_MARKERS
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return Some(format!("sudo authentication failed: {normalized}"));
    }
    let mode = auth.flow_mode();
    let kind = auth.classify(&normalized)?;
    if *auth_rounds >= MAX_AUTH_ROUNDS {
        return None;
    }
    if matches!(kind, PromptKind::Totp | PromptKind::Combined) && *otp_answered {
        return None;
    }
    if !can_respond_to_prompt(mode, kind, *password_answered) {
        return None;
    }
    let answer = auth.answer_for(kind)?;
    let mut payload = answer.into_bytes();
    payload.push(b'\n');
    if let Err(error) = channel.data(&payload[..]).await {
        return Some(format!("Failed to write auth answer: {error}"));
    }
    match kind {
        PromptKind::Password => *password_answered = true,
        PromptKind::Totp => *otp_answered = true,
        PromptKind::Combined => {
            *password_answered = true;
            // OTP 模式下的合并应答携带了验证码：标记已用，避免后续提示
            // 在同一窗口里再烧一码。
            if mode.answers_merged_prompt_with_otp() && auth.totp_configured() {
                *otp_answered = true;
            }
        }
    }
    *auth_rounds += 1;
    None
}

/// Validates a cached sudo timestamp without prompting; used by the
/// background keepalive loop.
pub async fn validate_sudo_timestamp(handle: &Handle<SshClient>) -> Result<(), String> {
    let mut channel = open_session_channel_labeled(handle, "sudo/keepalive")
        .await
        .map_err(|error| format!("Failed to open sudo keepalive channel: {error}"))?;
    if let Err(error) = channel.exec(true, b"sudo -nv").await {
        return Err(abort_exec_channel(
            &mut channel,
            format!("Failed to start sudo keepalive: {error}"),
        )
        .await);
    }
    let outcome = match run_to_completion(
        &mut channel,
        Some(Duration::from_secs(15)),
        None,
        &mut never_cancels(),
        None,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => return Err(abort_exec_channel(&mut channel, error).await),
    };
    if outcome.exit_code == 0 {
        Ok(())
    } else {
        Err("sudo timestamp expired".to_string())
    }
}
