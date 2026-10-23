//! Tree Download operations for [`SshRuntime`].

use super::*;

impl SshRuntime {
    /// `sftp/download/tree/start`: recursive folder download. Pure SFTP — the
    /// remote tree is walked with `read_dir` (no shell, no remote temp
    /// archive), regular files stream one at a time through the same chunked
    /// `sftp/download/next` pipeline as plain downloads, and the local layout
    /// mirrors the remote one under a fresh, collision-free folder (empty
    /// directories included). Symlinks are never followed (cycle protection);
    /// per-file problems are recorded and skipped so one bad file cannot sink
    /// the whole tree.
    pub async fn start_tree_download(
        self: &Arc<Self>,
        session_id: &str,
        remote_path: &str,
        download_dir: Option<&str>,
        encoding: NameEncoding,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        if self.active_transfer_count(session_id)? as u64 >= self.transfer_depth_limit() {
            return Err(format!(
                "This SSH session already has {} active transfers",
                self.transfer_depth_limit()
            ));
        }
        let root_remote = normalize_remote_path(remote_path)?;
        let sftp = self.sftp(session_id).await?;
        // 本地根目录：与单文件下载共用目录语义（偏好下载目录 / 自定义绝对
        // 目录），根名撞车让位 " (n)"。落点在 start 时定死，任务取消或未完成
        // 时整树删除，所以提前占名不会留下悬空目录。
        let base_dir = download_dir
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                local_downloads::downloads_base_dir(|key| std::env::var_os(key), &self.data_dir)
            });
        if !base_dir.is_absolute() {
            return Err("Download directory must be an absolute path".to_string());
        }
        std::fs::create_dir_all(&base_dir).map_err(|error| {
            format!(
                "Failed to create download directory '{}': {error}",
                base_dir.display()
            )
        })?;
        // latin-1：本地根名用原始字节的 latin-1 解码显示名（wire 转义名按
        // 字面量落盘会变成 "%E9" 这类乱名）；auto 维持 wire 字符串。
        let root_raw = sftp_name::unescape_wire(&root_remote);
        let root_name = if encoding == NameEncoding::Latin1 {
            root_raw
                .split(|&byte| byte == b'/')
                .rev()
                .find(|part| !part.is_empty())
                .map(|part| sftp_name::decode_display_name(part, encoding).text)
                .unwrap_or_else(|| "download".to_string())
        } else {
            root_remote
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
                .unwrap_or("download")
                .to_string()
        };
        // latin-1：远端遍历走裸包 READDIR（同一通道 LSTAT 预检 + 递归），
        // 整树路径字节保真；裸包通道建立失败回退高层遍历（只读，安全）。
        // auto 维持高层客户端（合法 UTF-8 服务器字节往返无损）。扫描前置
        // （压缩决策要总字节），扫描只读远端，与本地落点创建无顺序耦合。
        let scan = if encoding == NameEncoding::Latin1 {
            match self.raw_sftp_client(session_id).await {
                Ok(mut client) => scan_tree_with_raw(&mut client, &root_remote).await,
                Err(error) => {
                    eprintln!(
                        "[dbx-plugin-sshx] raw byte tree scan unavailable, falling back: {error}"
                    );
                    scan_remote_tree(&sftp, &root_remote).await
                }
            }
        } else {
            scan_remote_tree(&sftp, &root_remote).await
        };
        let scan = match scan {
            Ok(scan) => scan,
            Err(error) => return Err(error),
        };
        let total = scan.total_bytes();
        let file_count = scan.file_count();
        let dir_count = scan.dir_count();
        let skipped = scan.skipped;
        let task_id = Uuid::new_v4().to_string();
        // 限速快照（issue #66）：树下载与单文件同口径——偏好现值只在启动
        // 时读一次，逐文件分块循环共用同一限速器。
        let throttle = Throttle::new(crate::core::preferences::transfer_download_limit_kib(
            &self.data_dir,
        ));
        // 压缩树通道决策（M33）：latin-1 裸包车道强制 None（路径不进
        // shell）；空树没有传输量可省。树不做比率守卫——归档对混合内容
        // 几乎必赚，tar 头开销可忽略。（W6 单入口：latin-1 作为决策参数
        // 传入，不再在外层重复挡一道。）
        let compression = if file_count == 0 {
            CompressionMode::None
        } else {
            let read_only = self
                .session(session_id)
                .await
                .map(|session| session.read_only)
                .unwrap_or(true);
            self.decide_transfer_compression(
                session_id,
                total,
                None,
                true,
                read_only,
                encoding == NameEncoding::Latin1,
            )
            .await
        };
        // 压缩树的 staging 根建在落点目录内的隐藏目录：与最终落点同卷，
        // 完成时 rename 零拷贝落位；取消/未完成整目录拆除（与普通树的
        // root_local 清理同路径）。普通树维持现状（start 时占名）。
        let root_local = if compression == CompressionMode::Gzip {
            base_dir.join(format!(".dbx-tree-{task_id}.staging"))
        } else {
            local_downloads::final_download_path(&base_dir, &root_name, false)
        };
        std::fs::create_dir_all(&root_local).map_err(|error| {
            format!(
                "Failed to create download folder '{}': {error}",
                root_local.display()
            )
        })?;
        // 本地目录骨架先行：空目录也保留（压缩树由解包建目录，跳过）。
        if compression != CompressionMode::Gzip {
            for relative in &scan.dirs {
                let Some(path) = sftp_tree::safe_tree_path(&root_local, relative) else {
                    continue;
                };
                if let Err(error) = std::fs::create_dir_all(&path) {
                    let _ = std::fs::remove_dir_all(&root_local);
                    return Err(format!(
                        "Failed to create local folder '{}': {error}",
                        path.display()
                    ));
                }
            }
        }
        let file_name = if compression == CompressionMode::Gzip {
            // staging 目录用任务 id 命名，显示名取根名；最终落点名在完成
            // 时经 final_download_path 让位 "(n)"。
            root_name.clone()
        } else {
            root_local
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| root_name.to_string())
        };
        // 压缩树的压缩流暂存与远端临时件路径（文件由 prep 任务创建）。
        let gz_staging = (compression == CompressionMode::Gzip).then(|| {
            self.transfer_dir
                .join("downloads")
                .join(format!("tree-{task_id}.tgz"))
        });
        let remote_temp = (compression == CompressionMode::Gzip).then(|| {
            let parent = root_remote
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .filter(|parent| !parent.is_empty())
                .unwrap_or("/");
            format!("{}/.dbx-tree-{task_id}.tgz", parent.trim_end_matches('/'))
        });
        // Validate the compressed staging pair before registering the task
        // (same policy as the single-file path: an invariant violation must
        // surface as an error, not a panic on the RPC worker).
        let compressed_paths = if compression == CompressionMode::Gzip {
            match (gz_staging.clone(), remote_temp.clone()) {
                (Some(gz), Some(remote)) => Some((gz, remote)),
                _ => {
                    // The local skeleton (and, for a compressed tree, the
                    // staging root) was already created above; an invariant
                    // failure must not leave it behind.
                    let _ = std::fs::remove_dir_all(&root_local);
                    return Err("Compressed tree download staging paths are missing".to_string());
                }
            }
        } else {
            None
        };
        let insertion = self
            .downloads
            .lock()
            .map_err(|_| "Download registry is poisoned".to_string());
        let mut downloads = match insertion {
            Ok(downloads) => downloads,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&root_local);
                return Err(error);
            }
        };
        downloads.insert(
            task_id.clone(),
            DownloadState {
                session_id: session_id.to_string(),
                remote_path: root_remote.clone(),
                file_name: file_name.clone(),
                size: total,
                next_offset: 0,
                sink: None,
                tree: Some(Arc::new(TreeDownloadState::new(
                    root_local.clone(),
                    file_count,
                    skipped,
                    encoding == NameEncoding::Latin1,
                    TreeDownloadProgress {
                        files: VecDeque::from(scan.files),
                        current: None,
                        current_offset: 0,
                        sink: None,
                        files_done: 0,
                        failures: scan.failures,
                        read_handle: None,
                        handle_path: None,
                    },
                ))),
                sudo_tmp: None,
                // 树任务的逐文件读取按 TreeDownloadState.latin1 判分支。
                latin1: false,
                throttle,
                progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
                compression,
                ready: false,
                serving_plain: false,
                staged_plain: None,
                gz_staging: gz_staging.clone(),
                remote_temp: remote_temp.clone(),
                final_dir: (compression == CompressionMode::Gzip).then(|| base_dir.clone()),
                overwrite: false,
                read_handle: Arc::new(AsyncMutex::new(None)),
                raw_client: Arc::new(AsyncMutex::new(None)),
            },
        );
        drop(downloads);
        if let Some((gz_path, remote_path_for_prep)) = compressed_paths {
            // prep 注册 + 后台任务（远端 tar.gz → 拉取 → 本地解包 → ready）。
            let cancelled = Arc::new(AtomicBool::new(false));
            let cancel_reason = Arc::new(Mutex::new(None::<String>));
            let phase = Arc::new(Mutex::new("compressing"));
            // Fallback 补建空目录骨架用（评审 L-3）：普通树在 start 建骨架，
            // 压缩树由解包建目录、跳过空目录——回退普通管线后由 prep 补。
            let tree_dirs: Arc<[String]> = scan.dirs.clone().into();
            self.register_download_prep(
                &task_id,
                DownloadPrep {
                    session_id: session_id.to_string(),
                    cancelled: cancelled.clone(),
                    cancel_reason: cancel_reason.clone(),
                    phase: phase.clone(),
                    remote_temp: Some(remote_path_for_prep.clone()),
                    gz_staging: gz_path.clone(),
                },
            )
            .await?;
            let this = self.clone();
            let prep_task_id = task_id.clone();
            let prep_session_id = session_id.to_string();
            let prep_remote_root = root_remote.clone();
            let prep_emitter = emitter.clone();
            let prep_staging_root = root_local.clone();
            tokio::spawn(async move {
                this.run_tree_prep(
                    prep_task_id,
                    prep_session_id,
                    prep_remote_root,
                    total,
                    gz_path,
                    prep_staging_root,
                    remote_path_for_prep,
                    throttle,
                    tree_dirs,
                    cancelled,
                    cancel_reason,
                    phase,
                    prep_emitter,
                )
                .await;
            });
        }
        if let Err(error) = emitter.event(
            "sftp/transfer/progress",
            json!({ "taskId": task_id, "sessionId": session_id, "direction": "download", "fileName": file_name, "transferred": 0, "size": total, "status": "queued", "fileCount": file_count, "compression": compression.as_str() }),
        ) {
            // The caller never learns the task id when start fails, so nothing
            // would ever cancel/finish it: roll the registry entry and the
            // local skeleton back instead of leaving staging dirs behind.
            if let Ok(mut downloads) = self.downloads.lock() {
                downloads.remove(&task_id);
            }
            let _ = std::fs::remove_dir_all(&root_local);
            return Err(plugin_error(error));
        }
        let connection_id = self.session_connection_id(session_id).await;
        self.record_transfer_start(
            &task_id,
            session_id,
            &connection_id,
            "download",
            &file_name,
            total,
        );
        Ok(
            json!({ "taskId": task_id, "fileName": file_name, "size": total, "chunkSize": TRANSFER_CHUNK_SIZE, "fileCount": file_count, "dirCount": dir_count, "skippedCount": skipped, "compression": compression.as_str() }),
        )
    }

    /// 压缩树 prep 终态分发（M33）：与单文件 run_download_prep 同构——
    /// Ready 发 ready(gzip)；Fallback 清压缩产物、registry 回普通管线并发
    /// ready(none)；Cancelled 走取消清理。staging 树（root_local）不属于
    /// 压缩产物：Fallback/Cancelled 的整树拆除由 cancel/complete 的既有
    /// root_local 语义负责，这里只清 .tgz 两侧临时件。
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_tree_prep(
        self: &Arc<Self>,
        task_id: String,
        session_id: String,
        remote_root: String,
        size: u64,
        gz_staging: PathBuf,
        staging_root: PathBuf,
        remote_temp: String,
        throttle: Throttle,
        dirs: Arc<[String]>,
        cancelled: Arc<AtomicBool>,
        _cancel_reason: Arc<Mutex<Option<String>>>,
        phase: Arc<Mutex<&'static str>>,
        emitter: PluginEmitter,
    ) {
        let outcome = self
            .prep_tree_staging(
                &task_id,
                &session_id,
                &remote_root,
                size,
                &gz_staging,
                &staging_root,
                &remote_temp,
                &throttle,
                &cancelled,
                &phase,
                &emitter,
            )
            .await;
        match outcome {
            PrepOutcome::Ready => {
                {
                    let mut downloads = self
                        .downloads
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner());
                    if let Some(current) = downloads.get_mut(&task_id) {
                        current.ready = true;
                    }
                }
                self.download_preps
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .remove(&task_id);
                let _ = emitter.event(
                    "sftp/transfer/progress",
                    json!({ "taskId": task_id, "sessionId": session_id, "direction": "download", "transferred": 0, "size": size, "status": "running", "phase": "ready", "compression": "gzip" }),
                );
            }
            PrepOutcome::Fallback(reason) => {
                eprintln!("[sftp] tree download {task_id} compressed prep fallback: {reason}");
                self.cleanup_compressed_download(
                    &session_id,
                    Some(&remote_temp),
                    Some(&gz_staging),
                    None,
                )
                .await;
                // 空目录也保留（与普通树同语义，评审 L-3）：压缩决策的骨架
                // 跳过在 Fallback 时补建——分块泵只为文件建父目录，空目录
                // 只能在这里补。失败不阻断（后续文件上传会按父目录建链）。
                for relative in dirs.iter() {
                    let Some(path) = sftp_tree::safe_tree_path(&staging_root, relative) else {
                        continue;
                    };
                    if let Err(error) = std::fs::create_dir_all(&path) {
                        eprintln!(
                            "[sftp] tree download {task_id} fallback skeleton failed: {error}"
                        );
                        break;
                    }
                }
                {
                    // compression 保持原决策值（status 的 compression/ready
                    // 字段继续输出，前端等待环据此收口）；serving_plain 让
                    // 分块泵走普通管线（树/单文件同语义）。
                    let mut downloads = self
                        .downloads
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner());
                    if let Some(current) = downloads.get_mut(&task_id) {
                        current.serving_plain = true;
                        current.ready = true;
                    }
                }
                self.download_preps
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .remove(&task_id);
                let _ = emitter.event(
                    "sftp/transfer/progress",
                    json!({ "taskId": task_id, "sessionId": session_id, "direction": "download", "transferred": 0, "size": size, "status": "running", "phase": "ready", "compression": "none" }),
                );
            }
            PrepOutcome::Cancelled => {
                self.cleanup_compressed_download(
                    &session_id,
                    Some(&remote_temp),
                    Some(&gz_staging),
                    None,
                )
                .await;
                self.download_preps
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .remove(&task_id);
            }
        }
    }

    /// 压缩树 prep 主体（M33）：远端 `tar -czf`（相对根目录打包，成员名
    /// 不带绝对路径）→ 拉取 .tgz → 本地解包进 staging 根（跳过链接与特
    /// 殊文件，与普通树「不跟随 symlink」语义对齐）。不做比率守卫（归档
    /// 对混合内容几乎必赚）。任一步取消即中断；远端 tar 失败/解包失败回
    /// 落普通树管线。
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn prep_tree_staging(
        self: &Arc<Self>,
        task_id: &str,
        session_id: &str,
        remote_root: &str,
        size: u64,
        gz_staging: &Path,
        staging_root: &Path,
        remote_temp: &str,
        throttle: &Throttle,
        cancelled: &Arc<AtomicBool>,
        phase: &Arc<Mutex<&'static str>>,
        emitter: &PluginEmitter,
    ) -> PrepOutcome {
        let emit_phase = |phase_name: &str,
                          transferred: u64,
                          denominator: u64,
                          compression: &str|
         -> Result<(), String> {
            emitter
                .event(
                    "sftp/transfer/progress",
                    json!({ "taskId": task_id, "sessionId": session_id, "direction": "download", "transferred": transferred, "size": denominator, "status": "running", "phase": phase_name, "compression": compression }),
                )
                .map_err(plugin_error)
        };
        let set_phase = |value: &'static str| {
            if let Ok(mut slot) = phase.lock() {
                *slot = value;
            }
        };
        let is_cancelled = || {
            cancelled.load(Ordering::Acquire)
                || self
                    .downloads
                    .lock()
                    .map(|downloads| !downloads.contains_key(task_id))
                    .unwrap_or(false)
        };
        set_phase("compressing");
        if let Err(error) = emit_phase("compressing", 0, size, "gzip") {
            return PrepOutcome::Fallback(error);
        }
        // 相对打包：`-C <root> .` 让成员名形如 `./relative/path`，解包后
        // 直接对齐扫描得到的相对路径（免 --strip-components 的兼容面）。
        let command = transfer_compress::remote_tar_command(remote_root, remote_temp);
        match self
            .exec(session_id, None, &command, false, Some(300))
            .await
        {
            Ok(outcome) if outcome.get("exitCode").and_then(Value::as_i64) == Some(0) => {}
            Ok(outcome) => {
                let output = outcome
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(200)
                    .collect::<String>();
                return PrepOutcome::Fallback(format!("remote tar failed: {output}"));
            }
            Err(error) => return PrepOutcome::Fallback(format!("remote tar failed: {error}")),
        }
        if is_cancelled() {
            return PrepOutcome::Cancelled;
        }
        let tgz_size = match self.sftp(session_id).await {
            Ok(sftp) => match sftp.lock().await.metadata(remote_temp.to_string()).await {
                Ok(attributes) => attributes.size.unwrap_or(0),
                Err(error) => {
                    return PrepOutcome::Fallback(format!("remote tgz stat failed: {error}"))
                }
            },
            Err(error) => return PrepOutcome::Fallback(format!("session unavailable: {error}")),
        };
        set_phase("fetching");
        if let Err(error) = emit_phase("fetching", 0, tgz_size, "gzip") {
            return PrepOutcome::Fallback(error);
        }
        if let Err(error) = self
            .pull_remote_gzip(
                session_id,
                remote_temp,
                gz_staging,
                tgz_size,
                task_id,
                throttle,
                cancelled,
                phase,
                emitter,
            )
            .await
        {
            if is_cancelled() {
                return PrepOutcome::Cancelled;
            }
            return PrepOutcome::Fallback(format!("tgz fetch failed: {error}"));
        }
        if is_cancelled() {
            return PrepOutcome::Cancelled;
        }
        set_phase("decompressing");
        if let Err(error) = emit_phase("decompressing", 0, size, "gzip") {
            return PrepOutcome::Fallback(error);
        }
        let extracted = {
            let archive = gz_staging.to_path_buf();
            let root = staging_root.to_path_buf();
            let cancelled = cancelled.clone();
            let emitter = emitter.clone();
            let task_id = task_id.to_string();
            let session_id = session_id.to_string();
            let throttle = Arc::new(Mutex::new(
                crate::features::transfer::progress::ProgressThrottle::default(),
            ));
            let progress: transfer_compress::ProgressSink = Arc::new(move |bytes: u64| {
                let emit = throttle
                    .lock()
                    .map(|mut throttle| throttle.should_emit(size > 0 && bytes >= size))
                    .unwrap_or(true);
                if !emit {
                    return;
                }
                let _ = emitter.event(
                    "sftp/transfer/progress",
                    json!({ "taskId": task_id, "sessionId": session_id, "direction": "download", "transferred": bytes, "size": size, "status": "running", "phase": "decompressing", "compression": "gzip" }),
                );
            });
            tokio::task::spawn_blocking(move || {
                transfer_compress::extract_tar_gz(&archive, &root, &cancelled, &*progress)
            })
            .await
        };
        match extracted {
            Ok(Ok(_bytes)) => PrepOutcome::Ready,
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::Interrupted => {
                PrepOutcome::Cancelled
            }
            Ok(Err(error)) => PrepOutcome::Fallback(format!("tar extract failed: {error}")),
            Err(join_error) => PrepOutcome::Fallback(format!("extract task failed: {join_error}")),
        }
    }

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
    pub(super) async fn download_tree_chunk(
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

    /// Whether the task is still registered (cancel/finish remove it first).
    /// Checked between chunk I/O steps so an in-flight chunk aborts early.
    fn tree_download_active(&self, task_id: &str) -> Result<bool, String> {
        self.downloads
            .lock()
            .map(|downloads| downloads.contains_key(task_id))
            .map_err(|_| "Download registry is poisoned".to_string())
    }

    /// Opens the high-level read handle for one tree file. Production opens an
    /// SFTP handle; tests open `<local_source_root>/<relative>` so the pump's
    /// reuse/seek/drop semantics are covered without a server.
    async fn open_tree_read_handle(
        &self,
        _tree: &TreeDownloadState,
        session_id: &str,
        file: &sftp_tree::TreeFile,
    ) -> Result<TreeReadHandle, String> {
        #[cfg(test)]
        if let Some(root) = _tree.local_source_root.as_ref() {
            _tree.opened_handles.fetch_add(1, Ordering::Relaxed);
            let source = tokio::fs::File::open(root.join(&file.relative))
                .await
                .map_err(|error| format!("SFTP download failed: {error}"))?;
            return Ok(Box::new(source));
        }
        let sftp = self.sftp(session_id).await?;
        let guard = sftp.lock().await;
        let opened = guard
            .open(file.remote_path.clone())
            .await
            .map_err(sftp_error)?;
        drop(guard);
        #[cfg(test)]
        _tree.opened_handles.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(opened))
    }

    /// High-level lane for one chunk: reuses the cached handle for the same
    /// file, opens a fresh one on file switch, and moves the handle out of
    /// `progress` across seek/read so cancel/finish never wait on the network.
    async fn read_tree_high_level(
        &self,
        tree: &TreeDownloadState,
        session_id: &str,
        file: &sftp_tree::TreeFile,
        current_offset: u64,
        requested: usize,
    ) -> Result<(Vec<u8>, usize, TreeReadHandle), TreeReadFailure> {
        let cached = {
            let mut progress = tree.progress.lock().await;
            if progress.handle_path.as_deref() == Some(file.remote_path.as_str()) {
                progress.read_handle.take()
            } else {
                // 切换文件：旧句柄的 Drop 发 close_nowait，槽位置 None 即完成
                // 关闭；新文件在锁外重新 open。
                progress.read_handle = None;
                progress.handle_path = None;
                None
            }
        };
        let mut source = match cached {
            Some(source) => source,
            None => match self.open_tree_read_handle(tree, session_id, file).await {
                Ok(source) => source,
                Err(message) => {
                    let mut progress = tree.progress.lock().await;
                    progress.read_handle = None;
                    progress.handle_path = None;
                    return Err(TreeReadFailure {
                        channel_level: is_channel_level_sftp_error(&message),
                        message,
                    });
                }
            },
        };
        if let Err(error) = source.seek(std::io::SeekFrom::Start(current_offset)).await {
            let mut progress = tree.progress.lock().await;
            progress.read_handle = None;
            progress.handle_path = None;
            return Err(TreeReadFailure {
                message: format!("SFTP download seek failed: {error}"),
                channel_level: false,
            });
        }
        let mut chunk = vec![0_u8; requested];
        match source.read(&mut chunk).await {
            Ok(length) => Ok((chunk, length, source)),
            Err(error) => {
                let message = format!("SFTP download failed: {error}");
                let channel_level = is_channel_level_sftp_error(&message);
                let mut progress = tree.progress.lock().await;
                progress.read_handle = None;
                progress.handle_path = None;
                Err(TreeReadFailure {
                    message,
                    channel_level,
                })
            }
        }
    }
}

/// Failure of one tree-file read. `channel_level` errors invalidate the whole
/// SFTP client cache and abort the batch instead of failing every queued file.
struct TreeReadFailure {
    message: String,
    channel_level: bool,
}
