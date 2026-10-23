//! Metrics support.

use super::*;

/// Mutable state owned by the per-watch pump task; the registry only reads
/// the immutable identity fields.
pub(crate) struct WatchPump {
    baseline: Option<FileFingerprint>,
    /// Deadline of the currently armed debounce window, if any.
    pending_at: Option<std::time::Instant>,
    /// When the watch was registered (suppression window anchor).
    started_at: std::time::Instant,
}

pub(crate) struct WatchEntry {
    session_id: String,
    /// Identity snapshot from `watch/start`: the pump event payload and
    /// `watch/upload` both read the pair from here, so the workbench only
    /// ever has to remember the watchId.
    local_path: PathBuf,
    remote_path: String,
    /// Keeping the notify watcher alive is what keeps events flowing; dropping
    /// it (registry removal) is also how the pump task learns to stop.
    _watcher: notify::RecommendedWatcher,
}

pub struct WatchRuntime {
    watches: Arc<AsyncRwLock<HashMap<String, WatchEntry>>>,
    /// `{sessionId}:{canonicalLocalPath}` -> watchId, so re-opening the same
    /// file in an external editor never stacks a second watcher.
    dedup: Arc<AsyncRwLock<HashMap<String, String>>>,
    tunables: Tunables,
    clock: Arc<dyn Clock>,
}

impl WatchRuntime {
    pub fn new() -> Self {
        Self::with_tunables(Tunables::production())
    }

    pub fn with_tunables(tunables: Tunables) -> Self {
        Self {
            watches: Arc::new(AsyncRwLock::new(HashMap::new())),
            dedup: Arc::new(AsyncRwLock::new(HashMap::new())),
            tunables,
            clock: Arc::new(SystemClock),
        }
    }

    /// `watch/start` — registers a non-recursive watcher on `local_path`.
    /// Desktop-only (see [`crate::filesystem::local::downloads::can_save_local`]); refuses
    /// anything that is not an existing regular file inside the plugin's own
    /// `remote-edit/` download directory (path-origin gate — a watchId is a
    /// bearer token, so an arbitrary local path must never be watchable).
    pub async fn start(
        &self,
        request: WatchStartRequest,
        publisher: Arc<dyn EventPublisher>,
        probe: SessionProbe,
        data_dir: &Path,
    ) -> Result<Value, String> {
        let desktop_ok = self.tunables.desktop_gate_override.unwrap_or_else(|| {
            crate::filesystem::local::downloads::can_save_local(|key| std::env::var_os(key))
        });
        if !desktop_ok {
            return Err(
                "File watching is only available on desktop — open the file from a local download instead"
                    .to_string(),
            );
        }
        let session_id = request.session_id.trim().to_string();
        let remote_path = request.remote_path.trim().to_string();
        if session_id.is_empty() || remote_path.is_empty() {
            return Err("sessionId and remotePath are required".to_string());
        }
        // Canonicalizes and refuses anything outside `remote-edit/` (also the
        // "does not exist" rejection for missing paths). Downstream code only
        // ever sees the canonical path.
        let local_path =
            self.validate_local_origin(Path::new(request.local_path.trim()), data_dir)?;
        let baseline = file_fingerprint(&local_path).ok_or_else(|| {
            format!(
                "Watched file '{}' does not exist or is not a regular file",
                local_path.display()
            )
        })?;

        let dedup_key = dedup_key(&session_id, &local_path);
        // Bind BEFORE the if-let: a guard temporary in the `if let` scrutinee
        // lives until the end of the block (pre-2024 editions), which would
        // hold the dedup read lock across `stop()` below and self-deadlock on
        // its dedup write.
        let existing = self.dedup.read().await.get(&dedup_key).cloned();
        if let Some(existing) = existing {
            // Re-opening the same file: the fresh download is the new baseline,
            // so the stale watcher is torn down before the replacement starts.
            let _ = self.stop(&existing).await;
        }

        let watch_id = uuid::Uuid::new_v4().to_string();
        // Watch the parent DIRECTORY, not the file: atomic saves (write temp +
        // rename over the target — how WPS/Office and some VS Code paths save)
        // swap the inode, which kills a file-level inotify watch on the first
        // save and goes deaf from then on. Directory watches survive renames
        // on every backend; sibling noise (lock/temp files) only re-arms the
        // debounce and is gated by the fingerprint, so it cannot misfire.
        let watch_target = local_path
            .parent()
            .filter(|dir| dir.is_dir())
            .ok_or_else(|| {
                format!(
                    "Watched file '{}' has no watchable parent directory",
                    local_path.display()
                )
            })?;
        let (event_tx, event_rx) = mpsc::unbounded_channel::<()>();
        let mut watcher =
            notify::recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
                if result.is_ok() {
                    // Payload is irrelevant: any filesystem activity arms the pump.
                    let _ = event_tx.send(());
                }
            })
            .map_err(|error| format!("Failed to watch '{}': {error}", watch_target.display()))?;
        watcher
            .watch(watch_target, notify::RecursiveMode::NonRecursive)
            .map_err(|error| format!("Failed to watch '{}': {error}", watch_target.display()))?;

        let started_at = self.clock.now();
        let pump = Arc::new(AsyncRwLock::new(WatchPump {
            baseline: Some(baseline),
            pending_at: None,
            started_at,
        }));
        self.watches.write().await.insert(
            watch_id.clone(),
            WatchEntry {
                session_id: session_id.clone(),
                local_path: local_path.clone(),
                remote_path: remote_path.clone(),
                _watcher: watcher,
            },
        );
        self.dedup.write().await.insert(dedup_key, watch_id.clone());

        let runtime_self = Self {
            watches: self.watches.clone(),
            dedup: self.dedup.clone(),
            tunables: self.tunables.clone(),
            clock: self.clock.clone(),
        };
        tokio::spawn(Self::pump(
            runtime_self,
            watch_id.clone(),
            session_id,
            local_path,
            remote_path,
            pump,
            event_rx,
            publisher,
            probe,
        ));
        Ok(json!({ "watchId": watch_id }))
    }

    /// Shared path-origin gate for `watch/start` and `watch/upload`: reuses
    /// `sftp_ext::validate_remote_edit_path` (same check as `sftp/upload-local`)
    /// so only files under the plugin's own `remote-edit/` download directory
    /// pass. Returns the canonicalized path on success.
    fn validate_local_origin(&self, local_path: &Path, data_dir: &Path) -> Result<PathBuf, String> {
        match &self.tunables.remote_edit_downloads_override {
            Some(base) => crate::filesystem::sftp::ext::validate_remote_edit_root(local_path, base),
            None => crate::filesystem::sftp::ext::validate_remote_edit_path(
                local_path,
                data_dir,
                |key| std::env::var_os(key),
            ),
        }
    }

    /// Event pump for one watcher: debounces notify bursts, confirms content
    /// changes against the baseline, and publishes once per confirmed change.
    /// Exits when the registry drops the entry (all notify senders gone) or
    /// after self-cleanup on file loss / dead session.
    #[allow(clippy::too_many_arguments)]
    async fn pump(
        runtime: Self,
        watch_id: String,
        session_id: String,
        local_path: PathBuf,
        remote_path: String,
        pump: Arc<AsyncRwLock<WatchPump>>,
        mut event_rx: mpsc::UnboundedReceiver<()>,
        publisher: Arc<dyn EventPublisher>,
        probe: SessionProbe,
    ) {
        loop {
            let deadline = pump.read().await.pending_at;
            tokio::select! {
                // The channel closes when the registry entry (and with it the
                // notify watcher owning the sender) is dropped; `None` is the
                // pump's exit signal — without the break the closed channel
                // would re-arm the debounce forever.
                signal = event_rx.recv() => {
                    if signal.is_none() {
                        break;
                    }
                    let mut state = pump.write().await;
                    if runtime.clock.now() < state.started_at + runtime.tunables.suppress {
                        // Startup suppression window: editor warm-up noise is
                        // dropped outright, not delayed into a prompt.
                        continue;
                    }
                    if state.pending_at.is_none() {
                        state.pending_at = Some(runtime.clock.now() + runtime.tunables.debounce);
                    }
                }
                _ = sleep_until(deadline), if deadline.is_some() => {
                    let verdict = {
                        let mut state = pump.write().await;
                        state.pending_at = None;
                        let current = spawn_fingerprint(local_path.clone()).await;
                        classify_change(&state.baseline, &current)
                    };
                    match verdict {
                        ChangeVerdict::Same => {}
                        ChangeVerdict::Undeterminable => {
                            if !local_path.exists() {
                                // The file (or its whole remote-edit folder) is
                                // gone; keep no watcher on a ghost.
                                let _ = runtime.stop(&watch_id).await;
                                break;
                            }
                        }
                        ChangeVerdict::ContentChanged => {
                            if !probe(session_id.clone()).await {
                                let _ = runtime.stop(&watch_id).await;
                                break;
                            }
                            publisher.publish(json!({
                                "watchId": watch_id,
                                "sessionId": session_id,
                                "localPath": local_path.to_string_lossy(),
                                "remotePath": remote_path,
                            }));
                            // The emitted state becomes the new baseline so a
                            // repeated identical save does not re-fire.
                            if let Some(current) = spawn_fingerprint(local_path.clone()).await {
                                pump.write().await.baseline = Some(current);
                            }
                        }
                    }
                }
            }
        }
    }

    /// `watch/stop` — tears down one watcher.
    pub async fn stop(&self, watch_id: &str) -> Result<(), String> {
        let removed = self.watches.write().await.remove(watch_id).is_some();
        self.dedup.write().await.retain(|_, id| id != watch_id);
        if removed {
            Ok(())
        } else {
            Err("Watch was not found".to_string())
        }
    }

    /// `watch/upload` — pushes the current on-disk bytes of a watched file
    /// back to its remote path. The workbench cannot read local files by path
    /// (the host file-transfer bridge only exposes user-picked handles), so
    /// the sidecar — which downloaded the file into `remote-edit/` in the
    /// first place — reads the bytes and streams them through the same atomic
    /// temporary-file commit as `sftp/write` (permissions preserved).
    /// The write gate (`ensure_writable`) applies exactly as for any other
    /// SFTP write. Origin is re-validated here instead of trusting the
    /// watchId: the id is a bearer token and the watched path may have been
    /// swapped for a symlink after `start`, so `validate_local_origin`
    /// re-canonicalizes and refuses anything outside `remote-edit/`.
    /// watchId → 所属 sessionId（连接级 SFTP 编码判定用，M16 集成）。
    /// watch 已消失返回 None，调用方按未覆盖（跟随全局）处理。
    pub async fn session_for_watch(&self, watch_id: &str) -> Option<String> {
        self.watches
            .read()
            .await
            .get(watch_id)
            .map(|entry| entry.session_id.clone())
    }

    pub async fn upload_back(
        &self,
        ssh: &crate::sessions::ssh::SshRuntime,
        watch_id: &str,
        encoding: crate::filesystem::sftp::name::NameEncoding,
    ) -> Result<Value, String> {
        let (session_id, registered_path, remote_path) = {
            let watches = self.watches.read().await;
            let entry = watches
                .get(watch_id)
                .ok_or_else(|| "Watch was not found".to_string())?;
            (
                entry.session_id.clone(),
                entry.local_path.clone(),
                entry.remote_path.clone(),
            )
        };
        let local_path = self.validate_local_origin(&registered_path, &ssh.data_dir())?;
        // Size gate before reading: the round-trip buffers the whole file in
        // memory on purpose (single atomic commit), so refuse oversize
        // without ever loading it.
        let metadata = tokio::fs::metadata(&local_path).await.map_err(|error| {
            format!(
                "Could not read watched file '{}': {error}",
                local_path.display()
            )
        })?;
        if metadata.len() > MAX_UPLOAD_BYTES {
            return Err(upload_limit_error(&local_path));
        }
        // File IO stays off the async worker; the re-check after the read
        // closes the metadata→read race (file grew past the limit meanwhile).
        let data = {
            let read_path = local_path.clone();
            tokio::task::spawn_blocking(move || std::fs::read(read_path))
                .await
                .map_err(|error| format!("Watched file read failed: {error}"))?
                .map_err(|error| {
                    format!(
                        "Could not read watched file '{}': {error}",
                        local_path.display()
                    )
                })?
        };
        if data.len() as u64 > MAX_UPLOAD_BYTES {
            return Err(upload_limit_error(&local_path));
        }
        let size = data.len();
        crate::filesystem::sftp::ext::write_bytes(ssh, &session_id, &remote_path, &data, encoding)
            .await?;
        Ok(json!({
            "remotePath": remote_path,
            "size": size,
        }))
    }

    /// `watch/stop-all` (and the `ssh/session/close` hook) — drops every
    /// watcher owned by one SSH session.
    pub async fn stop_session(&self, session_id: &str) -> usize {
        let ids: Vec<String> = self
            .watches
            .read()
            .await
            .iter()
            .filter(|(_, entry)| entry.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        let mut watches = self.watches.write().await;
        let mut dedup = self.dedup.write().await;
        for id in &ids {
            watches.remove(id);
            dedup.retain(|_, watch_id| watch_id != id);
        }
        ids.len()
    }

    /// Test/inspection helper: number of live watchers.
    #[cfg(test)]
    pub async fn live_count(&self) -> usize {
        self.watches.read().await.len()
    }
}

/// Uniform "too big for the watch-upload round-trip" rejection (used by both
/// the metadata pre-check and the post-read re-check in `upload_back`).
pub(crate) fn upload_limit_error(local_path: &Path) -> String {
    format!(
        "Watched file '{}' is larger than the {} MiB watch-upload limit; upload it manually instead",
        local_path.display(),
        MAX_UPLOAD_BYTES / (1024 * 1024)
    )
}

/// Dedup identity: session plus the canonicalized local path so `a/b` and
/// `a/./b` map to the same watcher.
pub(crate) fn dedup_key(session_id: &str, local_path: &Path) -> String {
    let canonical = std::fs::canonicalize(local_path).unwrap_or_else(|_| local_path.to_path_buf());
    format!("{session_id}:{}", canonical.to_string_lossy())
}

/// Sleeps until `deadline`, finishing immediately (without error) when the
/// deadline is already in the past. `None` never resolves — `select!` only
/// reaches this branch when a deadline is armed.
pub(crate) async fn sleep_until(deadline: Option<std::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
        // Park forever; select! discards this branch unless the guard flips.
        None => std::future::pending::<()>().await,
    }
}
