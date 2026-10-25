//! Terminal output frames and keyboard input for local shells.
//!
//! Output is pushed into the replay buffer, encoded as a binary frame and
//! emitted on `local/terminal/out/{sessionId}`; the active asciicast recorder
//! observes the same bursts. Input is queued without blocking the shared SDK
//! worker, and replay re-emits buffered frames after a reload.

use super::*;

impl LocalTerminalRuntime {
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
        for frame in &frames {
            emitter
                .binary(&format!("local/terminal/out/{session_id}"), &frame.encode())
                .map_err(|error| error.message)?;
        }
        Ok(json!({
            "frameCount": frames.len(),
            "firstAvailableSequence": first_available_sequence,
            "tailSequence": tail_sequence,
            "complete": after_sequence.saturating_add(1) >= first_available_sequence
        }))
    }

    /// `local/recording/start`: attaches an asciicast recorder to a live local
    /// shell. Mirrors `ssh/recording/start` in shape so the workbench's record
    /// toggle can target either session kind; recordings land in the same
    /// directory the list/replay family reads, with `host: "local"` metadata.
    pub async fn recording_start(
        &self,
        session_id: &str,
        data_dir: &Path,
    ) -> Result<Value, String> {
        let session = self.session(session_id).await?;
        let mut slot = session
            .recorder
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if slot.is_some() {
            return Err("This session is already being recorded".to_string());
        }
        let recording_id = uuid::Uuid::new_v4().to_string();
        let recorder = session_recording::SessionRecorder::start(
            data_dir,
            &recording_id,
            "local",
            "local",
            session_id,
            u32::from(session.cols),
            u32::from(session.rows),
        )?;
        *slot = Some(recorder);
        Ok(json!({ "recordingId": recording_id, "recording": true }))
    }

    /// `local/recording/stop`: finalizes the active recording (if any) and
    /// returns its summary — the same document `ssh/recording/stop` reports.
    pub async fn recording_stop(&self, session_id: &str) -> Result<Value, String> {
        let session = self.session(session_id).await?;
        let recorder = session
            .recorder
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .ok_or("This session has no active recording")?;
        recorder.finish()
    }

    /// Keyboard input from the SDK's blocking binary-handler thread. A full
    /// queue is retried briefly (bounded) and then the chunk is dropped so one
    /// flooded session cannot park a shared SDK worker, and a transient drop
    /// never surfaces as `local/terminal/error` (which the workbench treats as
    /// session death). Only a closed queue is an error.
    pub fn write_input(&self, session_id: &str, data: Vec<u8>) -> Result<(), String> {
        // Do not hold the session-map read guard while enqueueing: closing a
        // dead session needs the write lock to drop the receiver.
        let terminal_tx = {
            let sessions = self.sessions.blocking_read();
            sessions
                .get(session_id)
                .map(|session| session.terminal_tx.clone())
                // Same string contract as the SSH mirror — the workbench's
                // dead-session detection matches on the error event.
                .ok_or("Local session was not found or expired")?
        };
        crate::sessions::input_queue::enqueue_bounded(
            &terminal_tx,
            LocalTerminalCommand::Input(data),
            "Local terminal",
        )
    }
}

pub(super) async fn publish_local_terminal(
    session_id: &str,
    stream: TerminalStream,
    data: Vec<u8>,
    replay: &Arc<AsyncMutex<ReplayBuffer>>,
    emitter: &PluginEmitter,
) {
    let frame = replay.lock().await.push(stream, data);
    if let Err(error) = emitter.binary(&format!("local/terminal/out/{session_id}"), &frame.encode())
    {
        eprintln!(
            "[dbx-plugin-sshx] local terminal output failed: {}",
            error.message
        );
    }
}

/// Feeds the active recorder (if any) with an output burst about to be
/// published. Stdout-only by construction — State frames never pass through
/// here, mirroring the SSH pump. Sync and lock-scoped: never held across an
/// await. `pub(crate)` only so the runtime root can re-export it to
/// `local::tests` (the sibling test module exercises the recorder feed).
pub(crate) fn observe_local_recording(
    recorder: &Arc<Mutex<Option<session_recording::SessionRecorder>>>,
    data: &[u8],
) {
    if let Ok(mut slot) = recorder.lock() {
        if let Some(active) = slot.as_mut() {
            active.observe(data);
        }
    }
}
