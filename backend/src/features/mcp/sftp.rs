//! MCP sftp helpers.

use super::*;

impl McpState {
    pub(super) async fn sftp_tool(&self, name: &str, arguments: &Value) -> Result<Value, String> {
        // Same lazy-establish contract as the transfer tools: resolve the
        // saved-connection reference (connectionId / connectionName /
        // endpoint) and dial on first use, so browsing a host the caller
        // never ssh_exec'd first works instead of demanding a pre-existing
        // pool entry.
        self.connection(arguments).await?;
        // 写锁只圈住"取句柄 + 惰性建立高层 sftp 客户端"：传输本体全部在锁外
        // 跑——写锁活到函数结束会让一次大文件读/写把其他连接的所有工具调用
        // 堵在 connection() 读锁上数分钟。sftp 建立失败不在此硬上抛：
        // sftp_copy 分支的 shell 回退语义要求 None + 错误串随行。
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
        // sftp_ready 为 None 的分支专用绑定：硬错误分支经它还原原错误文案。
        let sftp_bind =
            |error: &str| -> Result<std::sync::Arc<tokio::sync::Mutex<SftpSession>>, String> {
                sftp_ready.clone().ok_or_else(|| error.to_string())
            };
        // 文件名编码判定（M17，连接级）：连接覆盖 > 全局偏好 > 缺省 auto。
        // dispatch 层已把 connectionName/端点选择器归一化为显式 connectionId
        // （见 call_tool_inner），这里只需读 connectionId；内联拨号按未覆盖
        // 处理（跟随全局），与工作台 resolve_sftp_encoding_opt 同构。
        let encoding = mcp_sftp_encoding(&self.runtime.data_dir(), arguments);
        match name {
            "sftp_list_dir" => {
                // M25：与工作台车道同口径——组件归一（绝对化 + 去 ./../空段）。
                // latin-1 显示域内的 clean 名不受影响；这是纯字符级清洗，
                // 归一在编码还原之前完成。
                let path = normalize_remote_path(required_str(arguments, "path")?)?;
                if encoding == sftp_name::NameEncoding::Latin1 {
                    // latin-1（M17）：裸包 READDIR 保原始字节，名字口径为显示
                    // 形式（latin-1 解码忠实可逆，AI 回传同一路径即落回原始
                    // 字节）。裸包路径任何失败回退高层（读操作回退安全，与
                    // 工作台 sftp_list_path 同策略）。
                    match open_raw_sftp(&handle).await {
                        Ok(mut client) => match client
                            .readdir(&sftp_name::latin1_encode_display(&path))
                            .await
                        {
                            Ok(raw_entries) => {
                                return Ok(json!({
                                    "path": path,
                                    "entries": raw_list_items(&path, raw_entries),
                                }));
                            }
                            Err(error) => {
                                eprintln!(
                                    "[ssh] MCP sftp_list_dir: raw byte listing unavailable, falling back: {error}"
                                );
                            }
                        },
                        Err(error) => {
                            eprintln!(
                                "[ssh] MCP sftp_list_dir: raw byte client unavailable, falling back: {error}"
                            );
                        }
                    }
                }
                let sftp = sftp_bind(&sftp_ready_error)?;
                let entries = sftp
                    .lock()
                    .await
                    .read_dir(&path)
                    .await
                    .map_err(sftp_error)?;
                let items: Vec<Value> = entries
                    .map(|entry| {
                        let metadata = entry.metadata();
                        json!({
                            "name": entry.file_name(),
                            "path": entry.path(),
                            "kind": match entry.file_type() {
                                FileType::Dir => "directory",
                                FileType::Symlink => "symlink",
                                FileType::File => "file",
                                FileType::Other => "other",
                            },
                            "size": metadata.size,
                            "modifiedAt": metadata.mtime,
                        })
                    })
                    .collect();
                Ok(json!({ "path": path, "entries": items }))
            }
            "sftp_stat" => {
                // M25：组件归一，与工作台 sftp/stat 同口径。
                let path = normalize_remote_path(required_str(arguments, "path")?)?;
                if encoding == sftp_name::NameEncoding::Latin1 {
                    // latin-1（M18）：显示路径还原字节后走裸包 LSTAT（不跟随
                    // 符号链接，与工作台 sftp/stat M17 同口径）。裸包 v3
                    // attrs 不携带 uid/gid：shell 查询尽力而为补齐（M17 先例，
                    // 非 ASCII 名的字节参数边界登记在案）。仅裸包客户端建立
                    // 失败回退高层；操作错误原样上抛（工作台 stat 同策略）。
                    match open_raw_sftp(&handle).await {
                        Ok(mut client) => {
                            let attrs = client
                                .lstat(&sftp_name::latin1_encode_display(&path))
                                .await?;
                            let (uid, gid) = lookup_remote_uid_gid(&handle, &path).await;
                            return Ok(raw_stat_json(&path, attrs, uid, gid));
                        }
                        Err(error) => {
                            eprintln!(
                                "[ssh] MCP sftp_stat: raw byte client unavailable, falling back: {error}"
                            );
                        }
                    }
                }
                let sftp = sftp_bind(&sftp_ready_error)?;
                let metadata = sftp
                    .lock()
                    .await
                    .metadata(&path)
                    .await
                    .map_err(sftp_error)?;
                Ok(json!({
                    "path": path,
                    "size": metadata.size,
                    "permissions": metadata.permissions.map(|bits| format!("{:04o}", bits & 0o7777)),
                    "modifiedAt": metadata.mtime,
                    "accessedAt": metadata.atime,
                    "uid": metadata.uid,
                    "gid": metadata.gid,
                }))
            }
            "sftp_read_file" => {
                // M25：组件归一，与工作台读取族同口径。
                let path = normalize_remote_path(required_str(arguments, "path")?)?;
                // Settings move the default and the ceiling (down freely, up
                // within the soft caps); the clamp itself stays as the
                // defensive hard limit against oversized requests.
                let limits = self.size_limits();
                let max_bytes = arg_u64(arguments, "maxBytes")?
                    .unwrap_or(limits.max_read_bytes)
                    .clamp(1, limits.max_download_bytes);
                let as_base64 = arg_bool(arguments, "base64")?.unwrap_or(false);
                let offset = arg_u64(arguments, "offset")?.unwrap_or(0);
                if encoding == sftp_name::NameEncoding::Latin1 {
                    // latin-1（M18）：显示路径还原字节后 OPEN(READ)+READ 分块
                    // （32 KiB 粒度）。大文件策略沿既有 MCP 边界：单次至多
                    // maxBytes（上限 maxDownloadBytes），超出标记 truncated。
                    // 读操作回退安全（M17-B readdir 同策略）：裸包路径任何
                    // 失败回退高层重读。
                    match open_raw_sftp(&handle).await {
                        Ok(mut client) => {
                            match raw_sftp_read_file(&mut client, &path, offset, max_bytes).await {
                                Ok((data, truncated)) => {
                                    return Ok(read_file_response(
                                        &path, &data, truncated, as_base64,
                                    ));
                                }
                                Err(error) => {
                                    eprintln!(
                                        "[ssh] MCP sftp_read_file: raw byte read unavailable, falling back: {error}"
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            eprintln!(
                                "[ssh] MCP sftp_read_file: raw byte client unavailable, falling back: {error}"
                            );
                        }
                    }
                }
                let sftp = sftp_bind(&sftp_ready_error)?;
                let mut file = sftp.lock().await.open(&path).await.map_err(sftp_error)?;
                if offset > 0 {
                    use tokio::io::AsyncSeekExt;
                    // SeekFrom::Start only moves the local read cursor; an
                    // offset at/after EOF simply yields no data.
                    file.seek(std::io::SeekFrom::Start(offset))
                        .await
                        .map_err(|error| format!("SFTP seek failed: {error}"))?;
                }
                let mut data = Vec::new();
                file.take(max_bytes.saturating_add(1))
                    .read_to_end(&mut data)
                    .await
                    .map_err(|error| format!("SFTP read failed: {error}"))?;
                let truncated = data.len() as u64 > max_bytes;
                data.truncate(max_bytes as usize);
                Ok(read_file_response(&path, &data, truncated, as_base64))
            }
            "sftp_write_file" => {
                // M25：远端路径组件归一（content 是本地文本参数，不归一）。
                let path = normalize_remote_path(required_str(arguments, "path")?)?;
                let content = required_str(arguments, "content")?;
                let upload_limit = self.size_limits().max_upload_bytes;
                if content.len() as u64 > upload_limit {
                    return Err(format!(
                        "Content of {} bytes exceeds the MCP upload limit of {upload_limit} bytes \
                         (adjust maxUploadBytes via mcp/settings/set)",
                        content.len()
                    ));
                }
                let overwrite = arg_bool(arguments, "overwrite")?.unwrap_or(false);
                if encoding == sftp_name::NameEncoding::Latin1 {
                    // latin-1（M18）：显示路径还原字节后走裸包直写（选型：MCP
                    // 面沿既有 sftp_write_file 直写语义——OPEN(CREAT|WRITE|
                    // TRUNC) 截断 + WRITE 32 KiB 分块，无工作台上传族的
                    // `.dbx-part` 暂存需求）。覆盖预检与写入同一字节口径
                    // （裸包 LSTAT）。仅裸包客户端建立失败回退高层（M17-B 写
                    // 工具同策略）；操作错误原样上抛，不回退（不会重复执行）。
                    match open_raw_sftp(&handle).await {
                        Ok(mut client) => {
                            return raw_sftp_write_file(
                                &mut client,
                                &path,
                                content.as_bytes(),
                                overwrite,
                            )
                            .await;
                        }
                        Err(error) => {
                            eprintln!(
                                "[ssh] MCP sftp_write_file: raw byte client unavailable, falling back: {error}"
                            );
                        }
                    }
                }
                let sftp = sftp_bind(&sftp_ready_error)?;
                if !overwrite && sftp.lock().await.metadata(&path).await.is_ok() {
                    return Err(format!(
                        "Remote path already exists: {path} (pass overwrite=true to replace)"
                    ));
                }
                let mut file = sftp.lock().await.create(&path).await.map_err(sftp_error)?;
                tokio::io::AsyncWriteExt::write_all(&mut file, content.as_bytes())
                    .await
                    .map_err(|error| format!("SFTP write failed: {error}"))?;
                tokio::io::AsyncWriteExt::flush(&mut file)
                    .await
                    .map_err(|error| format!("SFTP write flush failed: {error}"))?;
                Ok(json!({ "path": path, "bytes": content.len() }))
            }
            other => Err(format!("Unknown tool: {other}")),
        }
    }

    /// Containment gate for agent-supplied local transfer paths; see
    /// [`ensure_local_transfer_allowed_in`] for the policy.
    pub(super) fn ensure_local_transfer_allowed(&self, path: &Path) -> Result<(), String> {
        let configured = self.size_limits().local_transfer_root;
        let roots = local_transfer_roots_for(&configured, &self.runtime.data_dir())?;
        ensure_local_transfer_allowed_in(&roots, &configured, path)
    }

    /// `sftp_upload`: transfers one local file to the remote server. The
    /// local side is validated before any connection I/O (readable, inside
    /// the allowed transfer roots, clear of the sensitive-path blocklist,
    /// within the configured `maxUploadBytes`) so bad paths fail fast and
    /// refusals leave the pooled connection untouched. SFTP transport
    /// errors drop the cached connection so the next call reconnects.
    pub(super) async fn sftp_upload_tool(&self, arguments: &Value) -> Result<Value, String> {
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

    pub(super) async fn upload_via_sftp(
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

    /// `sftp_download`: transfers one remote file to a local path. The local
    /// target is validated before dialing (those refusals keep the pooled
    /// connection); the remote size is checked against the configured
    /// `maxDownloadBytes` and transport errors drop the cached connection.
    pub(super) async fn sftp_download_tool(&self, arguments: &Value) -> Result<Value, String> {
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

    pub(super) async fn download_via_sftp(
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

/// MCP 工具面的连接级文件名编码判定（M17）：arguments 携带的 connectionId
/// （保存连接；dispatch 层已把 connectionName/端点选择器归一化为该字段）
/// 命中 `sftp_name_encoding_overrides` 时优先，否则跟随全局
/// `sftp_name_encoding`，缺省 auto——与工作台
/// `Plugin::resolve_sftp_encoding_opt` 同构（内联拨号按未覆盖处理）。
pub(super) fn mcp_sftp_encoding(data_dir: &Path, arguments: &Value) -> sftp_name::NameEncoding {
    preferences::sftp_name_encoding_for(data_dir, non_empty_argument(arguments, "connectionId"))
}

/// latin-1 裸包列表条目 → MCP 工具响应条目（M17）。名字口径为**显示形式**：
/// `name`/`path` 都是 latin-1 解码文本（解码输出恒在 U+0000..=U+00FF 域内，
/// `latin1_encode_display` 是其精确逆变换——AI 把返回的 path 原样回传给
/// sftp_mkdir/sftp_remove/sftp_rename 即落回服务器原始字节，不引入 %XX
/// 转义噪声）。kind 按 v3 permissions 类型位归类（缺 permissions 退回
/// file，非标准服务器不会把普通文件误渲染成目录）；`.`/`..` 跳过。
pub(super) fn raw_list_items(dir: &str, entries: Vec<sftp_raw::RawEntry>) -> Vec<Value> {
    entries
        .into_iter()
        .filter(|entry| entry.name.as_slice() != b"." && entry.name.as_slice() != b"..")
        .map(|entry| {
            let display =
                sftp_name::decode_display_name(&entry.name, sftp_name::NameEncoding::Latin1);
            // 目录 + 显示名的拼接与 join_wire_name 同形（纯字符串 join，无转
            // 义语义），直接复用避免重复实现。
            json!({
                "name": display.text,
                "path": sftp_name::join_wire_name(dir, &display.text),
                "kind": classify_raw_kind(entry.attrs.permissions),
                "size": entry.attrs.size,
                "modifiedAt": entry.attrs.mtime,
            })
        })
        .collect()
}

/// `sftp_stat` 响应装配（auto 与 latin-1 共用）。口径与 auto 分支一致：
/// permissions 四位八进制、秒级时间戳。uid/gid 由调用方尽力而为补齐（裸包
/// v3 attrs 不携带，见 [`lookup_remote_uid_gid`]）。
pub(super) fn raw_stat_json(
    path: &str,
    attrs: sftp_raw::RawAttrs,
    uid: Option<u32>,
    gid: Option<u32>,
) -> Value {
    json!({
        "path": path,
        "size": attrs.size,
        "permissions": attrs.permissions.map(|bits| format!("{:04o}", bits & 0o7777)),
        "modifiedAt": attrs.mtime,
        "accessedAt": attrs.atime,
        "uid": uid,
        "gid": gid,
    })
}

/// latin-1（M18）裸包 attrs 无 uid/gid（工作台 `sftp/stat` M17 先例同口径）：
/// shell `stat -c '%u %g'` 尽力而为补齐属主数字。命令串是 UTF-8 String，非
/// ASCII 显示名的字节参数不可控（M17-A 登记边界），exec 失败/解析不出按
/// `(None, None)` 处理，主元数据不受影响。
pub(super) async fn lookup_remote_uid_gid(
    handle: &Handle<SshClient>,
    path: &str,
) -> (Option<u32>, Option<u32>) {
    let command = format!("stat -c '%u %g' -- {}", exec::shell_quote(path));
    let Ok(outcome) = exec::exec_plain(handle, &command, Duration::from_secs(10), &[]).await else {
        return (None, None);
    };
    let mut fields = outcome.output.split_whitespace();
    (
        fields.next().and_then(|field| field.parse().ok()),
        fields.next().and_then(|field| field.parse().ok()),
    )
}

/// latin-1（M18）`sftp_read_file` 裸包分支：显示路径还原字节后
/// OPEN(READ) + READ 分块循环（`RawSftp::read_file`，32 KiB 粒度，v3 规范
/// 建议口径）。大文件策略沿既有 MCP 边界：单次至多 `max_bytes`（上限
/// maxDownloadBytes），用 `max_bytes + 1` 探测截断——与 auto 分支 `take()`
/// 语义一致。返回 `(数据, 是否截断)`。
pub(super) async fn raw_sftp_read_file<S>(
    client: &mut sftp_raw::RawSftp<S>,
    path: &str,
    offset: u64,
    max_bytes: u64,
) -> Result<(Vec<u8>, bool), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + std::marker::Unpin,
{
    let mut data = client
        .read_file(
            &sftp_name::latin1_encode_display(path),
            offset,
            max_bytes.saturating_add(1),
        )
        .await?;
    let truncated = data.len() as u64 > max_bytes;
    if truncated {
        data.truncate(max_bytes as usize);
    }
    Ok((data, truncated))
}

/// `sftp_read_file` 响应装配（auto 与 latin-1 共用）。
pub(super) fn read_file_response(
    path: &str,
    data: &[u8],
    truncated: bool,
    as_base64: bool,
) -> Value {
    if as_base64 {
        json!({
            "path": path,
            "dataBase64": BASE64_STANDARD.encode(data),
            "truncated": truncated,
        })
    } else {
        json!({
            "path": path,
            "content": String::from_utf8_lossy(data),
            "truncated": truncated,
        })
    }
}

/// latin-1（M18）`sftp_write_file` 裸包分支：显示路径还原字节后
/// OPEN(CREAT|WRITE|TRUNC) 截断直写 + WRITE 32 KiB 分块（选型：MCP 面
/// 沿既有 sftp_write_file 直写语义，无工作台上传族的 `.dbx-part` 暂存
/// 需求）。`overwrite=false` 的覆盖预检走裸包 LSTAT，与写入同一字节口径；
/// 操作错误原样上抛不回退（与 M17-B 写工具同策略，避免重复执行）。
pub(super) async fn raw_sftp_write_file<S>(
    client: &mut sftp_raw::RawSftp<S>,
    path: &str,
    content: &[u8],
    overwrite: bool,
) -> Result<Value, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + std::marker::Unpin,
{
    let bytes = raw_sftp_write_bytes(client, path, content, overwrite).await?;
    Ok(json!({ "path": path, "bytes": bytes }))
}

/// latin-1（M19）`sftp_upload` 裸包车道共用核心：与 [`raw_sftp_write_file`]
/// 同一直写语义，返回写入字节数供调用方按工具各自的响应形状组装
/// （sftp_write_file → `{path, bytes}`；sftp_upload → `{localPath,
/// remotePath, bytes}`）。
pub(super) async fn raw_sftp_write_bytes<S>(
    client: &mut sftp_raw::RawSftp<S>,
    path: &str,
    content: &[u8],
    overwrite: bool,
) -> Result<usize, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + std::marker::Unpin,
{
    let raw_path = sftp_name::latin1_encode_display(path);
    if !overwrite && client.lstat(&raw_path).await.is_ok() {
        return Err(format!(
            "Remote path already exists: {path} (pass overwrite=true to replace)"
        ));
    }
    let handle = client.open_write(&raw_path).await?;
    let result = async {
        for (index, chunk) in content.chunks(sftp_raw::MAX_WRITE_CHUNK).enumerate() {
            client
                .write_chunk(&handle, (index * sftp_raw::MAX_WRITE_CHUNK) as u64, chunk)
                .await?;
        }
        Ok::<(), String>(())
    }
    .await;
    // 写失败也先 CLOSE 释放句柄，再上抛原错误。
    let closed = client.close(&handle).await;
    result?;
    closed?;
    Ok(content.len())
}

pub(super) fn sftp_error(error: impl std::fmt::Display) -> String {
    format!("SFTP operation failed: {error}")
}

/// Resolves the allowed local roots for agent-driven `sftp_upload` /
/// `sftp_download` transfers: the configured `localTransferRoot` when set
/// (it must resolve — an unreachable root is an error, never a silent
/// fallback), otherwise the OS temp dir plus the plugin data dir. Roots
/// are canonicalized so macOS `/var` → `/private/var` style aliasing
/// cannot dodge the `starts_with` containment check.
pub(super) fn local_transfer_roots_for(
    configured: &str,
    data_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    if !configured.is_empty() {
        return std::fs::canonicalize(configured)
            .map(|root| vec![root])
            .map_err(|error| {
                format!(
                    "Configured localTransferRoot {configured} cannot be resolved: {error} \
                     (create the directory or reset it via mcp/settings/set)"
                )
            });
    }
    Ok([std::env::temp_dir(), data_dir.to_path_buf()]
        .into_iter()
        .filter_map(|root| std::fs::canonicalize(&root).ok())
        .collect())
}

/// Containment gate for every agent-supplied local path behind
/// `sftp_upload` (read side) and `sftp_download` (write side): the
/// canonical path must stay inside an allowed root and clear the
/// sensitive-path blocklist (credential stores, shell bootstrap files) in
/// every mode, so a configured root cannot be used to reach them either.
/// `path` must already be canonical — symlink and `..` traversal artifacts
/// are resolved by the caller (`std::fs::canonicalize`, or the
/// canonical-parent rejoin on the download side).
pub(super) fn ensure_local_transfer_allowed_in(
    roots: &[PathBuf],
    configured: &str,
    path: &Path,
) -> Result<(), String> {
    if is_sensitive_local_path(&path.to_string_lossy()) {
        return Err(format!(
            "Refusing to transfer the local sensitive path {} via sftp_upload/sftp_download",
            path.display()
        ));
    }
    if roots.iter().any(|root| path.starts_with(root)) {
        return Ok(());
    }
    Err(if configured.is_empty() {
        format!(
            "Local path {} is outside the allowed transfer roots (OS temp dir and plugin data \
             dir); set localTransferRoot via mcp/settings/set to allow more",
            path.display()
        )
    } else {
        format!(
            "Local path {} is outside the configured localTransferRoot {configured}",
            path.display()
        )
    })
}
