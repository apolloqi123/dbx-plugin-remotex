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
}
