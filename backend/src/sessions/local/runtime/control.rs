//! Resize and close control plane for local shells.
//!
//! These are the RPC-level commands; the pump applies them to the PTY (see
//! `lifecycle`), enforcing same-size dedup and the startup resize gate.

use super::*;

impl LocalTerminalRuntime {
    pub async fn resize(&self, session_id: &str, cols: u32, rows: u32) -> Result<(), String> {
        self.session(session_id)
            .await?
            .terminal_tx
            .send(LocalTerminalCommand::Resize { cols, rows })
            .await
            .map_err(|_| "Local terminal is closed".to_string())
    }

    pub async fn close(&self, session_id: &str) -> Result<(), String> {
        let session = self
            .sessions
            .write()
            .await
            .remove(session_id)
            .ok_or("Local session was not found")?;
        let _ = session.terminal_tx.send(LocalTerminalCommand::Close).await;
        Ok(())
    }

    /// Closing a workbench tears down its local shells; a webview reload does
    /// NOT pass through here, so a reload finds the shell again via
    /// `local/session/list` + replay.
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
}
