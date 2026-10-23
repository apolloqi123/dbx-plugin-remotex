//! SSH support.

use super::*;

/// How the next SSH hop is reached: a fresh TCP connection, or a
/// direct-tcpip channel tunneled through an established jump-host handle.
pub(crate) enum DialTarget {
    Tcp((String, u16)),
    JumpStream(russh::ChannelStream<russh::client::Msg>),
}

impl DialTarget {
    pub(crate) async fn through_jump(
        jump: &Handle<SshClient>,
        host: &str,
        port: u16,
    ) -> Result<Self, String> {
        let channel = jump
            .channel_open_direct_tcpip(host, u32::from(port), "127.0.0.1", 0)
            .await
            .map_err(|error| format!("Failed to open tunnel to {host}:{port}: {error}"))?;
        Ok(Self::JumpStream(channel.into_stream()))
    }
}

pub(crate) const DIRECTORY_HANDSHAKE_LIMIT: usize = 64 * 1024;

pub(crate) const DIRECTORY_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) const REMOTE_SHELL_DETECTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Error shown when a terminal-routed agent command finds no live workbench
/// PTY for its connection; the wording doubles as user guidance.
pub(crate) const NO_TERMINAL_SESSION_MESSAGE: &str =
    "No open terminal session for this connection; open the SSH workbench terminal first";

/// Shared dial deadline. While a host-key challenge waits for the user, the
/// dial timeout budget is suspended so the SSH connect future is not killed
/// underneath the confirmation dialog.
#[derive(Default)]
pub(crate) struct DialDeadline {
    pub(crate) deadline_ms: AtomicU64,
    pub(crate) challenge_pending: AtomicBool,
}

impl DialDeadline {
    pub(crate) fn start(timeout: Duration) -> Arc<Self> {
        let state = Arc::new(Self::default());
        state.deadline_ms.store(
            Self::epoch_ms() + timeout.as_millis() as u64,
            Ordering::SeqCst,
        );
        state
    }

    pub(crate) fn epoch_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_millis() as u64)
            .unwrap_or(0)
    }

    pub(crate) fn enter_challenge(&self, extra: Duration) {
        self.challenge_pending.store(true, Ordering::SeqCst);
        self.deadline_ms.store(
            Self::epoch_ms() + extra.as_millis() as u64,
            Ordering::SeqCst,
        );
    }

    pub(crate) fn exit_challenge(&self, timeout: Duration) {
        self.challenge_pending.store(false, Ordering::SeqCst);
        self.deadline_ms.store(
            Self::epoch_ms() + timeout.as_millis() as u64,
            Ordering::SeqCst,
        );
    }

    pub(crate) fn remaining(&self) -> Option<Duration> {
        let now = Self::epoch_ms();
        let deadline = self.deadline_ms.load(Ordering::SeqCst);
        deadline.checked_sub(now).map(Duration::from_millis)
    }
}
