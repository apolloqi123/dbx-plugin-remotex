//! Error mapping for opening an interactive SSH session.
//!
//! Every message keeps the wording the caller surfaces to the workbench; the
//! helpers exist so the channel/PTY setup can fail at each step without
//! repeating the format strings.

use std::fmt::Display;

/// Channel-open failure. A reused authenticated transport that can no longer
/// serve a new channel gets its own wording, because retrying the same
/// transport can never succeed.
pub(super) fn channel_open_error(reused_transport: bool, error: impl Display) -> String {
    if reused_transport {
        format!("The authenticated SSH connection can no longer be reused; use New session to reconnect: {error}")
    } else {
        format!("Failed to open terminal channel: {error}")
    }
}

/// PTY request failure (`pty-req`).
pub(super) fn pty_error(error: impl Display) -> String {
    format!("Failed to request SSH PTY: {error}")
}

/// Interactive shell start failure (`shell` request).
pub(super) fn shell_start_error(error: impl Display) -> String {
    format!("Failed to start SSH shell: {error}")
}

/// Remote-command start failure, shared by WT-4 spawn commands and the
/// connection-level `remote_command`.
pub(super) fn remote_command_start_error(error: impl Display) -> String {
    format!("Failed to start remote command: {error}")
}
