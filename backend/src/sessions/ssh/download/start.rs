//! SSH transfer support.

use super::*;

impl SshRuntime {
    /// Creates the optional local sink (staging `.part` file) shared by
    /// `sftp/download/start` and `sudo/download/start`.
    pub(crate) async fn build_download_sink(
        &self,
        task_id: &str,
        download_dir: Option<&str>,
        conflict: Option<&str>,
    ) -> Result<Option<Arc<DownloadSink>>, String> {
        let staging = self.transfer_dir.join("downloads");
        std::fs::create_dir_all(&staging)
            .map_err(|error| format!("Failed to create download staging directory: {error}"))?;
        let part_path = staging.join(format!("download-{task_id}.part"));
        let final_dir = download_dir
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                local_downloads::downloads_base_dir(|key| std::env::var_os(key), &self.data_dir)
            });
        if !final_dir.is_absolute() {
            return Err("Download directory must be an absolute path".to_string());
        }
        std::fs::create_dir_all(&final_dir).map_err(|error| {
            format!(
                "Failed to create download directory '{}': {error}",
                final_dir.display()
            )
        })?;
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&part_path)
            .await
            .map_err(|error| {
                format!(
                    "Failed to create local download file '{}': {error}",
                    part_path.display()
                )
            })?;
        Ok(Some(Arc::new(DownloadSink {
            part_path,
            final_dir,
            overwrite: matches!(conflict, Some("overwrite")),
            file: AsyncMutex::new(file),
        })))
    }

    /// Registers a compressed transfer's prep record after its download state
    /// was inserted. A poisoned registry can never accept the record: no prep
    /// task will exist to own or finish the task, so roll the just-registered
    /// download (registry entry plus local staging assets) back before the
    /// error surfaces — the caller never learns the task id, so nothing else
    /// would ever clean it up.
    pub(crate) async fn register_download_prep(
        &self,
        task_id: &str,
        prep: DownloadPrep,
    ) -> Result<(), String> {
        match self.download_preps.lock() {
            Ok(mut preps) => {
                preps.insert(task_id.to_string(), prep);
                Ok(())
            }
            Err(_) => {
                self.rollback_registered_download(task_id).await;
                Err("Download prep registry is poisoned".to_string())
            }
        }
    }

    /// Removes a start-path download whose task id never reached the caller
    /// and tears its local staging assets down (sink `.part`, tree staging
    /// root, compression temporaries).
    async fn rollback_registered_download(&self, task_id: &str) {
        let download = self
            .downloads
            .lock()
            .ok()
            .and_then(|mut downloads| downloads.remove(task_id));
        if let Some(download) = download {
            self.cleanup_download_assets(&download.session_id, &download)
                .await;
        }
    }

    /// `sudo/download/start`: root-owned files streamed through the regular
    /// SFTP download pipeline (M14-C DownloadSudo). The source is staged into
    /// a same-directory sudo temp file (`sudo_download::stage_source`) which
    /// is registered as the task's read source; `sftp/download/next`,
    /// `sftp/download/finish` and the progress events are reused unchanged.
    /// The temp file is removed on finish, cancel, error and session close
    /// (finally semantics; cleanup failures surface as a non-fatal warning).
    #[allow(clippy::too_many_arguments)]
    pub async fn start_sudo_download(
        &self,
        session_id: &str,
        path: &str,
        save_to_local: bool,
        download_dir: Option<&str>,
        conflict: Option<&str>,
        emit_chunks: bool,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        // 与其余 start 路径同走 transfer_depth_limit（M14-B 偏好与 compat
        // 模式强制 1 都要约束 sudo 车道；此前字面 3 绕过了两者）。
        if self.active_transfer_count(session_id)? as u64 >= self.transfer_depth_limit() {
            return Err(format!(
                "This SSH session already has {} active transfers",
                self.transfer_depth_limit()
            ));
        }
        let staged = sudo_download::stage_source(self, session_id, path).await?;
        let outcome = self
            .start_sudo_download_staged(
                session_id,
                &staged,
                save_to_local,
                download_dir,
                conflict,
                emit_chunks,
                emitter,
            )
            .await;
        if outcome.is_err() {
            // 注册失败也要把远端临时件收掉，不能等下载循环来清。
            if let Err(error) = sudo_download::discard_tmp(self, session_id, &staged.tmp_path).await
            {
                eprintln!(
                    "[sudo-download] staging temp cleanup failed ({}): {error}",
                    staged.tmp_path
                );
            }
        }
        outcome
    }

    pub(crate) async fn start_sudo_download_staged(
        &self,
        session_id: &str,
        staged: &sudo_download::StagedSource,
        save_to_local: bool,
        download_dir: Option<&str>,
        conflict: Option<&str>,
        emit_chunks: bool,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        if staged.size > MAX_TRANSFER_SIZE {
            return Err(format!(
                "Transfers are limited to {MAX_TRANSFER_SIZE} bytes"
            ));
        }
        // SFTP 提前探测：源目录对登录用户不可穿越（如 /root 0700）时，普通
        // SFTP 读不到临时件——在这里给出明确错误，而不是第一块分块才失败。
        let sftp = self.sftp(session_id).await?;
        sftp.lock()
            .await
            .metadata(staged.tmp_path.clone())
            .await
            .map_err(sftp_error)?;
        let task_id = Uuid::new_v4().to_string();
        let sink = if save_to_local {
            self.build_download_sink(&task_id, download_dir, conflict)
                .await?
        } else {
            None
        };
        let mut downloads = match self.downloads.lock() {
            Ok(downloads) => downloads,
            Err(_) => {
                if let Some(sink) = sink.as_ref() {
                    let _ = std::fs::remove_file(&sink.part_path);
                }
                return Err("Download registry is poisoned".to_string());
            }
        };
        downloads.insert(
            task_id.clone(),
            DownloadState {
                session_id: session_id.to_string(),
                remote_path: staged.tmp_path.clone(),
                file_name: staged.file_name.clone(),
                size: staged.size,
                next_offset: 0,
                sink,
                emit_chunks,
                tree: None,
                sudo_tmp: Some(staged.tmp_path.clone()),
                // sudo 下载走独立车道（sudo_download.rs），读侧不经
                // has_wire_lane 判分支；字面 false 只做字段填充。
                latin1: false,
                // sudo 车道本期不在限速范围（issue #66 收敛在 SFTP
                // 下载），字段照常填充但快照取 0（不限速）。
                throttle: Throttle::new(0),
                progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
                // sudo 车道不参与压缩通道（M33）：远端临时件机制独立。
                compression: CompressionMode::None,
                ready: false,
                serving_plain: false,
                staged_plain: None,
                gz_staging: None,
                remote_temp: None,
                final_dir: None,
                overwrite: false,
                read_handle: Arc::new(AsyncMutex::new(None)),
                raw_client: Arc::new(AsyncMutex::new(None)),
            },
        );
        drop(downloads);
        if let Err(error) = emitter.event(
            "sftp/transfer/progress",
            json!({ "taskId": task_id, "sessionId": session_id, "direction": "download", "transferred": 0, "size": staged.size, "status": "queued" }),
        ) {
            if let Ok(mut downloads) = self.downloads.lock() {
                if let Some(task) = downloads.remove(&task_id) {
                    if let Some(sink) = task.sink {
                        let _ = std::fs::remove_file(&sink.part_path);
                    }
                }
            }
            return Err(plugin_error(error));
        }
        let connection_id = self.session_connection_id(session_id).await;
        self.record_transfer_start(
            &task_id,
            session_id,
            &connection_id,
            "download",
            &staged.file_name,
            staged.size,
        );
        Ok(
            json!({ "taskId": task_id, "fileName": staged.file_name, "size": staged.size, "chunkSize": TRANSFER_CHUNK_SIZE, "resumeOffset": 0_u64, "saveToLocal": save_to_local, "sudo": true }),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start_download(
        self: &Arc<Self>,
        session_id: &str,
        remote_path: &str,
        offset: u64,
        save_to_local: bool,
        download_dir: Option<&str>,
        conflict: Option<&str>,
        encoding: NameEncoding,
        emit_chunks: bool,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let remote_path = normalize_remote_path(remote_path)?;
        if self.active_transfer_count(session_id)? as u64 >= self.transfer_depth_limit() {
            return Err(format!(
                "This SSH session already has {} active transfers",
                self.transfer_depth_limit()
            ));
        }
        // Resume re-attaches with bytes the caller already holds locally, which
        // the staging file would be missing — only fresh downloads may sink.
        if save_to_local && offset > 0 {
            return Err("Local save downloads cannot resume from an offset".to_string());
        }
        // latin-1 生效且 wire 含 `%XX` 转义（latin-1 列表产出的非 UTF-8
        // 名字）时，走裸包 STAT 取真实字节数——高层客户端会把转义串按字
        // 面量发出去，命中不了远端文件（M14-B：传输用服务器原始字节）。
        // auto 一律走高层（M28-B 修 D-7）：auto 列表 uri 字面 `%` 未经
        // `%25` 自转义，wire 串里的 `%XX` 是文件名字面量而非转义，不能
        // 还原；车道判定统一收口在 `has_wire_lane`。
        let latin1 = encoding == NameEncoding::Latin1;
        let size = if sftp_name::has_wire_lane(&remote_path, encoding) {
            let mut client = self.raw_sftp_client(session_id).await?;
            // 整条 wire 还原为服务器字节再 LSTAT——探测点曾漏掉这一步（把
            // 字面 "%XX" 字节当路径，转义名单文件下载 start 即 NO_SUCH_FILE；
            // M21 smoke 的 latin-1 watcher 用例真机曝露）。
            client
                .stat(&sftp_name::unescape_wire(&remote_path))
                .await
                .map_err(sftp_error)?
                .size
                .unwrap_or(0)
        } else {
            let sftp = self.sftp(session_id).await?;
            let size = sftp
                .lock()
                .await
                .metadata(remote_path.clone())
                .await
                .map_err(sftp_error)?
                .size
                .unwrap_or(0);
            size
        };
        if size > MAX_TRANSFER_SIZE {
            return Err(format!(
                "Transfers are limited to {MAX_TRANSFER_SIZE} bytes"
            ));
        }
        // Resume support: the caller already holds the first `offset` bytes
        // locally and re-attaches mid-stream. Size-only identity check (a
        // changed file with the same size would splice mismatched content);
        // documented as best-effort.
        if offset > size {
            return Err(format!(
                "Resume offset {offset} is beyond the remote file size {size}"
            ));
        }
        // 压缩通道决策（M33，W6 单入口）：offset > 0（续传）与压缩互斥——
        // 压缩任务的分块读本地解压流，不支持断点拼接，续传一律回落普通管
        // 线（前端按响应 compression 字段走既有语义）。只读连接与 latin-1
        // 车道强制 None：前者远端 gzip 要写临时件，后者用户路径不进 shell。
        let compression = if offset > 0 {
            CompressionMode::None
        } else {
            let read_only = self
                .session(session_id)
                .await
                .map(|session| session.read_only)
                .unwrap_or(true);
            let decided_name = remote_path.rsplit('/').next().unwrap_or("");
            self.decide_transfer_compression(
                session_id,
                size,
                Some(decided_name),
                false,
                read_only,
                latin1,
            )
            .await
        };
        let file_name = remote_path
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or("download")
            .to_string();
        let task_id = Uuid::new_v4().to_string();
        // 压缩任务不建 sink：解压直写独立 staged_plain，落盘目标单独记录
        // （saveToLocal 才有；浏览器模式字节在前端，终态直接删暂存）。
        let sink = if compression == CompressionMode::Gzip {
            None
        } else {
            self.build_download_sink(&task_id, download_dir, conflict)
                .await?
        };
        let final_dir = if compression == CompressionMode::Gzip && save_to_local {
            let dir = download_dir
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    local_downloads::downloads_base_dir(|key| std::env::var_os(key), &self.data_dir)
                });
            if !dir.is_absolute() {
                return Err("Download directory must be an absolute path".to_string());
            }
            std::fs::create_dir_all(&dir).map_err(|error| {
                format!(
                    "Failed to create download directory '{}': {error}",
                    dir.display()
                )
            })?;
            Some(dir)
        } else {
            None
        };
        // 限速快照（issue #66）：偏好现值只在任务启动时读一次，整个任务
        // 沿用——设置改动对下一个下载任务生效，进行中任务节奏不抖动。
        let throttle = Throttle::new(crate::core::preferences::transfer_download_limit_kib(
            &self.data_dir,
        ));
        // 压缩任务的暂存/临时件路径（文件由 prep 任务创建）。
        let gz_staging = (compression == CompressionMode::Gzip).then(|| {
            self.transfer_dir
                .join("downloads")
                .join(format!("download-{task_id}.part.gz"))
        });
        let staged_plain = (compression == CompressionMode::Gzip).then(|| {
            self.transfer_dir
                .join("downloads")
                .join(format!("download-{task_id}.plain"))
        });
        let remote_temp = (compression == CompressionMode::Gzip).then(|| {
            let parent = remote_path
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .filter(|parent| !parent.is_empty())
                .unwrap_or("/");
            format!(
                "{}/.dbx-download-{task_id}.gz",
                parent.trim_end_matches('/')
            )
        });
        // prep 任务用的远端源路径（registry 插入会 move 原值，先克隆）。
        let prep_remote_path = remote_path.clone();
        // Validate the compressed staging trio before registering the task:
        // a missing path is an internal invariant violation, but returning an
        // error beats panicking on the RPC worker and leaving a half-task.
        let compressed_paths = if compression == CompressionMode::Gzip {
            match (
                gz_staging.clone(),
                staged_plain.clone(),
                remote_temp.clone(),
            ) {
                (Some(gz), Some(staged), Some(remote)) => Some((gz, staged, remote)),
                _ => {
                    // Invariant failure before registration: no task will ever
                    // clean the sink, so remove its staging `.part` here.
                    if let Some(sink) = sink.as_ref() {
                        let _ = std::fs::remove_file(&sink.part_path);
                    }
                    return Err("Compressed download staging paths are missing".to_string());
                }
            }
        } else {
            None
        };
        let mut downloads = match self.downloads.lock() {
            Ok(downloads) => downloads,
            Err(_) => {
                // No task will ever own the staging `.part`: clean it here.
                if let Some(sink) = sink.as_ref() {
                    let _ = std::fs::remove_file(&sink.part_path);
                }
                return Err("Download registry is poisoned".to_string());
            }
        };
        downloads.insert(
            task_id.clone(),
            DownloadState {
                session_id: session_id.to_string(),
                remote_path,
                file_name: file_name.clone(),
                size,
                next_offset: offset,
                sink,
                emit_chunks,
                tree: None,
                sudo_tmp: None,
                latin1,
                throttle,
                progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
                compression,
                ready: false,
                serving_plain: false,
                staged_plain: staged_plain.clone(),
                gz_staging: gz_staging.clone(),
                remote_temp: remote_temp.clone(),
                final_dir,
                overwrite: matches!(conflict, Some("overwrite")),
                read_handle: Arc::new(AsyncMutex::new(None)),
                raw_client: Arc::new(AsyncMutex::new(None)),
            },
        );
        drop(downloads);
        if let Some((gz_path, staged_path, remote_path_for_prep)) = compressed_paths {
            // prep 注册 + 后台任务（远端 gzip → 拉取 → 解压 → ready）。
            // 登记先于 spawn：取消 RPC 立即可见。
            let cancelled = Arc::new(AtomicBool::new(false));
            let cancel_reason = Arc::new(Mutex::new(None::<String>));
            let phase = Arc::new(Mutex::new("compressing"));
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
            let prep_emitter = emitter.clone();
            tokio::spawn(async move {
                this.run_download_prep(
                    prep_task_id,
                    prep_session_id,
                    prep_remote_path,
                    size,
                    gz_path,
                    staged_path,
                    remote_path_for_prep,
                    throttle,
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
            json!({ "taskId": task_id, "sessionId": session_id, "direction": "download", "transferred": offset, "size": size, "status": if offset > 0 { "running" } else { "queued" } }),
        ) {
            // The caller never learns the task id on a failed start: roll the
            // registry entry and staging `.part` back. A compressed-prep task
            // sees its missing registry entry and tears its own staging down.
            if let Ok(mut downloads) = self.downloads.lock() {
                if let Some(task) = downloads.remove(&task_id) {
                    if let Some(sink) = task.sink {
                        let _ = std::fs::remove_file(&sink.part_path);
                    }
                }
            }
            return Err(plugin_error(error));
        }
        let connection_id = self.session_connection_id(session_id).await;
        self.record_transfer_start(
            &task_id,
            session_id,
            &connection_id,
            "download",
            &file_name,
            size,
        );
        Ok(
            json!({ "taskId": task_id, "fileName": file_name, "size": size, "chunkSize": TRANSFER_CHUNK_SIZE, "resumeOffset": offset, "saveToLocal": save_to_local, "compression": compression.as_str() }),
        )
    }
}
