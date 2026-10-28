//! Telnet layer.

use super::*;

use crate::core::time::unix_now_secs;

impl TelnetSessionRuntime {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Opens a Telnet session: registers the table entry immediately (so the
    /// UI can bind its terminal), then the pump dials the host with the 10s
    /// timeout and publishes `connecting` → `connected`/`error` state events.
    pub async fn start(
        &self,
        request: TelnetStartRequest,
        emitter: PluginEmitter,
    ) -> Result<Value, String> {
        let host = request.host.trim().to_string();
        if host.is_empty() {
            return Err("telnet/start: host is required".to_string());
        }
        let port = request.port.unwrap_or(23);
        if port == 0 {
            return Err("telnet/start: port must be between 1 and 65535".to_string());
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
            return Err("telnet/start: runtimePort must be between 1 and 65535".to_string());
        }
        let cols = request.cols.unwrap_or(120).clamp(2, u16::MAX as u32) as u16;
        let rows = request.rows.unwrap_or(32).clamp(2, u16::MAX as u32) as u16;
        let enter_mode = request.enter_mode.unwrap_or_default();
        let backspace_mode = request.backspace_mode.unwrap_or_default();
        // Invalid auto-login configuration fails the start (SSH D7 contract:
        // never degrade silently) — and the error text is configuration
        // feedback, it never includes secret values.
        let auto_login_plan = match &request.auto_login {
            Some(spec) => parse_auto_login(spec)?,
            None => None,
        };
        let session_id = uuid::Uuid::new_v4().to_string();
        let replay = Arc::new(tokio::sync::Mutex::new(ReplayBuffer::default()));
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        self.sessions.write().await.insert(
            session_id.clone(),
            Arc::new(TelnetSession {
                connection_id: request.connection_id.clone(),
                workbench_id: request.workbench_id.clone(),
                host: host.clone(),
                port,
                runtime_host: runtime_host.clone(),
                runtime_port,
                created_at_secs: unix_now_secs(),
                cmd_tx,
                replay: replay.clone(),
            }),
        );
        spawn_pump(
            session_id.clone(),
            request.workbench_id,
            host.clone(),
            port,
            runtime_host,
            runtime_port,
            cols,
            rows,
            enter_mode,
            backspace_mode,
            auto_login_plan,
            cmd_rx,
            replay,
            emitter,
            self.sessions.clone(),
        );
        Ok(json!({
            "sessionId": session_id,
            "host": host,
            "port": port,
        }))
    }

    async fn session(&self, session_id: &str) -> Result<Arc<TelnetSession>, String> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| "Telnet session was not found".to_string())
    }

    pub async fn resize(&self, session_id: &str, cols: u32, rows: u32) -> Result<(), String> {
        self.session(session_id)
            .await?
            .cmd_tx
            .send(TelnetCommand::Resize {
                cols: cols.clamp(1, u16::MAX as u32) as u16,
                rows: rows.clamp(1, u16::MAX as u32) as u16,
            })
            .await
            .map_err(|_| "Telnet session is closed".to_string())
    }

    pub async fn replay(
        &self,
        session_id: &str,
        after_sequence: u64,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let session = self.session(session_id).await?;
        let replay = session.replay.lock().await;
        let first_available_sequence = replay.first_sequence();
        let tail_sequence = replay.tail_sequence();
        let frames = replay.after(after_sequence);
        drop(replay);
        let channel = format!("telnet/terminal/out/{session_id}");
        for frame in &frames {
            emitter
                .binary(&channel, &frame.encode())
                .map_err(|error| error.message)?;
        }
        Ok(json!({
            "frameCount": frames.len(),
            "firstAvailableSequence": first_available_sequence,
            "tailSequence": tail_sequence,
            "complete": after_sequence.saturating_add(1) >= first_available_sequence
        }))
    }

    pub async fn close(&self, session_id: &str) -> Result<(), String> {
        let session = self
            .sessions
            .write()
            .await
            .remove(session_id)
            .ok_or("Telnet session was not found")?;
        let _ = session.cmd_tx.send(TelnetCommand::Close).await;
        Ok(())
    }

    /// Closing a workbench tears down its Telnet sessions (same contract as
    /// the local shells); a webview reload does NOT pass through here.
    pub async fn close_connection(&self, connection_id: &str) {
        let session_ids: Vec<String> = self
            .sessions
            .read()
            .await
            .iter()
            .filter(|(_, session)| session.connection_id.as_deref() == Some(connection_id))
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
            .filter(|(_, session)| session.workbench_id == workbench_id)
            .map(|(session_id, _)| session_id.clone())
            .collect();
        for session_id in session_ids {
            let _ = self.close(&session_id).await;
        }
    }

    /// Read-only inventory of live Telnet sessions (workbench reattach hook).
    pub async fn list(&self) -> Value {
        let sessions = self.sessions.read().await;
        let mut list: Vec<Value> = sessions
            .iter()
            .map(|(session_id, session)| {
                json!({
                    "sessionId": session_id,
                    "connectionId": session.connection_id,
                    "workbenchId": session.workbench_id,
                    "host": session.host,
                    "port": session.port,
                    "runtimeHost": session.runtime_host,
                    "runtimePort": session.runtime_port,
                    "createdAt": session.created_at_secs,
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

    /// Keyboard input from the SDK's blocking binary-handler thread / the
    /// `telnet/write` fallback. A full queue is retried briefly and then the
    /// chunk is dropped (bounded, so a flooded session cannot park a shared SDK
    /// worker); only a closed queue is an error, so a transient drop never
    /// surfaces as `telnet/terminal/error` (session death for the workbench).
    pub fn write_input(&self, session_id: &str, data: Vec<u8>) -> Result<(), String> {
        // Do not hold the session-map read guard while enqueueing: closing a
        // dead session needs the write lock to drop the receiver.
        let cmd_tx = {
            let sessions = self.sessions.blocking_read();
            sessions
                .get(session_id)
                .map(|session| session.cmd_tx.clone())
                // Same string contract as the SSH/local mirrors — the
                // workbench's dead-session detection matches on this error.
                .ok_or("Telnet session was not found or expired")?
        };
        crate::sessions::input_queue::enqueue_bounded(
            &cmd_tx,
            TelnetCommand::Input(data),
            "Telnet session",
        )
    }
}
