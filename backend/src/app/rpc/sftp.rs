//! Sftp RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_sftp(
        &self,
        method: &str,
        params: Value,
        _emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "sftp/home" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = self.runtime.block_on(self.ssh.sftp_home(session_id))?;
                Ok(json!({ "path": path }))
            }

            "sftp/list" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                let include_owner = params
                    .get("includeOwner")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                // 文件名编码判定（M16 连接级）：连接覆盖 > 全局偏好（latin-1 时列表走原始字节路径）。
                let encoding = self.resolve_sftp_encoding(session_id);
                let entries = self.runtime.block_on(self.ssh.sftp_list_path(
                    session_id,
                    path,
                    include_owner,
                    encoding,
                ))?;
                Ok(json!({ "entries": entries }))
            }

            "sftp/read" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                let offset = optional_u64(&params, "offset", 0);
                let max_bytes = bounded_bytes(&params, "maxBytes", 256 * 1024);
                // latin-1 车道（M19.5 落地）：path 是整条 wire 形式（列表回传），
                // 还原服务器字节后走裸包 READ（download 分片的 raw_read_chunk
                // 先例）；多读 1 字节对齐高层的 truncated 语义。auto 走高层。
                let encoding = self.resolve_sftp_encoding(session_id);
                let (data, truncated) = if encoding
                    == crate::filesystem::sftp::name::NameEncoding::Latin1
                {
                    let mut data = self.runtime.block_on(self.ssh.raw_read_chunk(
                        session_id,
                        path,
                        offset,
                        max_bytes.saturating_add(1) as u32,
                    ))?;
                    let truncated = data.len() > max_bytes;
                    data.truncate(max_bytes);
                    (data, truncated)
                } else {
                    self.runtime
                        .block_on(self.ssh.sftp_read_path(session_id, path, offset, max_bytes))?
                };
                Ok(json!({ "dataBase64": BASE64_STANDARD.encode(data), "truncated": truncated }))
            }

            "sftp/createDirectory" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                // 文件名编码判定（M16 连接级）：连接覆盖 > 全局偏好（latin-1 时写操作走原始字节路径）。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(self.ssh.sftp_create_directory(session_id, path, encoding))?;
                Ok(json!({ "success": true }))
            }

            "sftp/rename" => {
                let session_id = required_string(&params, "sessionId")?;
                let source = required_string(&params, "sourcePath")?;
                let target = required_string(&params, "targetPath")?;
                // latin-1：源按 wire 还原、目标按显示文本编码，raw RENAME。
                // 判定走连接级优先级（M16）：连接覆盖 > 全局偏好。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(self.ssh.sftp_rename(session_id, source, target, encoding))?;
                Ok(json!({ "success": true }))
            }

            "sftp/chmod" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                let mode = params
                    .get("mode")
                    .and_then(|value| {
                        value
                            .as_str()
                            .and_then(|text| u32::from_str_radix(text, 8).ok())
                            .or_else(|| value.as_u64().and_then(|v| u32::try_from(v).ok()))
                    })
                    .filter(|value| *value <= 0o7777)
                    .ok_or("Mode must be an octal value up to 7777")?;
                // latin-1（M17 增量③）：整条 wire 路径还原字节后 raw SETSTAT。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(self.ssh.sftp_chmod(session_id, path, mode, encoding))?;
                Ok(json!({ "success": true }))
            }

            "sftp/diskUsage" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                self.runtime
                    .block_on(self.ssh.sftp_disk_usage(session_id, path))
            }

            "sftp/stat" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                // latin-1（M17 增量③）：整条 wire 路径还原字节后 raw LSTAT。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime.block_on(crate::filesystem::sftp::ext::stat(
                    &self.ssh, session_id, path, encoding,
                ))
            }

            "sftp/exists" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                // 路径形式（M17 增量①）：缺省「wire 目录前缀 + 显示末段」
                // （rename 覆盖预检、上传撞名预检）；`form: "wire"` 表示整条
                // 都是列表回传的 wire 形式（粘贴预检）。latin-1（M16）下分别
                // 按 write_path_bytes / unescape_wire 还原字节，raw LSTAT 探测。
                let whole_wire = params.get("form").and_then(Value::as_str) == Some("wire");
                let encoding = self.resolve_sftp_encoding(session_id);
                let exists = self.runtime.block_on(crate::filesystem::sftp::ext::exists(
                    &self.ssh, session_id, path, encoding, whole_wire,
                ))?;
                Ok(json!({ "exists": exists }))
            }

            "sftp/rename-unique" => {
                let session_id = required_string(&params, "sessionId")?;
                let dir = required_string(&params, "dir")?;
                let name = required_string(&params, "name")?;
                // sudo（UploadSudo 配套）：sudo 车道上传的撞名探测——目标目录
                // （如 /root）对登录用户不可读，SFTP LSTAT 探测必失败，改走
                // sudo test -e 逐候选探测。
                let sudo = params.get("sudo").and_then(Value::as_bool).unwrap_or(false);
                // latin-1（M16）：dir 按 wire 还原、name 是新输入显示文本，
                // raw LSTAT 逐候选探测。sudo 探测不进裸包快路（exec 命令串
                // 是 UTF-8 String，非 UTF-8 字节名是登记边界）。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(crate::filesystem::sftp::ext::rename_unique(
                        &self.ssh, session_id, dir, name, encoding, sudo,
                    ))
            }

            "sftp/touch" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                // latin-1（M16）：新建文件名为用户新输入显示文本，raw
                // LSTAT/SETSTAT/OPEN。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime.block_on(crate::filesystem::sftp::ext::touch(
                    &self.ssh, session_id, path, encoding,
                ))?;
                Ok(json!({ "success": true }))
            }

            // 符号链接三命令：创建/读取指向/改指向。写操作走 ensure_writable
            // 只读门禁（对齐 sftp/chmod）；target 允许相对路径（symlink 语义）。
            "sftp/symlink-create" => {
                let session_id = required_string(&params, "sessionId")?;
                let target = required_string(&params, "target")?;
                let link_path = required_string(&params, "linkPath")?;
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(crate::filesystem::sftp::ext::symlink_create(
                        &self.ssh, session_id, target, link_path, encoding,
                    ))?;
                Ok(json!({ "success": true }))
            }

            "sftp/symlink-read" => {
                let session_id = required_string(&params, "sessionId")?;
                let link_path = required_string(&params, "linkPath")?;
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(crate::filesystem::sftp::ext::symlink_read(
                        &self.ssh, session_id, link_path, encoding,
                    ))
            }

            "sftp/symlink-update" => {
                let session_id = required_string(&params, "sessionId")?;
                let link_path = required_string(&params, "linkPath")?;
                let target = required_string(&params, "target")?;
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(crate::filesystem::sftp::ext::symlink_update(
                        &self.ssh, session_id, link_path, target, encoding,
                    ))?;
                Ok(json!({ "success": true }))
            }

            "sftp/write" => {
                let session_id = required_string(&params, "sessionId")?;
                let remote_path = required_string(&params, "remotePath")?;
                let data_base64 = required_string(&params, "dataBase64")?;
                // latin-1（M16）：remotePath 是整条 wire 形式（列表回传），
                // 整条还原字节后 raw 暂存提交。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(crate::filesystem::sftp::ext::write_file(
                        &self.ssh,
                        session_id,
                        remote_path,
                        data_base64,
                        encoding,
                    ))?;
                Ok(json!({ "success": true }))
            }

            "sftp/archive" => {
                let session_id = required_string(&params, "sessionId")?;
                let source_paths = params
                    .get("sourcePaths")
                    .and_then(Value::as_array)
                    .ok_or("Missing sourcePaths")?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_string)
                            .ok_or_else(|| "sourcePaths must be strings".to_string())
                    })
                    .collect::<Result<Vec<String>, String>>()?;
                let archive_path = required_string(&params, "archivePath")?;
                self.runtime.block_on(crate::filesystem::sftp::ext::archive(
                    &self.ssh,
                    session_id,
                    &source_paths,
                    archive_path,
                ))
            }

            "sftp/extract" => {
                let session_id = required_string(&params, "sessionId")?;
                let archive_path = required_string(&params, "archivePath")?;
                let destination_path = required_string(&params, "destinationPath")?;
                let overwrite = params
                    .get("overwrite")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                // sudo（文件夹整包上传车道配套）：列档/解包走 sudo 编排。
                let sudo = params.get("sudo").and_then(Value::as_bool).unwrap_or(false);
                self.runtime
                    .block_on(crate::filesystem::sftp::ext::extract(
                        &self.ssh,
                        session_id,
                        archive_path,
                        destination_path,
                        overwrite,
                        sudo,
                    ))?;
                Ok(json!({ "success": true }))
            }

            // 外部编辑器回传：把 watcher 交付的 remote-edit 本地文件推回远端。
            // 安全边界见 crate::filesystem::sftp::ext::validate_remote_edit_path —— 只收
            // <下载目录>/remote-edit/ 之下、经 canonicalize 校验的文件。
            "sftp/upload-local" => {
                let session_id = required_string(&params, "sessionId")?;
                let local_path = required_string(&params, "localPath")?;
                let remote_path = required_string(&params, "remotePath")?;
                // latin-1（M16）：remotePath 是 watcher 登记的整条 wire 形式，
                // 整条还原字节后 raw 暂存提交。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(crate::filesystem::sftp::ext::upload_watched_file(
                        &self.ssh,
                        session_id,
                        local_path,
                        remote_path,
                        encoding,
                    ))
            }

            "sftp/copy" => {
                let session_id = self.filesystem_session(&params)?;
                // latin-1（M17 增量①）：from/toDir 是列表回传的 wire 形式，
                // 存在性预检与同名目录 move 快路径走裸包字节保真；底层 shell
                // cp/mv 的 exec 字节参数边界见 sftp_copy。
                let encoding = self.resolve_sftp_encoding(&session_id);
                self.runtime.block_on(crate::filesystem::sftp::copy::run(
                    &self.ssh,
                    &session_id,
                    crate::filesystem::sftp::copy::CopyOp::Copy,
                    &params,
                    encoding,
                ))
            }

            "sftp/move" => {
                let session_id = self.filesystem_session(&params)?;
                let encoding = self.resolve_sftp_encoding(&session_id);
                self.runtime.block_on(crate::filesystem::sftp::copy::run(
                    &self.ssh,
                    &session_id,
                    crate::filesystem::sftp::copy::CopyOp::Move,
                    &params,
                    encoding,
                ))
            }

            "sftp/delete" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                let recursive = params
                    .get("recursive")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                // latin-1：wire 路径还原为原始字节，raw REMOVE/RMDIR/树删。
                // 判定走连接级优先级（M16）：连接覆盖 > 全局偏好。
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(self.ssh.sftp_delete(session_id, path, recursive, encoding))?;
                Ok(json!({ "success": true }))
            }

            "sftp/deleteMany" => {
                let session_id = required_string(&params, "sessionId")?;
                let items = params
                    .get("items")
                    .and_then(Value::as_array)
                    .ok_or("Missing items")?
                    .iter()
                    .map(|value| {
                        let path = value
                            .get("path")
                            .and_then(Value::as_str)
                            .ok_or_else(|| "items[].path must be a string".to_string())?
                            .to_string();
                        let recursive = value
                            .get("recursive")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        Ok((path, recursive))
                    })
                    .collect::<Result<Vec<(String, bool)>, String>>()?;
                let encoding = self.resolve_sftp_encoding(session_id);
                self.runtime
                    .block_on(self.ssh.sftp_delete_many(session_id, &items, encoding))
            }

            "sftp/bookmarks/list" => crate::filesystem::sftp::bookmarks::list(&self.ssh.data_dir()),

            "sftp/bookmarks/save" => {
                crate::filesystem::sftp::bookmarks::save(&self.ssh.data_dir(), &params)
            }

            "sftp/bookmarks/delete" => {
                let id = required_string(&params, "id")?;
                crate::filesystem::sftp::bookmarks::delete(&self.ssh.data_dir(), id)
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
