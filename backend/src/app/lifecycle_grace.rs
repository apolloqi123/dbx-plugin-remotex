//! Short grace windows around host-initiated SSH teardown.
//!
//! DBX may tear down and immediately re-push an unchanged plugin connection
//! while the user edits another connection. The host-side recycle closes the
//! old plugin pool and calls `connection/disconnect`, then connects again; if
//! the sidecar acted immediately, every live PTY would close and the terminal
//! would visibly reconnect. Deferring those SSH-only closes for a few seconds
//! lets the following `connection/connect` (same transport identity) or
//! `ssh/session/attach` cancel the teardown and keep the PTY alive.
//!
//! The guard is deliberately SSH-only and bounded: a real disconnect with no
//! reattach still closes after the grace, and a reconnect whose host, port or
//! credentials changed performs the immediate disconnect so the new settings
//! are applied.

use std::future::Future;

use crate::platform::diagnostics::log_lifecycle_debug;
use super::*;

// Host workbench re-inits can take much longer than a connection re-push
// (measured ~16s in the wild). Keep the PTY until the restored webview has
// had time to attach; an explicit user close still reclaims after this bound.
const SSH_LIFECYCLE_GRACE: std::time::Duration = std::time::Duration::from_secs(20);
const SSH_SESSION_CLOSE_GRACE: std::time::Duration = std::time::Duration::from_secs(20);

impl Plugin {
    fn next_lifecycle_grace_generation(&self) -> u64 {
        self.lifecycle_grace_seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn schedule_after_grace<F>(
        &self,
        pending: Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
        key: String,
        grace: std::time::Duration,
        future: F,
    ) where
        F: Future<Output = ()> + Send + 'static,
    {
        let generation = self.next_lifecycle_grace_generation();
        if let Ok(mut entries) = pending.lock() {
            entries.insert(key.clone(), generation);
        }
        self.runtime.spawn(async move {
            tokio::time::sleep(grace).await;
            let fire = pending.lock().map(|entries| entries.get(&key).copied() == Some(generation)).unwrap_or(false);
            if !fire {
                return;
            }
            if let Ok(mut entries) = pending.lock() {
                entries.remove(&key);
            }
            future.await;
        });
    }

    /// Defers an SSH disconnect when the connection still owns live sessions.
    /// Returns false when the caller should disconnect immediately.
    pub(super) fn schedule_ssh_disconnect_if_live(&self, connection_id: &str) -> bool {
        // Defer when either the registry entry or a PTY still exists. A host
        // teardown can close the session first and issue `connection/disconnect`
        // immediately after; requiring both would defeat the grace in exactly
        // that order.
        let registered = self.ssh.has_connection(connection_id);
        let live = self.runtime.block_on(self.ssh.has_live_session(connection_id));
        if !registered && !live {
            log_lifecycle_debug(format!(
                "connection-disconnect immediate id={connection_id} registered=false live=false"
            ));
            return false;
        }
        log_lifecycle_debug(format!(
            "connection-disconnect deferred id={connection_id} registered={registered} live={live}"
        ));
        self.schedule_ssh_disconnect(connection_id);
        true
    }

    fn schedule_ssh_disconnect(&self, connection_id: &str) {
        let ssh = self.ssh.clone();
        let connection_id = connection_id.to_string();
        self.schedule_after_grace(self.pending_ssh_disconnects.clone(), connection_id.clone(), SSH_LIFECYCLE_GRACE, async move {
            let _ = ssh.disconnect_connection(&connection_id).await;
        });
    }

    /// Pushes an already scheduled disconnect back by another grace window.
    /// A live webview does this while its session is attached, so a spurious
    /// host teardown (editing another connection, workbench re-init) cannot
    /// close the PTY; once the webview is gone the timer finally lands.
    pub(super) fn extend_pending_ssh_disconnect(&self, connection_id: &str) {
        let pending = self
            .pending_ssh_disconnects
            .lock()
            .map(|entries| entries.contains_key(connection_id))
            .unwrap_or(false);
        if pending {
            self.schedule_ssh_disconnect(connection_id);
        }
    }

    /// Cancels a deferred disconnect and returns the old stored connection so
    /// the caller can compare runtime identity before keeping live sessions.
    pub(super) fn cancel_pending_ssh_disconnect(&self, connection_id: &str) -> Option<StoredConnection> {
        self.pending_ssh_disconnects.lock().ok()?.remove(connection_id)?;
        log_lifecycle_debug(format!("connection-disconnect canceled id={connection_id}"));
        self.runtime.block_on(self.ssh.stored_connection(connection_id)).ok()
    }

    /// Defers the SSH portion of `workbench/close`. The next attach/open on
    /// this workbench cancels it; otherwise the PTY closes after the grace.
    pub(super) fn schedule_ssh_workbench_close(&self, workbench_id: &str) {
        log_lifecycle_debug(format!("workbench-close deferred workbench={workbench_id}"));
        self.schedule_workbench_close(workbench_id);
    }

    fn schedule_workbench_close(&self, workbench_id: &str) {
        let ssh = self.ssh.clone();
        let workbench_id = workbench_id.to_string();
        self.schedule_after_grace(self.pending_workbench_closes.clone(), workbench_id.clone(), SSH_LIFECYCLE_GRACE, async move {
            let _ = ssh.close_workbench(&workbench_id).await;
        });
    }

    pub(super) fn cancel_pending_workbench_close(&self, workbench_id: &str) {
        let removed = self
            .pending_workbench_closes
            .lock()
            .map(|mut entries| entries.remove(workbench_id).is_some())
            .unwrap_or(false);
        if removed {
            log_lifecycle_debug(format!("workbench-close canceled workbench={workbench_id}"));
        }
    }

    /// Renews a pending workbench teardown while the webview is still alive.
    pub(super) fn extend_pending_workbench_close(&self, workbench_id: &str) {
        let pending = self
            .pending_workbench_closes
            .lock()
            .map(|entries| entries.contains_key(workbench_id))
            .unwrap_or(false);
        if pending {
            self.schedule_workbench_close(workbench_id);
        }
    }

    /// Defers the webview's `ssh/session/close` during host workbench teardown.
    /// The restored webview attaches to the same session id within the grace,
    /// which cancels this close and preserves the PTY/scrollback.
    pub(super) fn schedule_ssh_session_close(&self, session_id: &str) {
        log_lifecycle_debug(format!("session-close deferred session={session_id}"));
        self.schedule_session_close(session_id);
    }

    fn schedule_session_close(&self, session_id: &str) {
        let ssh = self.ssh.clone();
        let session_id = session_id.to_string();
        self.schedule_after_grace(
            self.pending_ssh_session_closes.clone(),
            session_id.clone(),
            SSH_SESSION_CLOSE_GRACE,
            async move {
                let _ = ssh.close_session(&session_id).await;
            },
        );
    }

    pub(super) fn cancel_pending_ssh_session_close(&self, session_id: &str) {
        let removed = self
            .pending_ssh_session_closes
            .lock()
            .map(|mut entries| entries.remove(session_id).is_some())
            .unwrap_or(false);
        if removed {
            log_lifecycle_debug(format!("session-close canceled session={session_id}"));
        }
    }

    /// Whether any per-session teardown is waiting. Session-id enumeration on
    /// the heartbeat path is only needed to renew these entries; in the steady
    /// state (no teardown scheduled) the heartbeat stays map-lookup cheap.
    pub(super) fn has_pending_ssh_session_closes(&self) -> bool {
        self.pending_ssh_session_closes
            .lock()
            .map(|entries| !entries.is_empty())
            .unwrap_or(false)
    }

    /// Renews a pending session teardown while its webview keeps checking in.
    pub(super) fn extend_pending_ssh_session_close(&self, session_id: &str) {
        let pending = self
            .pending_ssh_session_closes
            .lock()
            .map(|entries| entries.contains_key(session_id))
            .unwrap_or(false);
        if pending {
            self.schedule_session_close(session_id);
        }
    }
}

/// Whether the two lifecycle snapshots describe the same live SSH transport.
/// UI/behavior-only fields (name, color, sudo/2FA tuning, timeouts, triggers)
/// deliberately do not participate: `StoreConnection` can update those on the
/// existing runtime without closing the PTY.
pub(super) fn same_ssh_transport(left: &StoredConnection, right: &StoredConnection) -> bool {
    left.host == right.host
        && left.port == right.port
        && left.runtime_host == right.runtime_host
        && left.runtime_port == right.runtime_port
        && left.username == right.username
        && left.password == right.password
        && left.authentication == right.authentication
        && left.private_key_path == right.private_key_path
        && left.private_key == right.private_key
        && left.private_key_passphrase == right.private_key_passphrase
        && left.agent_socket == right.agent_socket
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::AuthenticationMethod;

    fn connection(host: &str, port: u16, username: &str) -> StoredConnection {
        StoredConnection::from_lifecycle_params(&json!({
            "connection": {
                "id": "c1",
                "host": host,
                "port": port,
                "username": username,
                "password": "secret",
                "external_config": { "authentication": "password" },
            }
        }))
        .expect("parse")
    }

    #[test]
    fn same_transport_ignores_ui_only_changes() {
        let mut renamed = connection("10.0.0.8", 22, "root");
        renamed.name = Some("renamed in the sidebar".to_string());
        renamed.read_only = true;
        renamed.connect_timeout_secs = 99;
        assert!(same_ssh_transport(
            &connection("10.0.0.8", 22, "root"),
            &renamed
        ));
    }

    #[test]
    fn same_transport_rejects_endpoint_and_credential_changes() {
        let base = connection("10.0.0.8", 22, "root");
        assert!(!same_ssh_transport(&base, &connection("10.0.0.9", 22, "root")));
        assert!(!same_ssh_transport(&base, &connection("10.0.0.8", 2222, "root")));
        assert!(!same_ssh_transport(&base, &connection("10.0.0.8", 22, "admin")));

        let mut changed_password = base.clone();
        changed_password.password = "other".to_string();
        assert!(!same_ssh_transport(&base, &changed_password));

        let mut changed_authentication = base.clone();
        changed_authentication.authentication = AuthenticationMethod::PrivateKey;
        assert!(!same_ssh_transport(&base, &changed_authentication));
    }
}
