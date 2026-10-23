//! Metrics support.

use super::*;

/// Bounded backpressure for input/clipboard/advertise commands.
pub(crate) const COMMAND_CHANNEL_CAPACITY: usize = 256;

/// 首帧前的连接进度预算：TCP 被 accept 但 TLS/CredSSP 停滞的目标会让引擎
/// 线程静默悬挂——没有失败事件、没有日志，工作台永远停在"连接中"（真机
/// wtr 实测复现：sidecar 到 3389 的 ESTABLISHED 连接挂在握手期数小时）。
/// 任一输出事件都会重置预算；证书确认窗（120s 用户等待）与已连接期豁免。
/// 超时按可重试传输失败收场，交给退避梯子，UI 得到明确的错误出口。
pub(crate) const CONNECT_PROGRESS_BUDGET: Duration = Duration::from_secs(30);

/// Automatic reconnects after a transport failure (NyaTerm default 5).
pub(crate) const DEFAULT_RECONNECT_ATTEMPTS: u32 = 5;

pub(crate) const MAX_RECONNECT_ATTEMPTS: u32 = 10;

/// Cumulative automatic-reconnect budget per session worker (D3,
/// rdp-security-review): an active generation resets the backoff ladder
/// step only — never this total — so a hostile server cannot keep the
/// client dialling forever by alternating one active frame with
/// transport-class failures. Manual `rdp/reconnect` spawns a fresh worker
/// (user-driven gate, NyaTerm's explicit-reconnect semantics).
pub(crate) const MAX_TOTAL_RECONNECTS: u32 = 50;

// —— runtime ——

pub(crate) enum RdpCommand {
    Input(RdpWireInput),
    /// The staged clipboard text was updated; advertise it to the remote.
    ClipboardAdvertise,
    Close,
}

// One retained 4K RGBA desktop is 31,850,496 bytes. Keeping one composite
// replay framebuffer per session (RdpFrameState) bounds resident memory to
// that order.

pub(crate) struct RdpSessionEntry {
    pub(crate) workbench_id: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) domain: String,
    /// Zeroizing-held credential; materialized once per generation at the
    /// vendored `with_password` boundary, dropped (zeroized) with the entry.
    pub(crate) password: Option<Zeroizing<String>>,
    /// Requested desktop size; read at every generation's config build, so a
    /// `rdp/resize` can retarget the *next* generation (resize = reconnect,
    /// see `RdpSessionRuntime::resize`).
    pub(crate) width: AtomicU16,
    pub(crate) height: AtomicU16,
    pub(crate) use_nla: bool,
    pub(crate) cert_policy: CertPolicy,
    pub(crate) clipboard_enabled: bool,
    pub(crate) reconnect_attempts: u32,
    pub(crate) created_at_secs: u64,
    /// Bumped before every (re)connect; superseded workers exit without
    /// emitting and every prompt decision across generations is rejected.
    pub(crate) generation: Arc<AtomicU64>,
    /// Serializes realtime mutation, sequence allocation and replay into one
    /// ordered stream while retaining at most one bounded complete desktop.
    pub(crate) frame_state: tokio::sync::Mutex<RdpFrameState>,
    pub(crate) close_requested: AtomicBool,
    /// 证书确认等待期标记（verifier 置位/清零）：首帧前连接进度预算据此豁免
    /// 120s 用户确认窗——等待期引擎线程阻塞在 verify，不会有输出事件流动。
    pub(crate) cert_wait: Arc<AtomicBool>,
    /// Sender of the *current* generation's command channel; `None` while
    /// dialling/reconnecting (input fails fast instead of queueing).
    pub(crate) command_sender: tokio::sync::Mutex<Option<mpsc::Sender<RdpCommand>>>,
    /// Local clipboard text staged for the remote (`rdp/set-clipboard`).
    pub(crate) clipboard_stage: Arc<ClipboardStage>,
    pub(crate) broker: Arc<CertPromptBroker>,
    pub(crate) known_certs: KnownCertStore,
}

pub struct RdpSessionRuntime {
    pub(crate) sessions: Arc<tokio::sync::RwLock<HashMap<String, Arc<RdpSessionEntry>>>>,
    pub(crate) broker: Arc<CertPromptBroker>,
}
