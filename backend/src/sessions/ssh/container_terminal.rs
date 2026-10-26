//! Interactive PTY channel for container terminals.
//!
//! One channel per container shell, opened on the session's already
//! authenticated transport. Output is forwarded as raw bytes; input and
//! resize arrive on bounded queues; cancellation closes the channel.

use super::*;

use tokio::sync::watch;

/// `connection::open::pty`'s alias is only `pub(super)` to its own module, so
/// the driver names russh's concrete channel type directly.
type TerminalChannel = russh::Channel<client::Msg>;

/// One message the driver cares about. russh's full message set is mapped in
/// [`ChannelPty`] so this module stays testable without a network peer.
pub(crate) enum PtyMessage {
    Data(Vec<u8>),
    ExitStatus(i32),
    Closed,
}

pub(crate) trait PtyChannel {
    async fn send_data(&mut self, data: &[u8]) -> Result<(), String>;
    async fn resize(&mut self, cols: u32, rows: u32) -> Result<(), String>;
    async fn close(&mut self) -> Result<(), String>;
    async fn next(&mut self) -> Option<PtyMessage>;
}

/// Multiplexes one PTY channel until it closes or is cancelled. Returns the
/// remote exit status when the peer reported one.
pub(crate) async fn drive_pty_channel<C: PtyChannel>(
    channel: &mut C,
    sink: &mpsc::Sender<Vec<u8>>,
    input: &mut mpsc::Receiver<Vec<u8>>,
    resize: &mut mpsc::Receiver<(u32, u32)>,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Option<i32>, String> {
    let mut exit_code = None;
    loop {
        if *cancel.borrow() {
            let _ = channel.close().await;
            return Ok(exit_code);
        }
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    let _ = channel.close().await;
                    return Ok(exit_code);
                }
            }
            data = input.recv() => match data {
                Some(bytes) => channel.send_data(&bytes).await?,
                None => {
                    let _ = channel.close().await;
                    return Ok(exit_code);
                }
            },
            size = resize.recv() => if let Some((cols, rows)) = size {
                channel.resize(cols, rows).await?;
            },
            message = channel.next() => match message {
                Some(PtyMessage::Data(bytes)) => {
                    if sink.send(bytes).await.is_err() {
                        let _ = channel.close().await;
                        return Ok(exit_code);
                    }
                }
                Some(PtyMessage::ExitStatus(code)) => exit_code = Some(code),
                Some(PtyMessage::Closed) | None => {
                    let _ = channel.close().await;
                    return Ok(exit_code);
                }
            },
        }
    }
}

/// russh-backed channel adapter.
struct ChannelPty {
    channel: TerminalChannel,
}

impl PtyChannel for ChannelPty {
    async fn send_data(&mut self, data: &[u8]) -> Result<(), String> {
        // russh 0.62 reads from an `AsyncRead` (`data`) or takes owned bytes
        // (`data_bytes`); a borrowed slice is not accepted by either.
        self.channel
            .data_bytes(data.to_vec())
            .await
            .map_err(|error| format!("Failed to write to the container terminal: {error}"))
    }

    async fn resize(&mut self, cols: u32, rows: u32) -> Result<(), String> {
        self.channel
            .window_change(cols.max(1), rows.max(1), 0, 0)
            .await
            .map_err(|error| format!("Failed to resize the container terminal: {error}"))
    }

    async fn close(&mut self) -> Result<(), String> {
        self.channel
            .close()
            .await
            .map_err(|error| format!("Failed to close the container terminal: {error}"))
    }

    async fn next(&mut self) -> Option<PtyMessage> {
        // Channel bookkeeping (window adjustments, success replies, inbound
        // requests) carries no terminal payload; skip it instead of surfacing
        // empty writes.
        loop {
            match self.channel.wait().await? {
                ChannelMsg::Data { data } => return Some(PtyMessage::Data(data.to_vec())),
                ChannelMsg::ExtendedData { data, .. } => {
                    return Some(PtyMessage::Data(data.to_vec()));
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    return Some(PtyMessage::ExitStatus(exit_status as i32));
                }
                ChannelMsg::Eof | ChannelMsg::Close | ChannelMsg::ExitSignal { .. } => {
                    return Some(PtyMessage::Closed);
                }
                _ => continue,
            }
        }
    }
}

impl SshRuntime {
    /// Opens a PTY channel and runs `command` in it (an `exec` request after
    /// `pty-req`, exactly like `ssh -t host <command>`).
    async fn open_container_channel(
        &self,
        session_id: &str,
        command: &str,
        cols: u32,
        rows: u32,
    ) -> Result<TerminalChannel, String> {
        let entry = self.session(session_id).await?;
        let channel = entry
            .handle
            .channel_open_session()
            .await
            .map_err(|error| format!("Failed to open the container terminal channel: {error}"))?;
        channel
            .request_pty(true, "xterm-256color", cols.max(1), rows.max(1), 0, 0, &[])
            .await
            .map_err(|error| format!("The host refused the terminal PTY request: {error}"))?;
        channel
            .exec(true, command.as_bytes())
            .await
            .map_err(|error| format!("Failed to start the container shell: {error}"))?;
        Ok(channel)
    }

    /// Runs one container shell: opens the channel, then drives it until the
    /// shell exits, the peer closes, or the caller cancels.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_container_terminal(
        &self,
        session_id: &str,
        command: &str,
        cols: u32,
        rows: u32,
        sink: mpsc::Sender<Vec<u8>>,
        mut input: mpsc::Receiver<Vec<u8>>,
        mut resize: mpsc::Receiver<(u32, u32)>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<Option<i32>, String> {
        let channel = tokio::time::timeout(
            Duration::from_secs(30),
            self.open_container_channel(session_id, command, cols, rows),
        )
        .await
        .map_err(|_| "Timed out opening the container terminal".to_string())??;
        let mut channel = ChannelPty { channel };
        drive_pty_channel(&mut channel, &sink, &mut input, &mut resize, &mut cancel).await
    }
}
