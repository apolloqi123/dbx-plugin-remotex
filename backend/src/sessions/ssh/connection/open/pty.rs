//! PTY and channel setup for one interactive SSH session.
//!
//! The channel is primed in ssh(1) order — PTY first, env next, shell/exec
//! last — and handed to the read loop only once the remote side accepted it.

use super::*;

use russh::Channel;

use super::error::{channel_open_error, pty_error, remote_command_start_error, shell_start_error};

/// Fully primed interactive channel, ready for the read loop to own.
pub(super) type TerminalChannel = Channel<client::Msg>;

impl SshRuntime {
    /// Opens the session channel, requests the PTY, applies the connection's
    /// environment and starts the shell (or the exec payload).
    pub(super) async fn open_terminal_channel(
        &self,
        handle: &Arc<Handle<SshClient>>,
        connection: &StoredConnection,
        reused_transport: bool,
        cols: u32,
        rows: u32,
        spawn_exec: Option<&str>,
    ) -> Result<(RemoteShell, bool, TerminalChannel), String> {
        let remote_shell = RemoteShell::Unknown;
        let directory_tracking_supported = true;
        let mut channel = open_session_channel_labeled(handle, "ssh/pty")
            .await
            .map_err(|error| channel_open_error(reused_transport, &error))?;
        // From here on the channel is open on the server: every failure must
        // close it explicitly (a dropped russh Channel never sends CLOSE and
        // would hold a MaxSessions slot until the connection ends).
        if let Err(error) = channel
            .request_pty(true, "xterm-256color", cols.max(1), rows.max(1), 0, 0, &[])
            .await
        {
            exec::close_setup_channel(&mut channel).await;
            return Err(pty_error(error));
        }
        // Client-specified SetEnv rides on the interactive session too,
        // in ssh(1) order: PTY first, env next, shell/exec last.
        if let Err(error) = exec::apply_connection_env(&mut channel, &connection.set_env).await {
            exec::close_setup_channel(&mut channel).await;
            return Err(error);
        }
        // X11 forwarding (OpenSSH -X parity): gated on the connection-scoped
        // preference; a refused request only disables X11, never the shell.
        if crate::features::forwarding::x11::enabled_from(&self.data_dir) {
            match crate::features::forwarding::x11::arm_session() {
                Ok(cookie_hex) => {
                    if let Err(error) = channel
                        .request_x11(
                            true,
                            false,
                            crate::features::forwarding::x11::X11_AUTH_PROTOCOL,
                            cookie_hex,
                            0,
                        )
                        .await
                    {
                        eprintln!("[x11] x11-req refused by server: {error}");
                    }
                }
                Err(error) => eprintln!("[x11] cannot arm the forwarding gate: {error}"),
            }
        }
        let started = if let Some(command) = spawn_exec {
            // WT-4 (command session): exec `sh -c '<escaped command>'`
            // instead of a shell — see docs/architecture/PROTOCOL.zh-CN.md「同
            // transport 命令会话」. Explicit per-open request wins over
            // the connection-level remote_command.
            channel
                .exec(true, command.as_bytes())
                .await
                .map_err(remote_command_start_error)
        } else if connection.remote_command.is_empty() {
            channel.request_shell(true).await.map_err(shell_start_error)
        } else {
            channel
                .exec(true, connection.remote_command.as_bytes())
                .await
                .map_err(remote_command_start_error)
        };
        if let Err(error) = started {
            exec::close_setup_channel(&mut channel).await;
            return Err(error);
        }
        Ok((remote_shell, directory_tracking_supported, channel))
    }
}
