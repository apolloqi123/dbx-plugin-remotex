//! Exec terminal layer.

use super::*;

/// In-terminal Quick Sudo, ported from tiny-rdm's detectAndHandleSudo and
/// handleOrchestrationPrompt: watches PTY output and answers sudo password /
/// 2FA prompts automatically while the user keeps typing normal commands.
/// The auth settings are read through a shared lock on every chunk, so
/// runtime settings updates apply without reopening the terminal.
pub struct TerminalAutoSudo {
    auth: Arc<RwLock<SudoAuth>>,
    sudo_pending: bool,
    password_sent: bool,
    otp_sent: bool,
    /// 推迟的 OTP 应答（并发同靶场景）：同一 otp_ledger_scope（user@host:port）
    /// 的多个终端会话几乎同时弹出 OTP 提示时，当前窗口唯一的码已被先到的
    /// 会话提交，重放保护会拒绝重复注入——后到的提示不应就此晾死，而是
    /// 记下"下一个窗口再答"，由终端读循环周期性重试（`take_deferred_otp`）。
    /// 仅轮转密钥（Key）适用；静态恢复码永远不变，重试无意义。
    otp_deferred_kind: Option<PromptKind>,
    otp_deferred_until: Option<u64>,
    /// True while a full-screen program owns the alternate screen buffer.
    /// Redraws there are file content, so auto-answers must stay off.
    alt_screen: bool,
    /// Tail of an escape sequence split across chunk boundaries.
    escape_carry: String,
}

impl TerminalAutoSudo {
    pub fn new(auth: Arc<RwLock<SudoAuth>>) -> Self {
        Self {
            auth,
            sudo_pending: false,
            password_sent: false,
            otp_sent: false,
            otp_deferred_kind: None,
            otp_deferred_until: None,
            alt_screen: false,
            escape_carry: String::new(),
        }
    }

    /// Tracks `CSI ? <modes> h|l` transitions for the alternate screen buffer.
    /// A fragment cut by a chunk boundary is carried over, so entry/exit is
    /// never lost.
    fn track_alternate_screen(&mut self, chunk: &str) {
        let mut scan = std::mem::take(&mut self.escape_carry);
        scan.push_str(chunk);
        let bytes = scan.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != 0x1b {
                index += 1;
                continue;
            }
            if index + 2 >= bytes.len() || bytes[index + 1] != b'[' || bytes[index + 2] != b'?' {
                index += 1;
                continue;
            }
            let mut cursor = index + 3;
            let mut modes: Vec<u16> = Vec::new();
            let mut value: u32 = 0;
            let mut has_digits = false;
            let mut terminator = None;
            while cursor < bytes.len() {
                let byte = bytes[cursor];
                if byte.is_ascii_digit() {
                    value = value
                        .saturating_mul(10)
                        .saturating_add(u32::from(byte - b'0'));
                    has_digits = true;
                    cursor += 1;
                    continue;
                }
                if has_digits {
                    modes.push(value as u16);
                    value = 0;
                    has_digits = false;
                }
                if byte == b';' {
                    cursor += 1;
                    continue;
                }
                if byte == b'h' || byte == b'l' {
                    terminator = Some(byte);
                    cursor += 1;
                }
                break;
            }
            match terminator {
                Some(final_byte) => {
                    if modes.iter().any(|mode| ALT_SCREEN_MODES.contains(mode)) {
                        self.alt_screen = final_byte == b'h';
                    }
                    index = cursor;
                }
                None => {
                    // Unterminated sequence: hold the fragment for the next
                    // chunk instead of dropping the mode change on the floor.
                    let carry = &scan[index..];
                    if carry.len() <= MAX_ESCAPE_CARRY {
                        self.escape_carry.push_str(carry);
                    }
                    return;
                }
            }
        }
    }

    /// Feeds one terminal output chunk and returns the answer to type back
    /// (without the carriage return), if any prompt was answered.
    pub fn observe(&mut self, chunk: &str) -> Option<(AutoSudoKind, String)> {
        self.track_alternate_screen(chunk);
        if self.alt_screen {
            // A full-screen program is repainting the terminal: prompt-shaped
            // text in its output is file content, never a live question.
            return None;
        }
        let auth = self
            .auth
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let auth = &auth;
        let normalized = normalize_auth_prompt_text(chunk);
        if normalized.is_empty() {
            return None;
        }
        if has_shell_prompt(&normalized) {
            self.sudo_pending = false;
            self.password_sent = false;
            self.otp_sent = false;
            self.otp_deferred_kind = None;
            self.otp_deferred_until = None;
            return None;
        }
        let lower = normalized.to_lowercase();
        let mode = auth.flow_mode();
        let kind = classify_auth_prompt(&lower, auth);
        // Credentials are only typed into a live prompt line, never into a
        // screen of output that happens to mention a password. The shape check
        // runs on the escape-stripped chunk before the leading trim: that trim
        // would move an indented file line into the prompt column.
        let shape_text = strip_ansi_control_sequences(&chunk.replace('\r', "\n"));
        let prompt_line = prompt_tail_line(&shape_text).is_some();
        let password_prompt = is_password_prompt_line(&shape_text);

        if self.sudo_pending {
            match kind {
                Some(PromptKind::Totp) => {
                    if self.otp_sent
                        || !can_respond_to_prompt(mode, PromptKind::Totp, self.password_sent)
                    {
                        return None;
                    }
                }
                _ => return None,
            }
        }

        // Direct sudo password prompts are answered unconditionally — but a
        // merged prompt ([sudo] password + verification code in one line) is
        // not a plain password prompt: sending only the password half always
        // fails, so it falls through to the combined handling below.
        // The chunk also has to *look* like a prompt: its last line starts at
        // column 0 and ends with the prompt colon. A config key, a log line or
        // a grep hit that merely contains `password:` never matches.
        if password_prompt && !matches!(kind, Some(PromptKind::Totp | PromptKind::Combined)) {
            if auth.password.is_empty() {
                // Make the silent no-answer case diagnosable: the watcher is
                // armed but holds no credential (sudo source off/unbound, or
                // neither the connection nor the login path has a password).
                eprintln!(
                    "[ssh] terminal auto-sudo: sudo password prompt detected but no sudo password is configured (check the connection's sudo source / global profile binding)"
                );
                return None;
            }
            self.sudo_pending = true;
            self.password_sent = true;
            return Some((AutoSudoKind::Password, auth.password.clone()));
        }

        // Generic prompts are only auto-answered when custom hints are
        // configured; broad default patterns would false-positive on other
        // interactive programs (tiny-rdm applies the same guard). Exception: a
        // merged prompt that already carries a password-pattern hit ([sudo]
        // password + code in one line) is specific enough to answer without a
        // hint — answering only its password half would always fail.
        let has_custom_hint = !auth.password_prompt_hint.trim().is_empty()
            || !auth.totp_prompt_hint.trim().is_empty();
        let sudo_like_merged = matches!(kind, Some(PromptKind::Combined)) && password_prompt;
        // Hints are matched the same way: only a live prompt line, never a
        // screen of output that happens to contain the hint text.
        if !prompt_line {
            return None;
        }
        if !has_custom_hint && !sudo_like_merged {
            return None;
        }
        let kind = kind?;
        if !can_respond_to_prompt(mode, kind, self.password_sent) {
            return None;
        }
        let (answer, otp_retry_at) = auth.answer_for_with_retry(kind);
        let Some(answer) = answer else {
            // 码已在重放窗口内被同靶的其他会话提交：不注入、不晾死，
            // 推迟到下一个 TOTP 窗口由读循环重试补答。
            self.otp_deferred_kind = Some(kind);
            self.otp_deferred_until = otp_retry_at;
            return None;
        };
        let auto_kind = match kind {
            PromptKind::Totp => AutoSudoKind::Totp,
            _ => AutoSudoKind::Password,
        };
        if matches!(kind, PromptKind::Password | PromptKind::Combined) {
            self.password_sent = true;
        }
        if matches!(kind, PromptKind::Totp)
            || (kind == PromptKind::Combined && mode.answers_merged_prompt_with_otp())
        {
            self.otp_sent = true;
        }
        self.sudo_pending = true;
        Some((auto_kind, answer))
    }

    /// 重试被推迟的 OTP 应答；由终端读循环周期性调用（`now` 为调用方传入的
    /// 当前 unix 秒，便于测试注入）。仅当提示序列仍挂起（未被 shell prompt
    /// 复位，也尚未直接应答过 OTP）且推迟时刻已到时才尝试；拿到新窗口的码
    /// 即清掉推迟态并返回应答，仍拿不到则顺延到下一个窗口。
    pub fn take_deferred_otp(&mut self, now: u64) -> Option<(AutoSudoKind, String)> {
        if self.otp_sent {
            self.otp_deferred_kind = None;
            self.otp_deferred_until = None;
            return None;
        }
        let kind = self.otp_deferred_kind?;
        let due = self.otp_deferred_until?;
        if now < due {
            return None;
        }
        let auth = self
            .auth
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let (answer, retry_at) = auth.answer_for_with_retry(kind);
        match answer {
            Some(text) => {
                self.otp_deferred_kind = None;
                self.otp_deferred_until = None;
                self.otp_sent = true;
                let auto_kind = if kind == PromptKind::Totp {
                    AutoSudoKind::Totp
                } else {
                    AutoSudoKind::Password
                };
                Some((auto_kind, text))
            }
            None => {
                // 新窗口的码又被同靶会话抢了：继续顺延（重试时刻必然后移）。
                self.otp_deferred_until = retry_at.or(self.otp_deferred_until);
                None
            }
        }
    }
}

/// What the state machine decided to type into the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoSudoKind {
    Password,
    Totp,
}

/// True when the last non-empty line of the terminal output looks like a
/// shell prompt, meaning any in-flight auth sequence has finished.
pub(crate) fn has_shell_prompt(normalized: &str) -> bool {
    normalized
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| {
            let line = line.trim_end();
            line.ends_with('$') || line.ends_with('#')
        })
}

/// Returns the last visible line of a chunk when the chunk still looks like a
/// live prompt: few lines, prompt starting at column 0. Screen dumps are
/// rejected here so their content can never be mistaken for a prompt.
pub(crate) fn prompt_tail_line(normalized: &str) -> Option<&str> {
    let lines: Vec<&str> = normalized
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.len() > MAX_PROMPT_CHUNK_LINES {
        return None;
    }
    let last = lines.last()?.trim_end();
    if last.is_empty() || last.len() > MAX_PROMPT_LINE_LEN {
        return None;
    }
    // Real prompts start at column 0; indented text is file content (a nested
    // YAML key, a diff hunk, a log continuation line).
    if last.starts_with([' ', '\t']) {
        return None;
    }
    Some(last)
}

/// True when the chunk's last line is shaped like a password prompt: it ends
/// with the prompt colon and opens with one of the built-in password patterns.
/// A YAML `password: value` key, a grep hit or a log line ends with its value
/// (or is indented), so it never matches.
pub(crate) fn is_password_prompt_line(normalized: &str) -> bool {
    let Some(line) = prompt_tail_line(normalized) else {
        return false;
    };
    if !(line.ends_with(':') || line.ends_with('：')) {
        return false;
    }
    let lower = line.to_lowercase();
    // sudo names its own prompt and may follow the lecture sentence, so an
    // inline match is fine. The bare forms (password:/passphrase:/密码:/口令:)
    // have to open the line — prompt column 0 — so a nested YAML key, a log
    // continuation or a diff hunk never qualifies.
    if lower.contains("[sudo] password for") {
        return true;
    }
    PASSWORD_PROMPT_PATTERS
        .iter()
        .any(|pattern| lower.starts_with(pattern))
}

/// A live prompt arrives alone or right behind the echoed command. Anything
/// longer is a screen dump (`cat`, `grep`, a pager, a vim redraw) whose text
/// can end with a config key that reads exactly like a prompt.
pub(super) const MAX_PROMPT_CHUNK_LINES: usize = 8;

/// Real prompts are short; a file line that merely mentions `password:` is
/// usually not.
pub(super) const MAX_PROMPT_LINE_LEN: usize = 160;

/// DEC private modes that switch the terminal to its alternate screen buffer.
/// While one of them is set, a full-screen program (vim, less, top, man) owns
/// the terminal and repaints arbitrary file content into it.
pub(super) const ALT_SCREEN_MODES: [u16; 3] = [47, 1047, 1049];

/// Longest escape-sequence fragment carried across chunk boundaries so a
/// terminal-mode switch split by the transport is never lost.
pub(super) const MAX_ESCAPE_CARRY: usize = 32;
