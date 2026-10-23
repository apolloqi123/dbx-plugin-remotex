//! Exec RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_exec(
        &self,
        method: &str,
        params: Value,
        _emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "ssh/exec" => {
                let session_id = required_string(&params, "sessionId")?;
                let command = required_string(&params, "command")?;
                let sudo = params.get("sudo").and_then(Value::as_bool).unwrap_or(false);
                let timeout_secs = params.get("timeoutSecs").and_then(Value::as_u64);
                let exec_id = params.get("execId").and_then(Value::as_str);
                // Connection-level sudoers-style allowlist (mirrors the MCP
                // gate); structured sudo_fs ops stay exempt. With the gate on
                // the rebuild — not the raw text — is what executes.
                let command: String = if sudo {
                    self.runtime
                        .block_on(self.ssh.ensure_sudo_allowed(session_id, command))?
                } else {
                    command.to_string()
                };
                self.runtime.block_on(self.ssh.exec(
                    session_id,
                    exec_id,
                    &command,
                    sudo,
                    timeout_secs,
                ))
            }

            "ssh/exec/cancel" => {
                let exec_id = required_string(&params, "execId")?;
                self.ssh.cancel_exec(exec_id)?;
                Ok(json!({ "success": true }))
            }

            // FIG 补全引擎 generator 执行（wave-1 lane B）：反序列化 →
            // 安全校验/收紧 → 按 target 分派（local 短子进程 / ssh 复用
            // exec 通道）。审计与 ssh/exec 臂同款：该臂无审计调用，此处
            // 同样不加。
            "completion/execute" => {
                let mut request: crate::features::completion::protocol::CompletionExecuteRequest =
                    parse(params)?;
                crate::features::completion::security::validate_and_clamp(&mut request)?;
                let result =
                    self.runtime
                        .block_on(crate::features::completion::executor::dispatch(
                            self.ssh.as_ref(),
                            &request,
                        ))?;
                serde_json::to_value(result)
                    .map_err(|error| format!("completion: failed to encode result: {error}"))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
