//! Metrics support.

use super::*;

/// Collapse a burst of editor save events into one evaluation.
pub const DEBOUNCE: Duration = Duration::from_millis(500);

/// Post-registration suppression window. Deliberately ZERO: the SHA-256
/// baseline fingerprint already filters warm-up noise (a re-write of the just
/// downloaded bytes classifies as `Same` and never emits), while a fixed
/// window silently ate REAL early saves — an editor that was already running
/// (or a user saving seconds after `open`) writes inside 2s, the event was
/// dropped with no feedback and the upload never happened (真机回归). Keep
/// the tunable so tests can still exercise the suppression branch.
pub const SUPPRESS_WINDOW: Duration = Duration::ZERO;

/// Files above this size are not hashed: the fingerprint degrades to
/// size+mtime (see [`file_fingerprint`]). Hashing a multi-hundred-MB file on
/// every save would hurt more than the coarser change detection it buys.
pub const MAX_HASH_BYTES: u64 = 64 * 1024 * 1024;

/// `watch/upload` refuses files above this size: the round-trip is buffered
/// in memory on purpose (single atomic commit), so multi-GB editor saves must
/// go through the regular upload slot instead. Aligned with the manual
/// `sftp/upload-local` cap (`sftp_ext::MAX_UPLOAD_LOCAL_SIZE`) so external
/// editing supports exactly the sizes it can open — a 157MB SQL dump must
/// survive watch/start AND save-round-trip, not the former only.
pub const MAX_UPLOAD_BYTES: u64 = crate::filesystem::sftp::ext::MAX_UPLOAD_LOCAL_SIZE;

/// Async session-liveness probe injected from main.rs: the watcher cannot
/// reach into the SSH session table directly (module boundaries), so each
/// emission confirms the owning session still exists before prompting.
pub type SessionProbe =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// Sink for confirmed changes. Production forwards to the plugin host as a
/// `watch/file-modified` event; tests collect into a channel instead.
pub trait EventPublisher: Send + Sync + 'static {
    fn publish(&self, payload: Value);
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchStartRequest {
    pub session_id: String,
    pub remote_path: String,
    pub local_path: String,
}

/// Content fingerprint of one snapshot of the watched file. `len` +
/// `modified_ms` act as the cheap pre-filter; `sha256` is the authoritative
/// comparison so an editor's mtime-only touch never fires the event. `None`
/// sha256 marks an oversized snapshot (over [`MAX_HASH_BYTES`]): size+mtime
/// is then the whole fingerprint, so classification is coarser but the file
/// stays watchable — refusing it outright killed external editing of large
/// dumps (「远端文件不存在」 false alarm, 2026-10-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFingerprint {
    pub len: u64,
    pub modified_ms: u64,
    pub sha256: Option<[u8; 32]>,
}

/// Outcome of comparing a fresh snapshot against the watch baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeVerdict {
    /// Content provably identical (or mtime untouched) — no event.
    Same,
    /// Bytes really differ from the baseline — emit.
    ContentChanged,
    /// Snapshot unavailable (missing/too large to hash) — skip the emit and
    /// keep the watch; the next writable snapshot decides.
    Undeterminable,
}

/// Injection point for clocks so the debounce/suppression logic is unit
/// testable without sleeping through the production windows.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> std::time::Instant;
}

#[derive(Default)]
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
}

/// Tunables for [`WatchRuntime`]; production uses [`Tunables::production`].
#[derive(Clone)]
pub struct Tunables {
    pub debounce: Duration,
    pub suppress: Duration,
    /// `Some(true)` short-circuits the desktop probe (`local_downloads::can_save_local`)
    /// so the watcher suite runs on CI runners without desktop download dirs;
    /// `None` keeps the production env probe.
    pub desktop_gate_override: Option<bool>,
    /// Pins the downloads base for the remote-edit origin check. `None` keeps
    /// the production resolution (`local_downloads::downloads_base_dir`); the
    /// watcher suite sets it so validation never depends on the runner's real
    /// Downloads layout.
    pub remote_edit_downloads_override: Option<PathBuf>,
}

impl Tunables {
    pub fn production() -> Self {
        Self {
            debounce: DEBOUNCE,
            suppress: SUPPRESS_WINDOW,
            desktop_gate_override: None,
            remote_edit_downloads_override: None,
        }
    }
}
