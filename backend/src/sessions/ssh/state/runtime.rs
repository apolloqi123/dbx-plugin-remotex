//! Transport lease and task-transport lifecycle helpers for the SSH runtime.
//! These types own their atomics/handles directly and never take the runtime's
//! `Mutex`/`AsyncRwLock` guards, so moving them cannot change lock scope.

use super::*;

/// Explicit users of one authenticated SSH transport: every registered
/// session owns one use, and a copied session reserves one before its first
/// await. That pending reservation closes the open/close race where the last
/// old session could otherwise tear down a jump chain while the new PTY was
/// still being created.
pub(crate) struct TransportLeaseCounter(AtomicUsize);

/// `#[allow(deprecated)]`: the newest stable renamed `fetch_update` to
/// `try_update` (cosmetic; semantics unchanged). Calling the new name would
/// hard-break every older rustc (including the 1.88 floor), so keep the old
/// name until the toolchain floor moves past the renaming release.
#[allow(deprecated)]
impl TransportLeaseCounter {
    pub(crate) fn new() -> Self {
        Self(AtomicUsize::new(1))
    }

    pub(crate) fn retain(&self) {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .expect("SSH transport lease count overflow");
    }

    /// Returns true when the released use was the final one.
    pub(crate) fn release(&self) -> bool {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_sub(1)
            })
            .expect("SSH transport lease released more than once")
            == 1
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

pub(crate) struct SharedTransportLease {
    pub(crate) uses: TransportLeaseCounter,
    pub(crate) jump_chain: Vec<Arc<Handle<SshClient>>>,
}

impl SharedTransportLease {
    pub(crate) fn new(jump_chain: Vec<Arc<Handle<SshClient>>>) -> Self {
        Self {
            uses: TransportLeaseCounter::new(),
            jump_chain,
        }
    }

    pub(crate) fn retain(&self) {
        self.uses.retain();
    }

    pub(crate) async fn release(&self) {
        if !self.uses.release() {
            return;
        }
        for jump in &self.jump_chain {
            let _ = jump
                .disconnect(
                    Disconnect::ByApplication,
                    "DBX SSH session closed",
                    "English",
                )
                .await;
        }
    }
}

/// Execution transport for the scheduler task provider: either the live
/// session's shared authenticated handle (borrowed lifetime of the session)
/// or a dedicated headless dial (owned; disconnected on release).
pub(crate) enum TaskTransport {
    Session {
        handle: Arc<Handle<SshClient>>,
    },
    Dedicated {
        handle: Handle<SshClient>,
        jumps: Vec<Handle<SshClient>>,
    },
}

impl TaskTransport {
    pub(crate) fn handle(&self) -> &Handle<SshClient> {
        match self {
            Self::Session { handle } => handle,
            Self::Dedicated { handle, .. } => handle,
        }
    }

    /// Tears down a dedicated dial (jump chain included). Session-borrowed
    /// transports are left alone — the workbench owns their lifecycle.
    pub(crate) async fn release(self) {
        if let Self::Dedicated { handle, jumps } = self {
            let _ = handle
                .disconnect(
                    Disconnect::ByApplication,
                    "DBX scheduled task finished",
                    "English",
                )
                .await;
            for jump in jumps {
                let _ = jump
                    .disconnect(
                        Disconnect::ByApplication,
                        "DBX scheduled task jump closed",
                        "English",
                    )
                    .await;
            }
        }
    }
}
