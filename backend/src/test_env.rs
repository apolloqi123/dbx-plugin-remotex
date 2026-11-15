//! Cross-module test coordination for process-wide environment state.
//!
//! Cargo runs every test in one binary on parallel threads, so `std::env` is
//! shared. Tests that redirect `HOME` to a temporary directory hold
//! [`lock_env`], and tests that spawn a child through `portable-pty` — which
//! uses the process `HOME` as the child's working directory — hold it too, so
//! a spawn can never observe a `HOME` whose directory was already deleted.

use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Serialises tests that read or write process-wide environment state.
pub(crate) fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
