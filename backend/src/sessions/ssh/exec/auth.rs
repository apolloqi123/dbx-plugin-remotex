//! Exec auth layer.

mod keyboard;
mod otp;
mod prompt;

pub use keyboard::*;
pub use otp::*;
pub use prompt::*;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 变体名刻意与协议值一一对应（off / password_only / password_plus_otp /
// password_then_otp），共享的 Password 前缀是契约不是冗余。
#[allow(clippy::enum_variant_names)]
pub enum AuthFlowMode {
    /// 2FA 自动应答关闭（0.4.77 起连接表单默认值）：密码类提示照常应答，
    /// OTP/组合提示永不自动回码。存量连接缺省该字段时仍走 PasswordThenOtp，
    /// 运行时语义不受新默认值影响。
    Off,
    PasswordOnly,
    PasswordPlusOtp,
    PasswordThenOtp,
}

impl AuthFlowMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "disabled" => Self::Off,
            "password" | "password_only" => Self::PasswordOnly,
            "password+otp" | "password_totp" | "password_plus_otp" => Self::PasswordPlusOtp,
            _ => Self::PasswordThenOtp,
        }
    }

    /// Canonical protocol name (as stored in Quick Sudo profiles and
    /// reported by `ssh/settings/get`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::PasswordOnly => "password_only",
            Self::PasswordPlusOtp => "password_plus_otp",
            Self::PasswordThenOtp => "password_then_otp",
        }
    }

    fn allows_otp_after_password(self) -> bool {
        match self {
            Self::Off => false,
            Self::PasswordOnly => false,
            Self::PasswordPlusOtp => true,
            Self::PasswordThenOtp => true,
        }
    }

    /// True when a *merged* prompt (one question that asks for both the
    /// password and the code) must be answered with the concatenation. Both
    /// OTP modes do this: a merged prompt cannot be split, so sending only the
    /// password half always fails. `PasswordOnly` / `Off` keep sending the
    /// password half alone — they never auto-answer OTP by definition.
    pub(super) fn answers_merged_prompt_with_otp(self) -> bool {
        matches!(self, Self::PasswordPlusOtp | Self::PasswordThenOtp)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptKind {
    Password,
    Totp,
    Combined,
}

/// Quick Sudo orchestration settings resolved from a stored connection.
/// Shared behind an `Arc<RwLock<…>>` so runtime settings updates apply to
/// running terminals and exec calls immediately, mirroring tiny-rdm's
/// per-output `resolveAuthOrchestrationForProfile` lookups.
#[derive(Debug, Clone, Default)]
pub struct SudoAuth {
    /// Password piped to `sudo -S`; falls back to the login password.
    pub password: String,
    /// One or more TOTP secrets (newline/semicolon separated in the source
    /// field); rotating OTP selection prefers unused codes with the longest
    /// remaining validity, exactly like tiny-rdm's resolveRotatingOTP.
    /// Usage/committed state lives in the process-global OTP ledgers (see
    /// [`otp_usage_ledger`] / [`committed_otp_ledger`]) so it outlives this
    /// instance.
    pub totp_secrets: Vec<TotpSecret>,
    /// Ledger scope for OTP usage/committed marks (see
    /// [`otp_usage_ledger`] / [`committed_otp_ledger`]): identifies the
    /// credential consumer as `user@host:port` so two connections sharing
    /// one secret do not swallow each other's codes — a code burned on
    /// server A must still be submittable on server B within the same
    /// window. Set by the credential-resolution sites; empty = unscoped
    /// (tests).
    pub otp_ledger_scope: String,
    pub password_prompt_hint: String,
    pub totp_prompt_hint: String,
    pub flow_mode: Option<AuthFlowMode>,
}

impl SudoAuth {
    pub fn new(sudo_password: &str, login_password: &str, totp_secret: &str, hints: Hints) -> Self {
        let password = if sudo_password.trim().is_empty() {
            login_password.to_string()
        } else {
            sudo_password.trim().to_string()
        };
        Self {
            password,
            totp_secrets: parse_totp_secrets(totp_secret),
            password_prompt_hint: hints.password,
            totp_prompt_hint: hints.totp,
            flow_mode: hints.flow_mode,
            ..Default::default()
        }
    }

    pub fn totp_configured(&self) -> bool {
        !self.totp_secrets.is_empty()
    }

    /// True when the auth can answer something (a sudo password or a TOTP
    /// secret); drives whether the in-terminal watcher is attached. With the
    /// flow mode off, a TOTP secret alone answers nothing, so it no longer
    /// justifies arming the watcher.
    pub fn useful(&self) -> bool {
        !self.password.is_empty()
            || (self.totp_configured() && self.flow_mode() != AuthFlowMode::Off)
    }

    pub(super) fn flow_mode(&self) -> AuthFlowMode {
        self.flow_mode.unwrap_or(AuthFlowMode::PasswordThenOtp)
    }

    pub(super) fn classify(&self, prompt: &str) -> Option<PromptKind> {
        classify_auth_prompt(prompt, self)
    }

    /// Returns the answer to send for a detected prompt, if one is configured.
    /// OTP answers go through [`Self::take_totp_answer`], so a code that was
    /// already submitted within its replay window is skipped (with a log)
    /// instead of being injected twice.
    pub(crate) fn answer_for(&self, kind: PromptKind) -> Option<String> {
        match kind {
            PromptKind::Password => (!self.password.is_empty()).then(|| self.password.clone()),
            // Flow off: the OTP secret is never spent on prompts (manual 2FA).
            PromptKind::Totp => {
                if self.flow_mode() == AuthFlowMode::Off {
                    None
                } else {
                    self.totp_answer_logged()
                }
            }
            PromptKind::Combined => {
                let password = (!self.password.is_empty()).then(|| self.password.clone())?;
                // 同一条提问里既有密码又有验证码（合并提问）：两种 OTP 模式都
                // 必须拼接——只回密码对真正的合并提问无法通过。没有可用验证码
                // （未配置密钥 / 落在重放窗口）时退回密码单发，把"再问一次"
                // 交给服务器。
                if self.flow_mode().answers_merged_prompt_with_otp() {
                    match self.totp_answer_logged() {
                        Some(code) => Some(format!("{password}{code}")),
                        None => Some(password),
                    }
                } else {
                    Some(password)
                }
            }
        }
    }

    /// Resolves the OTP answer for a prompt, logging (and skipping) when the
    /// only available code was already committed inside its replay window.
    pub(super) fn totp_answer_logged(&self) -> Option<String> {
        match self.take_totp_answer() {
            Ok(code) => Some(code),
            Err(reason) => {
                eprintln!("[ssh] otp auto-answer skipped: {reason}");
                None
            }
        }
    }

    /// [`Self::answer_for`] 的终端 watcher 变体：OTP 承载型回答（Totp，以及
    /// password_plus_otp 的 Combined）在"唯一可用码已提交且重放窗口未关"时
    /// 返回 `(None, Some(下次可答时刻))`，由调用方推迟到下一个窗口重试；
    /// 其余情形与 `answer_for` 完全一致（永不推迟）。静态恢复码不变，
    /// 重试无意义，同样不推迟。
    pub(crate) fn answer_for_with_retry(&self, kind: PromptKind) -> (Option<String>, Option<u64>) {
        let otp_bearing = self.flow_mode() != AuthFlowMode::Off
            && (matches!(kind, PromptKind::Totp)
                || (kind == PromptKind::Combined
                    && self.flow_mode().answers_merged_prompt_with_otp()));
        if !otp_bearing {
            return (self.answer_for(kind), None);
        }
        match self.take_totp_answer() {
            Ok(code) => {
                let answer = if kind == PromptKind::Combined {
                    format!("{}{code}", self.password)
                } else {
                    code
                };
                (Some(answer), None)
            }
            Err(reason) => {
                eprintln!("[ssh] otp auto-answer deferred: {reason}");
                (None, self.otp_retry_boundary())
            }
        }
    }

    /// 下一个 TOTP 窗口边界（+1s 余量）：此刻之后 `take_totp_answer` 会选中
    /// 全新窗口的码。仅当存在轮转密钥时才有意义；纯静态密钥返回 None。
    fn otp_retry_boundary(&self) -> Option<u64> {
        let now = unix_now();
        let period = self
            .totp_secrets
            .iter()
            .filter_map(|secret| match secret {
                TotpSecret::Key { period, .. } => Some(*period),
                TotpSecret::Static(_) => None,
            })
            .min()?;
        Some((now / period + 1) * period + 1)
    }

    /// Picks the code to auto-submit for an OTP prompt, or explains why the
    /// submission must be skipped: a code that was already injected while its
    /// acceptance window (±1 step) is still open must not be replayed
    /// (tiny-rdm's isOTPUsageMarked semantics, hardened to a hard skip). The
    /// chosen code is marked committed on return, so later prompts in the
    /// same window are skipped instead of re-submitting it.
    pub(crate) fn take_totp_answer(&self) -> Result<String, String> {
        let now = unix_now();
        let selection = self
            .current_totp_selection()
            .ok_or_else(|| "no TOTP secret is configured or currently valid".to_string())?;
        let mut committed = committed_otp_ledger()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        committed.retain(|_, until| is_totp_in_replay_window(now, *until));
        let key = format!(
            "{}|{}|{}",
            self.otp_ledger_scope, selection.fingerprint, selection.code
        );
        if let Some(until) = committed.get(&key).copied() {
            return Err(format!(
                "code {} was already submitted and its replay window (±1 step) stays open for {}s",
                selection.code,
                until.saturating_sub(now)
            ));
        }
        committed.insert(
            key,
            totp_replay_window_expiry(selection.valid_until, selection.period),
        );
        Ok(selection.code)
    }

    /// Picks the OTP code to use right now across all configured secrets,
    /// sorted like tiny-rdm's resolveRotatingOTP: unused codes first, then
    /// longest remaining validity, configured order as the stable tie-break;
    /// when every candidate is used the best code is still returned as a
    /// fallback (the replay guard in [`Self::take_totp_answer`] decides
    /// whether it may actually go out). Selections are marked in the
    /// process-global usage ledger until their window expires, so
    /// back-to-back MCP exec calls — each resolving a fresh `SudoAuth` —
    /// keep rotating to an unused secret instead of resubmitting the first
    /// secret's code.
    pub(super) fn current_totp_selection(&self) -> Option<TotpSelection> {
        let now = unix_now();
        // Scope marks to this auth's credential consumer so the rotation
        // ledger is shared across calls/sessions for the same target while
        // distinct targets (which validate codes independently) never
        // interfere.
        let scope = self.otp_ledger_scope.as_str();
        let mut usage = otp_usage_ledger()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        usage.retain(|_, valid_until| *valid_until > now);

        struct Candidate {
            fingerprint: String,
            code: String,
            valid_until: u64,
            period: u64,
            remaining: u64,
            usage_key: String,
            used: bool,
        }
        let mut candidates: Vec<Candidate> = self
            .totp_secrets
            .iter()
            .map(|secret| {
                let fingerprint = otp_secret_fingerprint(secret);
                match secret {
                    TotpSecret::Static(code) => {
                        let valid_until = now.saturating_add(OTP_STATIC_WINDOW);
                        // Static codes carry no aligned window: keying usage
                        // on the fingerprint alone keeps marks stable across
                        // calls whose `now + window` boundary moved.
                        Candidate {
                            usage_key: format!("{scope}|{fingerprint}|{code}"),
                            fingerprint,
                            code: code.clone(),
                            valid_until,
                            period: OTP_STATIC_WINDOW,
                            remaining: OTP_STATIC_WINDOW,
                            used: false,
                        }
                    }
                    TotpSecret::Key {
                        key,
                        digits,
                        period,
                        algorithm,
                    } => {
                        let counter = now / period;
                        let valid_until = (counter + 1) * period;
                        let code = hotp(key, counter, *digits, *algorithm);
                        Candidate {
                            usage_key: format!("{scope}|{fingerprint}|{valid_until}|{code}"),
                            fingerprint,
                            code,
                            valid_until,
                            period: *period,
                            remaining: valid_until.saturating_sub(now),
                            used: false,
                        }
                    }
                }
            })
            .filter(|candidate| candidate.valid_until > now)
            .collect();
        for candidate in &mut candidates {
            candidate.used = usage.contains_key(&candidate.usage_key);
        }
        // `sort_by` is stable, so the configured secret order stays the
        // tie-break, exactly like tiny-rdm's ref comparison.
        candidates.sort_by(|a, b| a.used.cmp(&b.used).then(b.remaining.cmp(&a.remaining)));
        let chosen = candidates.first()?;
        usage.insert(chosen.usage_key.clone(), chosen.valid_until);
        Some(TotpSelection {
            fingerprint: chosen.fingerprint.clone(),
            code: chosen.code.clone(),
            valid_until: chosen.valid_until,
            period: chosen.period,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Hints {
    pub password: String,
    pub totp: String,
    pub flow_mode: Option<AuthFlowMode>,
}

/// Advances the sudo keepalive consecutive-failure counter: success resets
/// it, a failure increments it, and reaching
/// [`SUDO_KEEPALIVE_MAX_FAILURES`] returns `None` — the caller must stop the
/// loop and drop its registration.
pub fn keepalive_failure_step(failures: u32, succeeded: bool) -> Option<u32> {
    if succeeded {
        return Some(0);
    }
    let next = failures + 1;
    (next < SUDO_KEEPALIVE_MAX_FAILURES).then_some(next)
}
