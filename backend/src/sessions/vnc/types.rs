//! VNC session support.

use super::*;

/// Classic VNC-Auth passwords are exactly the first 8 bytes (bit-reversed
/// into the DES key); longer secrets truncate silently, so reject up front.
pub(crate) const MAX_PASSWORD_BYTES: usize = 8;

/// Clipboard bounds (vnc-rs exchanges UTF-8 cut text; classic RFB is Latin-1).
pub(crate) const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;

/// Default automatic reconnects after a transport failure.
pub(crate) const DEFAULT_RECONNECT_ATTEMPTS: u32 = 3;

pub(crate) const MAX_RECONNECT_ATTEMPTS: u32 = 10;

// —— request/response shapes ——

/// Frontend canvas scaling mode. `fit` also enables automatic remote
/// SetDesktopSize negotiation (`vnc/resize`); `stretch`/`actual` never touch
/// the remote resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScaleMode {
    #[default]
    Fit,
    Stretch,
    Actual,
}

impl ScaleMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "fit" => Some(Self::Fit),
            "stretch" => Some(Self::Stretch),
            "actual" => Some(Self::Actual),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fit => "fit",
            Self::Stretch => "stretch",
            Self::Actual => "actual",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VncStartRequest {
    pub connection_id: Option<String>,
    pub workbench_id: String,
    /// Logical endpoint retained for identity and display.
    pub host: String,
    /// Logical endpoint port. Defaults to 5900.
    pub port: Option<u16>,
    /// Runtime endpoint supplied by the host transport; it is the only target
    /// dialed by the TCP pump. Missing fields fall back to the logical endpoint.
    pub runtime_host: Option<String>,
    pub runtime_port: Option<u16>,
    /// Absent/empty → RFB None security. Present → classic VNC-Auth
    /// (≤ 8 bytes). Held in `Zeroizing`, never logged.
    pub password: Option<String>,
    /// Frontend scaling hint: fit (default) / stretch / actual.
    pub scale_mode: Option<String>,
    /// Automatic reconnects after transport failures (default 3, max 10).
    pub reconnect_attempts: Option<u32>,
}

/// `vnc/input` payload. The frontend maps DOM events to X keysyms
/// (`vncFrame`/`vncInput` helpers), so the backend stays transport-only.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum VncInputEvent {
    #[serde(rename = "key")]
    Key { keysym: u32, pressed: bool },
    #[serde(rename = "pointer")]
    Pointer {
        x: u16,
        y: u16,
        /// Wire contract is camelCase; snake_case stays as an alias for
        /// programmatic callers.
        #[serde(rename = "buttonMask", alias = "button_mask")]
        button_mask: u8,
    },
    #[serde(rename = "release-all")]
    ReleaseAll,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VncInputRequest {
    pub session_id: String,
    #[serde(flatten)]
    pub event: VncInputEvent,
}

/// `vnc/resize` payload: live SetDesktopSize via ExtendedDesktopSize.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VncResizeRequest {
    pub session_id: String,
    pub width: u16,
    pub height: u16,
}

/// Classic VNC-Auth secret → DES key: the first ≤8 password bytes, each
/// bit-reversed (RFC 6143 7.2.2). Spec-reference helper: upstream applies the
/// same transform inside `AuthHelper` (not part of its public API), so this
/// lives under `#[cfg(test)]` next to the known-vector pin.
#[cfg(test)]
pub fn vnc_auth_key(password: &str) -> [u8; 8] {
    let mut key = [0u8; 8];
    for (slot, byte) in key.iter_mut().zip(password.as_bytes()) {
        *slot = byte.reverse_bits();
    }
    key
}

/// `vnc/start` parameter validation shared with the docs contract: classic
/// VNC authentication cannot carry more than 8 bytes.
pub fn validate_password(password: Option<&str>) -> Result<(), String> {
    if let Some(password) = password {
        // `str::len` is byte length — exactly the classic-auth bound.
        if password.len() > MAX_PASSWORD_BYTES {
            return Err(format!(
                "vnc/start: password must be at most {MAX_PASSWORD_BYTES} bytes (classic VNC authentication limit)"
            ));
        }
    }
    Ok(())
}

/// Clipboard text guard: the vendored `vnc-rs` chain carries cut text as
/// UTF-8 bytes (the classic RFB field is nominally Latin-1, but modern servers
/// exchange UTF-8). Bound it by bytes; do not reject non-ASCII text.
pub fn validate_clipboard_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_CLIPBOARD_BYTES {
        return Err("vnc/set-clipboard: text exceeds the 1 MiB clipboard limit".to_string());
    }
    Ok(())
}

/// Remote resize guard (`vnc/resize`): RFB SetDesktopSize carries u16 dims,
/// but the retained framebuffer contract (and patch payload budget) is
/// 3840×2160. Keep a sane floor so a collapsed pane cannot ask for 1×1.
pub(crate) fn validate_resize_size(width: u16, height: u16) -> Result<(), String> {
    if width < 200 || height < 200 || width > MAX_FRAMEBUFFER_WIDTH || height > MAX_FRAMEBUFFER_HEIGHT {
        return Err(format!(
            "vnc/resize: desktop size must be within 200x200 .. {MAX_FRAMEBUFFER_WIDTH}x{MAX_FRAMEBUFFER_HEIGHT}"
        ));
    }
    Ok(())
}

/// Backoff between reconnect attempts: 1s, 2s, then a flat 4s cap so a dead
/// server cannot pin a worker on long sleeps. Pure — unit-tested.
pub(crate) fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_secs(match attempt {
        0 | 1 => 1,
        2 => 2,
        _ => 4,
    })
}

// —— runtime ——

pub(crate) enum VncCommand {
    Input(VncInputEvent),
    /// Live RFB SetDesktopSize (ExtendedDesktopSize pseudo-encoding). The
    /// server applies it and answers with a DesktopUpdate; the generation
    /// and session id stay put.
    Resize { width: u16, height: u16 },
    Clipboard(String),
    Close,
}

pub(crate) struct VncSessionEntry {
    pub(crate) connection_id: Option<String>,
    pub(crate) workbench_id: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) runtime_host: String,
    pub(crate) runtime_port: u16,
    pub(crate) password: Option<Zeroizing<String>>,
    pub(crate) scale_mode: ScaleMode,
    pub(crate) reconnect_attempts: u32,
    pub(crate) created_at_secs: u64,
    /// Bumped before every (re)connect; the superseded worker exits without
    /// emitting so stale frames/state can never cross generations.
    pub(crate) generation: AtomicU64,
    /// Monotonic across generations so the frontend can drop out-of-order
    /// patches after a reconnect.
    pub(crate) frame_sequence: AtomicU64,
    pub(crate) close_requested: AtomicBool,
    /// Sender of the *current* generation's command channel; `None` while
    /// dialling/reconnecting (input fails fast instead of queueing).
    pub(crate) command_sender: tokio::sync::Mutex<Option<mpsc::Sender<VncCommand>>>,
    /// Retained desktop for reconnect repaints; owned by the live worker
    /// between `SetResolution` and teardown.
    pub(crate) framebuffer: tokio::sync::Mutex<Option<VncFramebuffer>>,
}

pub struct VncSessionRuntime {
    pub(crate) sessions: Arc<RwLock<HashMap<String, Arc<VncSessionEntry>>>>,
}

impl VncSessionEntry {
    /// Locks the retained desktop and lazily creates it from a rect when the
    /// server streamed pixels before the DesktopSize pseudo-encoding (same
    /// fallback NyaTerm uses). Callers must hold the guard while applying the
    /// patch so realtime mutation stays ordered.
    pub(crate) async fn framebuffer_for(
        &self,
        rect: Rect,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<VncFramebuffer>>, String> {
        let mut framebuffer = self.framebuffer.lock().await;
        if framebuffer.is_none() {
            let width = u32::from(rect.x)
                .checked_add(u32::from(rect.width))
                .and_then(NonZeroU32::new)
                .and_then(|size| u16::try_from(size.get()).ok())
                .ok_or_else(|| "VNC rectangle bounds overflow".to_string())?;
            let height = u32::from(rect.y)
                .checked_add(u32::from(rect.height))
                .and_then(NonZeroU32::new)
                .and_then(|size| u16::try_from(size.get()).ok())
                .ok_or_else(|| "VNC rectangle bounds overflow".to_string())?;
            *framebuffer = Some(VncFramebuffer::new(width, height)?);
        }
        Ok(framebuffer)
    }
}

/// Why one protocol generation ended. `Superseded` means a newer generation
/// took over (reconnect/close) — the worker must exit silently.
pub(crate) enum GenerationEnd {
    Closed,
    Superseded,
    Failed {
        error: String,
        retryable: bool,
        was_active: bool,
    },
}

impl GenerationEnd {
    /// Wraps an engine error through the classifier.
    pub(crate) fn failed(error: VncError) -> Self {
        let (message, retryable) = classify_vnc_error(error);
        GenerationEnd::Failed {
            error: message,
            retryable,
            was_active: false,
        }
    }
}
