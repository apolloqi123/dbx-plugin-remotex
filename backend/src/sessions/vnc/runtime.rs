//! VNC session support.

use super::*;

use crate::core::time::unix_now_secs;
use crate::platform::diagnostics::log_rdp_debug;

impl VncSessionRuntime {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Registers the session row immediately (so the UI can bind its pane)
    /// and spawns the pump, which publishes `connecting` → `connected` /
    /// `error` state events and framebuffer patches.
    pub async fn start(
        &self,
        request: VncStartRequest,
        emitter: PluginEmitter,
    ) -> Result<Value, String> {
        let host = request.host.trim().to_string();
        if host.is_empty() {
            return Err("vnc/start: host is required".to_string());
        }
        let port = request.port.unwrap_or(5900);
        if port == 0 {
            return Err("vnc/start: port must be between 1 and 65535".to_string());
        }
        let runtime_host = request
            .runtime_host
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(host.as_str())
            .to_string();
        let runtime_port = request.runtime_port.unwrap_or(port);
        if runtime_port == 0 {
            return Err("vnc/start: runtimePort must be between 1 and 65535".to_string());
        }
        let password = request
            .password
            .filter(|password| !password.is_empty())
            .map(Zeroizing::new);
        validate_password(password.as_ref().map(|p| p.as_str()))?;
        let scale_mode = match request.scale_mode.as_deref() {
            None | Some("") => ScaleMode::default(),
            Some(value) => ScaleMode::parse(value)
                .ok_or_else(|| "vnc/start: scaleMode must be fit, stretch or actual".to_string())?,
        };
        let reconnect_attempts = request
            .reconnect_attempts
            .unwrap_or(DEFAULT_RECONNECT_ATTEMPTS)
            .min(MAX_RECONNECT_ATTEMPTS);
        let session_id = uuid::Uuid::new_v4().to_string();
        let entry = Arc::new(VncSessionEntry {
            connection_id: request.connection_id.clone(),
            workbench_id: request.workbench_id.clone(),
            host: host.clone(),
            port,
            runtime_host,
            runtime_port,
            password,
            scale_mode,
            reconnect_attempts,
            created_at_secs: unix_now_secs(),
            generation: AtomicU64::new(0),
            frame_sequence: AtomicU64::new(0),
            close_requested: AtomicBool::new(false),
            command_sender: tokio::sync::Mutex::new(None),
            framebuffer: tokio::sync::Mutex::new(None),
        });
        self.sessions
            .write()
            .await
            .insert(session_id.clone(), entry.clone());
        spawn_worker(session_id.clone(), entry, emitter, self.sessions.clone());
        Ok(json!({
            "sessionId": session_id,
            "host": host,
            "port": port,
            "scaleMode": scale_mode.as_str(),
            "reconnectAttempts": reconnect_attempts,
        }))
    }

    async fn session(&self, session_id: &str) -> Result<Arc<VncSessionEntry>, String> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| "VNC session was not found".to_string())
    }

    /// Keyboard/pointer event forwarding (`vnc/input`, alias `vnc/write`).
    pub async fn input(&self, session_id: &str, event: VncInputEvent) -> Result<(), String> {
        let entry = self.session(session_id).await?;
        let sender = entry
            .command_sender
            .lock()
            .await
            .clone()
            .ok_or_else(|| "VNC session is not connected".to_string())?;
        sender
            .send(VncCommand::Input(event))
            .await
            .map_err(|_| "VNC session is closed".to_string())
    }

    /// Live remote resize (`vnc/resize`): queues RFB SetDesktopSize for the
    /// current engine. The server confirms with an ExtendedDesktopSize update
    /// (DesktopUpdate → retained framebuffer swap + full refresh), so the
    /// generation/session id stay put. Returns `live=false` when no engine is
    /// attached (dialling/backoff); the request is then dropped.
    pub async fn resize(&self, session_id: &str, width: u16, height: u16) -> Result<Value, String> {
        validate_resize_size(width, height)?;
        let entry = self.session(session_id).await?;
        if entry.close_requested.load(Ordering::Acquire) {
            return Err("VNC session is closed".to_string());
        }
        let sender = entry.command_sender.lock().await.clone();
        let Some(sender) = sender else {
            return Ok(json!({
                "sessionId": session_id,
                "success": true,
                "live": false,
            }));
        };
        let live = sender
            .send(VncCommand::Resize { width, height })
            .await
            .is_ok();
        Ok(json!({
            "sessionId": session_id,
            "success": true,
            "live": live,
        }))
    }

    /// Local clipboard → remote (`vnc/set-clipboard`).
    pub async fn set_clipboard(&self, session_id: &str, text: String) -> Result<(), String> {
        validate_clipboard_text(&text)?;
        let entry = self.session(session_id).await?;
        log_rdp_debug(format!(
            "vnc clipboard session={session_id} set-clipboard len={}",
            text.len()
        ));
        let sender = entry
            .command_sender
            .lock()
            .await
            .clone()
            .ok_or_else(|| "VNC session is not connected".to_string())?;
        sender
            .send(VncCommand::Clipboard(text))
            .await
            .map_err(|_| "VNC session is closed".to_string())
    }

    /// Manual reconnect: kill the current generation, keep the framebuffer
    /// (the new generation repaints it after `connected`), restart the pump.
    pub async fn reconnect(
        &self,
        session_id: &str,
        emitter: PluginEmitter,
    ) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        entry.close_requested.store(false, Ordering::Release);
        entry.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(sender) = entry.command_sender.lock().await.take() {
            let _ = sender.send(VncCommand::Close).await;
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
            .ok_or("VNC session was not found")?;
        entry.close_requested.store(true, Ordering::Release);
        entry.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(sender) = entry.command_sender.lock().await.take() {
            let _ = sender.send(VncCommand::Close).await;
        }
        Ok(())
    }

    /// Closing a workbench tears down its VNC sessions (same contract as the
    /// local shells and Telnet); a webview reload never passes through here.
    pub async fn close_connection(&self, connection_id: &str) {
        let session_ids: Vec<String> = self
            .sessions
            .read()
            .await
            .iter()
            .filter(|(_, entry)| entry.connection_id.as_deref() == Some(connection_id))
            .map(|(session_id, _)| session_id.clone())
            .collect();
        for session_id in session_ids {
            let _ = self.close(&session_id).await;
        }
    }

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

    /// Read-only inventory of live VNC sessions.
    /// Re-emits the retained desktop as a full patch after a webview remount.
    /// VNC frames are stateful images rather than terminal bytes, so one
    /// complete repaint is the protocol's replay unit.
    pub async fn replay(&self, session_id: &str, emitter: &PluginEmitter) -> Result<Value, String> {
        let entry = self.session(session_id).await?;
        let framebuffer = entry.framebuffer.lock().await;
        let Some(desktop) = framebuffer.as_ref() else {
            return Ok(json!({ "frameCount": 0, "complete": true }));
        };
        let sequence = entry.frame_sequence.fetch_add(1, Ordering::AcqRel);
        let frame = desktop.full_frame_bytes(sequence)?;
        publish_frame(session_id, &frame, emitter);
        Ok(json!({ "frameCount": 1, "complete": true }))
    }

    pub async fn list(&self) -> Value {
        let sessions = self.sessions.read().await;
        let mut list: Vec<Value> = sessions
            .iter()
            .map(|(session_id, entry)| {
                json!({
                    "sessionId": session_id,
                    "connectionId": entry.connection_id,
                    "workbenchId": entry.workbench_id,
                    "host": entry.host,
                    "port": entry.port,
                    "runtimeHost": entry.runtime_host,
                    "runtimePort": entry.runtime_port,
                    "hasPassword": entry.password.is_some(),
                    "scaleMode": entry.scale_mode.as_str(),
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
}

/// Maps engine errors onto `(message, retryable)`. Authentication and
/// protocol failures never retry; transport hiccups do.
pub(crate) fn classify_vnc_error(error: VncError) -> (String, bool) {
    match error {
        VncError::NoPassword | VncError::WrongPassword | VncError::ConnectError => (
            "VNC authentication failed (the server requires classic VNC-Auth and the \
             password was missing or wrong)"
                .to_string(),
            false,
        ),
        VncError::InvalidSecurityTyep(_) => (
            "The VNC server requires an unsupported security type (supported: None and \
             classic VNC-Auth only)"
                .to_string(),
            false,
        ),
        VncError::IoError(error) => (format!("VNC connection failed: {error}"), true),
        VncError::ClientNotRunning => ("VNC client stopped".to_string(), false),
        // NoEncoding / WrongPixelFormat / WrongServerMessage / InvalidImageData
        // / General — protocol violations never retry (the enum is
        // #[non_exhaustive], so future variants land here too).
        other => (other.to_string(), false),
    }
}
