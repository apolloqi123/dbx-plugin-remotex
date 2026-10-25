//! Plugin-level UI preferences persisted in `<plugin_data_dir>/preferences.json`.
//! The workbench iframe is sandboxed (`sandbox="allow-scripts"`, opaque origin),
//! so `window.localStorage` throws and this sidecar file is the only durable
//! store for workbench preferences such as the download directory. The schema
//! is a fixed allowlist — arbitrary keys from the renderer are dropped.

use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Map, Value};

mod sanitize;
pub(crate) mod startup_commands;
mod store;
pub(crate) mod transfer;
mod wallpaper;

pub(crate) mod env;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use sanitize::*;
#[allow(unused_imports)]
pub use startup_commands::*;
#[allow(unused_imports)]
pub use store::*;
#[allow(unused_imports)]
pub use transfer::*;
#[allow(unused_imports)]
pub use wallpaper::*;
