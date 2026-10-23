//! `PluginHandler` implementation: RPC requests and binary channels.

use super::diagnostics::TERMINAL_INPUT_FRAMES_RECEIVED;
use super::*;

impl PluginHandler for Plugin {
    fn handle(
        &self,
        _context: RequestContext,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, PluginError> {
        self.handle_request(method, params, emitter)
            .map_err(to_plugin_error)
    }

    fn handle_binary(
        &self,
        channel: &str,
        data: Vec<u8>,
        emitter: &PluginEmitter,
    ) -> Result<(), PluginError> {
        if let Some(session_id) = channel.strip_prefix("ssh/terminal/in/") {
            TERMINAL_INPUT_FRAMES_RECEIVED.fetch_add(1, Ordering::Relaxed);
            let (sequence, payload) = match decode_sequenced_input(&data) {
                Ok(split) => split,
                Err(error) => return Err(to_plugin_error(error)),
            };
            if let Err(error) = self.ssh.write_terminal(session_id, payload) {
                // Binary handler failures are only logged by the SDK loop, so a
                // workbench typing into a dead session (host-pushed disconnect,
                // sidecar restart) would otherwise learn nothing: the tab keeps
                // looking alive while every keystroke is swallowed. Mirror the
                // failure as an event the workbench can auto-reconnect on.
                let _ = emitter.event(
                    "ssh/terminal/error",
                    json!({ "sessionId": session_id, "error": error }),
                );
                return Err(to_plugin_error(error));
            }
            emitter.event(
                "ssh/terminal/inputAck",
                json!({ "sessionId": session_id, "sequence": sequence }),
            )?;
            return Ok(());
        }
        if let Some(session_id) = channel.strip_prefix("local/terminal/in/") {
            let (sequence, payload) = match decode_sequenced_input(&data) {
                Ok(split) => split,
                Err(error) => return Err(to_plugin_error(error)),
            };
            if let Err(error) = self.local.write_input(session_id, payload) {
                // Mirror the SSH branch: without an event the tab keeps
                // looking alive while every keystroke is swallowed.
                let _ = emitter.event(
                    "local/terminal/error",
                    json!({ "sessionId": session_id, "error": error }),
                );
                return Err(to_plugin_error(error));
            }
            emitter.event(
                "local/terminal/inputAck",
                json!({ "sessionId": session_id, "sequence": sequence }),
            )?;
            return Ok(());
        }
        if let Some(session_id) = channel.strip_prefix("telnet/terminal/in/") {
            let (sequence, payload) = match decode_sequenced_input(&data) {
                Ok(split) => split,
                Err(error) => return Err(to_plugin_error(error)),
            };
            if let Err(error) = self.telnet.write_input(session_id, payload) {
                // Mirror the SSH/local branches: without an event the tab
                // keeps looking alive while every keystroke is swallowed.
                let _ = emitter.event(
                    "telnet/terminal/error",
                    json!({ "sessionId": session_id, "error": error }),
                );
                return Err(to_plugin_error(error));
            }
            emitter.event(
                "telnet/terminal/inputAck",
                json!({ "sessionId": session_id, "sequence": sequence }),
            )?;
            return Ok(());
        }
        if let Some(session_id) = channel.strip_prefix("serial/terminal/in/") {
            // 串口 B1 二进制写通道：帧与输出同构（流标签 + u64 序号 + 数据），
            // 非 Stdin 标签/截断帧 → 参数错误。上传活动期间一律拒绝（互斥
            // 后盾，第一道闸门在前端）；死会话/互斥拒绝镜像 `serial/terminal/
            // error` 事件，工作台不至于看着在线却打不进字。
            TERMINAL_INPUT_FRAMES_RECEIVED.fetch_add(1, Ordering::Relaxed);
            let (sequence, payload) = match crate::sessions::serial::decode_input_frame(&data) {
                Ok(split) => split,
                Err(error) => return Err(to_plugin_error(error)),
            };
            let session = match self.runtime.block_on(self.serial.session(session_id)) {
                Ok(session) => session,
                Err(error) => {
                    let _ = emitter.event(
                        "serial/terminal/error",
                        json!({ "sessionId": session_id, "error": error }),
                    );
                    return Err(to_plugin_error(error));
                }
            };
            if session.upload_active() {
                let error =
                    "Serial input is rejected while a file upload is in progress".to_string();
                let _ = emitter.event(
                    "serial/terminal/error",
                    json!({ "sessionId": session_id, "error": error }),
                );
                return Err(to_plugin_error(error));
            }
            if let Err(error) = self
                .serial
                .write_input(&session, session_id, &payload, emitter)
            {
                let _ = emitter.event(
                    "serial/terminal/error",
                    json!({ "sessionId": session_id, "error": error }),
                );
                return Err(to_plugin_error(error));
            }
            emitter.event(
                "serial/terminal/inputAck",
                json!({ "sessionId": session_id, "sequence": sequence }),
            )?;
            return Ok(());
        }
        if let Some(rest) = channel.strip_prefix("import/preview/") {
            let Some((task_id, part)) = rest.rsplit_once('/') else {
                return Err(PluginError::new(
                    -32601,
                    format!("Unknown import preview binary channel: {channel}"),
                ));
            };
            match self.connection_import.append(task_id, part, &data) {
                Ok(next_offset) => {
                    emitter.event(
                        "import/preview/ack",
                        json!({ "taskId": task_id, "part": part, "nextOffset": next_offset }),
                    )?;
                    return Ok(());
                }
                Err(error) => {
                    let _ = emitter.event(
                        "import/preview/error",
                        json!({ "taskId": task_id, "part": part, "error": error }),
                    );
                    return Err(to_plugin_error(error));
                }
            }
        }
        if let Some(task_id) = channel.strip_prefix("sftp/upload/") {
            // Binary handler failures are only logged by the SDK loop, so the
            // workbench would otherwise learn about a desynced/missing upload
            // only through a 30s ack timeout. Mirror the failure as an event
            // it can react to immediately.
            if let Err(error) = self.ssh.append_upload(task_id, &data, emitter) {
                let _ = emitter.event(
                    "sftp/upload/error",
                    json!({ "taskId": task_id, "error": error }),
                );
                return Err(to_plugin_error(error));
            }
            return Ok(());
        }
        Err(PluginError::new(
            -32601,
            format!("Unknown binary channel: {channel}"),
        ))
    }
}

/// Terminal binary input frames carry an 8-byte BE sequence ahead of the
/// keystrokes; the sequence is only echoed in the inputAck event for the
/// workbench's bookkeeping — ordering comes from the SDK's per-channel lane.
fn decode_sequenced_input(data: &[u8]) -> Result<(u64, Vec<u8>), String> {
    if data.len() < 8 {
        return Err("terminal input is missing its sequence".to_string());
    }
    let sequence = u64::from_be_bytes(
        data[..8]
            .try_into()
            .map_err(|_| "invalid terminal input sequence".to_string())?,
    );
    Ok((sequence, data[8..].to_vec()))
}

fn to_plugin_error(error: String) -> PluginError {
    PluginError::new(-32000, error)
}
