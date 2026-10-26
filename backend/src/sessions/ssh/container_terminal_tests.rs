//! PTY driver tests: a fake channel proves the select-loop forwards output,
//! input, resize and cancellation without touching russh.

use super::container_terminal::{drive_pty_channel, PtyChannel, PtyMessage};
use super::is_transient_channel_open_failure;
use tokio::sync::{mpsc, watch};

#[test]
fn transient_channel_open_failures_are_recognized() {
    use russh::{ChannelOpenFailure, Error};
    assert!(is_transient_channel_open_failure(&Error::ChannelOpenFailure(
        ChannelOpenFailure::AdministrativelyProhibited
    )));
    assert!(is_transient_channel_open_failure(&Error::ChannelOpenFailure(
        ChannelOpenFailure::ResourceShortage
    )));
    assert!(!is_transient_channel_open_failure(&Error::ChannelOpenFailure(
        ChannelOpenFailure::UnknownChannelType
    )));
    assert!(!is_transient_channel_open_failure(&Error::Disconnect));
}

#[derive(Default)]
struct FakeChannel {
    sent_data: Vec<Vec<u8>>,
    resizes: Vec<(u32, u32)>,
    closed: bool,
    incoming: Vec<PtyMessage>,
}

impl PtyChannel for FakeChannel {
    async fn send_data(&mut self, data: &[u8]) -> Result<(), String> {
        self.sent_data.push(data.to_vec());
        Ok(())
    }

    async fn resize(&mut self, cols: u32, rows: u32) -> Result<(), String> {
        self.resizes.push((cols, rows));
        Ok(())
    }

    async fn close(&mut self) -> Result<(), String> {
        self.closed = true;
        Ok(())
    }

    async fn next(&mut self) -> Option<PtyMessage> {
        if self.incoming.is_empty() {
            std::future::pending::<()>().await;
        }
        Some(self.incoming.remove(0))
    }
}

#[tokio::test]
async fn drive_forwards_output_input_resize_and_exit() {
    let (sink, mut chunks) = mpsc::channel(8);
    let (input_tx, mut input) = mpsc::channel(8);
    let (resize_tx, mut resize) = mpsc::channel(8);
    let (_cancel_tx, mut cancel) = watch::channel(false);
    let mut channel = FakeChannel {
        incoming: vec![
            PtyMessage::Data(b"hello".to_vec()),
            PtyMessage::Data(b" world".to_vec()),
            PtyMessage::ExitStatus(7),
            PtyMessage::Closed,
        ],
        ..Default::default()
    };

    input_tx.send(b"ls\r".to_vec()).await.expect("input");
    resize_tx.send((120, 40)).await.expect("resize");

    let code = drive_pty_channel(&mut channel, &sink, &mut input, &mut resize, &mut cancel)
        .await
        .expect("driver");

    assert_eq!(code, Some(7));
    assert_eq!(channel.sent_data, vec![b"ls\r".to_vec()]);
    assert_eq!(channel.resizes, vec![(120, 40)]);
    assert_eq!(chunks.recv().await.as_deref(), Some(b"hello".as_slice()));
    assert_eq!(chunks.recv().await.as_deref(), Some(b" world".as_slice()));
}

#[tokio::test]
async fn peer_close_without_exit_status_is_a_transport_error() {
    let (sink, _chunks) = mpsc::channel(8);
    let (_input_tx, mut input) = mpsc::channel(8);
    let (_resize_tx, mut resize) = mpsc::channel(8);
    let (_cancel_tx, mut cancel) = watch::channel(false);
    let mut channel = FakeChannel {
        incoming: vec![PtyMessage::Closed],
        ..Default::default()
    };

    let error = drive_pty_channel(&mut channel, &sink, &mut input, &mut resize, &mut cancel)
        .await
        .expect_err("a close without exit-status is transport loss");

    assert_eq!(error, "SSH session closed");
    assert!(channel.closed);
}

#[tokio::test]
async fn cancel_closes_the_channel() {
    let (sink, _chunks) = mpsc::channel(8);
    let (_input_tx, mut input) = mpsc::channel(8);
    let (_resize_tx, mut resize) = mpsc::channel(8);
    let (cancel_tx, mut cancel) = watch::channel(false);
    let mut channel = FakeChannel::default();

    let run = drive_pty_channel(&mut channel, &sink, &mut input, &mut resize, &mut cancel);
    let cancel_task = async move {
        cancel_tx.send(true).expect("cancel");
    };
    let (code, ()) = tokio::join!(run, cancel_task);

    assert_eq!(code.expect("cancelled").map(|_| ()), None);
    assert!(channel.closed);
}
