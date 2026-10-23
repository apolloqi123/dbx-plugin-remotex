//! Global Quick Sudo profiles: named credential/policy presets persisted in
//! `<plugin_data_dir>/quick-sudo-profiles.json` and selectable per connection
//! (tiny-rdm's global Manual Sudo + per-profile QuickSudo override, adapted
//! to the DBX plugin shape). Secrets are sealed at rest (field-level
//! AES-256-GCM via `crate::core::vault`, v2 file format); every view returned to
//! callers reports `sudoPasswordSet` / `totpConfigured` booleans and never
//! echoes the values.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::core::exec::{self, AuthFlowMode, SudoAuth};
use crate::core::vault::{
    delete_keychain_dek, keyfile_path, resolve_provider, KeyProvider, KeyStorage, KeyfileProvider,
    Vault, FIELD_SUDO_PASSWORD, FIELD_TOTP_SECRET,
};

mod json;
mod ops;
mod storage;
mod types;

#[cfg(test)]
mod tests;

pub use json::*;
pub use ops::*;
pub use storage::*;
pub use types::*;
