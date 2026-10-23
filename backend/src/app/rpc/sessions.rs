//! Sessions RPC handlers.

use super::*;
use crate::core::plugin_ids::PLUGIN_ID;

impl Plugin {
    pub(super) fn handle_sessions(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "ssh/session/open" => {
                let operation_id = operation_id(&params);
                let request: SessionOpenRequest = parse(params)?;
                self.runtime.block_on(self.ssh.open_session(
                    &request,
                    &operation_id,
                    emitter.clone(),
                ))
            }

            "ssh/session/attach" => {
                let connection_id = required_string(&params, "connectionId")?;
                let workbench_id = required_string(&params, "workbenchId")?;
                let after_sequence = params
                    .get("afterSequence")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                self.runtime.block_on(self.ssh.attach_session(
                    connection_id,
                    workbench_id,
                    after_sequence,
                    emitter,
                ))
            }

            "ssh/session/close" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.ssh.close_session(session_id))?;
                // External-editor watchers belong to the session; the workbench
                // usually stops them first via watch/stop-all, this is the
                // backend-side backstop.
                self.runtime.block_on(self.watcher.stop_session(session_id));
                Ok(json!({ "success": true }))
            }

            "workbench/close" => {
                let workbench_id = required_string(&params, "workbenchId")?;
                self.runtime
                    .block_on(self.ssh.close_workbench(workbench_id))?;
                // Local shells, Telnet and VNC sessions belong to the closing
                // tab too; a webview reload never calls this, so live
                // sessions stay reattachable there.
                self.runtime
                    .block_on(self.local.close_workbench(workbench_id));
                self.runtime
                    .block_on(self.telnet.close_workbench(workbench_id));
                self.runtime
                    .block_on(self.vnc.close_workbench(workbench_id));
                self.runtime
                    .block_on(self.rdp.close_workbench(workbench_id));
                self.runtime
                    .block_on(self.serial.close_workbench(workbench_id));
                Ok(json!({ "success": true }))
            }

            "ssh/sessions/list" => Ok(self.runtime.block_on(self.ssh.list_sessions())),

            "ssh/status" => Ok(json!({
                "ok": true,
                "plugin": PLUGIN_ID,
                "maxTransferSize": MAX_TRANSFER_SIZE
            })),
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
