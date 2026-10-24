//! Keyboard-interactive challenge handling.

use super::*;

/// Tracks whether the first authentication factor is already behind us during
/// a keyboard-interactive handshake, mirroring tiny-rdm's per-connection
/// state. `password_then_otp` gates OTP answers on this flag so a bare OTP
/// question cannot be answered before the password step.
#[derive(Debug, Default)]
pub struct KeyboardInteractiveState {
    pub(in crate::core::exec) password_answered: bool,
}

impl KeyboardInteractiveState {
    /// Seeds the state for a keyboard-interactive exchange that starts *after*
    /// the first factor was already accepted. JumpServer/koko accepts the
    /// password (or publickey) with `partial_success` and only then asks for
    /// the MFA code over keyboard-interactive; without this seed the MFA
    /// question looks like "the password has not been sent yet" and
    /// `password_then_otp` leaves it blank, so the login can never succeed.
    pub fn first_factor_accepted() -> Self {
        Self {
            password_answered: true,
        }
    }
}

/// Joins a keyboard-interactive challenge's `name` and `instructions` into the
/// fallback text used to classify prompts whose own wording is unrecognized.
/// koko sends the readable "Please Enter MFA Code." as instructions while the
/// prompt itself is "[OTP Code]: ", so users who copy the visible line into
/// the OTP hint would otherwise never match anything. Server-controlled text,
/// so it is stripped of control characters and capped like a user hint.
pub fn keyboard_interactive_challenge_context(name: &str, instructions: &str) -> String {
    let mut context = String::new();
    for part in [name, instructions] {
        let part = sanitize_prompt_hint(part);
        if part.is_empty() || context.contains(&part) {
            continue;
        }
        if !context.is_empty() {
            context.push('\n');
        }
        context.push_str(&part);
    }
    context
}

/// Builds automatic answers for a keyboard-interactive login round. Prompts
/// are classified individually; a prompt whose own wording is unrecognized
/// falls back to the challenge `name`/`instructions`. Unknown or unanswered
/// prompts get an empty string so the server can re-prompt or fail cleanly.
/// Ported from tiny-rdm's keyboardInteractivePasswordAuth callback.
pub fn keyboard_interactive_answers(
    auth: &SudoAuth,
    state: &mut KeyboardInteractiveState,
    challenge_context: &str,
    prompts: &[russh::client::Prompt],
) -> Vec<String> {
    let mode = auth.flow_mode();
    let mut password_answered_round = false;
    let context_kind = (!challenge_context.trim().is_empty())
        .then(|| classify_auth_prompt(challenge_context, auth))
        .flatten();
    let answers = prompts
        .iter()
        .map(
            |prompt| match classify_auth_prompt(&prompt.prompt, auth).or(context_kind) {
                Some(PromptKind::Password) => {
                    if auth.password.is_empty() {
                        String::new()
                    } else {
                        password_answered_round = true;
                        auth.password.clone()
                    }
                }
                Some(PromptKind::Totp) => {
                    if !can_respond_to_prompt(
                        mode,
                        PromptKind::Totp,
                        state.password_answered || password_answered_round,
                    ) {
                        String::new()
                    } else {
                        auth.totp_answer_logged().unwrap_or_default()
                    }
                }
                Some(PromptKind::Combined) => {
                    if auth.password.is_empty() {
                        String::new()
                    } else if mode.answers_merged_prompt_with_otp() {
                        match auth.totp_answer_logged() {
                            Some(code) => {
                                password_answered_round = true;
                                format!("{}{}", auth.password, code)
                            }
                            // 没有可用验证码（未配置密钥 / 仍在重放窗口）：退回
                            // 密码单发，让服务器自行重问，而不是整条留空。
                            None => {
                                password_answered_round = true;
                                auth.password.clone()
                            }
                        }
                    } else {
                        password_answered_round = true;
                        auth.password.clone()
                    }
                }
                None => String::new(),
            },
        )
        .collect();
    if password_answered_round {
        state.password_answered = true;
    }
    answers
}
