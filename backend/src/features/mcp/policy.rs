//! MCP policy layer.

use super::*;

/// Ceilings for the MCP size settings (tiny-rdm's PreferencesMCPSFTP
/// equivalent): `mcp/settings/set` may lower a limit freely or raise it up
/// to these values, never beyond. The same ceilings clamp the values after
/// loading `mcp-settings.json`, so a corrupted or hand-edited file cannot
/// disable the caps.
pub(crate) const READ_LIMIT_CEILING: u64 = 8 * 1024 * 1024;

pub(crate) const UPLOAD_LIMIT_CEILING: u64 = 2 * 1024 * 1024 * 1024;

pub(crate) const DOWNLOAD_LIMIT_CEILING: u64 = 2 * 1024 * 1024 * 1024;

/// Size preferences for the MCP tools, persisted in
/// `<plugin_data_dir>/mcp-settings.json`:
/// - `max_read_bytes`: default size of a single `sftp_read_file` call
///   (previously the `DEFAULT_READ_BYTES` constant);
/// - `max_download_bytes`: ceiling for an explicit `maxBytes` request,
///   i.e. the most a single read/download may return (previously the
///   `MAX_READ_BYTES` constant);
/// - `max_upload_bytes`: ceiling for the content of a single
///   `sftp_write_file` call (new; writes were previously uncapped);
/// - `local_transfer_root`: confinement root for the local side of
///   `sftp_upload` / `sftp_download`; empty = the default roots (OS temp
///   dir + plugin data dir). See
///   [`ensure_local_transfer_allowed_in`].
///
/// §1.3 permission settings ride the same file (`execPermissionMode`,
/// `connectionScope`), parsed into [`McpPermission`] separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpLimits {
    pub max_read_bytes: u64,
    pub max_upload_bytes: u64,
    pub max_download_bytes: u64,
    pub local_transfer_root: String,
}

impl Default for McpLimits {
    fn default() -> Self {
        Self {
            max_read_bytes: 256 * 1024,
            max_upload_bytes: 16 * 1024 * 1024,
            max_download_bytes: 1024 * 1024,
            local_transfer_root: String::new(),
        }
    }
}

impl McpLimits {
    /// Clamps each field into `1..=ceiling`. Applied after loading the
    /// settings file and after every update so out-of-range values can
    /// never reach the tool implementations.
    pub(crate) fn sanitized(self) -> Self {
        Self {
            max_read_bytes: self.max_read_bytes.clamp(1, READ_LIMIT_CEILING),
            max_upload_bytes: self.max_upload_bytes.clamp(1, UPLOAD_LIMIT_CEILING),
            max_download_bytes: self.max_download_bytes.clamp(1, DOWNLOAD_LIMIT_CEILING),
            local_transfer_root: self.local_transfer_root,
        }
    }
    /// Parses the persisted JSON, falling back per-field to the defaults for
    /// missing or non-numeric entries.
    fn from_json(value: &Value) -> Self {
        let defaults = Self::default();
        let field =
            |key: &str, fallback: u64| value.get(key).and_then(Value::as_u64).unwrap_or(fallback);
        Self {
            max_read_bytes: field("maxReadBytes", defaults.max_read_bytes),
            max_upload_bytes: field("maxUploadBytes", defaults.max_upload_bytes),
            max_download_bytes: field("maxDownloadBytes", defaults.max_download_bytes),
            local_transfer_root: value
                .get("localTransferRoot")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
        }
        .sanitized()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "maxReadBytes": self.max_read_bytes,
            "maxUploadBytes": self.max_upload_bytes,
            "maxDownloadBytes": self.max_download_bytes,
            "localTransferRoot": self.local_transfer_root,
        })
    }

    /// Reads the persisted settings; a missing or corrupted file falls back
    /// to the defaults and never fails.
    pub(crate) fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .map(|value| Self::from_json(&value))
            .unwrap_or_default()
    }
}

/// §1.3 permission settings (`IMPL_PLAN_NETCATTY_PARITY`), persisted in the
/// same `mcp-settings.json` as [`McpLimits`]:
/// - `exec_permission_mode`: `"autonomous"` (default; existing gate-only
///   behavior) or `"confirm"` (write/exec tools additionally raise a
///   human approval challenge before executing);
/// - `connection_scope`: allowlist of connection references (connection id,
///   connection name, or host — ASCII case-insensitive); empty = no
///   restriction. While non-empty, calls whose resolved target is out of
///   scope are refused, inline-credential dialing is refused outright
///   (fail closed), and `ssh_list_connections` only reports in-scope
///   entries.
///
/// Process env vars override the persisted values when present
/// (`DBX_SSH_MCP_PERMISSION_MODE`, `DBX_SSH_MCP_CONNECTION_SCOPE` as a
/// comma-separated list) so MCP clients can pin policy per server entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPermission {
    pub exec_permission_mode: String,
    pub connection_scope: Vec<String>,
}

pub const PERMISSION_MODE_AUTONOMOUS: &str = "autonomous";

pub const PERMISSION_MODE_CONFIRM: &str = "confirm";

/// Cap on scope entries so a pathological list cannot balloon the gate.
pub const CONNECTION_SCOPE_MAX_ENTRIES: usize = 64;

impl Default for McpPermission {
    fn default() -> Self {
        Self {
            exec_permission_mode: PERMISSION_MODE_AUTONOMOUS.to_string(),
            connection_scope: Vec::new(),
        }
    }
}

impl McpPermission {
    /// Parses the permission section of the persisted JSON; missing or
    /// invalid fields fall back to the defaults (a hand-edited file can
    /// loosen nothing below the defaults).
    fn from_json(value: &Value) -> Self {
        let mode = value
            .get("execPermissionMode")
            .and_then(Value::as_str)
            .filter(|text| matches!(*text, PERMISSION_MODE_AUTONOMOUS | PERMISSION_MODE_CONFIRM))
            .unwrap_or(PERMISSION_MODE_AUTONOMOUS);
        let scope = value
            .get("connectionScope")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|entry| entry.trim().to_string())
                    .filter(|entry| !entry.is_empty())
                    .take(CONNECTION_SCOPE_MAX_ENTRIES)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            exec_permission_mode: mode.to_string(),
            connection_scope: scope,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "execPermissionMode": self.exec_permission_mode,
            "connectionScope": self.connection_scope,
        })
    }

    /// Reads the persisted permission section; a missing or corrupted file
    /// falls back to the defaults and never fails.
    pub(crate) fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .map(|value| Self::from_json(&value))
            .unwrap_or_default()
    }
}

/// Writes the combined MCP settings document (size limits + §1.3
/// permission keys) in one pass, so either section's writer can never
/// clobber the other's keys.
pub(crate) fn write_settings_document(
    path: &Path,
    limits: &McpLimits,
    permission: &McpPermission,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut document = json!({});
    {
        let map = document.as_object_mut().expect("json!({}) is an object");
        let limit_entries = limits.clone().to_json();
        if let Some(limit_map) = limit_entries.as_object() {
            for (key, value) in limit_map {
                map.insert(key.clone(), value.clone());
            }
        }
        let permission_entries = permission.clone().to_json();
        if let Some(permission_map) = permission_entries.as_object() {
            for (key, value) in permission_map {
                map.insert(key.clone(), value.clone());
            }
        }
    }
    let text = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("Failed to encode MCP settings: {error}"))?;
    std::fs::write(path, text)
        .map_err(|error| format!("Failed to write MCP settings {}: {error}", path.display()))
}

/// Parses `DBX_SSH_MCP_PERMISSION_MODE`: present-and-valid wins, any other
/// value (unset, blank, unknown mode) leaves the persisted setting in force.
pub(crate) fn permission_mode_from_env(
    lookup: impl FnOnce(&str) -> Option<String>,
) -> Option<String> {
    let value = lookup("DBX_SSH_MCP_PERMISSION_MODE")?;
    let trimmed = value.trim();
    if trimmed == PERMISSION_MODE_AUTONOMOUS || trimmed == PERMISSION_MODE_CONFIRM {
        Some(trimmed.to_string())
    } else {
        None
    }
}

/// Validates the `localTransferRoot` setting: an absolute path string, or
/// empty to clear the override (falling back to the default transfer
/// roots).
pub(crate) fn validated_transfer_root(value: &Value) -> Result<String, String> {
    let root = value
        .as_str()
        .ok_or_else(|| {
            "localTransferRoot must be a string (absolute path, or empty to reset)".to_string()
        })?
        .trim()
        .to_string();
    if !root.is_empty() && !Path::new(&root).is_absolute() {
        return Err("localTransferRoot must be an absolute path (or empty to reset)".to_string());
    }
    Ok(root)
}

/// Validates one `mcp/settings/set` field: an unsigned integer within
/// `1..=ceiling`.
pub(crate) fn validated_limit(value: &Value, name: &str, ceiling: u64) -> Result<u64, String> {
    let bytes = value
        .as_u64()
        .ok_or_else(|| format!("{name} must be a positive integer number of bytes"))?;
    if bytes == 0 || bytes > ceiling {
        return Err(format!("{name} must be between 1 and {ceiling} bytes"));
    }
    Ok(bytes)
}
