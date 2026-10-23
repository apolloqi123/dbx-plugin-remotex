//! User-facing port mapping over a live SSH session (Xshell-style 端口映射,
//! ssh(1) -L/-R/-D parity).
//!
//! Three forward directions:
//! - `local` (-L): we bind `listen_host:listen_port` on the client machine and
//!   open a `direct-tcpip` channel per accepted connection; the server then
//!   dials `target_host:target_port` from its side.
//! - `remote` (-R): we ask the server to bind the port via the `tcpip-forward`
//!   global request; incoming `forwarded-tcpip` channels are relayed to
//!   `target_host:target_port` dialed from the *client* machine.
//! - `dynamic` (-D): a local SOCKS5 CONNECT listener opens direct-tcpip
//!   channels for destinations chosen by each client request.
//!
//! Every mapping lives in the sidecar registry keyed by forward id. Session
//! mappings die with their terminal; independent mappings own a PTY-free SSH
//! transport and survive terminal closure until stopped or disconnected.
//! All protocol-facing fields are camelCase, matching the sibling domains.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dbx_plugin_sdk::PluginEmitter;
use russh::client::Handle;
use serde_json::{json, Value};
use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::sessions::ssh::{RemoteForwardTable, SshClient};

mod registry;
mod relay;
mod spec;
mod types;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use registry::*;
#[allow(unused_imports)]
pub(crate) use relay::*;
#[allow(unused_imports)]
pub(crate) use spec::*;
#[allow(unused_imports)]
pub(crate) use types::*;
