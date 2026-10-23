//! Recording operations for [`SshRuntime`].

use super::*;

impl SshRuntime {
    /// `ssh/recording/start`: attaches a cast recorder to the session's
    /// output stream. One recording per session at a time.
    pub async fn recording_start(&self, session_id: &str) -> Result<Value, String> {
        let session = self.session(session_id).await?;
        let host = self
            .connections
            .read()
            .map_err(|_| "Connection registry is poisoned".to_string())?
            .get(&session.connection_id)
            .map(|connection| connection.host.clone())
            .unwrap_or_default();
        let mut slot = session
            .session_recorder
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if slot.is_some() {
            return Err("This session is already being recorded".to_string());
        }
        let recording_id = Uuid::new_v4().to_string();
        let recorder = session_recording::SessionRecorder::start(
            &self.data_dir,
            &recording_id,
            &session.connection_id,
            &host,
            session_id,
            80,
            24,
        )?;
        *slot = Some(recorder);
        Ok(json!({ "recordingId": recording_id, "recording": true }))
    }

    /// `ssh/recording/stop`: finalizes the active recording (if any) and
    /// returns its summary.
    pub async fn recording_stop(&self, session_id: &str) -> Result<Value, String> {
        let session = self.session(session_id).await?;
        let recorder = session
            .session_recorder
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .ok_or("This session has no active recording")?;
        recorder.finish()
    }
}
