//! Cross-module test coordination for process-wide state.
//!
//! Cargo runs every test in one binary on parallel threads, so process-wide
//! state is shared:
//!
//! - Tests that redirect `HOME` to a temporary directory hold [`lock_env`],
//!   and tests that spawn a child through `portable-pty` — which uses the
//!   process `HOME` as the child's working directory — hold it too, so a spawn
//!   can never observe a `HOME` whose directory was already deleted.
//! - Mock servers bind through [`bind_unique_loopback`] so an ephemeral port is
//!   never handed out twice in one test run. OTP replay/rotation ledgers are
//!   process-global and scoped by `user@host:port`, so a recycled port would
//!   make a later test inherit an earlier test's committed code.

use std::io;
use std::sync::{Mutex, MutexGuard};
use tokio::net::TcpListener;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Serialises tests that read or write process-wide environment state.
pub(crate) fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Binds an ephemeral loopback port that no earlier test in this process has
/// used.
pub(crate) async fn bind_unique_loopback() -> io::Result<TcpListener> {
    static USED_PORTS: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    loop {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let mut used = USED_PORTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !used.contains(&port) {
            used.push(port);
            return Ok(listener);
        }
    }
}
