//! Terminal RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_terminal(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "ssh/terminal/resize" => {
                let session_id = required_string(&params, "sessionId")?;
                let cols = required_u32(&params, "cols")?;
                let rows = required_u32(&params, "rows")?;
                self.runtime
                    .block_on(self.ssh.resize_terminal(session_id, cols, rows))?;
                Ok(json!({ "success": true }))
            }

            "ssh/terminal/batchInput" => {
                let session_ids = params
                    .get("sessionIds")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(Value::as_str)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .filter(|list| !list.is_empty())
                    .ok_or("Missing sessionIds")?;
                let command = required_string(&params, "command")?;
                let append_newline = params
                    .get("appendNewline")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                Ok(self.runtime.block_on(self.ssh.batch_terminal_input(
                    &session_ids,
                    command,
                    append_newline,
                )))
            }

            "ssh/terminal/directoryTracking" => {
                let session_id = required_string(&params, "sessionId")?;
                let enabled = params
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.runtime
                    .block_on(self.ssh.set_directory_tracking(session_id, enabled))?;
                Ok(json!({ "success": true }))
            }

            "ssh/terminal/replay" => {
                let session_id = required_string(&params, "sessionId")?;
                let after_sequence = params
                    .get("afterSequence")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let replay = self.runtime.block_on(self.ssh.replay_terminal(
                    session_id,
                    after_sequence,
                    emitter,
                ))?;
                Ok(replay)
            }

            "ssh/batchBar/state" => {
                // 批量发送命令条的跨工作台状态同步：把调用方（source 标识的
                // webview）的草稿/下拉选择/开关原样广播给所有插件 webview，
                // 各端按 source 过滤掉自己的回声。纯转发，sidecar 不落存储。
                emitter
                    .event("ssh/batchBar/state", params)
                    .map_err(|error| error.message)?;
                Ok(json!({ "broadcast": true }))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
