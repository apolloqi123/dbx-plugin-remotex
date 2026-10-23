//! MCP settings operations.

use super::*;

impl McpState {
    /// Snapshot of the MCP size preferences (already clamped to ceilings).
    pub(super) fn size_limits(&self) -> McpLimits {
        self.limits
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// `mcp/settings/get`: the effective MCP size preferences plus the
    /// §1.3 permission settings (env overrides applied, `persisted*` keys
    /// showing what a later env removal would restore).
    pub fn settings_get(&self) -> Value {
        let mut payload = self.size_limits().to_json();
        let (mode, scope) = self.effective_permission();
        if let Some(object) = payload.as_object_mut() {
            object.insert("execPermissionMode".to_string(), json!(mode));
            object.insert("connectionScope".to_string(), json!(scope.entries()));
            object.insert(
                "connectionScopeDenyAll".to_string(),
                json!(matches!(scope, ConnectionScope::DenyAll)),
            );
            let persisted = self
                .permission
                .read()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone();
            object.insert(
                "persistedExecPermissionMode".to_string(),
                json!(persisted.exec_permission_mode),
            );
            object.insert(
                "persistedConnectionScope".to_string(),
                json!(persisted.connection_scope),
            );
        }
        payload
    }

    /// `mcp/settings/set`: partial update with per-field validation
    /// (unsigned integers in `1..=ceiling` only), persisted to
    /// `mcp-settings.json`; returns the complete updated object. Every
    /// field is validated before anything is written — a rejected update
    /// must not touch the persisted file at all.
    pub fn settings_set(&self, updates: &Value) -> Result<Value, String> {
        let mut limits = self.size_limits();
        if let Some(value) = updates.get("maxReadBytes") {
            limits.max_read_bytes = validated_limit(value, "maxReadBytes", READ_LIMIT_CEILING)?;
        }
        if let Some(value) = updates.get("maxUploadBytes") {
            limits.max_upload_bytes =
                validated_limit(value, "maxUploadBytes", UPLOAD_LIMIT_CEILING)?;
        }
        if let Some(value) = updates.get("maxDownloadBytes") {
            limits.max_download_bytes =
                validated_limit(value, "maxDownloadBytes", DOWNLOAD_LIMIT_CEILING)?;
        }
        if let Some(value) = updates.get("localTransferRoot") {
            limits.local_transfer_root = validated_transfer_root(value)?;
        }
        limits = limits.sanitized();
        let mut permission = self
            .permission
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        if let Some(value) = updates.get("execPermissionMode") {
            permission.exec_permission_mode = value
                .as_str()
                .filter(|text| {
                    matches!(*text, PERMISSION_MODE_AUTONOMOUS | PERMISSION_MODE_CONFIRM)
                })
                .ok_or_else(|| {
                    format!(
                        "execPermissionMode must be \"{PERMISSION_MODE_AUTONOMOUS}\" or \
                         \"{PERMISSION_MODE_CONFIRM}\""
                    )
                })?
                .to_string();
        }
        if let Some(value) = updates.get("connectionScope") {
            let entries = value
                .as_array()
                .ok_or_else(|| "connectionScope must be an array of strings".to_string())?;
            let mut scope = Vec::new();
            for entry in entries {
                let text = entry
                    .as_str()
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty())
                    .ok_or_else(|| {
                        "connectionScope entries must be non-empty strings".to_string()
                    })?;
                scope.push(text);
            }
            if scope.len() > CONNECTION_SCOPE_MAX_ENTRIES {
                return Err(format!(
                    "connectionScope is capped at {CONNECTION_SCOPE_MAX_ENTRIES} entries"
                ));
            }
            permission.connection_scope = scope;
        }
        // All validation passed: one merged write keeps both sections in
        // the single settings document (the size-limit writer used to
        // replace the file wholesale and drop the permission keys).
        write_settings_document(&self.limits_path, &limits, &permission)?;
        *self
            .limits
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = limits;
        *self
            .permission
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = permission;
        Ok(self.settings_get())
    }

    /// Effective (env-overridden) permission pair: `(mode, scope)`. The env
    /// vars win when present-and-valid; the persisted values apply
    /// otherwise.
    pub(super) fn effective_permission(&self) -> (String, ConnectionScope) {
        let persisted = self
            .permission
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let mode = permission_mode_from_env(|key| std::env::var(key).ok())
            .unwrap_or(persisted.exec_permission_mode);
        let scope = scope_from_env(|key| std::env::var(key).ok())
            .unwrap_or_else(|| ConnectionScope::from_persisted(persisted.connection_scope));
        (mode, scope)
    }

    /// True when the call needs a human confirm challenge on top of the
    /// existing gates (§1.3 `confirm` mode + gated tool set).
    pub(super) fn exec_confirm_required(&self, name: &str) -> bool {
        let (mode, _) = self.effective_permission();
        mode == PERMISSION_MODE_CONFIRM && is_confirm_gated_tool(name)
    }

    /// Effective scope for gate checks.
    pub(super) fn effective_scope(&self) -> ConnectionScope {
        self.effective_permission().1
    }
}
