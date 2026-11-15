//! MCP `sftp_upload` helpers.

use super::*;

impl McpState {
    /// `sftp_upload`: transfers one local file to the remote server. The
    /// local side is validated before any connection I/O (readable, inside
    /// the allowed transfer roots, clear of the sensitive-path blocklist,
    /// within the configured `maxUploadBytes`) so bad paths fail fast and
    /// refusals leave the pooled connection untouched. SFTP transport
    /// errors drop the cached connection so the next call reconnects.
    pub(in crate::features::mcp) async fn sftp_upload_tool(
        &self,
        arguments: &Value,
    ) -> Result<Value, String> {
        // Both gaps named in one error (round 6 enumeration contract).
        missing_required(arguments, &["localPath", "remotePath"])?;
        let local_path = required_str(arguments, "localPath")?;
        // M25：远端路径组件归一（localPath 是本地文件参数，不归一）。
        let remote_path = normalize_remote_path(required_str(arguments, "remotePath")?)?;
        let overwrite = arg_bool(arguments, "overwrite")?.unwrap_or(false);
        let local_source = std::fs::canonicalize(local_path)
            .map_err(|error| format!("Cannot read local file {local_path}: {error}"))?;
        self.ensure_local_transfer_allowed(&local_source)?;
        // 大小预检先于读取：超限文件不再全量读进内存（limit 内的全量驻留
        // 保持——真正的流式上传登记为后续项）；tokio::fs 消除阻塞 worker
        // 线程的同步读。读后的二次检查防御 metadata 与 read 之间的竞窗。
        let upload_limit = self.size_limits().max_upload_bytes;
        let source_size = tokio::fs::metadata(&local_source)
            .await
            .map(|meta| meta.len())
            .map_err(|error| format!("Cannot read local file {local_path}: {error}"))?;
        if source_size > upload_limit {
            return Err(format!(
                "Local file {local_path} is {source_size} bytes and exceeds the MCP upload \
                 limit of {upload_limit} bytes (adjust maxUploadBytes via mcp/settings/set)"
            ));
        }
        let data = tokio::fs::read(&local_source)
            .await
            .map_err(|error| format!("Cannot read local file {local_path}: {error}"))?;
        if data.len() as u64 > upload_limit {
            return Err(format!(
                "Local file {local_path} is {} bytes and exceeds the MCP upload limit of \
                 {upload_limit} bytes (adjust maxUploadBytes via mcp/settings/set)",
                data.len()
            ));
        }
        let outcome = self
            .upload_via_sftp(arguments, local_path, &remote_path, &data, overwrite)
            .await;
        if outcome.is_err() {
            self.drop_connection(arguments).await;
        }
        outcome
    }

    pub(in crate::features::mcp) async fn upload_via_sftp(
        &self,
        arguments: &Value,
        local_path: &str,
        remote_path: &str,
        data: &[u8],
        overwrite: bool,
    ) -> Result<Value, String> {
        self.connection(arguments).await?;
        // 同 sftp_tool：写锁只圈句柄准备，上传本体锁外。
        let pool_key = connection_pool_key(arguments);
        let (handle, sftp_ready, sftp_ready_error) = {
            let mut guard = self.connections.write().await;
            let entry = guard
                .get_mut(&pool_key)
                .ok_or("Connection is not established")?;
            match entry.sftp().await {
                Ok(sftp) => (entry.handle.clone(), Some(sftp), String::new()),
                Err(error) => (entry.handle.clone(), None, error),
            }
        };
        let sftp_bind =
            |error: &str| -> Result<std::sync::Arc<tokio::sync::Mutex<SftpSession>>, String> {
                sftp_ready.clone().ok_or_else(|| error.to_string())
            };
        // latin-1（M19）：显示路径整条按 latin1_encode_display 还原字节后走
        // 裸包直写（选型：沿既有 sftp_upload 直写语义，无工作台上传族的
        // `.dbx-part` 暂存需求；覆盖预检与写入同一字节口径，复用 M18 的
        // raw_sftp_write_bytes）。仅裸包客户端建立失败回退高层（M18 写工具
        // 同策略）；操作错误原样上抛，不回退（不会重复执行）。
        if mcp_sftp_encoding(&self.runtime.data_dir(), arguments) == sftp_name::NameEncoding::Latin1
        {
            match open_raw_sftp(&handle).await {
                Ok(mut client) => {
                    let bytes =
                        raw_sftp_write_bytes(&mut client, remote_path, data, overwrite).await?;
                    return Ok(json!({
                        "localPath": local_path,
                        "remotePath": remote_path,
                        "bytes": bytes,
                    }));
                }
                Err(error) => {
                    eprintln!(
                        "[ssh] MCP sftp_upload: raw byte client unavailable, falling back: {error}"
                    );
                }
            }
        }
        let sftp = sftp_bind(&sftp_ready_error)?;
        if !overwrite && sftp.lock().await.metadata(remote_path).await.is_ok() {
            return Err(format!(
                "Remote path already exists: {remote_path} (pass overwrite=true to replace)"
            ));
        }
        let mut file = sftp
            .lock()
            .await
            .create(remote_path)
            .await
            .map_err(sftp_error)?;
        tokio::io::AsyncWriteExt::write_all(&mut file, data)
            .await
            .map_err(|error| format!("SFTP write failed: {error}"))?;
        tokio::io::AsyncWriteExt::flush(&mut file)
            .await
            .map_err(|error| format!("SFTP write flush failed: {error}"))?;
        Ok(json!({ "localPath": local_path, "remotePath": remote_path, "bytes": data.len() }))
    }
}
