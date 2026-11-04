//! Live log-stream registry: the bounded active set, per-scope/per-session
//! cancellation, and the closed-scope tombstones. Kept apart from the
//! forwarder (`stream.rs`) so both files stay inside the source-size budget;
//! `DockerStreams` is re-exported by `stream`.
use super::*;

use std::collections::HashMap;
use std::sync::MutexGuard;

use uuid::Uuid;

/// One registered stream: the cancellation switch the runner watches, the
/// start instant the TTL is measured from, and the signal the forwarder
/// raises once the slot is released.
pub(crate) struct StreamEntry {
    pub(super) cancel: watch::Sender<bool>,
    pub(super) started: Instant,
    pub(super) done: Arc<Notify>,
    /// Log-pane scope that owns this stream (the frontend instance id). A new
    /// start on the same scope cancels the previous stream, so switching
    /// containers cannot leave the old follower running even if the UI's stop
    /// RPC was lost.
    pub(super) owner: String,
    /// SSH session the stream runs on (empty for local-daemon streams). The
    /// session close path cancels by this key so a workbench teardown cannot
    /// leave a follower behind.
    pub(super) session: String,
}

/// Registry of live log streams keyed by the `streamId` handed to the host.
/// Entries are inserted before their tasks start (so a stop that races the
/// spawn still cancels) and removed by the forwarder when the stream ends.
pub struct DockerStreams {
    inner: Mutex<HashMap<String, StreamEntry>>,
    /// Owner scopes whose pane is gone. A start that raced the close must not
    /// resurrect a follower, even though nobody is left to send its stop.
    closed_owners: Mutex<HashMap<String, Instant>>,
}

impl DockerStreams {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            closed_owners: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, HashMap<String, StreamEntry>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn closed_lock(&self) -> MutexGuard<'_, HashMap<String, Instant>> {
        self.closed_owners
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Reserves one slot and inserts the entry. The caller owns the returned
    /// id for both the response and the chunk/end events.
    ///
    /// Everything that decides admission happens under the streams lock:
    /// tombstone check, cap check, and superseding an older stream of the same
    /// owner. Two racing starts on one owner therefore cannot both register
    /// (the pre-start `cancel_owner` is only a fast path). Cancels are sent
    /// after the lock is released.
    pub(super) fn register(
        &self,
        cancel: watch::Sender<bool>,
        owner: String,
        session: String,
    ) -> Result<(String, Arc<Notify>), String> {
        let mut inner = self.lock();
        // Authoritative tombstone check, atomic with the insert (streams
        // lock → closed lock, the same order `close_owner` uses). The
        // pre-checks in the start paths are advisory only.
        {
            let mut closed = self.closed_lock();
            let now = Instant::now();
            closed.retain(|_, at| now.duration_since(*at) < CLOSED_OWNER_TTL);
            if closed.contains_key(&owner) {
                return Err(CLOSED_OWNER_ERROR.to_string());
            }
        }
        // Replacing the owner's own stream does not consume another slot.
        // An empty owner (legacy tests / callers that skipped the fallback)
        // never matches other entries: it must not supersede unrelated streams.
        let replaces = !owner.is_empty() && inner.values().any(|entry| entry.owner == owner);
        if !replaces && inner.len() >= MAX_ACTIVE_STREAMS {
            return Err(format!(
                "Too many active log streams ({MAX_ACTIVE_STREAMS}). Stop one before starting another."
            ));
        }
        let mut superseded = Vec::new();
        if !owner.is_empty() {
            inner.retain(|_, entry| {
                if entry.owner == owner {
                    superseded.push(entry.cancel.clone());
                    false
                } else {
                    true
                }
            });
        }
        let stream_id = Uuid::new_v4().to_string();
        let done = Arc::new(Notify::new());
        inner.insert(
            stream_id.clone(),
            StreamEntry {
                cancel,
                started: Instant::now(),
                done: Arc::clone(&done),
                owner,
                session,
            },
        );
        drop(inner);
        for cancel in superseded {
            let _ = cancel.send(true);
        }
        Ok((stream_id, done))
    }

    /// Cancels every stream owned by `owner` (one log pane keeps at most one
    /// follower). The forwarders release their channels through the normal
    /// cancel path; a missing stop RPC cannot leak the old stream.
    pub(super) fn cancel_owner(&self, owner: &str) {
        if owner.is_empty() {
            return;
        }
        // Free the owner's slots immediately: a lost stop RPC must not make
        // the next start fail the global active-stream cap while the old
        // forwarders are still winding down.
        self.cancel_where(|entry| entry.owner == owner);
    }

    /// Cancels every stream running on one SSH session (workbench/session
    /// teardown). A follower whose pane vanished without a stop RPC would
    /// otherwise keep its remote `docker logs -f` alive until the TTL.
    pub(crate) fn cancel_session(&self, session: &str) {
        if session.is_empty() {
            return;
        }
        self.cancel_where(|entry| entry.session == session);
    }

    /// Releases an owner scope for good: cancels its streams and remembers the
    /// scope briefly so a start RPC still on the wire cannot register a
    /// follower after its pane (and its stop hook) is gone.
    ///
    /// The tombstone and the removal happen under the streams lock (same
    /// order as [`Self::register`]: streams → closed), so a concurrent start
    /// either registers before the close (and is removed here) or observes
    /// the tombstone; there is no window where both miss each other.
    pub(crate) fn close_owner(&self, owner: &str) {
        if owner.is_empty() {
            return;
        }
        let cancels: Vec<watch::Sender<bool>> = {
            let mut inner = self.lock();
            {
                let mut closed = self.closed_lock();
                let now = Instant::now();
                closed.retain(|_, at| now.duration_since(*at) < CLOSED_OWNER_TTL);
                closed.insert(owner.to_string(), now);
            }
            let mut cancels = Vec::new();
            inner.retain(|_, entry| {
                if entry.owner == owner {
                    cancels.push(entry.cancel.clone());
                    false
                } else {
                    true
                }
            });
            cancels
        };
        for cancel in cancels {
            let _ = cancel.send(true);
        }
    }

    pub(super) fn is_owner_closed(&self, owner: &str) -> bool {
        let mut closed = self.closed_lock();
        let now = Instant::now();
        closed.retain(|_, at| now.duration_since(*at) < CLOSED_OWNER_TTL);
        closed.contains_key(owner)
    }

    /// Retires matching entries and signals their runners. Slots are freed
    /// immediately so a replacement start cannot hit the registry cap while
    /// the old runner winds down.
    fn cancel_where<F: Fn(&StreamEntry) -> bool>(&self, matches: F) {
        let cancels: Vec<watch::Sender<bool>> = {
            let mut inner = self.lock();
            let mut cancels = Vec::new();
            inner.retain(|_, entry| {
                if matches(entry) {
                    cancels.push(entry.cancel.clone());
                    false
                } else {
                    true
                }
            });
            cancels
        };
        for cancel in cancels {
            let _ = cancel.send(true);
        }
    }

    pub(super) fn started_at(&self, stream_id: &str) -> Instant {
        self.lock()
            .get(stream_id)
            .map(|entry| entry.started)
            .unwrap_or_else(Instant::now)
    }

    /// Cancels one stream and waits (briefly) for its forwarder to release
    /// the slot. Serialising stop→start keeps the next `logs -f` channel from
    /// opening while the cancelled runner is still closing its own: a switch
    /// otherwise piles two sessions onto the server's `MaxSessions` budget.
    /// Returns false for unknown ids (already finished or never started), so
    /// the RPC can answer `stopped: false` without treating it as an error.
    pub async fn stop_and_wait(&self, stream_id: &str) -> bool {
        let (cancel, done) = {
            let inner = self.lock();
            match inner.get(stream_id) {
                Some(entry) => (entry.cancel.clone(), Arc::clone(&entry.done)),
                None => return false,
            }
        };
        let notified = done.notified();
        let _ = cancel.send(true);
        let _ = tokio::time::timeout(STOP_WAIT, notified).await;
        true
    }

    /// Cancels and drops every registered stream. This is the teardown path
    /// (registry `Drop`): the runners wind down through their own cancel
    /// paths, so remote processes are reaped instead of abandoned.
    pub fn stop_all(&self) {
        let entries: Vec<StreamEntry> = {
            let mut inner = self.lock();
            inner.drain().map(|(_, entry)| entry).collect()
        };
        for entry in entries {
            let _ = entry.cancel.send(true);
        }
    }

    /// Unregisters a finished stream (the forwarder's end path).
    pub(super) fn remove(&self, stream_id: &str) {
        self.lock().remove(stream_id);
    }
}

impl Default for DockerStreams {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DockerStreams {
    /// Cancels leftovers when the registry itself is dropped, so a torn-down
    /// plugin never leaves `docker logs -f` children or remote channels
    /// running behind a dead host bridge.
    fn drop(&mut self) {
        self.stop_all();
    }
}
