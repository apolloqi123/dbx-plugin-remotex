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
}

mod download;
mod paths;
mod upload;

pub(in crate::features::mcp) use paths::*;
