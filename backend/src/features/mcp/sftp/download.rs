//! MCP `sftp_download` helpers.

use super::*;

impl McpState {
    /// `sftp_download`: transfers one remote file to a local path. The local
    /// target is validated before dialing (those refusals keep the pooled
    /// connection); the remote size is checked against the configured
    /// `maxDownloadBytes` and transport errors drop the cached connection.
    pub(in crate::features::mcp) async fn sftp_download_tool(
        &self,
        arguments: &Value,
    ) -> Result<Value, String> {
        // Both gaps named in one error (round 6 enumeration contract), in
        // the schema's required order (remotePath before localPath).
        missing_required(arguments, &["remotePath", "localPath"])?;
        let local_path = required_str(arguments, "localPath")?;
        // M25：远端路径组件归一（localPath 是本地文件参数，不归一）。
        let remote_path = normalize_remote_path(required_str(arguments, "remotePath")?)?;
        // Local-write hygiene before anything else: remote content must not
        // land on shell bootstrap / scheduled-execution paths.
        if is_sensitive_local_path(local_path) {
            return Err(format!(
                "Refusing to write the local sensitive path {local_path} via sftp_download"
            ));
        }
        let overwrite = arg_bool(arguments, "overwrite")?.unwrap_or(false);
        let local = Path::new(local_path);
        if local.exists() && !overwrite {
            return Err(format!(
                "Local path already exists: {local_path} (pass overwrite=true to replace)"
            ));
        }
        let file_name = local
            .file_name()
            .ok_or_else(|| format!("Invalid local path: {local_path}"))?;
        if let Some(parent) = local
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Cannot create local directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        // Write through the canonical parent (symlinks and `..` resolved by
        // the OS) re-joined with the requested file name, so the target is
        // exactly the requested path and never a traversal artifact.
        let local_target = match local
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            Some(parent) => std::fs::canonicalize(parent)
                .map_err(|error| {
                    format!(
                        "Cannot resolve local directory {}: {error}",
                        parent.display()
                    )
                })?
                .join(file_name),
            None => PathBuf::from(file_name),
        };
        // Containment before dialing: refusals here keep the pooled
        // connection untouched (same contract as the other local checks).
        self.ensure_local_transfer_allowed(&local_target)?;
        let outcome = self
            .download_via_sftp(arguments, &remote_path, local_path, &local_target)
            .await;
        if outcome.is_err() {
            self.drop_connection(arguments).await;
        }
        outcome
    }

    pub(in crate::features::mcp) async fn download_via_sftp(
        &self,
        arguments: &Value,
        remote_path: &str,
        local_path: &str,
        local_target: &Path,
    ) -> Result<Value, String> {
        self.connection(arguments).await?;
        let download_limit = self.size_limits().max_download_bytes;
        // 同 sftp_tool：写锁只圈句柄准备，下载本体锁外。
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
        // latin-1（M19）：读侧沿 M18 sftp_read_file 同策略——显示路径整条
        // latin1_encode_display 还原字节后 OPEN(READ)+READ，裸包路径任何
        // 失败回退高层重读（读操作安全）。目录探测不单独走裸包 STAT：目录
        // 的 OPEN 会被服务器拒绝、落入回退，由高层给出与 auto 分支一致的
        // 「is a directory」错误；超限沿既有 post-read 口径报错（读取量以
        // download_limit+1 探测封顶，不做无界传输）。
        if mcp_sftp_encoding(&self.runtime.data_dir(), arguments) == sftp_name::NameEncoding::Latin1
        {
            match open_raw_sftp(&handle).await {
                Ok(mut client) => {
                    match raw_sftp_read_file(&mut client, remote_path, 0, download_limit).await {
                        Ok((data, truncated)) => {
                            if truncated {
                                return Err(format!(
                                    "Remote file {remote_path} exceeds the MCP download limit \
                                     of {download_limit} bytes (adjust maxDownloadBytes via \
                                     mcp/settings/set)"
                                ));
                            }
                            tokio::fs::write(local_target, &data)
                                .await
                                .map_err(|error| {
                                    format!(
                                        "Cannot write local file {}: {error}",
                                        local_target.display()
                                    )
                                })?;
                            return Ok(json!({
                                "remotePath": remote_path,
                                "localPath": local_path,
                                "bytes": data.len(),
                            }));
                        }
                        Err(error) => {
                            eprintln!(
                                "[ssh] MCP sftp_download: raw byte read unavailable, falling back: {error}"
                            );
                        }
                    }
                }
                Err(error) => {
                    eprintln!(
                        "[ssh] MCP sftp_download: raw byte client unavailable, falling back: {error}"
                    );
                }
            }
        }
        let sftp = sftp_bind(&sftp_ready_error)?;
        let metadata = sftp
            .lock()
            .await
            .metadata(remote_path)
            .await
            .map_err(sftp_error)?;
        if metadata.is_dir() {
            return Err(format!(
                "{remote_path} is a directory; sftp_download transfers a single file"
            ));
        }
        if let Some(size) = metadata.size {
            if size > download_limit {
                return Err(format!(
                    "Remote file {remote_path} is {size} bytes and exceeds the MCP download \
                     limit of {download_limit} bytes (adjust maxDownloadBytes via mcp/settings/set)"
                ));
            }
        }
        let file = sftp
            .lock()
            .await
            .open(remote_path)
            .await
            .map_err(sftp_error)?;
        // 流式落盘：边读远端边写本地，整个文件不再驻留内存（此前是全量
        // Vec + 同步写——阻塞 worker 且 RSS 峰值即文件大小）。`take` 仍是
        // 无 size 文件（或 stat 后变大的文件）的硬上限；stat 快路径照旧。
        let mut reader = file.take(download_limit.saturating_add(1));
        let mut local = tokio::fs::File::create(local_target)
            .await
            .map_err(|error| {
                format!(
                    "Cannot create local file {}: {error}",
                    local_target.display()
                )
            })?;
        let mut transferred: u64 = 0;
        let mut buffer = vec![0_u8; 256 * 1024];
        let copy_result: Result<(), String> = async {
            loop {
                let read = reader
                    .read(&mut buffer)
                    .await
                    .map_err(|error| format!("SFTP read failed: {error}"))?;
                if read == 0 {
                    break;
                }
                transferred += read as u64;
                local.write_all(&buffer[..read]).await.map_err(|error| {
                    format!(
                        "Cannot write local file {}: {error}",
                        local_target.display()
                    )
                })?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = copy_result {
            // 半成品不留本地（overwrite=false 语义：失败后目标保持原状）。
            let _ = tokio::fs::remove_file(local_target).await;
            return Err(error);
        }
        if transferred > download_limit {
            let _ = tokio::fs::remove_file(local_target).await;
            return Err(format!(
                "Remote file {remote_path} exceeds the MCP download limit of {download_limit} \
                 bytes (adjust maxDownloadBytes via mcp/settings/set)"
            ));
        }
        local.flush().await.map_err(|error| {
            format!(
                "Cannot flush local file {}: {error}",
                local_target.display()
            )
        })?;
        Ok(json!({ "remotePath": remote_path, "localPath": local_path, "bytes": transferred }))
    }
}
