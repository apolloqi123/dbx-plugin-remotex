//! Streamable exec variants: every stdout/stderr fragment is forwarded to a
//! bounded channel as it arrives instead of being buffered until exit. The
//! regular `*_cancellable` paths keep their collect-and-return behavior by
//! passing `sink: None`.

use super::*;

/// Shared body of [`exec_with_sudo_cancellable`] and [`exec_sudo_stream`]:
/// `timeout: None` disables the overall deadline (log following must not be
/// killed by an exec cap), `sink: None` keeps the buffered-only behavior.
pub(super) async fn exec_with_sudo_inner(
    handle: &Handle<SshClient>,
    auth: &SudoAuth,
    command: &str,
    use_pty: bool,
    set_env: &[(String, String)],
    timeout: Option<Duration>,
    cancel: &mut watch::Receiver<bool>,
    sink: Option<&tokio::sync::mpsc::Sender<Vec<u8>>>,
) -> Result<ExecOutcome, String> {
    if auth.password.is_empty() {
        let command_line = format!("sudo -n {}", sanitize_sudo_command(command));
        // Uncancellable dial: a dropped in-flight open leaks the channel the
        // server already allocated. Cancel is handled right after, via abort.
        let mut channel = open_session_channel(handle)
            .await
            .map_err(|error| format!("Failed to open sudo channel: {error}"))?;
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
            } => {
                if let Err(error) = started {
                    return Err(abort_exec_channel(&mut channel, error).await);
                }
            }
        }
        let outcome = match run_to_completion(&mut channel, timeout, None, cancel, sink).await {
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
    // Uncancellable dial (see the passwordless branch above): cancel lands at
    // the env/exec select and closes the channel through the abort path.
    let mut channel = open_session_channel(handle)
        .await
        .map_err(|error| format!("Failed to open sudo channel: {error}"))?;
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
        } => {
            if let Err(error) = started {
                return Err(abort_exec_channel(&mut channel, error).await);
            }
        }
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
        sink,
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

/// Plain (no sudo) streamable exec: opens a fresh channel, applies the
/// connection's client `setEnv` entries, and forwards output fragments to
/// `sink` as they arrive. Returns the remote exit code.
pub async fn exec_plain_stream(
    handle: &russh::client::Handle<SshClient>,
    command: &str,
    set_env: &[(String, String)],
    cancel: watch::Receiver<bool>,
    sink: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> Result<i32, String> {
    let mut cancel = cancel;
    // Uncancellable dial: dropping a channel-open future after sshd allocated
    // the channel leaks that session against `MaxSessions`. Cancellation is
    // honoured by the env/exec select below, which closes the channel.
    let mut channel = open_session_channel(handle)
        .await
        .map_err(|error| format!("Failed to open exec channel: {error}"))?;
    // env/exec start: the command may have been accepted by the time the
    // cancel lands, so this arm closes the channel before returning.
    tokio::select! {
        biased;
        _ = wait_cancelled(&mut cancel) => {
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
            if let Err(error) = started {
                return Err(abort_exec_channel(&mut channel, error).await);
            }
        }
    }
    match run_to_completion(&mut channel, None, None, &mut cancel, Some(&sink)).await {
        Ok(outcome) => Ok(outcome.exit_code),
        // Spec 4.2: cancellation must close the channel through the regular
        // abort path so sshd reaps `docker logs -f` instead of leaking it.
        Err(error) => Err(abort_exec_channel(&mut channel, error).await),
    }
}

/// Quick Sudo streamable exec: same orchestration as
/// [`exec_with_sudo_cancellable`] (password + OTP over stdin, prompt
/// watcher), with output fragments forwarded to `sink` as they arrive.
pub async fn exec_sudo_stream(
    handle: &russh::client::Handle<SshClient>,
    auth: &SudoAuth,
    command: &str,
    use_pty: bool,
    set_env: &[(String, String)],
    cancel: watch::Receiver<bool>,
    sink: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> Result<i32, String> {
    let mut cancel = cancel;
    let outcome = exec_with_sudo_inner(
        handle,
        auth,
        command,
        use_pty,
        set_env,
        None,
        &mut cancel,
        Some(&sink),
    )
    .await?;
    Ok(outcome.exit_code)
}
