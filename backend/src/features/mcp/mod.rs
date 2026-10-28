//! MCP stdio server mode (`--mcp`): exposes SSH exec and SFTP tools to MCP
//! clients over newline-delimited JSON-RPC, mirroring tiny-rdm's MCP tool
//! surface (ssh_exec, ssh_exec_sudo, sftp_*) adapted to inline connection
//! parameters with a per-process connection pool.

use crate::features::agent::approvals as agent_approvals;
use crate::features::agent::bridge as app_bridge;
use crate::features::agent::terminal as agent_terminal;
use crate::features::mcp::safety as mcp_safety;
use crate::features::security::audit as audit_log;
use crate::features::security::sudo::allowlist as sudo_allowlist;
use crate::features::security::sudo::profiles as sudo_profiles;
use crate::filesystem::sftp::name as sftp_name;
use crate::filesystem::sftp::raw as sftp_raw;
pub(crate) mod safety;

mod args;
mod bridge;
mod catalog;
mod connections;
mod dispatch;
mod execution;
mod policy;
mod scope;
mod settings;
mod sftp;
mod state;
mod stdio;

#[cfg(test)]
mod dbx_bridge_tests;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::future::Future;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use dbx_plugin_sdk::PluginEmitter;
use russh::client::Handle;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileType;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex as AsyncMutex, RwLock as AsyncRwLock};

pub(crate) use args::arg_bool;
use args::*;
#[allow(unused_imports)]
use bridge::*;
pub(crate) use catalog::{tool_definitions, tool_definitions_for_host};
use catalog::*;
use connections::*;
#[allow(unused_imports)]
use dispatch::*;
#[allow(unused_imports)]
use execution::*;
#[allow(unused_imports)]
use policy::*;
use scope::*;
#[allow(unused_imports)]
use settings::*;
#[allow(unused_imports)]
use sftp::*;
pub use state::McpState;
use state::*;
pub use stdio::run_mcp_stdio;
#[allow(unused_imports)]
use stdio::*;

use crate::sessions::ssh::exec::{self, AuthFlowMode, Hints, SudoAuth};
use crate::core::host_key::HostKeyVerifier;
use crate::core::model::{
    normalize_remote_path, AuthenticationMethod, JumpHost, StoredConnection, SudoSource,
};
use crate::core::preferences;
use crate::features::automation::multi_exec;
use crate::features::mcp::safety::CommandRisk;
use crate::sessions::ssh::{
    classify_raw_kind, sftp_client_config, RawSftpClient, SshClient, SshRuntime,
    NO_TERMINAL_SESSION_MESSAGE,
};

const PROTOCOL_VERSION: &str = "2024-11-05";
