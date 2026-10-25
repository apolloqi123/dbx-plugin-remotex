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
use russh::{ChannelMsg, Disconnect, MethodKind, MethodSet};
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
