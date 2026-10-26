//! Streamable exec for `SshRuntime`: opens a channel on the session's
//! connection and forwards output fragments to a channel sink, for log
//! following where collecting the whole output first would be useless.

use super::*;

impl SshRuntime {
    /// Runs a command on the session's connection, forwarding every
    /// stdout/stderr fragment to `sink` as it arrives. `sudo` uses the same
    /// Quick Sudo orchestration as [`SshRuntime::exec`]; there is no overall
    /// deadline (the caller stops the stream through `cancel`). Returns the
    /// remote exit code.
    pub async fn exec_stream(
        &self,
        session_id: &str,
        command: &str,
        sudo: bool,
        cancel: tokio::sync::watch::Receiver<bool>,
        sink: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) -> Result<i32, String> {
        let (handle, orchestration, set_env, use_pty) =
            self.stream_context(session_id, sudo).await?;
        if sudo {
            crate::sessions::ssh::exec::exec_sudo_stream(
                &handle,
                &orchestration,
                command,
                use_pty,
                &set_env,
                cancel,
                sink,
            )
            .await
        } else {
            crate::sessions::ssh::exec::exec_plain_stream(&handle, command, &set_env, cancel, sink)
                .await
        }
    }

    /// Mirrors the validation/context reads of [`SshRuntime::exec`] without
    /// its timeout/exec-id plumbing: session exists, sudo refuses read-only
    /// connections and connections with Quick Sudo disabled, then the live
    /// orchestration, `setEnv`, and per-profile PTY decision are read.
    async fn stream_context(
        &self,
        session_id: &str,
        sudo: bool,
    ) -> Result<(Arc<Handle<SshClient>>, SudoAuth, Vec<(String, String)>, bool), String> {
        let session = self.session(session_id).await?;
        if sudo && session.read_only {
            return Err(
                "Sudo execution is disabled by the read-only connection setting".to_string(),
            );
        }
        let connection = self
            .connections
            .read()
            .map_err(|_| "Connection registry is poisoned".to_string())?
            .get(&session.connection_id)
            .cloned()
            .ok_or("Connection is not active; reopen it from DBX".to_string())?;
        if sudo && !connection.sudo_enabled() {
            return Err("Quick Sudo is disabled for this connection".to_string());
        }
        let orchestration = session
            .orchestration
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let use_pty = if sudo {
            let store = sudo_profiles::load_store(&self.data_dir);
            sudo_profiles::effective_use_pty(
                connection.sudo_use_pty,
                effective_sudo_profile(&connection, &store).as_ref(),
            )
        } else {
            false
        };
        Ok((
            session.handle.clone(),
            orchestration,
            connection.set_env.clone(),
            use_pty,
        ))
    }
}
