//! Error mapping for opening an interactive SSH session.
//!
//! Every message keeps the wording the caller surfaces to the workbench; the
//! helpers exist so the channel/PTY setup can fail at each step without
//! repeating the format strings.

use std::fmt::Display;

use russh::Error as RusshError;

/// Channel-open failure.
///
/// A transient refusal (`MaxSessions` exhaustion) is *not* a dead connection:
/// it means other sessions of this transport still hold their slots. It gets
/// its own retryable wording so the workbench does not permanently mark the
/// connection unusable (the frontend classifies "can no longer be reused" as a
/// non-retryable duplicate-transport error). Only a non-transient failure on a
/// reused transport keeps the old "use New session" guidance.
pub(super) fn channel_open_error(reused_transport: bool, error: &RusshError) -> String {
    if crate::sessions::ssh::is_transient_channel_open_failure(error) {
        return format!(
            "The SSH server's session limit (MaxSessions) was reached; close another terminal or pane and retry: {error}"
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use russh::ChannelOpenFailure;

    fn refusal(reason: ChannelOpenFailure) -> RusshError {
        RusshError::ChannelOpenFailure(reason)
    }

    #[test]
    fn transient_refusal_is_retryable_not_a_dead_transport() {
        let message = channel_open_error(
            true,
            &refusal(ChannelOpenFailure::AdministrativelyProhibited),
        );
        assert!(message.contains("MaxSessions"), "{message}");
        assert!(!message.contains("can no longer be reused"), "{message}");
    }

    #[test]
    fn non_transient_failure_on_reused_transport_keeps_reconnect_guidance() {
        let message = channel_open_error(true, &RusshError::Disconnect);
        assert!(message.contains("can no longer be reused"), "{message}");
    }

    #[test]
    fn fresh_transport_failure_names_the_terminal_channel() {
        let message = channel_open_error(false, &RusshError::Disconnect);
        assert!(message.contains("terminal channel"), "{message}");
    }
}
