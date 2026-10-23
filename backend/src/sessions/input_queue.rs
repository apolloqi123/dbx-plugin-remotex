//! Bounded enqueue for terminal keyboard input.
//!
//! Every terminal transport (SSH, local PTY, Telnet) feeds keyboard input
//! through a bounded per-session queue. A full queue is a transient overload
//! signal — the PTY/TCP writer is briefly behind — not evidence that the
//! session died. Dropping one keystroke is acceptable; tearing down a healthy
//! session (the workbench treats a `*/terminal/error` event as session death)
//! is not. Callers therefore retry briefly and then drop silently (with a
//! rate-limited log); only a closed queue is surfaced as an error.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

/// Total wall-clock budget a full queue is retried before the chunk is
/// dropped. Kept short so a shared SDK binary-handler thread never parks for
/// long on one flooded session.
pub(crate) const INPUT_QUEUE_RETRY_BUDGET: Duration = Duration::from_millis(100);

const INPUT_QUEUE_RETRY_STEP: Duration = Duration::from_millis(2);
const DROP_LOG_INTERVAL_SECS: u64 = 5;

static LAST_DROP_LOG_SECS: AtomicU64 = AtomicU64::new(0);

/// Enqueues `command`, retrying a full queue for up to
/// [`INPUT_QUEUE_RETRY_BUDGET`]. A queue that stays full past the budget
/// drops the chunk (Ok: input loss is not session death); a closed queue is
/// still an error because the session is gone.
pub(crate) fn enqueue_bounded<T>(
    sender: &mpsc::Sender<T>,
    command: T,
    label: &str,
) -> Result<(), String> {
    let mut pending = command;
    let deadline = Instant::now() + INPUT_QUEUE_RETRY_BUDGET;
    loop {
        match sender.try_send(pending) {
            Ok(()) => return Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(format!("{label} input queue is closed"));
            }
            Err(mpsc::error::TrySendError::Full(returned)) => {
                let now = Instant::now();
                if now >= deadline {
                    log_drop(label);
                    return Ok(());
                }
                pending = returned;
                std::thread::sleep(INPUT_QUEUE_RETRY_STEP.min(deadline - now));
            }
        }
    }
}

/// Rate-limited (process-wide, once per [`DROP_LOG_INTERVAL_SECS`]) diagnostic
/// for a dropped keystroke. Deliberately not an event: a transient drop must
/// never reach the workbench's session-death handlers.
fn log_drop(label: &str) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let last = LAST_DROP_LOG_SECS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < DROP_LOG_INTERVAL_SECS {
        return;
    }
    if LAST_DROP_LOG_SECS
        .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    eprintln!("[dbx-plugin-sshx] {label} input queue stayed full; dropped one input chunk");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_queue_is_a_drop_not_an_error() {
        let (tx, mut rx) = mpsc::channel::<u8>(1);
        tx.try_send(1).expect("fill the queue");
        let started = Instant::now();
        assert!(
            enqueue_bounded(&tx, 2, "SSH").is_ok(),
            "a transient full queue must not surface as an error"
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed >= INPUT_QUEUE_RETRY_BUDGET,
            "the bounded retry must actually wait before dropping: {elapsed:?}"
        );
        assert!(
            elapsed < INPUT_QUEUE_RETRY_BUDGET + Duration::from_millis(250),
            "retry budget overshot: {elapsed:?}"
        );
        // The already-queued chunk is untouched; only the retried one dropped.
        assert_eq!(rx.try_recv().expect("queued chunk survives"), 1);
        assert!(rx.try_recv().is_err(), "the dropped chunk must not appear");
    }

    #[test]
    fn full_queue_retry_succeeds_when_the_consumer_drains() {
        let (tx, mut rx) = mpsc::channel::<u8>(1);
        tx.try_send(1).expect("fill the queue");
        let consumer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            (rx.blocking_recv(), rx.blocking_recv())
        });
        assert!(enqueue_bounded(&tx, 2, "Local terminal").is_ok());
        assert_eq!(consumer.join().expect("consumer"), (Some(1), Some(2)));
    }

    #[test]
    fn closed_queue_still_reports_session_death() {
        let (tx, rx) = mpsc::channel::<u8>(1);
        drop(rx);
        let error = enqueue_bounded(&tx, 1, "Telnet session").expect_err("closed queue");
        assert_eq!(error, "Telnet session input queue is closed");
    }
}
