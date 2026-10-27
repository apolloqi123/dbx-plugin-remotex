use crate::features::agent::approvals as agent_approvals;
use crate::features::agent::terminal as agent_terminal;
use crate::features::metrics::history as metrics_history;
use crate::features::recording as session_recording;
use crate::features::security::audit as audit_log;
use crate::features::security::sudo::download as sudo_download;
use crate::features::security::sudo::profiles as sudo_profiles;
use crate::features::transfer::compress as transfer_compress;
use crate::features::transfer::history as transfer_history;
use crate::filesystem::local::downloads as local_downloads;
use crate::filesystem::sftp::ext as sftp_ext;
use crate::filesystem::sftp::name as sftp_name;
use crate::filesystem::sftp::raw as sftp_raw;
use crate::filesystem::sftp::tree as sftp_tree;
use crate::sessions::ssh::algorithms as ssh_algorithms;
pub(crate) mod algorithms;

mod auth;
#[cfg(test)]
mod channel_error_tests;
mod container_terminal;
#[cfg(test)]
mod container_terminal_tests;
mod connection;
mod download;
pub(crate) mod exec;
mod execution;
mod filesystem;
mod forwarding;
mod observability;
mod recording;
mod session;
mod settings;
mod sftp_format;
mod sftp_support;
mod terminal_support;
#[cfg(test)]
mod tests;
mod transfer;
mod transfer_util;
mod tree_download;
mod tree_support;
mod upload;

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use dbx_plugin_sdk::{
    host_client, PluginEmitter, PluginError, UserInputAnswer, UserInputOption, UserInputPrompt,
    HOST_REQUEST_USER_INPUT_FEATURE, HOST_REQUEST_USER_INPUT_METHOD,
};
use russh::client::{self, AuthResult, Handle};
use russh::keys::agent::{client::AgentClient, AgentIdentity};
use russh::keys::ssh_key::HashAlg;
use russh::keys::{decode_secret_key, key::PrivateKeyWithHashAlg};
use russh::{ChannelMsg, ChannelOpenFailure, Disconnect, MethodKind, MethodSet};
use russh_sftp::client::{Config as SftpConfig, SftpSession};
use russh_sftp::protocol::FileType;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex, RwLock as AsyncRwLock};
use tokio::time::Instant;
use uuid::Uuid;

use auth::*;
#[cfg(test)]
use crate::core::keys::decode_error::looks_like_public_key;
use sftp_format::*;
pub(crate) use sftp_format::{
    classify_raw_kind, format_permissions, lookup_owner_group_names, RawSftpClient,
};
pub(crate) use sftp_support::sftp_client_config;
use sftp_support::*;
use terminal_support::*;
pub(crate) use terminal_support::{connection_id_param, filesystem_path};
pub(crate) use transfer_util::apply_preserved_permissions;
use transfer_util::*;
use tree_support::*;

use self::exec::{
    AuthFlowMode, ExecOutcome, Hints, SudoAuth, PLAIN_EXEC_TIMEOUT, SUDO_EXEC_TIMEOUT,
};
use crate::core::host_key::{HostKeyState, HostKeyVerifier};
use crate::core::model::{
    normalize_remote_path, path_from_sftp_uri, sftp_uri, AuthenticationMethod, SessionOpenRequest,
    SftpEntry, StoredConnection, SudoSource, TerminalFrame, TerminalStream, MAX_TRANSFER_SIZE,
    TERMINAL_REPLAY_LIMIT, TRANSFER_CHUNK_SIZE,
};
use crate::core::otp;
use crate::core::otp_store;
use crate::features::agent::terminal::{
    AgentDecision, AgentTerminalMode, CommandRisk, TerminalRecorder,
};
use crate::features::automation::triggers;
use crate::features::forwarding::forward;
use crate::features::metrics;
use crate::features::terminal::highlight_rules;
use crate::features::terminal::quick_commands;
use crate::features::terminal::startup_commands;
use crate::features::transfer::compress::CompressionMode;
use crate::features::transfer::throttle::Throttle;
use crate::filesystem::sftp::name::NameEncoding;

/// True when a channel-open refusal is the server's transient concurrency
/// guard rather than a policy denial. OpenSSH reports `MaxSessions` exhaustion
/// as `AdministrativelyProhibited` on most builds, but some versions map the
/// same "no more sessions" path to `ConnectFailed` (the server could not start
/// another session); rapid container/tab switching can hit either while the
/// previous channels are still winding down. A bounded retry is harmless for
/// both, while real policy denials (unknown channel type, disconnect) still
/// fail immediately.
pub(crate) fn is_transient_channel_open_failure(error: &russh::Error) -> bool {
    matches!(
        error,
        russh::Error::ChannelOpenFailure(
            ChannelOpenFailure::AdministrativelyProhibited
                | ChannelOpenFailure::ResourceShortage
                | ChannelOpenFailure::ConnectFailed
        )
    )
}

/// Opens a session channel, retrying transient concurrency refusals with a
/// short backoff so the previous pane's close can land. Permanent policy
/// denials fall through unchanged after the bounded retries.
pub(crate) async fn open_session_channel(
    handle: &Handle<SshClient>,
) -> Result<russh::Channel<client::Msg>, russh::Error> {
    // A transient refusal means the server is still counting sessions that
    // our previous panes closed; give those closes up to ~6s to land. The
    // shared counter de-synchronises retry loops from concurrent panes, so
    // they do not keep colliding on the same free slot.
    use std::sync::atomic::{AtomicU64, Ordering};
    static RETRY_TICK: AtomicU64 = AtomicU64::new(0);
    const DELAYS_MS: [u64; 6] = [100, 250, 500, 1_000, 1_500, 2_500];
    for (attempt, delay_ms) in DELAYS_MS.iter().enumerate() {
        match handle.channel_open_session().await {
            Ok(channel) => return Ok(channel),
            Err(error) if is_transient_channel_open_failure(&error) => {
                let attempt = attempt + 1;
                let jitter_ms = RETRY_TICK.fetch_add(23, Ordering::Relaxed) % 71;
                let wait_ms = delay_ms + jitter_ms;
                eprintln!(
                    "[dbx-plugin-sshx] SSH channel open refused ({error}); retry {attempt}/{} in {wait_ms}ms",
                    DELAYS_MS.len()
                );
                tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            }
            Err(error) => return Err(error),
        }
    }
    handle.channel_open_session().await
}

mod dial;
mod handler;
mod prompt;
mod state;
mod support;
mod terminal;

#[allow(unused_imports)]
pub(crate) use dial::*;
#[allow(unused_imports)]
pub(crate) use handler::*;
#[allow(unused_imports)]
pub(crate) use prompt::*;
#[allow(unused_imports)]
pub(crate) use state::*;
#[allow(unused_imports)]
pub(crate) use support::*;
#[allow(unused_imports)]
pub(crate) use terminal::*;
