use super::*;

impl SshRuntime {
    /// 压缩树 prep 终态分发（M33）：与单文件 run_download_prep 同构——
    /// Ready 发 ready(gzip)；Fallback 清压缩产物、registry 回普通管线并发
    /// ready(none)；Cancelled 走取消清理。staging 树（root_local）不属于
    /// 压缩产物：Fallback/Cancelled 的整树拆除由 cancel/complete 的既有
    /// root_local 语义负责，这里只清 .tgz 两侧临时件。
    #[allow(clippy::too_many_arguments)]
    pub(in super::super) async fn run_tree_prep(
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
    pub(in super::super) async fn prep_tree_staging(
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
}
