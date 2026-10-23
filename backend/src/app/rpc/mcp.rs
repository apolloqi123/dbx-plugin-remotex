//! Mcp RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_mcp(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "mcp/tools" => Ok(crate::features::mcp::tool_definitions()),

            "mcp/call" => self
                .runtime
                .block_on(self.mcp.call_dbx(&params, emitter.clone())),

            "mcp/settings/get" => Ok(self.mcp.settings_get()),

            "mcp/settings/set" => self.mcp.settings_set(&params),
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
