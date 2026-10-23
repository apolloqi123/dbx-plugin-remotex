//! Import RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_import(
        &self,
        method: &str,
        params: Value,
        _emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            // 会话导入：主文件与可选 WindTerm user.config 走二进制 offset
            // 分块，finish 只返回脱敏预览；不持久化连接或任何凭据。
            "import/preview/start" => self.connection_import.start(&params),

            "import/preview/finish" => self
                .connection_import
                .finish(required_string(&params, "taskId")?),

            "import/preview/cancel" => Ok(json!({
                "cancelled": self.connection_import.cancel(required_string(&params, "taskId")?)
            })),
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
