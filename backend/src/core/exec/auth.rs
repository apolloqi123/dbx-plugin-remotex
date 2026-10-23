//! Exec auth layer.

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

#[derive(Debug, Clone)]
pub enum TotpSecret {
    /// RFC 6238 shared secret with parameters.
    Key {
        key: Vec<u8>,
        digits: u32,
        period: u64,
        algorithm: TotpAlgorithm,
    },
    /// Static numeric code (4-10 digits) that is not time-based.
    Static(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
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

/// One OTP selection with the data the replay guard needs.
pub(super) struct TotpSelection {
    /// Ledger-key prefix identifying the secret the code came from.
    pub(super) fingerprint: String,
    pub(super) code: String,
    /// Unix seconds after which the code's own window expires.
    pub(super) valid_until: u64,
    /// Secret period (or the static fallback window).
    period: u64,
}

/// Process-global ledger of OTP selections, keyed by usage key (see
/// [`current_totp_selection`]) with the selection's window expiry as the
/// value. Global — not per `SudoAuth` — because MCP exec calls resolve a
/// fresh instance per call while rotation must survive across calls, which
/// mirrors tiny-rdm's service-level `markOTPUsage`; in-memory only, cleared
/// on sidecar restart.
pub(super) fn otp_usage_ledger() -> &'static Mutex<HashMap<String, u64>> {
    static LEDGER: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Process-global record of codes already auto-submitted, keyed by
/// `scope|secret-fingerprint|code` with their replay-window expiry
/// (`validUntil + period`, i.e. including the ±1-step acceptance slack).
/// Global for the same reason as [`otp_usage_ledger`].
pub(super) fn committed_otp_ledger() -> &'static Mutex<HashMap<String, u64>> {
    static LEDGER: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Serializes tests that touch the process-global OTP ledgers and hands
/// each one a clean slate: hold the returned guard for the whole test so
/// parallel tests neither share marks nor wipe each other mid-run.
#[cfg(test)]
pub(super) fn otp_ledger_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: OnceLock<Mutex<()>> = OnceLock::new();
    let serial = SERIAL.get_or_init(|| Mutex::new(()));
    let guard = serial.lock().unwrap_or_else(|poison| poison.into_inner());
    otp_usage_ledger()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clear();
    committed_otp_ledger()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clear();
    guard
}

/// Stable ledger-key prefix for a secret: a short SHA-256 fingerprint of
/// its canonical form (key material, digits, period, algorithm — or the
/// static code), so marks written by one resolved `SudoAuth` match lookups
/// from the next call regardless of how the secret list was ordered or
/// merged. Only the fingerprint is stored, never the secret.
pub(super) fn otp_secret_fingerprint(secret: &TotpSecret) -> String {
    let canonical = match secret {
        TotpSecret::Key {
            key,
            digits,
            period,
            algorithm,
        } => format!("k:{}:{digits}:{period}:{algorithm:?}", HEXLOWER.encode(key)),
        TotpSecret::Static(code) => format!("s:{code}"),
    };
    let digest = Sha256::digest(canonical.as_bytes());
    HEXLOWER.encode(&digest)[..16].to_string()
}

/// Ledger scope for the connection a resolved [`SudoAuth`] serves: the OTP
/// usage/committed marks are keyed per target so back-to-back calls against
/// the same host rotate across their shared ledger while a second host
/// reusing the same secret is never blocked by the first host's history.
pub fn otp_ledger_scope_for(username: &str, host: &str, port: u16) -> String {
    format!("{username}@{host}:{port}")
}

/// Replay-window expiry for a committed code: its nominal validity plus one
/// extra step, because servers commonly accept the previous and next
/// window's code alongside the current one (the ±1-step slack).
pub fn totp_replay_window_expiry(valid_until: u64, period: u64) -> u64 {
    valid_until.saturating_add(period)
}

/// True while a committed code must not be auto-submitted again.
pub fn is_totp_in_replay_window(now: u64, committed_until: u64) -> bool {
    now <= committed_until
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

pub(super) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

/// Parses one or more TOTP secrets separated by newlines or semicolons
/// (tiny-rdm's multi-secret `TOTPSecretRefs` equivalent).
pub fn parse_totp_secrets(input: &str) -> Vec<TotpSecret> {
    input
        .split(['\n', '\r', ';'])
        .filter_map(parse_totp_secret)
        .collect()
}

/// Parses a TOTP secret reference: an `otpauth://totp/…` URI, a base32 key,
/// or a static numeric code. Returns `None` for empty or unusable values.
pub fn parse_totp_secret(input: &str) -> Option<TotpSecret> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed
        .strip_prefix("otpauth://")
        .or_else(|| trimmed.strip_prefix("OTPAUTH://"))
    {
        return parse_otpauth(rest);
    }
    if is_numeric_otp(trimmed) {
        return Some(TotpSecret::Static(trimmed.to_string()));
    }
    let key = BASE32
        .decode(trimmed.to_ascii_uppercase().as_bytes())
        .ok()?;
    if key.is_empty() {
        return None;
    }
    Some(TotpSecret::Key {
        key,
        digits: 6,
        period: 30,
        algorithm: TotpAlgorithm::Sha1,
    })
}

pub(super) fn parse_otpauth(rest: &str) -> Option<TotpSecret> {
    let (scheme_path, query) = rest.split_once('?').unwrap_or((rest, ""));
    if !scheme_path
        .split('/')
        .next()
        .is_some_and(|label| label.eq_ignore_ascii_case("totp"))
    {
        return None;
    }
    let mut secret = None;
    let mut digits = 6_u32;
    let mut period = 30_u64;
    let mut algorithm = TotpAlgorithm::Sha1;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "secret" => {
                secret = BASE32
                    .decode(value.to_ascii_uppercase().as_bytes())
                    .ok()
                    .filter(|decoded| !decoded.is_empty());
            }
            "digits" => digits = value.parse().ok().filter(|d| (6..=8).contains(d))?,
            "period" => period = value.parse().ok().filter(|&p| (1..=120).contains(&p))?,
            "algorithm" => {
                algorithm = match value.to_ascii_uppercase().as_str() {
                    "SHA256" => TotpAlgorithm::Sha256,
                    "SHA512" => TotpAlgorithm::Sha512,
                    _ => TotpAlgorithm::Sha1,
                };
            }
            _ => {}
        }
    }
    Some(TotpSecret::Key {
        key: secret?,
        digits,
        period,
        algorithm,
    })
}

pub(super) fn is_numeric_otp(input: &str) -> bool {
    (4..=10).contains(&input.len()) && input.bytes().all(|byte| byte.is_ascii_digit())
}

pub(super) fn hotp(key: &[u8], counter: u64, digits: u32, algorithm: TotpAlgorithm) -> String {
    let message = counter.to_be_bytes();
    let digest: Vec<u8> = match algorithm {
        TotpAlgorithm::Sha1 => {
            let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(key)
                .expect("HMAC accepts keys of any length");
            mac.update(&message);
            mac.finalize().into_bytes().to_vec()
        }
        TotpAlgorithm::Sha256 => {
            let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
                .expect("HMAC accepts keys of any length");
            mac.update(&message);
            mac.finalize().into_bytes().to_vec()
        }
        TotpAlgorithm::Sha512 => {
            let mut mac = <Hmac<Sha512> as KeyInit>::new_from_slice(key)
                .expect("HMAC accepts keys of any length");
            mac.update(&message);
            mac.finalize().into_bytes().to_vec()
        }
    };
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | digest[offset + 3] as u32;
    let code = binary % 10_u32.pow(digits);
    format!("{code:0width$}", width = digits as usize)
}

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

/// Tracks whether the first authentication factor is already behind us during
/// a keyboard-interactive handshake, mirroring tiny-rdm's per-connection
/// state. `password_then_otp` gates OTP answers on this flag so a bare OTP
/// question cannot be answered before the password step.
#[derive(Debug, Default)]
pub struct KeyboardInteractiveState {
    pub(super) password_answered: bool,
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
