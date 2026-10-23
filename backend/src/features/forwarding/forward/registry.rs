//! Metrics support.

use super::*;

pub(crate) fn bound_display(entry: &ForwardEntry) -> u16 {
    let bound = entry.bound_port.load(Ordering::Relaxed);
    if bound == 0 {
        entry.listen_port
    } else {
        u16::try_from(bound).unwrap_or(entry.listen_port)
    }
}

/// Transitions the mapping state and notifies workbenches listening on the
/// `ssh/forward/state` event (skipped when the entry carries no emitter).
/// Emission failures are ignored by design: the registry stays authoritative
/// and the next `ssh/forward/list` corrects the UI.
pub(crate) fn set_state(entry: &ForwardEntry, state: ForwardState, error: Option<String>) {
    {
        let mut guard = entry
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        // A stopped mapping must never come back to active through a stray
        // relay task finishing after stop.
        if *guard == ForwardState::Stopped && state != ForwardState::Stopped {
            return;
        }
        *guard = state.clone();
    }
    *entry
        .error
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = error.clone();
    if let Some(emitter) = entry.emitter.as_ref() {
        let _ = emitter.event(
            "ssh/forward/state",
            json!({
                "id": entry.id,
                "sessionId": entry.session_id,
                "connectionId": entry.connection_id,
                "state": state.as_str(),
                "error": error,
            }),
        );
    }
}

/// Stops the background tasks of one mapping (accept loop + live relays).
/// Called by `ssh/forward/stop` and by session teardown; the remote listener
/// cancellation (`cancel-tcpip-forward`) stays with the caller because it
/// needs the SSH handle and must not block teardown on the network.
pub(crate) fn abort_tasks(entry: &ForwardEntry) {
    entry.stopping.store(true, Ordering::Relaxed);
    if let Some(task) = entry
        .listener_task
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
    {
        task.abort();
    }
    let mut relays = entry
        .relays
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    for handle in relays.drain(..) {
        handle.abort();
    }
}

/// Registry of live mappings, one per sidecar process (not per session) so
/// `ssh/forward/list` can answer for a whole connection or a single session.
#[derive(Default)]
pub(crate) struct ForwardRegistry {
    entries: Mutex<HashMap<String, Arc<ForwardEntry>>>,
}

impl ForwardRegistry {
    pub(crate) fn insert(&self, entry: Arc<ForwardEntry>) {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(entry.id.clone(), entry);
    }

    pub(crate) fn get(&self, id: &str) -> Option<Arc<ForwardEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(id)
            .cloned()
    }

    pub(crate) fn remove(&self, id: &str) -> Option<Arc<ForwardEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(id)
    }

    pub(crate) fn rows(&self) -> Vec<Arc<ForwardEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .values()
            .cloned()
            .collect()
    }
}
