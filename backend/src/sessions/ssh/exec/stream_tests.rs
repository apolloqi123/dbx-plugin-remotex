//! Tests for the streaming exec sink: retained-tail capping and the
//! send-or-cancel helper that forwards fragments to a bounded channel.

use super::*;

#[test]
fn append_tail_capped_keeps_only_the_latest_bytes() {
    let mut buffer = Vec::new();
    append_tail_capped(&mut buffer, b"hello");
    assert_eq!(buffer, b"hello");

    // Rolling window: appending past the cap drops the oldest bytes only.
    let mut buffer = vec![b'p'; MAX_STREAM_TAIL_BYTES - 1];
    append_tail_capped(&mut buffer, b"q");
    assert_eq!(buffer.len(), MAX_STREAM_TAIL_BYTES);
    assert_eq!(buffer[0], b'p');
    assert_eq!(*buffer.last().unwrap(), b'q');
    append_tail_capped(&mut buffer, b"r");
    assert_eq!(buffer.len(), MAX_STREAM_TAIL_BYTES);
    assert_eq!(buffer[0], b'p');
    assert_eq!(*buffer.last().unwrap(), b'r');

    // An oversized fragment replaces the buffer with its own tail.
    let mut oversized = vec![b'y'; MAX_STREAM_TAIL_BYTES + 8];
    oversized[0] = b'z';
    append_tail_capped(&mut buffer, &oversized);
    assert_eq!(buffer.len(), MAX_STREAM_TAIL_BYTES);
    assert!(buffer.iter().all(|byte| *byte == b'y'));
}

#[test]
fn streaming_tail_cap_is_explicit() {
    // 32 KiB per stream kept for the error report means 64 KiB per stream
    // (stdout + stderr), i.e. ~256 KiB across four concurrent log follows.
    assert_eq!(MAX_STREAM_TAIL_BYTES, 32 * 1024);
    assert_eq!(2 * MAX_STREAM_TAIL_BYTES, 64 * 1024);
}

#[tokio::test]
async fn send_or_cancel_forwards_fragments() {
    let (sink, mut receiver) = tokio::sync::mpsc::channel(2);
    let (_cancel_tx, mut cancel) = tokio::sync::watch::channel(false);
    send_or_cancel(&sink, b"chunk", &mut cancel).await.unwrap();
    drop(sink);
    assert_eq!(receiver.recv().await.unwrap(), b"chunk");
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn send_or_cancel_reports_closed_receiver() {
    let (sink, receiver) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
    drop(receiver);
    let (_cancel_tx, mut cancel) = tokio::sync::watch::channel(false);
    let error = send_or_cancel(&sink, b"x", &mut cancel).await.unwrap_err();
    assert_eq!(error, STREAM_SINK_CLOSED_MESSAGE);
}

#[tokio::test]
async fn cancelled_send_does_not_wait_for_capacity() {
    let (sink, _receiver) = tokio::sync::mpsc::channel(1);
    // Fill the only slot so the next send has to wait on back-pressure.
    sink.send(vec![b'a']).await.unwrap();
    let (cancel_tx, mut cancel) = tokio::sync::watch::channel(false);
    let send_task =
        tokio::spawn(async move { send_or_cancel(&sink, b"blocked", &mut cancel).await });
    tokio::task::yield_now().await;
    cancel_tx.send(true).unwrap();
    let error = send_task.await.unwrap().unwrap_err();
    assert_eq!(error, EXEC_CANCELLED_MESSAGE);
}

#[test]
fn stream_aborts_settle_before_the_kill_backstop() {
    // The POSIX logs supervisor cleans up its watchers in a HUP/TERM trap;
    // KILL cannot be caught, so a back-to-back KILL can orphan them. Pin the
    // scheduling window (see `abort_stream_channel`).
    assert_eq!(
        super::run::STREAM_SIGNAL_SETTLE,
        Duration::from_millis(100)
    );
}
