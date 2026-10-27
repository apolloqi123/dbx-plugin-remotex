//! Remote command execution with Quick Sudo support, ported from the
//! tiny-rdm auth-orchestration pipeline: `sudo -S` password injection over
//! stdin plus automatic follow-up answers for 2FA/TOTP prompts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use data_encoding::{BASE32, HEXLOWER};
use hmac::{Hmac, KeyInit, Mac};
use russh::client::Handle;
use russh::ChannelMsg;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use tokio::sync::watch;

mod auth;
mod metrics;
mod run;
mod streaming;
mod terminal;

#[cfg(test)]
mod rotation_tests;
#[cfg(test)]
mod stream_tests;
#[cfg(test)]
mod tests;

use crate::sessions::ssh::{channel_ledger, open_session_channel_labeled, SshClient};

pub const SUDO_EXEC_TIMEOUT: Duration = Duration::from_secs(90);
pub const PLAIN_EXEC_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_AUTH_ROUNDS: u32 = 3;

/// Cancellation outcome for the `*_cancellable` exec variants. Reaching the
/// caller as a regular `Err` is deliberate: every exec error path already
/// funnels through [`abort_exec_channel`], so a cancelled run closes the
/// channel (and lets sshd reap the remote process) instead of leaking it.
pub const EXEC_CANCELLED_MESSAGE: &str = "Remote command was cancelled";

pub use auth::*;
pub use metrics::*;
pub use run::*;
pub use streaming::*;
pub use terminal::*;

pub use crate::core::shell_quote::shell_quote;

const PASSWORD_PROMPT_PATTERS: &[&str] = &[
    "password:",
    "passphrase:",
    "pass phrase",
    "[sudo] password for",
    "密码:",
    "密码：",
    // 「口令」与既有「密码」同构（评审 round-3 经确认补录）；两档冒号宽度
    // 与「密码」一致。带冒号限定避免误中「口令错误：」这类失败消息。
    "口令:",
    "口令：",
];

const TOTP_PROMPT_PATTERS: &[&str] = &[
    "verification code",
    "verification code:",
    "otp:",
    // JumpServer 堡垒机（koko）的 MFA 提问：[OTP Code]: / [RADIUS Code]:，
    // 指令行 "Please Enter MFA Code."（见 koko pkg/auth/mfa_option.go）。
    "otp code",
    "mfa code",
    // 阿里云 Bastionhost 官方 SSH 运维提问原文："Please Input Mfa Code (SMS):"。
    // "mfa code" 子串本已命中，显式收录防止子串列表收紧后静默失效；
    // 说明行 "Two-Step Verification required" 刻意不入表——那是上下文不是输入位。
    "please input mfa code",
    "mfa:",
    "totp:",
    "2fa code",
    "one-time password",
    "one time password",
    "authentication code",
    "动态密码",
    "验证码",
    "一次性密码",
];

const AUTH_FAILURE_MARKERS: &[&str] = &[
    "sorry, try again",
    "authentication failure",
    "incorrect password",
];

/// Wraps a command in `sh -c '…'` with POSIX single-quote escaping so sudo
/// receives one argument and metacharacters cannot escape the quoting.
pub fn sanitize_sudo_command(command: &str) -> String {
    let escaped = command.replace('\'', r"'\''");
    format!("sh -c '{escaped}'")
}

/// Nominal replay window for static OTP codes, which carry no period of
/// their own (mirrors tiny-rdm's `otpReuseWindowFallback`).
const OTP_STATIC_WINDOW: u64 = 30;

/// How often the per-connection sudo keepalive refreshes the timestamp:
/// 4 minutes, comfortably under every common sudo `timestamp_timeout`
/// (tiny-rdm's sudoKeepaliveLoop refreshes on the same order).
pub const SUDO_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(240);

/// Consecutive failed `sudo -nv` validations after which the keepalive loop
/// stops itself (the timestamp is gone; the next sudo exec re-registers).
pub const SUDO_KEEPALIVE_MAX_FAILURES: u32 = 2;

/// Hard cap for custom prompt hints so a runaway `settings/set` update (or a
/// malformed host-provided config) cannot store unbounded junk on the
/// connection and every live session.
pub const MAX_PROMPT_HINT_LEN: usize = 512;

const SUDO_WAIT_TIMEOUT_MESSAGE: &str =
    "Timed out waiting for the remote command to finish. A SIGTERM was sent to \
     the remote process group and the channel was closed; on a server that does \
     not support the SSH signal request, check for stray processes or \
     package-manager locks before retrying. For long jobs start them detached \
     (ssh_run_bg + ssh_task_status) instead of extending the wait.";

/// Per-stream output ceiling for `run_to_completion` when the caller is not
/// streaming: a hostile or merely over-productive remote (`cat /dev/urandom`)
/// would otherwise balloon sidecar memory until the deadline; past the cap
/// chunks are dropped while the read loop keeps draining so the channel never
/// back-pressures. Sink-backed runs retain only a small error tail instead
/// (see `run::MAX_STREAM_TAIL_BYTES`).
const MAX_EXEC_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
