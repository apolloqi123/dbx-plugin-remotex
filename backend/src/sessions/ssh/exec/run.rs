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
    // Dial: no exec issued yet, so a plain error return leaks nothing.
    let mut channel = tokio::select! {
        biased;
        _ = wait_cancelled(cancel) => return Err(EXEC_CANCELLED_MESSAGE.to_string()),
        opened = handle.channel_open_session() =>
            opened.map_err(|error| format!("Failed to open exec channel: {error}"))?,
    };
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
        } => started?,
    }
    match run_to_completion(&mut channel, timeout, None, cancel).await {
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
    if auth.password.is_empty() {
        let command_line = format!("sudo -n {}", sanitize_sudo_command(command));
        let mut channel = tokio::select! {
            biased;
            _ = wait_cancelled(cancel) => return Err(EXEC_CANCELLED_MESSAGE.to_string()),
            opened = handle.channel_open_session() =>
                opened.map_err(|error| format!("Failed to open sudo channel: {error}"))?,
        };
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
                    .exec(true, command_line.as_bytes())
                    .await
                    .map_err(|error| format!("Failed to start sudo command: {error}"))
            } => started?,
        }
        let outcome = match run_to_completion(&mut channel, timeout, None, cancel).await {
            Ok(outcome) => outcome,
            Err(error) => return Err(abort_exec_channel(&mut channel, error).await),
        };
        if outcome.exit_code != 0 {
            return Err(format!(
                "sudo exited {}: {} (no password configured - run sudo in the terminal first or configure Quick Sudo)",
                outcome.exit_code, outcome.output
            ));
        }
        return Ok(outcome);
    }

    let command_line = format!("sudo -S -p '' {}", sanitize_sudo_command(command));
    let mut channel = tokio::select! {
        biased;
        _ = wait_cancelled(cancel) => return Err(EXEC_CANCELLED_MESSAGE.to_string()),
        opened = handle.channel_open_session() =>
            opened.map_err(|error| format!("Failed to open sudo channel: {error}"))?,
    };
    if use_pty {
        // Some PAM stacks only prompt correctly with a TTY. With a PTY the
        // prompt arrives on the merged stdout stream instead of stderr.
        let _ = channel
            .request_pty(true, "xterm-256color", 24, 80, 0, 0, &[])
            .await;
    }
    // Client setEnv entries are merged with (and win over) the internal
    // SUDO_ASKPASS clear: each variable is requested exactly once with the
    // user value taking precedence, so the two env sources cannot fight
    // over the same key via server-side ordering of duplicate requests.
    let env = merge_channel_env(&[("SUDO_ASKPASS", "")], set_env);
    tokio::select! {
        biased;
        _ = wait_cancelled(cancel) => {
            return Err(
                abort_exec_channel(&mut channel, EXEC_CANCELLED_MESSAGE.to_string()).await
            );
        }
        started = async {
            apply_channel_env(&mut channel, &env).await?;
            channel
                .exec(true, command_line.as_bytes())
                .await
                .map_err(|error| format!("Failed to start sudo command: {error}"))
        } => started?,
    }

    // Phase 1: hand sudo the password, with the OTP code (when configured)
    // queued right behind it - `sudo -S` and its PAM stack read the factors
    // sequentially from stdin. The exec channel stays TTY-less: with a PTY
    // each factor read flushes typed-ahead input (termios TCSAFLUSH in the
    // password readers), so automated writes race an unknowable per-host
    // timing; a plain pipe queues reliably no matter when the reads happen.
    let mut payload = format!("{}\n", auth.password).into_bytes();
    let mut otp_piped = false;
    if let Some(code) = auth.totp_answer_logged() {
        // Already-committed codes inside their replay window are skipped by
        // the watcher path below instead.
        let mut otp_line = code.into_bytes();
        otp_line.push(b'\n');
        payload.extend_from_slice(&otp_line);
        otp_piped = true;
    }
    if let Err(error) = channel.data(payload.as_slice()).await {
        return Err(abort_exec_channel(
            &mut channel,
            format!("Failed to write sudo password: {error}"),
        )
        .await);
    }

    // Phase 2: watch for follow-up prompts and collect output. Both factors
    // were piped, so the watcher only answers hosts that re-prompt and
    // reports sudo's authentication failures.
    let outcome = match run_to_completion(
        &mut channel,
        timeout,
        Some((auth, use_pty, otp_piped)),
        cancel,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => return Err(abort_exec_channel(&mut channel, error).await),
    };
    if outcome.exit_code != 0 {
        return Err(format!(
            "sudo exited {}: {}",
            outcome.exit_code, outcome.output
        ));
    }
    Ok(outcome)
}

/// Aborts an exec channel after a failed/aborted remote run. Dropping a
/// russh `Channel` does NOT send SSH_MSG_CHANNEL_CLOSE, so the remote
/// process would keep waiting on stdin forever - a mid-authentication
/// `sudo` holds the sudo timestamp lock and deadlocks every later sudo on
/// the connection. Close the channel so sshd reaps the process.
pub(super) async fn abort_exec_channel(
    channel: &mut russh::Channel<russh::client::Msg>,
    error: String,
) -> String {
    let _ = channel.eof().await;
    let _ = channel.close().await;
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

pub(super) async fn run_to_completion(
    channel: &mut russh::Channel<russh::client::Msg>,
    timeout: Duration,
    prompt_context: Option<PromptContext<'_>>,
    cancel: &mut watch::Receiver<bool>,
) -> Result<ExecOutcome, String> {
    let deadline = tokio::time::Instant::now() + timeout;
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
        let message = tokio::select! {
            biased;
            // Cancel lands here even mid-collection: returning Err routes
            // through the caller's abort_exec_channel, which closes the
            // channel so sshd reaps the remote process.
            _ = wait_cancelled(cancel) => return Err(EXEC_CANCELLED_MESSAGE.to_string()),
            waited = tokio::time::timeout_at(deadline, channel.wait()) =>
                waited.map_err(|_| SUDO_WAIT_TIMEOUT_MESSAGE.to_string())?,
        };
        let message = match message {
            Some(message) => message,
            None => break,
        };
        match message {
            ChannelMsg::Data { ref data } => {
                append_capped(&mut stdout, data);
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
            }
            ChannelMsg::ExtendedData { ref data, .. } => {
                append_capped(&mut stderr, data);
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
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|error| format!("Failed to open sudo keepalive channel: {error}"))?;
    channel
        .exec(true, b"sudo -nv")
        .await
        .map_err(|error| format!("Failed to start sudo keepalive: {error}"))?;
    let outcome = run_to_completion(
        &mut channel,
        Duration::from_secs(15),
        None,
        &mut never_cancels(),
    )
    .await?;
    if outcome.exit_code == 0 {
        Ok(())
    } else {
        Err("sudo timestamp expired".to_string())
    }
}
