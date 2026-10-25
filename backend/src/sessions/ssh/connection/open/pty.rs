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
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|error| channel_open_error(reused_transport, error))?;
        channel
            .request_pty(true, "xterm-256color", cols.max(1), rows.max(1), 0, 0, &[])
            .await
            .map_err(pty_error)?;
        // Client-specified SetEnv rides on the interactive session too,
        // in ssh(1) order: PTY first, env next, shell/exec last.
        exec::apply_connection_env(&mut channel, &connection.set_env).await?;
        // X11 forwarding (OpenSSH -X parity): gated on the connection-scoped
        // preference; a refused request only disables X11, never the shell.
        if crate::features::forwarding::x11::enabled_from(&self.data_dir) {
            match crate::features::forwarding::x11::arm_session() {
                Ok(cookie_hex) => {
                    if let Err(error) = channel
                        .request_x11(true, false, crate::features::forwarding::x11::X11_AUTH_PROTOCOL, cookie_hex, 0)
                        .await
                    {
                        eprintln!("[x11] x11-req refused by server: {error}");
                    }
                }
                Err(error) => eprintln!("[x11] cannot arm the forwarding gate: {error}"),
            }
        }
        if let Some(command) = spawn_exec {
            // WT-4 (command session): exec `sh -c '<escaped command>'`
            // instead of a shell — see docs/architecture/PROTOCOL.zh-CN.md「同
            // transport 命令会话」. Explicit per-open request wins over
            // the connection-level remote_command.
            channel
                .exec(true, command.as_bytes())
                .await
                .map_err(remote_command_start_error)?;
        } else if connection.remote_command.is_empty() {
            channel
                .request_shell(true)
                .await
                .map_err(shell_start_error)?;
        } else {
            channel
                .exec(true, connection.remote_command.as_bytes())
                .await
                .map_err(remote_command_start_error)?;
        }
        Ok((remote_shell, directory_tracking_supported, channel))
    }
}
