//! Terminal Support helpers for the SSH runtime.

use super::*;

pub(super) fn directory_tracking_marker(session_id: &str) -> Vec<u8> {
    format!("\x1b]777;dbx-directory-ready-{session_id}\x07").into_bytes()
}

/// Decodes one terminal output frame for the text observers, carrying an
/// incomplete multi-byte UTF-8 tail over to the next call. Returns the lossy
/// decoding of the longest decodable prefix; the tail bytes wait in `carry`.
/// `error_len() == None` only happens with ≤3 bytes left, so the carry stays
/// bounded by construction; invalid sequences are flushed lossily right away
/// (one U+FFFD per maximal invalid subsequence, same output as a whole-buffer
/// `from_utf8_lossy`) so a non-UTF-8 stream keeps flowing instead of stalling.
/// 不设长度阈值：大帧（现实 SSH 数据帧普遍远超 8 字节）尾部残缺字符同样
/// 跨帧拼装，否则逐帧独立 lossy 会把字符劈成双 U+FFFD（见读循环处注释）。
pub(super) fn decode_stream_text(carry: &mut Vec<u8>, data: &[u8]) -> String {
    // Steady state (no incomplete character pending): one validated copy
    // instead of the append-then-copy the carry path costs.
    if carry.is_empty() {
        if let Ok(text) = std::str::from_utf8(data) {
            return text.to_owned();
        }
    }
    carry.extend_from_slice(data);
    let mut text = String::new();
    loop {
        match std::str::from_utf8(carry) {
            Ok(valid) => {
                text.push_str(valid);
                carry.clear();
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    text.push_str(std::str::from_utf8(&carry[..valid]).expect("validated prefix"));
                    carry.drain(..valid);
                    continue;
                }
                match error.error_len() {
                    // 残缺尾部：等下一帧拼字。
                    None => break,
                    // 非法序列：按最大子序列立即冲刷，残量不滞留。
                    Some(invalid_len) => {
                        text.push('\u{FFFD}');
                        carry.drain(..invalid_len);
                    }
                }
            }
        }
    }
    text
}

/// Installs the per-prompt OSC 7 hook for one interactive shell session
/// (issue t8y2/dbx#10750): bash gets a `PROMPT_COMMAND` prepend, zsh a
/// `precmd_functions` entry, each emitting `ESC]7;file://$HOST$PWD BEL`
/// immediately before every prompt draw. The frontend consumes these frames
/// twice — SFTP directory follow (as before) and the prompt-newline fix
/// (insert CRLF when the preceding output did not end on a line start). The
/// hook is session-local and idempotent (`__DBX_CWD_ACTIVE` guard), so
/// re-arming on an already-hooked shell is a no-op remotely. Arming is now
/// unconditional for the session's lifetime: there is no teardown script
/// anymore — the SFTP follow toggle turning off must not strip the frame
/// the newline fix depends on (frontend merely stops navigating).
pub(super) fn directory_tracking_script(session_id: &str, shell: RemoteShell) -> String {
    let marker = format!("\\033]777;dbx-directory-ready-{session_id}\\007");
    // zsh's first `precmd_functions` run needs one extra line to trigger, so
    // the install line is followed by a blank Enter; bash's PROMPT_COMMAND
    // fires on the prompt that follows the install line by itself.
    let (body, history_flush) = match shell {
        RemoteShell::Bash => (
            r#"if [ -z "${__DBX_CWD_ACTIVE+x}" ]; then __DBX_CWD_ACTIVE=1; __DBX_OLD_HISTCONTROL_SET=${HISTCONTROL+x}; __DBX_OLD_HISTCONTROL=${HISTCONTROL-}; case ":${HISTCONTROL-}:" in *:ignorespace:*|*:ignoreboth:*) __DBX_HISTORY_NEEDS_DELETE=0 ;; *) __DBX_HISTORY_NEEDS_DELETE=1; HISTCONTROL="${HISTCONTROL:+$HISTCONTROL:}ignorespace" ;; esac; __DBX_OLD_PROMPT_COMMAND=${PROMPT_COMMAND-}; __dbx_emit_cwd(){ printf '\033]7;file://%s%s\007' "${HOSTNAME:-localhost}" "$PWD"; }; PROMPT_COMMAND='__dbx_emit_cwd;'"$__DBX_OLD_PROMPT_COMMAND"; if [ "$__DBX_HISTORY_NEEDS_DELETE" = 1 ]; then history -d $((HISTCMD-1)) 2>/dev/null || true; fi; unset __DBX_HISTORY_NEEDS_DELETE; fi"#,
            "",
        ),
        RemoteShell::Zsh => (
            r#"if [ -z "${__DBX_CWD_ACTIVE+x}" ]; then __DBX_CWD_ACTIVE=1; if [[ -o HIST_IGNORE_SPACE ]]; then __DBX_OLD_HIST_IGNORE_SPACE=1; else __DBX_OLD_HIST_IGNORE_SPACE=0; setopt HIST_IGNORE_SPACE; fi; __dbx_emit_cwd(){ printf '\033]7;file://%s%s\007' "${HOST:-localhost}" "$PWD"; }; typeset -ga precmd_functions; precmd_functions=(__dbx_emit_cwd ${precmd_functions:#__dbx_emit_cwd}); fi"#,
            " \r",
        ),
        // Callers only arm for bash/zsh; Unknown/Other degrade to a no-op line
        // so a stray invocation can never type garbage into a foreign shell.
        RemoteShell::Other | RemoteShell::Unknown => ("", ""),
    };
    format!(" {body}; printf '{marker}'\r{history_flush}")
}

/// Whether a terminal session opened as an interactive remote shell (and only
/// then): a spawn-exec command session or a connection-level `remote_command`
/// replaces the shell, so shell-syntax lines like the prompt hook must never
/// be typed into the channel — they would feed the command's stdin. Same
/// gating as the startup-commands injector (their input lines share the FIFO,
/// so relative order between the two is preserved whatever the send order).
pub(super) fn interactive_shell_session(spawn_exec: Option<&str>, remote_command: &str) -> bool {
    spawn_exec.is_none() && remote_command.is_empty()
}

/// A caller-supplied session id is honoured only when it is safe as a registry
/// key, event-channel suffix, and shell literal: non-empty, bounded, and
/// restricted to `[A-Za-z0-9._-]`. The id is interpolated into the
/// single-quoted `printf` marker of [`directory_tracking_script`], so anything
/// shell-active (`'`, `` ` ``, `$`, …) must fall back to a server-generated
/// UUID rather than reach the remote shell.
pub(super) fn valid_requested_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// WT-4 (command session) command size cap, same magnitude as the per-command
/// cap of startup commands (`startup_commands::MAX_COMMAND_BYTES`).
pub(super) const SPAWN_COMMAND_MAX_BYTES: usize = 4 * 1024;

/// Normalizes the WT-4 `spawnCommand` request into the exec payload, or `None`
/// when the caller did not ask for a command session (absent or blank keeps
/// the ordinary shell / connection `remote_command` semantics). Fails closed
/// on NUL bytes (exec strings cannot carry them) and over-cap commands
/// instead of silently truncating. Pure so tests can exercise the contract
/// without a server.
pub(super) fn normalized_spawn_command(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.contains('\0') {
        return Err("spawnCommand must not contain NUL bytes".to_string());
    }
    if trimmed.len() > SPAWN_COMMAND_MAX_BYTES {
        return Err(format!(
            "spawnCommand exceeds the {SPAWN_COMMAND_MAX_BYTES} byte limit"
        ));
    }
    Ok(Some(trimmed.to_owned()))
}

/// Wraps a normalized command session command as the exec payload:
/// `sh -c '<single-quote escaped command>'` (repo hard rule: remote commands
/// are single-quote escaped). The outer login shell receives exactly one safe
/// argument so metacharacters cannot escape the quoting, while the inner
/// `sh -c` keeps full shell semantics (pipes/redirects work as typed).
/// Reuses the existing `exec::shell_quote` escaper.
pub(super) fn spawn_exec_payload(command: &str) -> String {
    format!("sh -c {}", exec::shell_quote(command))
}

/// Resolves the login shell for a transport, probing at most once per
/// authenticated connection. Duplicate panes share the result, so opening N
/// panes no longer spends N extra session channels on a server whose
/// `MaxSessions` budget is already tight.
pub(super) async fn detect_remote_shell(
    handle: &Handle<SshClient>,
    lease: &SharedTransportLease,
) -> RemoteShell {
    // Concurrent pane opens queue here; the second caller then hits the cache
    // instead of racing a second probe onto the wire.
    let _probe = lease.shell_probe.lock().await;
    if let Some(shell) = lease.cached_remote_shell() {
        return shell;
    }
    match probe_remote_shell(handle).await {
        // Only a *completed* probe is cached: a refusal/timeout says nothing
        // about the account's shell, and caching `Other` would silently
        // disable directory tracking for the rest of the transport's life.
        Some(shell) => {
            lease.cache_remote_shell(shell);
            shell
        }
        None => RemoteShell::Other,
    }
}

/// Probes `$SHELL` on one exec channel.
///
/// The dial itself is deliberately *not* wrapped in a timeout: dropping an
/// in-flight `channel_open_session` future after the server allocated the
/// channel leaks that session for the connection's lifetime. Only the output
/// read is bounded, and the channel is always closed explicitly — a dropped
/// russh channel never sends CHANNEL_CLOSE either.
async fn probe_remote_shell(handle: &Handle<SshClient>) -> Option<RemoteShell> {
    let mut channel = open_session_channel_labeled(handle, "shell-probe")
        .await
        .ok()?;
    if channel.exec(true, "printf '%s' \"$SHELL\"").await.is_err() {
        exec::close_setup_channel(&mut channel).await;
        return None;
    }
    let mut output = Vec::new();
    let read = tokio::time::timeout(REMOTE_SHELL_DETECTION_TIMEOUT, async {
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } => output.extend_from_slice(&data),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
    })
    .await;
    if read.is_err() {
        exec::close_setup_channel(&mut channel).await;
        return None;
    }
    channel_ledger::closed(channel.id().number());
    Some(classify_remote_shell(&String::from_utf8_lossy(&output)))
}

pub(super) fn classify_remote_shell(value: &str) -> RemoteShell {
    let shell = value.trim().replace('\\', "/");
    match shell.rsplit('/').next().unwrap_or_default() {
        "bash" => RemoteShell::Bash,
        "zsh" => RemoteShell::Zsh,
        _ => RemoteShell::Other,
    }
}

pub(super) fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

pub fn filesystem_path(params: &Value) -> Result<String, String> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing filesystem URI".to_string())?;
    path_from_sftp_uri(uri)
}

pub fn connection_id_param(params: &Value) -> Result<&str, String> {
    params
        .get("connectionId")
        .and_then(Value::as_str)
        .ok_or("Missing connectionId".to_string())
}
