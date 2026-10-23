//! Transfers RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_transfers(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            // sudo 下载（M14-C DownloadSudo）：root 大文件二进制下载。start 把
            // 源文件 sudo 暂存进同目录 0600 临时件后登记进下载注册表，分块
            // （sftp/download/next）、finish 与进度事件复用既有下载管线；
            // cancel 与 sftp/transfer/cancel 同构（任务住同一个注册表）。
            "sudo/download/start" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                let save_to_local = params
                    .get("saveToLocal")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let download_dir = params
                    .get("downloadDir")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let conflict = params.get("conflict").and_then(Value::as_str);
                self.runtime.block_on(self.ssh.start_sudo_download(
                    session_id,
                    path,
                    save_to_local,
                    download_dir.as_deref(),
                    conflict,
                    emitter,
                ))
            }

            "sudo/download/cancel" => {
                let reason = params
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.chars().take(120).collect::<String>());
                self.runtime.block_on(self.ssh.cancel_transfer(
                    required_string(&params, "taskId")?,
                    reason.as_deref(),
                    emitter,
                ))?;
                Ok(json!({ "success": true }))
            }

            "sftp/upload/start" => {
                let session_id = required_string(&params, "sessionId")?.to_string();
                let remote_path = required_string(&params, "remotePath")?.to_string();
                let size = params
                    .get("size")
                    .and_then(Value::as_u64)
                    .ok_or("Missing upload size")?;
                let resume_task_id = params
                    .get("resumeTaskId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                // 压缩通道判定（M33）走连接级编码：latin-1 裸包车道强制
                // 普通传输（远端 gunzip 的 shell 路径不做 latin-1 字节）。
                let encoding = self.resolve_sftp_encoding(&session_id);
                self.runtime.block_on(self.ssh.start_upload(
                    session_id,
                    remote_path,
                    size,
                    resume_task_id,
                    encoding,
                    false,
                    emitter,
                ))
            }

            // UploadSudo：sudo 车道上传——参数族与 sftp/upload/start 一致
            // （remotePath，上传族命名）。分块二进制帧（sftp/upload/<taskId>）、
            // 进度事件与 sftp/transfer/cancel 全复用；只有 finish 换
            // sudo/upload/finish（推送落登录用户 home 暂存件，提交走 sudo mv）。
            "sudo/upload/start" => {
                let session_id = required_string(&params, "sessionId")?.to_string();
                let remote_path = required_string(&params, "remotePath")?.to_string();
                let size = params
                    .get("size")
                    .and_then(Value::as_u64)
                    .ok_or("Missing upload size")?;
                let resume_task_id = params
                    .get("resumeTaskId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                let encoding = self.resolve_sftp_encoding(&session_id);
                self.runtime.block_on(self.ssh.start_upload(
                    session_id,
                    remote_path,
                    size,
                    resume_task_id,
                    encoding,
                    true,
                    emitter,
                ))
            }

            "sudo/upload/finish" => {
                let task_id = required_string(&params, "taskId")?;
                // sudo 车道不进 latin-1 裸包快路（暂存件是 ASCII 路径，高层
                // 客户端无碍），编码取值只影响普通车道分支。
                let session_id = self.ssh.upload_session_id(task_id);
                let encoding = self.resolve_sftp_encoding_opt(session_id.as_deref());
                self.runtime
                    .block_on(self.ssh.finish_upload(task_id, encoding, emitter))
            }

            "sftp/upload/finish" => {
                let task_id = required_string(&params, "taskId")?;
                // latin-1（M16）：远端落盘路径还原为原始字节后走裸包暂存提交。
                let session_id = self.ssh.upload_session_id(task_id);
                let encoding = self.resolve_sftp_encoding_opt(session_id.as_deref());
                self.runtime
                    .block_on(self.ssh.finish_upload(task_id, encoding, emitter))
            }

            "sftp/download/start" => {
                let session_id = required_string(&params, "sessionId")?;
                let remote_path = required_string(&params, "remotePath")?;
                let offset = optional_u64(&params, "offset", 0);
                let save_to_local = params
                    .get("saveToLocal")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let download_dir = params
                    .get("downloadDir")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let conflict = params.get("conflict").and_then(Value::as_str);
                // 判定走连接级优先级（M16）：连接覆盖 > 全局偏好 > 缺省
                // auto。M28-B 修 D-7：单文件下载判分支按生效编码区分——
                // auto 一律走高层客户端（auto 列表 uri 字面 `%` 未自转义，
                // wire 串里的 `%XX` 是文件名字面量），latin-1 维持
                // has_wire_escapes 判裸包车道。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime.block_on(self.ssh.start_download(
                    session_id,
                    remote_path,
                    offset,
                    save_to_local,
                    download_dir.as_deref(),
                    conflict,
                    encoding,
                    emitter,
                ))
            }

            // 递归目录下载：远端 read_dir 走树（不碰 shell、不产生远端临时
            // 包），逐文件复用下方分块下载管线，本地按相对路径镜像；分块与
            // finish/cancel 与单文件下载共用（任务在同一个注册表里）。
            // latin-1：远端遍历走裸包 READDIR，整树路径字节保真。
            "sftp/download/tree/start" => {
                let session_id = required_string(&params, "sessionId")?;
                let remote_path = required_string(&params, "remotePath")?;
                let download_dir = params
                    .get("downloadDir")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                // 判定走连接级优先级（M16）：连接覆盖 > 全局偏好（整树 latin-1 遍历）。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime.block_on(self.ssh.start_tree_download(
                    session_id,
                    remote_path,
                    download_dir.as_deref(),
                    encoding,
                    emitter,
                ))
            }

            "sftp/download/next" => {
                let task_id = required_string(&params, "taskId")?;
                let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
                self.runtime
                    .block_on(self.ssh.download_chunk(task_id, offset as u64, emitter))
            }

            "sftp/download/finish" => self.runtime.block_on(
                self.ssh
                    .complete_download(required_string(&params, "taskId")?, emitter),
            ),

            "sftp/transfer/cancel" => {
                // Optional reason slug from the workbench ("user",
                // "ack-timeout", ...) surfaces in the ledger so a cancel can
                // be told apart from a server failure on the next bug report.
                let reason = params
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.chars().take(120).collect::<String>());
                self.runtime.block_on(self.ssh.cancel_transfer(
                    required_string(&params, "taskId")?,
                    reason.as_deref(),
                    emitter,
                ))?;
                Ok(json!({ "success": true }))
            }

            "sftp/transfer/resumable" => self.ssh.resumable_uploads(),

            "sftp/transfer/list" => self
                .ssh
                .transfer_list(required_string(&params, "sessionId")?),

            "sftp/transfer/status" => self.runtime.block_on(
                self.ssh
                    .transfer_status(required_string(&params, "taskId")?),
            ),

            "sftp/transfer/history" => {
                let session_id = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty());
                let limit = params
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, 200) as usize;
                self.runtime
                    .block_on(self.ssh.transfer_history_query(session_id, limit))
            }

            "sftp/transfer/history/clear" => {
                self.ssh.clear_transfer_history()?;
                Ok(json!({ "success": true }))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
