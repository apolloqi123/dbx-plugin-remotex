//! Metrics support.

use super::*;

/// Address snapshot of a remote mapping's local dial target, registered for
/// the forwarded-tcpip handler (which must dial `target_host:target_port`
/// from the client machine without knowing the registry) and carrying the
/// mapping row so relays update the same counters the UI reads.
#[derive(Clone)]
pub(crate) struct RelayTarget {
    pub host: String,
    pub port: u16,
    pub entry: Arc<ForwardEntry>,
}

/// What a listener failure means for the mapping's reported state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardKind {
    Local,
    Remote,
    Dynamic,
}

impl ForwardKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            ForwardKind::Local => "local",
            ForwardKind::Remote => "remote",
            ForwardKind::Dynamic => "dynamic",
        }
    }

    /// Protocol-facing parse: only the three supported kinds are accepted so a typo
    /// fails loudly instead of silently forwarding the wrong direction.
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        match value.as_str() {
            Some("local") => Ok(ForwardKind::Local),
            Some("remote") => Ok(ForwardKind::Remote),
            Some("dynamic") => Ok(ForwardKind::Dynamic),
            other => Err(format!(
                "Invalid forward kind: {other:?}; expected \"local\", \"remote\" or \"dynamic\""
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForwardState {
    Starting,
    Active,
    Stopped,
    Error,
}

impl ForwardState {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            ForwardState::Starting => "starting",
            ForwardState::Active => "active",
            ForwardState::Stopped => "stopped",
            ForwardState::Error => "error",
        }
    }
}

/// One live mapping row, shared between the registry, the accept task and the
/// forwarded-tcpip relays so counters and state stay consistent regardless of
/// which side observes them first.
pub(crate) struct ForwardEntry {
    pub id: String,
    pub session_id: String,
    pub connection_id: String,
    pub kind: ForwardKind,
    pub listen_host: String,
    /// Port requested at start. For remote forwards this may be 0 (server
    /// picks); `bound_port` carries what the server actually bound.
    pub listen_port: u16,
    pub target_host: String,
    pub target_port: u16,
    /// An independent mapping owns an authenticated transport without a PTY.
    /// Session-backed mappings leave this empty and use their session handle.
    pub transport: Option<Arc<ForwardTransport>>,
    /// Remote forwards only: the port the server bound (0 until confirmed).
    pub bound_port: AtomicU32,
    pub state: Mutex<ForwardState>,
    pub error: Mutex<Option<String>>,
    pub connections_total: AtomicU64,
    pub connections_active: AtomicI64,
    pub bytes_up: AtomicU64,
    pub bytes_down: AtomicU64,
    /// Local forwards: the accept loop. Aborted by stop / session close;
    /// dropping the listener unbinds the port immediately.
    pub listener_task: Mutex<Option<tokio::task::AbortHandle>>,
    /// Per-connection relay tasks; aborted on stop so an existing mapping
    /// cannot keep moving bytes after the user removed it.
    pub relays: Mutex<Vec<tokio::task::AbortHandle>>,
    /// `Some` in production (cloned from the RPC that started the mapping)
    /// so state transitions reach workbenches; unit tests build entries with
    /// `None` and the SDK has no public emitter constructor.
    pub emitter: Option<PluginEmitter>,
    pub stopping: AtomicBool,
}

pub(crate) struct ForwardTransport {
    pub handle: Arc<Handle<SshClient>>,
    pub jump_chain: Vec<Arc<Handle<SshClient>>>,
}

impl ForwardEntry {
    /// Protocol-facing row for `ssh/forward/list` and `ssh/forward/start`.
    pub(crate) fn payload(&self) -> Value {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let error = self
            .error
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let bound = self.bound_port.load(Ordering::Relaxed);
        json!({
            "id": self.id,
            "sessionId": self.session_id,
            "connectionId": self.connection_id,
            "kind": self.kind.as_str(),
            "listenHost": self.listen_host,
            "listenPort": u32::from(self.listen_port),
            "boundPort": bound,
            "targetHost": self.target_host,
            "targetPort": u32::from(self.target_port),
            "state": state.as_str(),
            "error": error,
            "connectionsTotal": self.connections_total.load(Ordering::Relaxed),
            "connectionsActive": self.connections_active.load(Ordering::Relaxed).max(0),
            "bytesUp": self.bytes_up.load(Ordering::Relaxed),
            "bytesDown": self.bytes_down.load(Ordering::Relaxed),
        })
    }
}
