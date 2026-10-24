use super::*;

impl SshRuntime {
    /// Chunk pump for folder downloads. `next_offset` stays the aggregate byte
    /// position across the tree; when the in-flight file completes it is
    /// renamed into place and the next queued file opens within the same call,
    /// so the caller's chunk loop is identical to a plain download. A tree
    /// whose queue is drained answers with an empty eof chunk (this is also
    /// how a zero-byte tree completes — the frontend loop runs until eof, not
    /// until the byte total).
    ///
    /// Locking: `tree.pump` serializes chunk calls (the caller loop is
    /// sequential per task and a stale duplicate must not read twice), while
    /// `tree.progress` is only held for state snapshots/commits — never across
    /// SFTP open/seek/read, local sink writes, throttle pacing or frame
    /// emission. Cancel/finish can therefore acquire `progress` promptly even
    /// mid-chunk. The cancel signal is the task leaving the download registry;
    /// it is re-checked between I/O steps so an in-flight chunk aborts early.
    pub(in super::super) async fn download_tree_chunk(
        &self,
        task_id: &str,
        offset: u64,
        download: DownloadState,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        enum TreeChunkStep {
            OpenSink,
            Read {
                file: sftp_tree::TreeFile,
                current_offset: u64,
                sink: Arc<DownloadSink>,
            },
            Drain,
        }

        let Some(tree) = download.tree.clone() else {
            return Err("Download task is not a folder download".to_string());
        };
        // One pump at a time; the progress lock stays free for cancel/finish.
        let _pump = tree.pump.lock().await;
        // Re-read the authoritative state after waiting: a duplicate/stale call
        // must be validated against the offset the previous chunk committed.
        let download = self
            .downloads
            .lock()
            .map_err(|_| "Download registry is poisoned".to_string())?
            .get(task_id)
            .cloned()
            .ok_or("Download task was cancelled")?;
        if download.compression == CompressionMode::Gzip && !download.ready {
            return Err("Compressed download is still preparing".to_string());
        }
        if offset != download.next_offset {
            return Err(format!(
                "Download offset mismatch: expected {}, received {offset}",
                download.next_offset
            ));
        }
        loop {
            let step = {
                let mut progress = tree.progress.lock().await;
                if progress.current.is_none() {
                    match progress.files.pop_front() {
                        Some(file) => {
                            progress.current = Some(file);
                            progress.current_offset = 0;
                            progress.sink = None;
                            TreeChunkStep::OpenSink
                        }
                        // 队列耗尽：空树或全部走完。最后一批文件的改名/失败登记
                        // 已就地写入共享树状态，finish 能看到完整汇总；随后补发
                        // 一个空 eof 块，让前端的分块等待器（只认二进制帧）与
                        // 单文件语义保持一致。
                        None => TreeChunkStep::Drain,
                    }
                } else if progress.sink.is_none() {
                    TreeChunkStep::OpenSink
                } else {
                    TreeChunkStep::Read {
                        file: progress.current.clone().expect("current checked above"),
                        current_offset: progress.current_offset,
                        sink: progress.sink.clone().expect("sink checked above"),
                    }
                }
            };
            if !self.tree_download_active(task_id)? {
                return Err("Download task was cancelled".to_string());
            }
            match step {
                TreeChunkStep::Drain => {
                    let mut payload = Vec::with_capacity(8);
                    payload.extend_from_slice(&offset.to_be_bytes());
                    emitter
                        .binary(&format!("sftp/download/{task_id}"), &payload)
                        .map_err(plugin_error)?;
                    return Ok(
                        json!({ "taskId": task_id, "offset": offset, "length": 0, "eof": true, "fileName": download.file_name }),
                    );
                }
                TreeChunkStep::OpenSink => {
                    let relative = {
                        let progress = tree.progress.lock().await;
                        progress
                            .current
                            .as_ref()
                            .map(|file| file.relative.clone())
                            .ok_or("Folder download state lost its current file")?
                    };
                    match open_tree_sink(&tree.root_local, &relative).await {
                        Ok(sink) => {
                            let sink = Arc::new(sink);
                            if !self.tree_download_active(task_id)? {
                                // Cancelled while the sink was opening: cleanup
                                // may already have removed the root; do not
                                // leave a re-created `.part` behind.
                                let _ = std::fs::remove_file(&sink.part_path);
                                let _ = std::fs::remove_dir_all(&tree.root_local);
                                return Err("Download task was cancelled".to_string());
                            }
                            let mut progress = tree.progress.lock().await;
                            // The pump lock makes this defensive: a stale sink
                            // must never attach to a different current file.
                            if progress.current.as_ref().map(|file| file.relative.as_str())
                                != Some(relative.as_str())
                            {
                                drop(progress);
                                let _ = std::fs::remove_file(&sink.part_path);
                                return Err("Folder download state changed while opening a file"
                                    .to_string());
                            }
                            progress.sink = Some(sink);
                        }
                        Err(error) => {
                            let stale = {
                                let mut progress = tree.progress.lock().await;
                                progress
                                    .failures
                                    .push(json!({ "path": relative, "error": error }));
                                discard_tree_current(&mut progress)
                            };
                            remove_tree_sink(stale);
                        }
                    }
                    continue;
                }
                TreeChunkStep::Read {
                    file,
                    current_offset,
                    sink,
                } => {
                    let remaining = file.size.saturating_sub(current_offset);
                    if remaining == 0 {
                        // 当前文件收尾：暂存 .part 改名落位（空文件也会在这一步
                        // 真实落地），失败记入汇总且不中断整树。
                        finalize_tree_current(self, &tree).await;
                        continue;
                    }
                    let requested = remaining.min(TRANSFER_CHUNK_SIZE as u64) as usize;
                    // 限速节奏（issue #66）：树下载逐文件分块循环与单文件同口径——
                    // 块耗时从读开始计量，读毕由 limiter 补足差额；缺省 0 零开销。
                    // 压缩树不限速（网络腿已在 prep 拉取段按压缩流限速，这里是
                    // 本地读，限速只添堵）。
                    let chunk_started = (download.throttle.enabled()
                        && (download.compression != CompressionMode::Gzip
                            || download.serving_plain))
                        .then(std::time::Instant::now);
                    let mut handle_out: Option<TreeReadHandle> = None;
                    let read_result: Result<(Vec<u8>, usize), TreeReadFailure> =
                        if download.compression == CompressionMode::Gzip && !download.serving_plain
                        {
                            // 压缩树（M33）：整树已由 prep 解包到本地 staging 根，
                            // 逐文件从本地镜像按 offset 读（不再远端 open）。单文件
                            // 读取失败照旧记入汇总跳过，不中断整树。
                            let read = async {
                                let mut source =
                                    tokio::fs::File::open(tree.root_local.join(&file.relative))
                                        .await
                                        .map_err(|error| format!("staged read failed: {error}"))?;
                                source
                                    .seek(std::io::SeekFrom::Start(current_offset))
                                    .await
                                    .map_err(|error| format!("staged seek failed: {error}"))?;
                                let mut chunk = vec![0_u8; requested];
                                let length = source
                                    .read(&mut chunk)
                                    .await
                                    .map_err(|error| format!("staged read failed: {error}"))?;
                                Ok::<(Vec<u8>, usize), String>((chunk, length))
                            }
                            .await;
                            read.map_err(|message| TreeReadFailure {
                                message,
                                channel_level: false,
                            })
                        } else if tree.latin1 {
                            // latin-1：远端路径是 wire 形式（raw 扫描），高层 open
                            // 按 UTF-8 找不到字节名文件——走裸包 READ（download
                            // 分片的 raw_read_chunk 同款：整条 unescape 后裸包
                            // OPEN/READ）。裸包文件句柄仍按分片 open/close
                            // （RawSftpClient 的一次性语义），客户端本身跨分片复用。
                            match self
                                .raw_read_chunk_cached(
                                    &download.session_id,
                                    &download.raw_client,
                                    &file.remote_path,
                                    current_offset,
                                    requested as u32,
                                )
                                .await
                            {
                                Ok(data) => {
                                    let length = data.len();
                                    Ok((data, length))
                                }
                                Err(message) => Err(TreeReadFailure {
                                    message,
                                    channel_level: false,
                                }),
                            }
                        } else {
                            // 句柄复用（与单文件路径同口径）：同一文件的后续分片
                            // 直接 seek+read，只有切换文件（handle_path 不匹配）
                            // 或错误/收尾才丢弃句柄重新 open。句柄在 I/O 期间被
                            // 临时移出 progress，完成后写回句柄槽。
                            match self
                                .read_tree_high_level(
                                    &tree,
                                    &download.session_id,
                                    &file,
                                    current_offset,
                                    requested,
                                )
                                .await
                            {
                                Ok((chunk, length, source)) => {
                                    handle_out = Some(source);
                                    Ok((chunk, length))
                                }
                                Err(failure) => Err(failure),
                            }
                        };
                    let (mut chunk, length) = match read_result {
                        Ok(read) => read,
                        Err(failure) => {
                            drop(handle_out);
                            let stale = {
                                let mut progress = tree.progress.lock().await;
                                progress.failures.push(
                                    json!({ "path": file.relative, "error": failure.message }),
                                );
                                discard_tree_current(&mut progress)
                            };
                            remove_tree_sink(stale);
                            if failure.channel_level {
                                self.invalidate_sftp_cache(&download.session_id).await;
                                return Err(
                                    "SFTP channel died during folder download; reconnect and retry the remaining files"
                                        .to_string(),
                                );
                            }
                            continue;
                        }
                    };
                    if length == 0 {
                        // 远端文件比扫描时短：只记失败，不把半成品留在本地。
                        drop(handle_out);
                        let stale = {
                            let mut progress = tree.progress.lock().await;
                            progress.failures.push(
                                json!({ "path": file.relative, "error": "file shrank below its scanned size" }),
                            );
                            discard_tree_current(&mut progress)
                        };
                        remove_tree_sink(stale);
                        continue;
                    }
                    chunk.truncate(length);
                    // Cache the reused handle only once the read succeeded; a
                    // cancel drops it instead of resurrecting the slot.
                    if let Some(source) = handle_out {
                        if self.tree_download_active(task_id)? {
                            let mut progress = tree.progress.lock().await;
                            progress.read_handle = Some(source);
                            progress.handle_path = Some(file.remote_path.clone());
                        }
                    }
                    // 限速等待读毕即执行（本地写之前），与单文件路径同口径。
                    // Pacing happens with `progress` released: a long throttled
                    // pause must never delay cancel/finish.
                    if download.throttle.enabled() {
                        if let Some(started) = chunk_started {
                            download.throttle.pace(length, started.elapsed()).await;
                        }
                    }
                    if !self.tree_download_active(task_id)? {
                        return Err("Download task was cancelled".to_string());
                    }
                    // Stage the bytes before emitting. A failed emission rolls
                    // the staging length back so a same-offset retry appends
                    // exactly once (offset commit below happens only after the
                    // frame left the emitter).
                    {
                        let mut local = sink.file.lock().await;
                        let written = async {
                            local.seek(std::io::SeekFrom::Start(current_offset)).await?;
                            local.write_all(&chunk).await
                        }
                        .await;
                        if let Err(error) = written {
                            let _ = local.set_len(current_offset).await;
                            drop(local);
                            let stale = {
                                let mut progress = tree.progress.lock().await;
                                progress.failures.push(json!({ "path": file.relative, "error": format!("Failed to write local download file: {error}") }));
                                discard_tree_current(&mut progress)
                            };
                            remove_tree_sink(stale);
                            continue;
                        }
                    }
                    if !self.tree_download_active(task_id)? {
                        let local = sink.file.lock().await;
                        let _ = local.set_len(current_offset).await;
                        return Err("Download task was cancelled".to_string());
                    }
                    let next_offset = offset + length as u64;
                    let mut payload = Vec::with_capacity(8 + length);
                    payload.extend_from_slice(&offset.to_be_bytes());
                    payload.extend_from_slice(&chunk);
                    if let Err(error) =
                        emitter.binary(&format!("sftp/download/{task_id}"), &payload)
                    {
                        // The caller never saw the frame: do not advance either
                        // offset, and undo the staged bytes so the retry of the
                        // same offset does not duplicate them.
                        let local = sink.file.lock().await;
                        let _ = local.set_len(current_offset).await;
                        return Err(plugin_error(error));
                    }
                    // 进度事件时间窗节流（progress_throttle），eof 帧必发兜底树
                    // 收尾/文件切换的最终进度可见。Both offsets commit only
                    // after the frame and the staging write succeeded.
                    {
                        // Validate against the registry without holding the
                        // std guard across the async progress lock (that would
                        // make this future non-Send).
                        let downloads = self
                            .downloads
                            .lock()
                            .map_err(|_| "Download registry is poisoned".to_string())?;
                        let current = downloads
                            .get(task_id)
                            .ok_or("Download task was cancelled")?;
                        if current.next_offset != offset {
                            return Err(
                                "Download task changed while a chunk was in flight".to_string()
                            );
                        }
                    }
                    let file_done = {
                        let mut progress = tree.progress.lock().await;
                        if progress
                            .current
                            .as_ref()
                            .map(|file| file.remote_path.as_str())
                            != Some(file.remote_path.as_str())
                            || progress.current_offset != current_offset
                        {
                            return Err(
                                "Folder download state changed while a chunk was in flight"
                                    .to_string(),
                            );
                        }
                        progress.current_offset = current_offset + length as u64;
                        progress.current_offset >= file.size
                    };
                    // The pump lock serializes chunk calls, so no other writer
                    // can have moved next_offset between the validation above
                    // and this commit (cancel/complete remove the entry and
                    // are handled by the re-check).
                    let emit_progress = {
                        let mut downloads = self
                            .downloads
                            .lock()
                            .map_err(|_| "Download registry is poisoned".to_string())?;
                        let current = downloads
                            .get_mut(task_id)
                            .ok_or("Download task was cancelled")?;
                        if current.next_offset != offset {
                            return Err(
                                "Download task changed while a chunk was in flight".to_string()
                            );
                        }
                        current.next_offset = next_offset;
                        current
                            .progress_throttle
                            .should_emit(download.size > 0 && next_offset >= download.size)
                    };
                    // 文件耗尽即在本调用内收尾（改名落位）：eof 与落位必须同帧，
                    // 否则 eof 后调用方直接 finish，最后一个文件会被记成未传输。
                    if file_done {
                        finalize_tree_current(self, &tree).await;
                    }
                    let (queued, current_remaining, files_done, current_file) = {
                        let progress = tree.progress.lock().await;
                        (
                            progress.files.len(),
                            progress
                                .current
                                .as_ref()
                                .map(|file| file.size.saturating_sub(progress.current_offset))
                                .unwrap_or(0),
                            progress.files_done,
                            progress.current.as_ref().map(|file| file.relative.clone()),
                        )
                    };
                    let eof = sftp_tree::tree_eof(queued, current_remaining);
                    let progress_event = (emit_progress || eof).then(|| {
                        // 压缩树（含 prep 回退）供给段标 phase=transferring：前端换
                        // 阶段重置计数，不再停在解包终值（M-1，2026-09-30 补齐）。
                        // 普通树维持无 phase（线上兼容）。
                        let mut event = json!({ "taskId": task_id, "sessionId": download.session_id, "direction": "download", "transferred": next_offset, "size": download.size, "status": "running", "fileCount": tree.file_count, "fileIndex": files_done + u64::from(current_file.is_some()), "currentFile": current_file });
                        if download.compression == CompressionMode::Gzip {
                            event["phase"] = json!("transferring");
                        }
                        event
                    });
                    if let Some(event) = progress_event {
                        emitter
                            .event("sftp/transfer/progress", event)
                            .map_err(plugin_error)?;
                    }
                    return Ok(
                        json!({ "taskId": task_id, "offset": offset, "length": length, "eof": eof, "fileName": download.file_name }),
                    );
                }
            }
        }
    }
}
