//! Telnet RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_telnet(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            // Telnet 会话（明文协议，P2-3）：入口在 SSH 工作台工具栏，用户显
            // 式点击才会创建。IAC 协商/NAWS/Expect 自动登录见 telnet_session.rs；
            // 输入走 `telnet/terminal/in/{id}` 二进制通道，输出走
            // `telnet/terminal/out/{id}`，生命周期事件 `telnet/session/state`。
            "telnet/start" => {
                let mut request: crate::sessions::telnet::TelnetStartRequest = parse(params)?;
                // 表单密码 → 声明式自动登录（提示词正则用内置默认词表）；请求
                // 已带 autoLogin（工具栏弹窗路径）时不覆盖。
                if request.auto_login.is_none() {
                    if let Some((username, password)) = self
                        .non_ssh_creds
                        .get(request.connection_id.as_deref().unwrap_or_default())
                    {
                        request.auto_login = Some(telnet_form_auto_login(&username, &password));
                    }
                }
                self.runtime
                    .block_on(self.telnet.start(request, emitter.clone()))
            }

            // `telnet/write` JSON 兜底（键盘主路径是二进制通道）。
            "telnet/write" => {
                let session_id = required_string(&params, "sessionId")?;
                let data_base64 = required_string(&params, "dataBase64")?;
                let data = crate::sessions::telnet::decode_write_payload(data_base64)?;
                self.telnet.write_input(session_id, data)?;
                Ok(json!({ "success": true }))
            }

            "telnet/resize" => {
                let session_id = required_string(&params, "sessionId")?;
                let cols = required_u32(&params, "cols")?;
                let rows = required_u32(&params, "rows")?;
                self.runtime
                    .block_on(self.telnet.resize(session_id, cols, rows))?;
                Ok(json!({ "success": true }))
            }

            "telnet/replay" => {
                let session_id = required_string(&params, "sessionId")?;
                let after_sequence = params
                    .get("afterSequence")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let replay = self.runtime.block_on(self.telnet.replay(
                    session_id,
                    after_sequence,
                    emitter,
                ))?;
                Ok(replay)
            }

            "telnet/close" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.telnet.close(session_id))?;
                Ok(json!({ "success": true }))
            }

            "telnet/list" => Ok(self.runtime.block_on(self.telnet.list())),
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
