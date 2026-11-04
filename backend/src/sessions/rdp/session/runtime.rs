//! Metrics support.

use super::*;

mod clipboard;

use crate::core::time::unix_now_secs;

impl RdpSessionRuntime {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            broker: Arc::new(CertPromptBroker::new()),
        }
    }

    /// Registers the session row immediately and spawns the worker, which
    /// publishes `connecting` → `connected`/`error` state events, frame
    /// patches and clipboard events.
    pub async fn start(
        &self,
        request: RdpStartRequest,
        emitter: PluginEmitter,
        data_dir: &Path,
    ) -> Result<Value, String> {
        let host = request.host.trim().to_string();
        if host.is_empty() {
            return Err("rdp/start: host is required".to_string());
        }
        // Identity bounds (B6/F6, rdp-security-review): reject oversized
        // wire identities instead of passing unbounded strings through to
        // the vendored CredSSP/Destination layer.
        if host.len() > MAX_HOST_LEN {
            return Err(format!(
                "rdp/start: host exceeds the {MAX_HOST_LEN} character limit"
            ));
        }
        if request.username.trim().is_empty() {
            return Err("rdp/start: username is required".to_string());
        }
        if request.username.trim().len() > MAX_USERNAME_LEN {
            return Err(format!(
                "rdp/start: username exceeds the {MAX_USERNAME_LEN} character limit"
            ));
        }
        let domain = request.domain.unwrap_or_default();
        if domain.len() > MAX_DOMAIN_LEN {
            return Err(format!(
                "rdp/start: domain exceeds the {MAX_DOMAIN_LEN} character limit"
            ));
        }
        let port = request.port.unwrap_or(3389);
        if port == 0 {
            return Err("rdp/start: port must be between 1 and 65535".to_string());
        }
        let width = request.width.unwrap_or(1280);
        let height = request.height.unwrap_or(800);
        validate_desktop_size(width, height)?;
        let security = {
            let prefs = crate::core::preferences::load_preferences(data_dir);
            resolve_security(
                request.use_nla,
                request.certificate_policy.as_deref(),
                &prefs,
            )?
        };
        let clipboard_enabled = request.clipboard.unwrap_or(true);
        let reconnect_attempts = request
            .reconnect_attempts
            .unwrap_or(DEFAULT_RECONNECT_ATTEMPTS)
            .min(MAX_RECONNECT_ATTEMPTS);
        let password = request
            .password
            .filter(|password| !password.is_empty())
            .map(Zeroizing::new);
        let session_id = uuid::Uuid::new_v4().to_string();
        let entry = Arc::new(RdpSessionEntry {
            connection_id: std::sync::Mutex::new(request.connection_id.clone()),
            workbench_id: request.workbench_id.clone(),
            host: host.clone(),
            port,
            username: request.username,
            domain,
            password,
            width: AtomicU16::new(width),
            height: AtomicU16::new(height),
            use_nla: security.use_nla,
            cert_policy: security.certificate_policy,
            clipboard_enabled,
            reconnect_attempts,
            created_at_secs: unix_now_secs(),
            generation: Arc::new(AtomicU64::new(0)),
            frame_state: tokio::sync::Mutex::new(RdpFrameState::default()),
            close_requested: AtomicBool::new(false),
            // 证书确认等待标记（CONNECT_PROGRESS_BUDGET 的豁免信号）：verifier
            // 在等用户点确认的 120s 里置位，离开等待（任何路径）即清零。
            cert_wait: Arc::new(AtomicBool::new(false)),
            command_sender: tokio::sync::Mutex::new(None),
            clipboard_stage: Arc::new(ClipboardStage::default()),
            clipboard_files: Arc::new(FileClipboardStage::new(session_id.clone())),
            broker: self.broker.clone(),
            known_certs: KnownCertStore::new(data_dir),
        });
        // —— 幂等查重（reconnect 双 start 竞态修复）——重连梯子/宿主 replay
        // 可能在同一秒内并发触发两次 rdp/start（同 workbenchId）：旧实现会创建
        // 两个活会话，Windows 单会话策略踢掉前一个 → 前端持有的旧 sessionId
        // 收到断开事件 → 重连梯子再触发 rdp/start，工作台卡死在"连接中"。
        // 这里把查重与 insert 收进同一写锁，并发双 start 只有一个能建立会话，
        // 另一个命中复用。终态会话（closed / 耗尽退避梯子的 error）已被 worker
        // 从表中移除，"仍在表中"即未进入终态的活会话，可安全复用。
        {
            let mut sessions = self.sessions.write().await;
            if let Some((existing_id, existing)) = sessions
                .iter()
                .find(|(_, candidate)| candidate.workbench_id == request.workbench_id)
                .map(|(session_id, entry)| (session_id.clone(), entry.clone()))
            {
                // Adopt a connection id that the first start could not supply
                // (legacy/dialog path), so a later disconnect still reclaims it.
                if let Some(connection_id) = request.connection_id.as_deref() {
                    let mut stored = existing
                        .connection_id
                        .lock()
                        .expect("rdp connection id mutex");
                    if stored.is_none() {
                        *stored = Some(connection_id.to_string());
                    }
                }
                crate::platform::diagnostics::log_rdp_debug(format!(
                    "rdp/start idempotent hit workbench={} -> reuse {existing_id}",
                    request.workbench_id
                ));
                return Ok(json!({
                    "sessionId": existing_id,
                    "host": existing.host.clone(),
                    "port": existing.port,
                    "useNla": existing.use_nla,
                    "certificatePolicy": existing.cert_policy.as_str(),
                    "clipboard": existing.clipboard_enabled,
                    "reconnectAttempts": existing.reconnect_attempts,
                }));
            }
            sessions.insert(session_id.clone(), entry.clone());
        }
        spawn_worker(session_id.clone(), entry, emitter, self.sessions.clone());
        Ok(json!({
            "sessionId": session_id,
            "host": host,
            "port": port,
            "useNla": security.use_nla,
            "certificatePolicy": security.certificate_policy.as_str(),
            "clipboard": clipboard_enabled,
            "reconnectAttempts": reconnect_attempts,
        }))
    }
    async fn session(&self, session_id: &str) -> Result<Arc<RdpSessionEntry>, String> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| "RDP session was not found".to_string())
    }

    async fn command_sender(&self, session_id: &str) -> Result<mpsc::Sender<RdpCommand>, String> {
        let entry = self.session(session_id).await?;
        let sender = entry.command_sender.lock().await.clone();
        sender.ok_or_else(|| "RDP session is not connected".to_string())
    }

    /// Keyboard/pointer forwarding (`rdp/input`, alias `rdp/write`).
    pub async fn input(&self, session_id: &str, input: RdpWireInput) -> Result<(), String> {
        // Unicode text bound (E5a, rdp-security-review): each character
        // expands into a press+release operation pair, so unbounded text
        // would materialize an unbounded operation list. Rejected whole.
        if let RdpWireInput::Unicode { text } = &input {
            let characters = text.chars().count();
            if characters > MAX_UNICODE_TEXT_CHARS {
                return Err(format!(
                    "rdp/input: unicode text exceeds the {MAX_UNICODE_TEXT_CHARS} character limit"
                ));
            }
        }
        self.session(session_id).await?;
        let sender = self.command_sender(session_id).await?;
        sender
            .send(RdpCommand::Input(input))
            .await
            .map_err(|_| "RDP session is closed".to_string())
    }

    /// Desktop resize (`rdp/resize`). Queues a live MS-RDPEDISP monitor-layout
    /// update on the current engine; the vendored client applies it through
    /// the Display Control DVC and the server's Deactivate-Reactivation
    /// sequence. The generation and session id stay put, so the frontend
    /// never falls back into `connecting`. Servers without display control are
    /// handled inside the same engine (it re-dials with the new size).
    ///
    /// When no engine is attached (dialling exhausted/backing off), only the
    /// stored size is retargeted; the next generation dials with it.
    pub async fn resize(&self, session_id: &str, width: u16, height: u16) -> Result<Value, String> {
        validate_desktop_size(width, height)?;
        let entry = self.session(session_id).await?;
        if entry.close_requested.load(Ordering::Acquire) {
            return Err("RDP session is closed".to_string());
        }
        let previous = (
            entry.width.load(Ordering::Acquire),
            entry.height.load(Ordering::Acquire),
        );
        if previous != (width, height) {
            crate::platform::diagnostics::log_rdp_debug(format!(
                "rdp/resize id={session_id}: {previous:?} -> ({width}, {height}) live"
            ));
            entry.width.store(width, Ordering::Release);
            entry.height.store(height, Ordering::Release);
        }
        let sender = entry.command_sender.lock().await.clone();
        let Some(sender) = sender else {
            return Ok(json!({
                "sessionId": session_id,
                "success": true,
                "live": false,
            }));
        };
        // A closed command channel belongs to a worker that is already
        // tearing down; the stored size is enough for the next generation.
        let live = sender
            .send(RdpCommand::Resize { width, height })
            .await
            .is_ok();
        Ok(json!({
            "sessionId": session_id,
            "success": true,
            "live": live,
        }))
    }

    /// Manual reconnect: kill the current generation and restart the pump.
    pub async fn reconnect(
        &self,
        session_id: &str,
        emitter: PluginEmitter,
    ) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        entry.close_requested.store(false, Ordering::Release);
        entry.generation.fetch_add(1, Ordering::AcqRel);
        entry.frame_state.lock().await.clear();
        // A new CLIPRDR channel invalidates staged/offered files and any
        // in-flight download.
        entry.clipboard_files.clear();
        self.broker.cancel_session(session_id);
        if let Some(sender) = entry.command_sender.lock().await.take() {
            let _ = sender.send(RdpCommand::Close).await;
        }
        spawn_worker(
            session_id.to_string(),
            entry,
            emitter,
            self.sessions.clone(),
        );
        Ok(json!({ "sessionId": session_id, "success": true }))
    }

    pub async fn close(&self, session_id: &str) -> Result<(), String> {
        let entry = self
            .sessions
            .write()
            .await
            .remove(session_id)
            .ok_or("RDP session was not found")?;
        entry.close_requested.store(true, Ordering::Release);
        entry.generation.fetch_add(1, Ordering::AcqRel);
        entry.frame_state.lock().await.clear();
        // Any pending certificate prompt of this session must reject now,
        // and the staged clipboard text is dropped with the entry.
        self.broker.cancel_session(session_id);
        entry.clipboard_stage.clear();
        entry.clipboard_files.clear();
        if let Some(sender) = entry.command_sender.lock().await.take() {
            let _ = sender.send(RdpCommand::Close).await;
        }
        Ok(())
    }

    /// Closing a connection tears down its RDP sessions (`connection/
    /// disconnect`), matching the Telnet/VNC/SSH lifecycle. A webview reload
    /// never calls this, so live sessions stay reattachable.
    pub async fn close_connection(&self, connection_id: &str) {
        let session_ids: Vec<String> = self
            .sessions
            .read()
            .await
            .iter()
            .filter(|(_, entry)| {
                entry
                    .connection_id
                    .lock()
                    .expect("rdp connection id mutex")
                    .as_deref()
                    == Some(connection_id)
            })
            .map(|(session_id, _)| session_id.clone())
            .collect();
        for session_id in session_ids {
            let _ = self.close(&session_id).await;
        }
    }

    /// Closing a workbench tears down its RDP sessions (same contract as the
    /// local shells, Telnet and VNC).
    pub async fn close_workbench(&self, workbench_id: &str) {
        let session_ids: Vec<String> = self
            .sessions
            .read()
            .await
            .iter()
            .filter(|(_, entry)| entry.workbench_id == workbench_id)
            .map(|(session_id, _)| session_id.clone())
            .collect();
        for session_id in session_ids {
            let _ = self.close(&session_id).await;
        }
    }

    /// Read-only inventory of live RDP sessions (no credentials, no state —
    /// state changes only surface as `rdp/session/state` events).
    pub async fn list(&self) -> Value {
        let sessions = self.sessions.read().await;
        let mut list: Vec<Value> = sessions
            .iter()
            .map(|(session_id, entry)| {
                json!({
                    "sessionId": session_id,
                    "connectionId": entry
                        .connection_id
                        .lock()
                        .expect("rdp connection id mutex")
                        .clone(),
                    "workbenchId": entry.workbench_id,
                    "host": entry.host,
                    "port": entry.port,
                    "username": entry.username,
                    "hasPassword": entry.password.is_some(),
                    "useNla": entry.use_nla,
                    "certificatePolicy": entry.cert_policy.as_str(),
                    "clipboard": entry.clipboard_enabled,
                    "createdAt": entry.created_at_secs,
                })
            })
            .collect();
        list.sort_by(|a, b| {
            a["createdAt"]
                .as_u64()
                .cmp(&b["createdAt"].as_u64())
                .then_with(|| a["sessionId"].as_str().cmp(&b["sessionId"].as_str()))
        });
        json!({ "sessions": list })
    }

    /// Re-emits one composite full desktop after a webview remount. Incremental
    /// RDP patches cannot restore a renderer independently, so this never
    /// replays partial patches as if they were a complete scene.
    pub async fn replay(&self, session_id: &str, emitter: &PluginEmitter) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        let mut frame_state = entry.frame_state.lock().await;
        let Ok(frame) = frame_state.replay() else {
            return Ok(json!({ "frameCount": 0, "complete": false }));
        };
        // Keep the state lock through publication. Realtime patch mutation and
        // replay now share one ordering boundary, so sequence N cannot be
        // allocated then overtaken by replay sequence N+1.
        publish_frame(session_id, &frame, emitter);
        Ok(json!({ "frameCount": 1, "complete": true }))
    }

    /// Certificate-prompt resolution (`rdp/certificate/resolve`).
    pub fn resolve_certificate(
        &self,
        challenge_id: &str,
        accept: bool,
        remember: bool,
    ) -> Result<(), String> {
        self.broker.resolve(challenge_id, accept, remember)
    }

    /// True when the challenge id belongs to an RDP certificate prompt (the
    /// shared `connection/challenge/resolve` dispatcher uses this to route).
    pub fn has_certificate_challenge(&self, challenge_id: &str) -> bool {
        self.broker
            .pending
            .lock()
            .map(|pending| pending.contains_key(challenge_id))
            .unwrap_or(false)
    }
}
