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
                if !request.workbench_id.is_empty() {
                    self.cancel_pending_workbench_close(&request.workbench_id);
                }
                self.runtime.block_on(self.ssh.open_session(
                    &request,
                    &operation_id,
                    emitter.clone(),
                ))
            }

            "ssh/session/attach" => {
                let connection_id = required_string(&params, "connectionId")?;
                let workbench_id = required_string(&params, "workbenchId")?;
                self.cancel_pending_workbench_close(workbench_id);
                let after_sequence = params
                    .get("afterSequence")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let result = self.runtime.block_on(self.ssh.attach_session(
                    connection_id,
                    workbench_id,
                    after_sequence,
                    emitter,
                ))?;
                if let Some(session_id) = result.get("sessionId").and_then(Value::as_str) {
                    self.cancel_pending_ssh_session_close(session_id);
                }
                Ok(result)
            }

            // Webview liveness ping for the SSH PTYs it still owns. Host-side
            // teardown notices (workbench re-init, editing another connection)
            // are grace-deferred; an alive workbench keeps renewing those
            // timers, so a spurious teardown never lands while the terminal
            // is on screen. A genuinely closed tab stops pinging and its
            // deferred close proceeds after the grace.
            "ssh/session/alive" => {
                let session_id = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty());
                let workbench_id = params
                    .get("workbenchId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty());
                let connection_id = params
                    .get("connectionId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty());
                if let Some(session_id) = session_id {
                    self.extend_pending_ssh_session_close(session_id);
                }
                // Connection-level disconnects are renewed only while a live
                // session proves the workbench still owns a PTY; after an
                // explicit disconnect the webview keeps running but stops
                // pinging, so the deferred registry cleanup still lands.
                let live_connection_id = session_id.and(connection_id);
                if let Some(connection_id) = live_connection_id {
                    self.extend_pending_ssh_disconnect(connection_id);
                }
                if let Some(workbench_id) = workbench_id {
                    self.extend_pending_workbench_close(workbench_id);
                }
                // Session enumeration takes the session registry lock; only do
                // it while an SSH teardown is actually pending, and scan once
                // for both scopes. A heartbeat during normal operation is then
                // just a few map lookups.
                if (workbench_id.is_some() || live_connection_id.is_some())
                    && self.has_pending_ssh_session_closes()
                {
                    for id in self.runtime.block_on(
                        self.ssh
                            .session_ids_touching(workbench_id, live_connection_id),
                    ) {
                        self.extend_pending_ssh_session_close(&id);
                    }
                }
                Ok(json!({ "success": true }))
            }

            "ssh/session/close" => {
                let session_id = required_string(&params, "sessionId")?;
                // Deferred by default: host workbench teardown (including a
                // direct RPC that does not come through workbench.onClose) may
                // be followed by an attach to the same session. Only explicit
                // user/UI closes opt into `immediate`.
                if params
                    .get("immediate")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    crate::platform::diagnostics::log_lifecycle_debug(format!(
                        "session-close immediate session={session_id}"
                    ));
                    self.cancel_pending_ssh_session_close(session_id);
                    // 容器日志流/PTY 由 SshRuntime 的 session-end hook 统一收口：
                    // 显式 close、workbench/close、connection/disconnect 与断链
                    // 都会走到；端口映射/传输仍由 close_session 自身清理。
                    self.runtime.block_on(self.ssh.close_session(session_id))?;
                } else {
                    self.schedule_ssh_session_close(session_id);
                }
                Ok(json!({ "success": true }))
            }

            "workbench/close" => {
                let workbench_id = required_string(&params, "workbenchId")?;
                // Host teardown can race an immediate workbench re-init (for
                // example while another connection is edited). Keep the SSH
                // PTY for a short grace; a following attach/open cancels the
                // close, otherwise it lands after the window.
                self.schedule_ssh_workbench_close(workbench_id);
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
