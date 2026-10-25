//! Auth prompt normalization, classification and response gating.

use super::*;

/// Strips ANSI escape sequences and non-printable control characters, then
/// trims the result so prompt fragments can be matched reliably.
pub fn normalize_auth_prompt_text(input: &str) -> String {
    let stripped = strip_ansi_control_sequences(&input.replace('\r', "\n"));
    let cleaned: String = stripped
        .chars()
        .filter(|&character| {
            character == '\n' || character == '\t' || character == ' ' || !character.is_control()
        })
        .collect();
    cleaned.trim().to_string()
}

/// Normalizes a user-provided prompt hint for reliable matching: strips ANSI
/// escape sequences and control characters, trims, and caps the length.
/// Malformed hints degrade to their printable residue instead of poisoning
/// the prompt classifier or the stored connection.
pub fn sanitize_prompt_hint(input: &str) -> String {
    let mut normalized = normalize_auth_prompt_text(input);
    if normalized.len() > MAX_PROMPT_HINT_LEN {
        // Cut on a char boundary so the hint stays valid UTF-8.
        let mut end = MAX_PROMPT_HINT_LEN;
        while !normalized.is_char_boundary(end) {
            end -= 1;
        }
        normalized.truncate(end);
    }
    normalized
}

/// Strips ANSI escape sequences (CSI/OSC/2-byte escapes) while keeping every
/// other byte, including CR/LF and whitespace. Shared by the auth-prompt
/// normalizer and the telnet declarative watch, which matches anchored
/// success/failure regexes against terminal output (NyaTerm strips ANSI the
/// same way before its auto-login matching).
pub(crate) fn strip_ansi_control_sequences(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0x1b {
            output.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 1 >= bytes.len() {
            break;
        }
        match bytes[index + 1] {
            b'[' => {
                index += 2;
                while index < bytes.len() && !(0x40..=0x7e).contains(&bytes[index]) {
                    index += 1;
                }
                index += 1;
            }
            b']' => {
                index += 2;
                while index < bytes.len() {
                    if bytes[index] == 0x07 {
                        break;
                    }
                    if bytes[index] == 0x1b && index + 1 < bytes.len() && bytes[index + 1] == b'\\'
                    {
                        index += 1;
                        break;
                    }
                    index += 1;
                }
                index += 1;
            }
            _ => index += 2,
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

pub(crate) fn classify_auth_prompt(prompt: &str, auth: &SudoAuth) -> Option<PromptKind> {
    let trimmed = normalize_auth_prompt_text(prompt);
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    // Hints are matched against the normalized prompt text, so legacy or
    // host-provided hints carrying control characters/ANSI sequences are
    // normalized the same way instead of never matching.
    let password_hint = normalize_auth_prompt_text(&auth.password_prompt_hint).to_lowercase();
    let password_hint_matched = !password_hint.is_empty() && lower.contains(&password_hint);
    let totp_hint = normalize_auth_prompt_text(&auth.totp_prompt_hint).to_lowercase();
    let totp_hint_matched = !totp_hint.is_empty() && lower.contains(&totp_hint);
    let password_builtin = PASSWORD_PROMPT_PATTERS.iter().any(|p| lower.contains(p));
    let totp_builtin = TOTP_PROMPT_PATTERS.iter().any(|p| lower.contains(p));
    // OTP 信号优先于用户自定义的"密码提示词"：堡垒机把 MFA 提问写成
    // "[OTP Code]: "，用户若把这串可见文案填进密码提示词字段，旧逻辑会把
    // 提问判成密码提问、把登录密码当作验证码回给服务器（必然认证失败，
    // issue #30）。只有内置密码模式才允许把提问拉回密码侧。
    let password_matched =
        password_builtin || (password_hint_matched && !totp_builtin && !totp_hint_matched);
    let totp_matched = totp_builtin || totp_hint_matched;
    match (password_matched, totp_matched) {
        (true, true) => Some(PromptKind::Combined),
        (false, true) => Some(PromptKind::Totp),
        (true, false) => Some(PromptKind::Password),
        (false, false) => None,
    }
}

pub(crate) fn can_respond_to_prompt(
    mode: AuthFlowMode,
    kind: PromptKind,
    password_answered: bool,
) -> bool {
    match kind {
        // `exec_with_sudo` pipes the password before any prompt is watched
        // (`password_answered` starts true); answering a visible password
        // prompt again would queue a second password line and shift every
        // later OTP answer one read out of position.
        PromptKind::Password => !password_answered,
        PromptKind::Totp => {
            mode.allows_otp_after_password()
                && (mode != AuthFlowMode::PasswordThenOtp || password_answered)
        }
        PromptKind::Combined => {
            // 合并提问（同一条提问里既有密码又有验证码）：密码尚未通过管道
            // 喂入/此前未应答时四种模式都会应答——OTP 模式拼接验证码，密码
            // 类模式只回密码半边。密码已经喂过后再出现的"合并提问"只可能
            // 来自命令自身的输出（伪提示词）：再答一次会把 Quick Sudo 密码
            // （甚至烧掉一枚 TOTP 码）写进命令 stdin，随命令输出回显泄漏。
            !password_answered
        }
    }
}
