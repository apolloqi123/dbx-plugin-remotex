//! Serial RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_serial(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "serial/ports/list" => Ok(self.serial.list_ports()),

            "serial/start" => {
                let request: crate::sessions::serial::SerialStartRequest = parse(params)?;
                self.runtime
                    .block_on(self.serial.start(request, emitter.clone()))
            }

            "serial/write" => {
                let session_id = required_string(&params, "sessionId")?;
                let data_base64 = required_string(&params, "dataBase64")?;
                let data = crate::sessions::serial::decode_write_payload(data_base64)?;
                let session = self.runtime.block_on(self.serial.session(session_id))?;
                self.serial
                    .write_input(&session, session_id, &data, emitter)?;
                Ok(json!({ "success": true }))
            }

            "serial/close" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.serial.close(session_id))?;
                Ok(json!({ "success": true }))
            }

            "serial/list" => Ok(self.runtime.block_on(self.serial.list())),

            // 序号制输出回放（设计稿 §3）：webview 重载/断线重连后恢复滚动区
            // 上下文；帧走既有 serial/terminal/out 二进制通道，摘要走 JSON。
            "serial/replay" => {
                let session_id = required_string(&params, "sessionId")?;
                let after_sequence = params
                    .get("afterSequence")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let replay = self.runtime.block_on(self.serial.replay(
                    session_id,
                    after_sequence,
                    emitter,
                ))?;
                Ok(replay)
            }

            // 串口文件上传（XMODEM/YMODEM/ZMODEM，NyaTerm 对齐）：引擎是纯
            // 状态机，由串口读线程喂数据/取输出；文件字节由前端 File API
            // 分块（≤64KiB）送入，sidecar 不落盘（web/docker 浏览器兜底）。
            "serial/upload/start" => {
                let session_id = required_string(&params, "sessionId")?;
                let protocol = crate::sessions::serial::xmodem::UploadProtocol::parse(
                    required_string(&params, "protocol")?,
                )?;
                let file_name = required_string(&params, "fileName")?;
                let total_size = params
                    .get("totalSize")
                    .and_then(Value::as_u64)
                    .ok_or("serial/upload/start: totalSize is required")?;
                self.runtime.block_on(self.serial.upload_start(
                    session_id,
                    protocol,
                    file_name.to_string(),
                    total_size,
                    emitter,
                ))
            }

            "serial/upload/data" => {
                let session_id = required_string(&params, "sessionId")?;
                let data_base64 = required_string(&params, "dataBase64")?;
                let data = crate::sessions::serial::decode_write_payload(data_base64)?;
                if data.len() > crate::sessions::serial::xmodem::UPLOAD_CHUNK_LIMIT {
                    return Err(format!(
                        "serial/upload/data: chunk of {} bytes exceeds the {} byte limit",
                        data.len(),
                        crate::sessions::serial::xmodem::UPLOAD_CHUNK_LIMIT
                    ));
                }
                let final_chunk = params
                    .get("final")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.runtime.block_on(self.serial.upload_data(
                    session_id,
                    data,
                    final_chunk,
                    emitter,
                ))
            }

            "serial/upload/cancel" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime
                    .block_on(self.serial.upload_cancel(session_id, emitter))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
