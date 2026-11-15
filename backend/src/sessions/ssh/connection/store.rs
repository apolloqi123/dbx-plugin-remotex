//! SSH connection support.

use super::*;

impl SshRuntime {
    /// Stored (hydrated) connection by id — the task provider's credential
    /// source. Secrets stay inside the plugin: the caller uses them for the
    /// dial and for log redaction, never for echo-back or task config.
    pub(crate) async fn stored_connection(
        &self,
        connection_id: &str,
    ) -> Result<StoredConnection, String> {
        self.connections
            .read()
            .map_err(|_| "Connection registry is poisoned".to_string())?
            .get(connection_id)
            .cloned()
            .ok_or_else(|| format!("Connection {connection_id} is not active; reopen it from DBX"))
    }

    /// Registers a hydrated `io.dbx.remotex.local` target (`connection/connect`
    /// lifecycle). Re-connecting over an existing entry replaces it, exactly
    /// like [`Self::store_connection`].
    pub(crate) fn store_local_target(
        &self,
        target: crate::sessions::local::target::LocalTarget,
    ) -> Result<(), String> {
        let id = target.id.clone();
        self.local_targets
            .write()
            .map_err(|_| "Local target registry is poisoned".to_string())?
            .insert(id, target);
        Ok(())
    }

    /// Drops a local target registration (`connection/disconnect`). Unknown
    /// ids are a no-op so SSH disconnects stay untouched.
    pub(crate) fn drop_local_target(&self, connection_id: &str) {
        if let Ok(mut targets) = self.local_targets.write() {
            targets.remove(connection_id);
        }
    }

    /// Hydrated local shell target by id — `None` means the connection is
    /// not a local target (SSH provider, or never connected in this process).
    pub(crate) async fn local_target(
        &self,
        connection_id: &str,
    ) -> Option<crate::sessions::local::target::LocalTarget> {
        self.local_targets.read().ok()?.get(connection_id).cloned()
    }

    /// True when at least one authenticated live session exists for the
    /// connection — the task provider's "session reuse covers the missing
    /// stored connection" check.
    pub(crate) async fn has_live_session(&self, connection_id: &str) -> bool {
        self.sessions.read().await.values().any(|session| {
            session.connection_id == connection_id && session.connected.load(Ordering::Acquire)
        })
    }

    /// Session ids matching either scope, in one registry pass. The webview
    /// heartbeat renews their deferred teardown while the workbench is alive;
    /// both scopes are passed together because a beat carries both and the
    /// registry is only scanned while a per-session close is actually pending.
    pub(crate) async fn session_ids_touching(
        &self,
        workbench_id: Option<&str>,
        connection_id: Option<&str>,
    ) -> Vec<String> {
        self.sessions
            .read()
            .await
            .iter()
            .filter(|(_, session)| {
                connection_id.is_some_and(|connection_id| session.connection_id == connection_id)
                    || workbench_id.is_some_and(|workbench_id| {
                        session
                            .workbench_id
                            .read()
                            .map(|workbench| workbench.as_str() == workbench_id)
                            .unwrap_or(false)
                    })
            })
            .map(|(session_id, _)| session_id.clone())
            .collect()
    }
}
