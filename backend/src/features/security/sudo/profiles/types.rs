//! Quick Sudo profiles: types layer.

use super::*;

pub(super) const STORAGE_VERSION: u64 = 2;

pub(super) const CRYPTO_SCHEME: &str = "aead-v1";

pub(super) const FILE_NAME: &str = "quick-sudo-profiles.json";

/// Cap aligned with the workbench quick-commands limit: enough for fleet
/// Segmentation (per environment / per team) without unbounded growth.
pub const MAX_PROFILES: usize = 20;

pub(super) const MAX_NAME_LEN: usize = 64;

/// One named Quick Sudo preset. Secrets are stored verbatim (trimming happens
/// at use time in `SudoAuth::new`, mirroring the connection secret pipeline).
#[derive(Debug, Clone, PartialEq)]
pub struct SudoProfile {
    pub id: String,
    pub name: String,
    pub sudo_password: String,
    pub totp_secret: String,
    /// Canonical `AuthFlowMode` name (`password_then_otp` / …).
    pub auth_flow_mode: String,
    pub password_prompt_hint: String,
    pub totp_prompt_hint: String,
    pub sudo_use_pty: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SudoProfileStore {
    pub profiles: Vec<SudoProfile>,
    /// connectionId -> profileId. Dangling entries (profile deleted by a
    /// hand-edited file) resolve to "no binding" at lookup time instead of
    /// failing the connection.
    pub bindings: HashMap<String, String>,
}
