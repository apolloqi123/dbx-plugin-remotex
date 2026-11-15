//! Recording RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_recording(
        &self,
        method: &str,
        params: Value,
        _emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "ssh/recording/start" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.ssh.recording_start(session_id))
            }

            "ssh/recording/stop" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.ssh.recording_stop(session_id))
            }

            "ssh/recording/list" => Ok(json!({
                "recordings": crate::features::recording::list_recordings(&self.ssh.data_dir())
            })),

            "ssh/recording/get" => {
                let recording_id = required_string(&params, "recordingId")?;
                let offset = optional_u64(&params, "offset", 0) as usize;
                let limit = params
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(crate::features::recording::PAGE_LIMIT as u64)
                    .clamp(1, crate::features::recording::PAGE_LIMIT as u64)
                    as usize;
                crate::features::recording::read_events(
                    &self.ssh.data_dir(),
                    recording_id,
                    offset,
                    limit,
                )
            }

            "ssh/recording/delete" => {
                let recording_id = required_string(&params, "recordingId")?;
                let path =
                    crate::features::recording::cast_path(&self.ssh.data_dir(), recording_id)?;
                std::fs::remove_file(&path)
                    .map_err(|error| format!("Failed to delete recording: {error}"))?;
                Ok(json!({ "success": true }))
            }

            // 一键清空：只删 recordings 目录里的 .cast 文件，返回删除数量。
            "ssh/recording/clear" => {
                let deleted = crate::features::recording::clear_recordings(&self.ssh.data_dir());
                Ok(json!({ "success": true, "deleted": deleted }))
            }

            // 录制搜索（M14）：无持久索引，对现存 .cast 即时扫描——名称命中
            // （host/recordingId 包含查询词，大小写不敏感）或内容命中（展平
            // stdout 文本包含查询词）。空查询返回空集（前端显示未过滤列表）。
            "ssh/recording/search" => {
                let query = params
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                Ok(json!({
                    "recordings": crate::features::recording::search_recordings(&self.ssh.data_dir(), &query),
                }))
            }

            // 在文件管理器中定位录制文件：按 recordingId 解析路径（校验过
            // 遍历），不暴露任意路径打开原语。
            "ssh/recording/reveal" => {
                let recording_id = required_string(&params, "recordingId")?;
                let path =
                    crate::features::recording::cast_path(&self.ssh.data_dir(), recording_id)?;
                if !path.exists() {
                    return Err("Recording file no longer exists".to_string());
                }
                crate::filesystem::local::downloads::reveal_in_file_manager(&path)?;
                Ok(json!({ "success": true }))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
