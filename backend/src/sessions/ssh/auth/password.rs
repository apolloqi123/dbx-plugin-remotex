//! SSH auth support.

use super::*;

/// Tries password authentication first, then keyboard-interactive. The
/// `offered` result of the preceding auth attempt tells which methods the
/// server still accepts. The initial method choice is reconciled against the
/// leading none probe (`advertised_none` — see [`reconcile_advertised`]), but
/// a successful first factor may cause the server to advertise a new MFA
/// method that was absent from the initial list.
/// Keyboard-interactive rounds are auto-answered from
/// the Quick Sudo orchestration config, covering PAM 2FA/TOTP logins.
pub(crate) async fn authenticate_password_or_interactive(
    session: &mut Handle<SshClient>,
    connection: &StoredConnection,
    orchestration: &SudoAuth,
    advertised_none: Option<&MethodSet>,
    offered: &AuthResult,
    prompts: &PromptBroker,
) -> Result<(), String> {
    if reconcile_advertised(advertised_none, offered, MethodKind::Password) {
        eprintln!("[ssh-trace] auth: password method offered, trying password");
        let result = try_password(session, connection).await?;
        eprintln!(
            "[ssh-trace] auth: password result success={}",
            result.success()
        );
        if result.success() {
            return Ok(());
        }
        // A password accepted as the first factor can unlock keyboard-interactive
        // MFA. Trust the password step's current advertisement here: intersecting
        // it with the earlier none probe would reject newly advertised MFA.
        if method_offered(&result, MethodKind::KeyboardInteractive) {
            eprintln!(
                "[ssh-trace] auth: falling back to keyboard-interactive (partial_success={})",
                auth_partial_success(&result)
            );
            // 密码已经提交过一次：接下来的 keyboard-interactive 就是第二因子
            // （koko/JumpServer 用 partial success 通知 MFA，PAM 栈甚至不置该位），
            // 因此 OTP 提问必须能应答，而不是被判成"密码还没到"。
            return authenticate_keyboard_interactive(
                session,
                connection,
                orchestration,
                true,
                prompts,
            )
            .await;
        }
        return Err("SSH password authentication was rejected".to_string());
    }
    if reconcile_advertised(advertised_none, offered, MethodKind::KeyboardInteractive) {
        eprintln!("[ssh-trace] auth: keyboard-interactive only, starting");
        // Servers with PasswordAuthentication disabled still accept the
        // password through keyboard-interactive (PAM), including hosts that
        // ask a 2FA/TOTP follow-up question.
        return authenticate_keyboard_interactive(
            session,
            connection,
            orchestration,
            auth_partial_success(offered),
            prompts,
        )
        .await;
    }
    eprintln!("[ssh-trace] auth: neither password nor keyboard-interactive offered");
    Err(advertisement_rejection_message(advertised_none))
}

pub(crate) async fn try_password(
    session: &mut Handle<SshClient>,
    connection: &StoredConnection,
) -> Result<AuthResult, String> {
    tokio::time::timeout(
        Duration::from_secs(connection.connect_timeout_secs),
        session.authenticate_password(&connection.username, &connection.password),
    )
    .await
    .map_err(|_| "SSH password authentication timed out".to_string())?
    .map_err(|error| format!("SSH password authentication failed: {error}"))
}

/// Rounds allowed for one keyboard-interactive exchange before the handshake
/// is given up (servers that keep re-asking see a deterministic failure).
pub(crate) const KEYBOARD_INTERACTIVE_MAX_ROUNDS: usize = 4;

/// Renders the server's challenge wording for an error message. Only the
/// prompt/instruction text is used (never an answer), stripped of control
/// characters and capped, so a hostile server cannot smuggle escape sequences
/// into the workbench error dialog.
pub(crate) fn challenge_display_text(text: &str, seen: &[String]) -> Option<String> {
    let sanitized = exec::sanitize_prompt_hint(text);
    if sanitized.is_empty() || seen.iter().any(|existing| existing == &sanitized) {
        return None;
    }
    Some(sanitized)
}

/// Explains an unanswered keyboard-interactive challenge: what the server
/// asked, and where the user has to put a credential for it to be answered.
/// `asked` holds the deduplicated prompt wording, `answered` says whether any
/// non-empty answer was sent in this exchange.
pub(crate) fn keyboard_interactive_guidance(asked: &[String], answered: bool) -> String {
    if asked.is_empty() {
        return String::new();
    }
    let quoted: Vec<String> = asked.iter().map(|text| format!("{text:?}")).collect();
    let mut message = format!("; the server asked for {}", quoted.join(", "));
    if answered {
        message.push_str(" and rejected the submitted answer");
    } else {
        message.push_str(
            " — nothing was answered. Configure the connection's TOTP secret or OTP prompt hint (Quick Sudo 2FA settings) so MFA prompts can be answered automatically",
        );
    }
    message
}

/// Builds the text shown in the one-time input dialog from server-controlled
/// keyboard-interactive fields. Every component is normalized and capped by
/// the same sanitizer used for diagnostics; duplicate lines are omitted.
pub(crate) fn keyboard_interactive_manual_challenge(context: &str, prompt: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for text in [context, prompt] {
        let text = exec::sanitize_prompt_hint(text);
        if text.is_empty() || parts.iter().any(|existing| existing == &text) {
            continue;
        }
        parts.push(text);
    }
    parts.join("\n")
}

/// Drives a keyboard-interactive handshake, answering each round from the
/// orchestration config (password plus optional TOTP follow-ups).
/// `first_factor_accepted` says whether the preceding auth step already
/// succeeded partially (koko: password/publickey accepted, MFA still owed) —
/// `password_then_otp` needs it to answer the MFA question.
pub(crate) async fn authenticate_keyboard_interactive(
    session: &mut Handle<SshClient>,
    connection: &StoredConnection,
    orchestration: &SudoAuth,
    first_factor_accepted: bool,
    prompt_broker: &PromptBroker,
) -> Result<(), String> {
    let timeout = Duration::from_secs(connection.connect_timeout_secs);
    let mut state = if first_factor_accepted {
        exec::KeyboardInteractiveState::first_factor_accepted()
    } else {
        exec::KeyboardInteractiveState::default()
    };
    let mut response = tokio::time::timeout(
        timeout,
        session.authenticate_keyboard_interactive_start(&connection.username, None),
    )
    .await
    .map_err(|_| "SSH keyboard-interactive authentication timed out".to_string())?
    .map_err(|error| format!("SSH keyboard-interactive authentication failed: {error}"))?;
    let mut asked: Vec<String> = Vec::new();
    let mut answered = false;
    for _ in 0..KEYBOARD_INTERACTIVE_MAX_ROUNDS {
        match response {
            client::KeyboardInteractiveAuthResponse::Success => return Ok(()),
            client::KeyboardInteractiveAuthResponse::Failure { .. } => {
                return Err(format!(
                    "SSH keyboard-interactive authentication was rejected{}",
                    keyboard_interactive_guidance(&asked, answered)
                ));
            }
            client::KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                // koko 把可读文案放在 instructions（"Please Enter MFA Code."），
                // 提问本身是 "[OTP Code]: "：两者都参与分类与错误说明。
                let context = exec::keyboard_interactive_challenge_context(&name, &instructions);
                for text in std::iter::once(&name)
                    .chain(std::iter::once(&instructions))
                    .chain(prompts.iter().map(|prompt| &prompt.prompt))
                {
                    if asked.len() >= 4 {
                        break;
                    }
                    if let Some(display) = challenge_display_text(text, &asked) {
                        asked.push(display);
                    }
                }
                let mut answers = exec::keyboard_interactive_answers(
                    orchestration,
                    &mut state,
                    &context,
                    &prompts,
                );
                // Values available from the connection (login password or a
                // configured TOTP seed) stay automatic. Any answer that is
                // still empty is a genuine keyboard-interactive question:
                // ask the user at connection time so rotating hardware/app
                // tokens never need to be stored as a fake static secret.
                for (answer, prompt) in answers.iter_mut().zip(prompts.iter()) {
                    if !answer.is_empty() {
                        continue;
                    }
                    let challenge = keyboard_interactive_manual_challenge(&context, &prompt.prompt);
                    if let Some(value) = prompt_broker
                        .request_keyboard_interactive_answer(
                            &connection.host,
                            connection.port,
                            &connection.username,
                            challenge,
                        )
                        .await?
                    {
                        *answer = value;
                    }
                }
                if answers.iter().any(|answer| !answer.is_empty()) {
                    answered = true;
                }
                response = tokio::time::timeout(
                    timeout,
                    session.authenticate_keyboard_interactive_respond(answers),
                )
                .await
                .map_err(|_| "SSH keyboard-interactive authentication timed out".to_string())?
                .map_err(|error| {
                    format!("SSH keyboard-interactive authentication failed: {error}")
                })?;
            }
        }
    }
    Err(format!(
        "SSH keyboard-interactive authentication did not finish after {KEYBOARD_INTERACTIVE_MAX_ROUNDS} rounds{}",
        keyboard_interactive_guidance(&asked, answered)
    ))
}
