//! Local file watchers backing the SFTP "open in external editor" flow.
//!
//! The frontend downloads a remote file into `<downloads>/remote-edit/<ts>/`,
//! opens it with the OS default application and registers a watcher here
//! (`watch/start`). When the external editor saves, the watcher confirms the
//! content really changed (fingerprint compare, not just mtime noise) and
//! pushes a `watch/file-modified` event so the workbench can offer to upload
//! it back. Watchers are desktop-only: on web/docker the sidecar does not run
//! on the user's machine, so `watch/start` refuses up front.
//!
//! Lifecycle mirrors the transfer registry: watchers are keyed per
//! `{sessionId}:{localPath}` (one watcher per file per session — restarting a
//! watch re-baselines the fingerprint), torn down by `watch/stop`,
//! `watch/stop-all` and the `ssh/session/close` hook in main.rs, and self-heal
//! when the watched file disappears or the owning session dies.
//!
//! Security: a watchId is a bearer token, so the real boundary is the path
//! origin, not "whoever holds the id". Both `watch/start` and `watch/upload`
//! re-check via `sftp_ext::validate_remote_edit_path` that the local file is
//! canonicalized inside the plugin's own `remote-edit/` download directory —
//! otherwise any locally readable file could be watched and pushed to a
//! remote host through `upload_back`.

use std::collections::HashMap;
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use notify::Watcher as _;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::sync::RwLock as AsyncRwLock;

mod fingerprint;
mod runtime;
mod types;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use fingerprint::*;
#[allow(unused_imports)]
pub(crate) use runtime::*;
#[allow(unused_imports)]
pub(crate) use types::*;
