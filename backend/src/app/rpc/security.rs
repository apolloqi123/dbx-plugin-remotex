//! Security RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_security(
        &self,
        method: &str,
        params: Value,
        _emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "ssh/audit/list" => {
                let limit = params
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(100)
                    .clamp(1, 500) as usize;
                let before_ts = params.get("beforeTs").and_then(Value::as_u64);
                // Fetch one extra entry to report truncation without a
                // second full read.
                let mut entries = crate::features::security::audit::tail(
                    &self.ssh.data_dir(),
                    limit + 1,
                    before_ts,
                )?;
                let truncated = entries.len() > limit;
                entries.truncate(limit);
                let items: Vec<Value> = entries
                    .iter()
                    .map(|entry| serde_json::to_value(entry).unwrap_or(Value::Null))
                    .collect();
                Ok(json!({ "entries": items, "truncated": truncated }))
            }

            "ssh/audit/clear" => {
                crate::features::security::audit::clear(&self.ssh.data_dir())?;
                Ok(json!({ "success": true }))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
