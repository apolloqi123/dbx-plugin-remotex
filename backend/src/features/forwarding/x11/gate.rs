//! Metrics support.

use super::*;

/// Concurrent x11 channels bridged per armed session. Eight is generous for
/// interactive use and bounds the local X connection fan-out of one remote
/// host (spike §3.6).
pub(crate) const MAX_BRIDGES_PER_SESSION: u32 = 8;

/// Per-SSH-connection admission gate shared between the session opener (arms
/// it with the fake cookie when the preference and read-only checks pass)
/// and the `server_channel_open_x11` handler. A gate that was never armed
/// rejects every channel — russh's default handler would accept, so this is
/// the fail-closed boundary (spike §3.4).
#[derive(Default)]
pub(crate) struct X11Gate {
    pub(crate) armed: AtomicBool,
    pub(crate) cookie: Mutex<Option<FakeCookie>>,
    pub(crate) bridges: AtomicU32,
}

/// Slot for one bridged x11 channel; dropping it releases the cap slot.
pub(crate) struct BridgePermit {
    gate: Arc<X11Gate>,
}

impl Drop for BridgePermit {
    fn drop(&mut self) {
        self.gate.bridges.fetch_sub(1, Ordering::AcqRel);
    }
}

impl X11Gate {
    /// Draws a fresh cookie and arms the gate; returns the gate plus the hex
    /// value to send in `x11-req`.
    pub(crate) fn armed() -> Result<(Arc<Self>, String), String> {
        let cookie = FakeCookie::generate()?;
        let hex = cookie.hex().to_string();
        let gate = Arc::new(Self {
            armed: AtomicBool::new(true),
            cookie: Mutex::new(Some(cookie)),
            bridges: AtomicU32::new(0),
        });
        Ok((gate, hex))
    }

    /// Clears the cookie on session teardown: channels that race the close
    /// fail closed at the next admission or first-packet check.
    pub(crate) fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
        if let Ok(mut slot) = self.cookie.lock() {
            *slot = None;
        }
    }

    /// Handler-side admission. `None` when X11 was never armed on this
    /// session, the session already closed, or the per-session bridge cap is
    /// exhausted — the caller must reject the channel.
    pub(crate) fn try_admit(self: &Arc<Self>) -> Option<(FakeCookie, BridgePermit)> {
        if !self.armed.load(Ordering::Acquire) {
            return None;
        }
        // Bump first and roll back on every failure path so concurrent
        // admissions keep the cap exact.
        if self.bridges.fetch_add(1, Ordering::AcqRel) >= MAX_BRIDGES_PER_SESSION {
            self.bridges.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        let cookie = match self.cookie.lock() {
            Ok(slot) => match slot.as_ref() {
                Some(cookie) => cookie.clone(),
                None => {
                    self.bridges.fetch_sub(1, Ordering::AcqRel);
                    return None;
                }
            },
            Err(_) => {
                self.bridges.fetch_sub(1, Ordering::AcqRel);
                return None;
            }
        };
        Some((cookie, BridgePermit { gate: self.clone() }))
    }
}

// —— 会话接线（ssh.rs 消费）———————————————————————————————

/// The gate of the most recent X11-enabled session (`armed()` output).
/// `None` until a session turns X11 on. Each re-arm replaces the slot and
/// disarms the previous gate, so only the newest armed session's fake
/// cookie is honored and admit counts stay per-gate. Concurrent X11
/// sessions therefore share the newest gate by design — a strictly
/// per-connection gate cannot be disarmed per session because reused
/// transports serve several sessions through one `SshClient`. When the
/// sidecar's last session closes, `ssh.rs` (`close_session`) calls
/// [`disarm_active`]; afterwards every channel fails closed until a new
/// session arms again.
pub(crate) static ACTIVE_GATE: Mutex<Option<Arc<X11Gate>>> = Mutex::new(None);

/// Arms a fresh gate for a session that turned X11 on; returns the fake
/// cookie hex to send in `x11-req`. Any previously armed gate is disarmed
/// first, so channels racing the re-arm fail closed on the stale cookie.
pub(crate) fn arm_session() -> Result<String, String> {
    let (gate, hex) = X11Gate::armed()?;
    if let Ok(mut slot) = ACTIVE_GATE.lock() {
        replace_gate(&mut slot, gate);
    }
    set_enabled(true);
    Ok(hex)
}

/// Slot replacement: disarm the previous gate, then store the new one.
pub(crate) fn replace_gate(slot: &mut Option<Arc<X11Gate>>, gate: Arc<X11Gate>) {
    if let Some(previous) = slot.take() {
        previous.disarm();
    }
    *slot = Some(gate);
}

/// Releases the active gate (last session closed): the cookie is cleared
/// and every subsequent admission fails closed.
pub(crate) fn disarm_active() {
    if let Ok(mut slot) = ACTIVE_GATE.lock() {
        if let Some(gate) = slot.take() {
            gate.disarm();
        }
    }
}

/// Admission for `server_channel_open_x11`: only succeeds while a session
/// has X11 armed and the armed gate's bridge cap is not exhausted.
pub(crate) fn try_admit_active() -> Option<(FakeCookie, BridgePermit)> {
    let gate = ACTIVE_GATE.lock().ok().and_then(|slot| slot.clone());
    gate.and_then(|gate| gate.try_admit())
}

/// Fast-path preference flag (mirrors `x11_forwarding` in preferences.json).
/// The Handler consults this before touching the registry; the on-disk file
/// stays the source of truth via [`enabled_from`].
pub(crate) static ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Reads `x11_forwarding` from the allowlisted preferences file.
/// The display target for the current session: the `DISPLAY` environment
/// variable parsed, falling back to the default unix socket (`:0`).
pub(crate) fn current_display_target() -> DisplayTarget {
    let display = std::env::var("DISPLAY").unwrap_or_default();
    parse_display(&display).unwrap_or_else(|| unix_display(0))
}

pub(crate) fn enabled_from(data_dir: &Path) -> bool {
    let text =
        std::fs::read_to_string(crate::core::preferences::store_path(data_dir)).unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| {
            value
                .get("x11_forwarding")
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false)
}
