//! SSH transfer support.

use super::*;

impl SshRuntime {
    /// 压缩任务的单文件分块泵（M33）：从本地解压后的 `staged_plain` 按
    /// offset 读（普通任务远端 seek+read）。限速不适用——网络腿已在 prep
    /// 拉取段按压缩流限速；二进制帧/进度/offset 校验与普通路径同语义。
    pub(crate) async fn download_staged_chunk(
        &self,
        task_id: &str,
        offset: u64,
        download: DownloadState,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let Some(staged) = download.staged_plain.as_ref() else {
            return Err("Compressed download staging is missing".to_string());
        };
        if !download.ready {
            return Err("Compressed download is still preparing".to_string());
        }
        if offset != download.next_offset {
            return Err(format!(
                "Download offset mismatch: expected {}, received {offset}",
                download.next_offset
            ));
        }
        let remaining = download.size.saturating_sub(offset);
        let requested = remaining.min(download.throttle.chunk_size(TRANSFER_CHUNK_SIZE) as u64) as usize;
        let mut source = tokio::fs::File::open(staged)
            .await
            .map_err(|error| format!("Failed to open decompressed staging file: {error}"))?;
        source
            .seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|error| format!("Staged file seek failed: {error}"))?;
        let mut chunk = vec![0_u8; requested];
        let length = read_filled(&mut source, &mut chunk)
            .await
            .map_err(|error| format!("Failed to read decompressed staging file: {error}"))?;
        chunk.truncate(length);
        let next_offset = offset.saturating_add(length as u64);
        if download.emit_chunks {
            let mut payload = Vec::with_capacity(8 + length);
            payload.extend_from_slice(&offset.to_be_bytes());
            payload.extend_from_slice(&chunk);
            emitter
                .binary(&format!("sftp/download/{task_id}"), &payload)
                .map_err(plugin_error)?;
        }
        let emit_progress = {
            let mut downloads = self
                .downloads
                .lock()
                .map_err(|_| "Download registry is poisoned".to_string())?;
            let current = downloads
                .get_mut(task_id)
                .ok_or("Download task was not found")?;
            if current.next_offset != offset {
                return Err("Download task changed while a chunk was in flight".to_string());
            }
            current.next_offset = next_offset;
            current
                .progress_throttle
                .should_emit(download.size > 0 && next_offset >= download.size)
        };
        if emit_progress {
            emitter
                .event(
                    "sftp/transfer/progress",
                    // phase=transferring（2026-09-30 补齐）：ready 之后从本地
                    // 解压暂存分块供给的真实网络外发段——前端归并器换阶段即
                    // 重置计数（分母=原始体积），进度条/速度不再卡在解压终值。
                    json!({ "taskId": task_id, "sessionId": download.session_id, "direction": "download", "transferred": next_offset, "size": download.size, "status": "running", "phase": "transferring", "compression": "gzip" }),
                )
                .map_err(plugin_error)?;
        }
        Ok(
            json!({ "taskId": task_id, "offset": offset, "length": length, "eof": next_offset >= download.size, "fileName": download.file_name }),
        )
    }

    pub async fn download_chunk(
        &self,
        task_id: &str,
        offset: u64,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let download = {
            let downloads = self
                .downloads
                .lock()
                .map_err(|_| "Download registry is poisoned".to_string())?;
            downloads
                .get(task_id)
                .cloned()
                .ok_or("Download task was not found")?
        };
        // 树任务不再深拷贝：`tree` 是 Arc，分块泵在 `tree.pump` 下就地推进
        // 队列/句柄/失败汇总，`progress` 只在快照与提交时短暂加锁（I/O 期间
        // 释放，cancel/finish 不会被限速块拖住）。
        if download.tree.is_some() {
            return self
                .download_tree_chunk(task_id, offset, download, emitter)
                .await;
        }
        if download.compression == CompressionMode::Gzip && !download.serving_plain {
            return self
                .download_staged_chunk(task_id, offset, download, emitter)
                .await;
        }
        if offset != download.next_offset {
            return Err(format!(
                "Download offset mismatch: expected {}, received {offset}",
                download.next_offset
            ));
        }
        let remaining = download.size.saturating_sub(offset);
        let requested = remaining.min(download.throttle.chunk_size(TRANSFER_CHUNK_SIZE) as u64) as usize;
        // 限速节奏（issue #66）：块耗时从读开始计量（含网络/磁盘），读毕
        // 由 limiter 补足差额等待。限速关闭时 Throttle::pace 是纯即时调用，
        // 热路径零开销。
        let chunk_started = download.throttle.enabled().then(std::time::Instant::now);
        // latin-1 转义路径走裸包 READ（raw 字节打开远端文件）；普通路径
        // 保持高层客户端的 seek+read。车道判定按 start 登记的生效编码
        // （M28-B 修 D-7）：auto 一律走高层——auto 列表 uri 字面 `%` 未
        // 未经 `%25` 自转义，wire 串里的 `%XX` 是文件名字面量，不能还原。
        // 每 chunk 独立 open/close：转义名是极少数派，简单性优先。raw EOF
        // 回空 chunk，与高层路径的 eof 语义一致。
        let chunk = if download.latin1 && sftp_name::has_wire_escapes(&download.remote_path) {
            self.raw_read_chunk_cached(
                &download.session_id,
                &download.raw_client,
                &download.remote_path,
                offset,
                requested as u32,
            )
            .await?
        } else {
            // 句柄复用 + 读满一片：首片 open 后缓存在任务槽（File 的 Drop
            // 发 close_nowait，槽位置 None 即完成关闭）。顺序分片只有首次
            // 打开/非连续 offset 才 seek——每片 seek 会清空 crate 的在途读
            // 窗口，read_filled 则把整个窗口的数据读满同一片，避免
            // “一个 32KiB 包换一次 RTT”。断点续传/重试跳片仍按绝对 offset
            // 重新对齐。
            let read_result: Result<Vec<u8>, String> = async {
                let sftp = self.sftp(&download.session_id).await?;
                let mut handle_slot = download.read_handle.lock().await;
                if handle_slot.is_none() {
                    let opened = sftp
                        .lock()
                        .await
                        .open(download.remote_path.clone())
                        .await
                        .map_err(sftp_error)?;
                    *handle_slot = Some((opened, 0));
                }
                let (source, cursor) = handle_slot.as_mut().expect("handle just ensured");
                if *cursor != offset {
                    source
                        .seek(std::io::SeekFrom::Start(offset))
                        .await
                        .map_err(|error| format!("SFTP download seek failed: {error}"))?;
                    *cursor = offset;
                }
                let mut chunk = vec![0_u8; requested];
                let length = read_filled(source, &mut chunk)
                    .await
                    .map_err(|error| format!("SFTP download failed: {error}"))?;
                *cursor = offset.saturating_add(length as u64);
                chunk.truncate(length);
                Ok(chunk)
            }
            .await;
            match read_result {
                Ok(chunk) => chunk,
                Err(error) => {
                    // 读失败即弃句柄（下一片惰性重建）；通道级错误连带失效
                    // 整个 SFTP 客户端缓存——死通道被原样复用会把该会话
                    // SFTP 打到永久报错（误判无害：重建只是多一次握手）。
                    *download.read_handle.lock().await = None;
                    if is_channel_level_sftp_error(&error) {
                        self.invalidate_sftp_cache(&download.session_id).await;
                    }
                    return Err(error);
                }
            }
        };
        // 限速等待在读毕、本地落盘之前：等待本身计入下一块之前的墙钟，
        // 且不会推迟本地写完成的应答语义（前端循环按块等待）。
        if let (true, Some(started)) = (download.throttle.enabled(), chunk_started) {
            download.throttle.pace(chunk.len(), started.elapsed()).await;
        }
        let length = chunk.len();
        if let Some(sink) = download.sink.as_ref() {
            sink.file
                .lock()
                .await
                .write_all(&chunk)
                .await
                .map_err(|error| format!("Failed to write local download file: {error}"))?;
        }
        let next_offset = offset.saturating_add(length as u64);
        if download.emit_chunks {
            let mut payload = Vec::with_capacity(8 + length);
            payload.extend_from_slice(&offset.to_be_bytes());
            payload.extend_from_slice(&chunk);
            emitter
                .binary(&format!("sftp/download/{task_id}"), &payload)
                .map_err(plugin_error)?;
        }
        // 进度事件时间窗节流（progress_throttle）：每块一条 IPC 事件在
        // 大文件下约 200 条/秒，直驱前端重渲染；终块必发保证进度条终值。
        let emit_progress = {
            let mut downloads = self
                .downloads
                .lock()
                .map_err(|_| "Download registry is poisoned".to_string())?;
            let current = downloads
                .get_mut(task_id)
                .ok_or("Download task was not found")?;
            if current.next_offset != offset {
                return Err("Download task changed while a chunk was in flight".to_string());
            }
            current.next_offset = next_offset;
            current
                .progress_throttle
                .should_emit(download.size > 0 && next_offset >= download.size)
        };
        if emit_progress {
            emitter
                .event(
                    "sftp/transfer/progress",
                    json!({ "taskId": task_id, "sessionId": download.session_id, "direction": "download", "transferred": next_offset, "size": download.size, "status": "running" }),
                )
                .map_err(plugin_error)?;
        }
        Ok(
            json!({ "taskId": task_id, "offset": offset, "length": length, "eof": next_offset >= download.size, "fileName": download.file_name }),
        )
    }

    /// `sftp/transfer/cancel`. `reason` is an optional workbench slug ("user",
    /// "ack-timeout", ...) recorded in the ledger event so a cancellation can
    /// be told apart from a server failure on the next bug report.
    /// 统一清理一个下载任务的全部临时资产（2026-09-30 收口，评审 H-2）：
    /// sink .part、树半成品根目录、sudo 远端暂存、压缩任务的 plain 解压
    /// 暂存/本地压缩流/远端 .gz。best-effort——路径可缺失，完成/取消/会话
    /// 关闭多路径穿过这里，重复删除只是一次无害的 NoSuchFile。远端删除
    /// 需要会话，会话已死时记 stderr（本地资产是会话关闭场景的主清理面）。
    pub(crate) async fn cleanup_download_assets(&self, session_id: &str, download: &DownloadState) {
        if let Some(sink) = download.sink.as_ref() {
            let _ = std::fs::remove_file(&sink.part_path);
        }
        // 文件夹下载：整棵半成品目录删除，不在下载目录里留部分内容（根
        // 目录是本任务创建的让位新目录，删除不伤及他物）。先短暂取 progress
        // 锁清掉缓存的远端读句柄（Drop 发 close_nowait）；分块泵的 I/O 不在
        // 该锁内，所以取消不会被整块阻塞。若泵正把句柄临时移出锁外读取，句柄
        // 在读取返回后自行丢弃（泵在提交前后检查任务是否仍在注册表）。
        if let Some(tree) = download.tree.as_ref() {
            {
                let mut progress = tree.progress.lock().await;
                progress.read_handle = None;
                progress.handle_path = None;
            }
            let _ = std::fs::remove_dir_all(&tree.root_local);
        }
        if let Some(tmp) = download.sudo_tmp.as_ref() {
            if let Err(error) = sudo_download::discard_tmp(self, session_id, tmp).await {
                eprintln!("[sudo-download] temp cleanup failed ({tmp}): {error}");
            }
        }
        if download.compression == CompressionMode::Gzip {
            if let Some(staged) = download.staged_plain.as_ref() {
                let _ = std::fs::remove_file(staged);
            }
            if let Some(gz) = download.gz_staging.as_ref() {
                let _ = std::fs::remove_file(gz);
            }
            if let Some(tmp) = download.remote_temp.as_ref() {
                match self.sftp(session_id).await {
                    Ok(sftp) => {
                        if let Err(error) = sftp.lock().await.remove_file(tmp.clone()).await {
                            eprintln!("[sftp] cleanup: remote temp not removed: {error}");
                        }
                    }
                    Err(error) => eprintln!("[sftp] cleanup: session unavailable: {error}"),
                }
            }
        }
    }
}
