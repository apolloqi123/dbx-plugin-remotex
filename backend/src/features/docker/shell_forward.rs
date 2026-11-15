//! Terminal forwarder and its test harness. Kept apart from the registry in
//! `shell.rs` so both files stay inside the source-size budget; the methods
//! are still inherent on `DockerTerminals`.

use super::*;

impl DockerTerminals {
    /// Drains raw PTY chunks into binary events, enforces the TTL, then emits
    /// the terminal state (unless the close was explicit) and releases the
    /// slot. A runner that ends immediately still lands here: the closed sink
    /// wakes the loop and its result becomes the state event.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn forward(
        self: Arc<Self>,
        terminal_id: String,
        emitter: PluginEmitter,
        closed: Arc<AtomicBool>,
        done: Arc<Notify>,
        cancel: watch::Sender<bool>,
        mut runner: JoinHandle<Result<Option<i32>, String>>,
        mut sink: mpsc::Receiver<Vec<u8>>,
        limits: TerminalLimits,
    ) {
        let started = self.started(&terminal_id);
        let mut ticker = tokio::time::interval(limits.tick());
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut expired = false;
        let result = loop {
            tokio::select! {
                biased;
                _ = ticker.tick() => {
                    if started.is_some_and(|started| started.elapsed() >= limits.ttl) {
                        expired = true;
                        let _ = cancel.send(true);
                        break stop_runner(&mut runner, limits.stop_grace).await;
                    }
                }
                chunk = sink.recv() => match chunk {
                    Some(bytes) if !bytes.is_empty() => {
                        let _ = emitter.binary(&format!("docker/terminal/out/{terminal_id}"), &bytes);
                    }
                    Some(_) => {}
                    None => break stop_runner(&mut runner, limits.stop_grace).await,
                },
                status = &mut runner => break status,
            }
        };
        while let Ok(bytes) = sink.try_recv() {
            if !bytes.is_empty() {
                let _ = emitter.binary(&format!("docker/terminal/out/{terminal_id}"), &bytes);
            }
        }
        self.lock().remove(&terminal_id);
        done.notify_one();
        // An explicit close is not a state transition the UI has to handle.
        if closed.load(Ordering::Relaxed) {
            return;
        }
        let payload = if expired {
            json!({ "terminalId": terminal_id, "state": "error", "error": TIMEOUT_ERROR })
        } else {
            match result {
                Ok(Ok(code)) => json!({
                    "terminalId": terminal_id,
                    "state": "exited",
                    "exitCode": code.unwrap_or(-1),
                }),
                Ok(Err(error)) => {
                    json!({ "terminalId": terminal_id, "state": "error", "error": error })
                }
                Err(error) => json!({
                    "terminalId": terminal_id,
                    "state": "error",
                    "error": format!("Container terminal task failed: {error}"),
                }),
            }
        };
        let _ = emitter.event("docker/terminal/state", payload);
    }

    fn started(&self, terminal_id: &str) -> Option<Instant> {
        self.lock().get(terminal_id).map(|entry| entry.started)
    }
}

/// Waits out a runner after the pump ends, bounded by `grace`. A runner that
/// ignores cancellation is aborted instead of awaited forever: the SSH
/// channel task is dropped and the local child is killed by its own pump.
async fn stop_runner(
    runner: &mut JoinHandle<Result<Option<i32>, String>>,
    grace: Duration,
) -> Result<Result<Option<i32>, String>, tokio::task::JoinError> {
    match tokio::time::timeout(grace, &mut *runner).await {
        Ok(result) => result,
        Err(_) => {
            runner.abort();
            runner.await
        }
    }
}

#[cfg(test)]
impl DockerTerminals {
    /// Inserts a placeholder entry (no runner) so cap tests stay cheap.
    pub(crate) async fn insert_for_test(&self, terminal_id: String) -> String {
        match self.reserve(&terminal_id, "", "") {
            Ok(reserved) => {
                drop(reserved);
                String::new()
            }
            Err(error) => error,
        }
    }

    pub(crate) fn active_for_test(&self) -> usize {
        self.lock().len()
    }

    /// Admission attempt with an explicit owner/session, no runner. Returns
    /// the cancellation receiver so tests can observe supersession.
    pub(crate) fn reserve_for_test(
        &self,
        terminal_id: &str,
        owner: &str,
        session: &str,
    ) -> Result<watch::Receiver<bool>, String> {
        self.reserve(terminal_id, owner, session)
            .map(|reserved| reserved.cancel_rx)
    }

    pub(crate) fn try_reserve_test(
        &self,
        terminal_id: &str,
        owner: &str,
        session: &str,
    ) -> Result<(), String> {
        self.reserve_for_test(terminal_id, owner, session)
            .map(|_| ())
    }

    /// Starts a runner that emits one chunk and then ignores cancellation, so
    /// only the TTL can reap it (the same shape as a wedged child).
    pub(crate) fn start_fake_for_test(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        ttl: Duration,
    ) -> Result<String, String> {
        self.start_fake_with(emitter, ttl, false)
    }

    /// Starts a runner that exits as soon as its cancellation fires, so a
    /// test can observe the explicit-close suppression path.
    pub(crate) fn start_cancellable_fake_for_test(
        self: &Arc<Self>,
        emitter: PluginEmitter,
    ) -> Result<String, String> {
        self.start_fake_with(emitter, Duration::from_secs(60), true)
    }

    fn start_fake_with(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        ttl: Duration,
        cancellable: bool,
    ) -> Result<String, String> {
        let terminal_id = requested_terminal_id(None)?;
        let Reserved {
            cancel,
            cancel_rx,
            input_rx,
            resize_rx,
            closed,
            done,
        } = self.reserve(&terminal_id, "", "")?;
        let (sink, sink_rx) = mpsc::channel(OUTPUT_QUEUE);
        let handle = tokio::spawn(async move {
            // Keep the queues open while the fake runner lives so resize/input
            // admission behaves like a real terminal.
            let _input_rx = input_rx;
            let _resize_rx = resize_rx;
            let _ = sink.send(b"ready".to_vec()).await;
            if cancellable {
                let mut cancel_rx = cancel_rx;
                let _ = cancel_rx.changed().await;
                return Ok(Some(0));
            }
            drop(cancel_rx);
            std::future::pending::<()>().await;
            unreachable!("the fake terminal is only aborted by the TTL")
        });
        let registry = Arc::clone(self);
        let forward_id = terminal_id.clone();
        let limits = TerminalLimits {
            ttl,
            stop_grace: Duration::from_millis(20),
        };
        tokio::spawn(async move {
            registry
                .forward(
                    forward_id, emitter, closed, done, cancel, handle, sink_rx, limits,
                )
                .await;
        });
        Ok(terminal_id)
    }
}
