//! Plain-text Telnet client sessions (RFC 854/855 subset, nyaterm-parity P2-3).
//!
//! Runtime layout mirrors `local_terminal.rs`: one entry per live session in
//! the runtime's own table (a Telnet connection carries a TCP stream and can
//! never live on the SSH `SessionEntry`), keystrokes arrive on the sequenced
//! binary channel `telnet/terminal/in/{sessionId}`, output answers on
//! `telnet/terminal/out/{sessionId}` with the shared 9-byte `TerminalFrame`
//! prefix backed by a [`ReplayBuffer`], and lifecycle changes surface as
//! `telnet/session/state` events (`connecting`/`connected`/`closed`/`error`).
//!
//! IAC handling is a hand-written byte state machine (zero new deps):
//! - `WILL ECHO`/`WILL SGA` → `DO`; `DO NAWS` → `WILL NAWS` (+ NAWS updates on
//!   resize); every other offer/request is politely declined (`DONT`/`WONT`);
//! - `IAC IAC` unescapes to a literal `0xFF` payload byte;
//! - the parser is chunk-crossing safe: an IAC sequence split across reads is
//!   completed from the next chunk (state machine, not per-chunk scans).
//!
//! Auto-login reuses the SSH expect-rule engine (`triggers.rs`) verbatim —
//! two input forms lower onto the same engine, so there is exactly one
//! matcher, one validator and one secret policy:
//! - **Rule form** (`autoLogin.rules`): the same tssh/JSON rule text the SSH
//!   side accepts, with `sendSecretKey` slots resolved from the start
//!   request's `secrets` array instead of the connection vault.
//! - **Declarative form** (`autoLogin.declarative`, NyaTerm-parity P0-1):
//!   username/password prompt regexes plus the credentials, success/failure
//!   regexes and a retry budget. The prompts become engine stages (username
//!   as plaintext `sendText`, password through the `trigger_answer_1` secret
//!   slot so it never appears in logs, events or `Debug`), while success and
//!   failure are supervised by [`DeclarativeWatch`] in the read loop: a
//!   failure hit re-arms answering against the retry budget (the host
//!   re-prompts; the engine cursor already wraps), a success hit after any
//!   answer publishes `telnet/auto_login {status:"success"}` (the workbench
//!   localizes the notice), and a failure hit past the budget closes the
//!   session into the existing exit overlay with a readable, content-free
//!   reason. Blank regex fields fall back to NyaTerm's built-in prompt
//!   vocabulary; every regex compiles through the shared `triggers` gate so
//!   an invalid pattern fails `telnet/start` with a readable error (D7).
//!
//! The decision/segment protocol is byte-identical to the SSH read loop
//! (`ssh/trigger` becomes `telnet/trigger` and never carries answer content).
//!
//! Safety note: Telnet is cleartext — credentials typed into it travel
//! unencrypted. The plugin surfaces that warning in the UI and never logs
//! keystrokes or auto-login answers.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

mod input;
mod login;
mod parser;
mod pump;
mod runtime;
mod types;
mod watch;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use input::*;
#[allow(unused_imports)]
pub use login::*;
#[allow(unused_imports)]
pub use parser::*;
#[allow(unused_imports)]
pub use pump::*;
#[allow(unused_imports)]
pub use runtime::*;
#[allow(unused_imports)]
pub use types::*;
#[allow(unused_imports)]
pub use watch::*;

use base64::Engine;
use dbx_plugin_sdk::PluginEmitter;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};

use crate::core::exec::strip_ansi_control_sequences;
use crate::core::model::TerminalStream;
use crate::features::automation::triggers;
use crate::sessions::ssh::ReplayBuffer;

/// Dial timeout for the initial TCP connect (contract P2-3).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Telnet IAC escape byte.
pub const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
/// Negotiated options: echo, suppress-go-ahead, window-size (RFC 1073).
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_NAWS: u8 = 31;

/// Built-in prompt vocabulary, mirroring NyaTerm's default regexes so a
/// declarative config works without any pattern field filled in. Matched
/// with the same regex engine the expect stages use (substring `find`
/// semantics over a rolling tail).
///
/// Both defaults are end-anchored (NyaTerm `auto_login.rs` matches its
/// defaults against line candidates the same way): a live prompt is keyword +
/// punctuation + only whitespace up to the end of the rolling buffer. A
/// `Last login: <time>` MOTD banner therefore cannot trigger an answer — the
/// trailing timestamp breaks `\s*$` — which is NyaTerm's explicit
/// `last_login_regex` exclusion expressed structurally (the Rust `regex`
/// engine has no look-around for a line-start negative guard, and a `^` anchor
/// would miss prompts split across chunk boundaries, where the per-chunk
/// normalization trims the separating newline). A banner line consisting of
/// only `Last login:` with the timestamp on the next line would still match —
/// the same unavoidable fused-line behavior NyaTerm's last-line exclusion
/// tolerates — but that banner shape does not occur in practice.
const DECL_USERNAME_PROMPT_DEFAULT: &str = r#"(?i)(?:user\s*name|username|login|logon|account|userid|user\s*id|用户名|帐号|账号|登录|登入)\s*[:：>]\s*$"#;
const DECL_PASSWORD_PROMPT_DEFAULT: &str =
    r#"(?i)(?:password|passwd|passcode|passphrase|pin|密码|口令)\s*[:：>]\s*$"#;
const DECL_SUCCESS_DEFAULT: &str = r#"[$#>]\s*$"#;
const DECL_FAILURE_DEFAULT: &str =
    r#"(?i)(login\s+incorrect|authentication\s+failed|access\s+denied|密码错误|认证失败)"#;
/// Retry budget ceiling: NyaTerm is `u8`-unbounded; the plugin caps the
/// abuse surface (each retry re-sends credentials).
const DECL_MAX_RETRIES: u8 = 10;
/// Watch rolling tail cap in chars: enough context for prompt/failure
/// regexes without unbounded growth on chatty banners.
const DECL_WATCH_TAIL_CHARS: usize = 4096;

/// Hard cap for one `telnet/write` / `telnet/terminal/in/{id}` payload. The
/// JSON fallback and the binary channel share the base64 path, so the limit
/// lives here rather than at each call site — the host's frame ceiling is not
/// a substitute for an input-channel bound of our own.
pub const WRITE_PAYLOAD_LIMIT: usize = 64 * 1024;
