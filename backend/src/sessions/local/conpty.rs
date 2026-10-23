//! Local terminal conpty support.

use super::*;

/// ConPTY's startup cursor-position query. conhost withholds all output behind
/// it, and the answer is a **cursor position**, not a viewport size: the
/// pseudo console is created with `PSEUDOCONSOLE_INHERIT_CURSOR`, whose
/// documented job is to "inherit cursor position from the parent terminal".
pub(super) const CONPTY_CPR_QUERY: &[u8] = b"\x1b[6n";

/// Windows ConPTY opens every session with a handshake: conhost sends the
/// terminal `CSI 6n` and withholds ALL output until a CPR reply arrives.
/// Because the pseudo console carries `PSEUDOCONSOLE_INHERIT_CURSOR`, the
/// reply is read as **where the terminal's cursor is**, and the child's first
/// output starts there.
///
/// That makes the reply's *value* load-bearing, and answering it with the
/// viewport size — the value this sidecar used to send — is wrong: `rows;cols`
/// tells conhost the cursor sits on the bottom row, so it emits the first text
/// batch where its own buffer origin was (the orphan glyph: cmd's `M`, WSL's
/// `d`) and then re-serializes the whole banner from the inherited bottom-row
/// position (PowerShell's doubled prompt). The workbench knows the real cursor
/// and passes it in [`LocalTerminalStartRequest::cursor_row`], so the sidecar
/// answers with the truth and the shell starts where the user sees the cursor.
///
/// The reply normally also travels terminal → PTY input, through the same
/// workbench binary bridge as keystrokes; when that path drops it every local
/// shell sits alive but permanently silent (no banner, no prompt, no echo —
/// the terminal looks dead while the child runs). The sidecar therefore
/// answers the first query itself, which also means the query must not reach
/// the terminal as well: a second answer would be ignored (measured), but the
/// query has no reader on the far side worth the round trip. So it is
/// swallowed here, before it is forwarded, and stripped at the source so it
/// cannot sit in the replay buffer either. `CSI ?6n` (DECDSR) and every later
/// `CSI 6n` from applications (PSReadLine, vim) pass through untouched for the
/// terminal to answer truthfully.
pub(super) struct ConptyHandshakeFilter {
    /// Handshake already answered (or not needed on this platform): from here
    /// on every byte passes through verbatim.
    pub(super) done: bool,
    /// Trailing bytes that may begin the query; held until the next chunk can
    /// complete — or rule out — the match.
    pub(super) held: Vec<u8>,
}

/// The CPR reply ConPTY's inherit-cursor handshake expects: `CSI row;col R` —
/// the terminal's 1-based cursor position, which is where the shell will start
/// drawing. Sending the viewport size instead inherits a bottom-row cursor and
/// produces the orphan-glyph/doubled-prompt artifact (see
/// [`ConptyHandshakeFilter`]).
pub(super) fn conpty_cpr_reply(row: u16, col: u16) -> Vec<u8> {
    format!("\x1b[{row};{col}R").into_bytes()
}

/// Normalizes the workbench-reported cursor into the viewport: absent (older
/// workbenches) means the top-left, and anything out of range is clamped so
/// conhost never inherits a position outside the screen it just sized.
pub(super) fn inherit_cursor(
    row: Option<u16>,
    col: Option<u16>,
    rows: u16,
    cols: u16,
) -> (u16, u16) {
    (
        row.unwrap_or(1).clamp(1, rows.max(1)),
        col.unwrap_or(1).clamp(1, cols.max(1)),
    )
}

/// Deadline for the resize-flush timer while nothing is pending. tokio's
/// `Instant::far_future` is crate-private in the version we pin, so park the
/// timer an hour out instead; every reset is anchored to `now`, so the timer
/// simply never fires while the gate has nothing to flush.
pub(super) fn idle_flush_deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_secs(3600)
}

/// Applies a resize to the PTY when the master is still alive; returns the
/// new tracked geometry so callers update their size cache only on success.
pub(super) fn apply_pty_resize(
    master: Option<&(dyn MasterPty + Send)>,
    size: PtySize,
) -> Option<(u16, u16)> {
    master?.resize(size).ok().map(|_| (size.rows, size.cols))
}

/// Gates client resize requests before they reach the PTY. ConPTY
/// re-serializes its whole screen buffer on every ResizePseudoConsole call —
/// same-size included — and right after spawn that redraw races the shell
/// banner into a duplicated prompt (an orphan first glyph on top, a banner
/// copy without it at the viewport bottom). Same-size requests are therefore
/// dropped; the tracked size doubles as the ConPTY CPR reply source.
pub(super) fn requested_pty_size(
    current_rows: u16,
    current_cols: u16,
    rows: u32,
    cols: u32,
) -> Option<PtySize> {
    let rows = rows.clamp(2, u16::MAX as u32) as u16;
    let cols = cols.clamp(2, u16::MAX as u32) as u16;
    if rows == current_rows && cols == current_cols {
        return None;
    }
    Some(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })
}

/// How long after the last startup output a deferred resize waits before the
/// screen is considered settled (banner and first prompt fully rendered).
pub(super) const STARTUP_RESIZE_QUIET: Duration = Duration::from_millis(500);

/// Hard cap on deferring: a chatty banner must not hold a real user resize
/// hostage for more than the typical shell cold start (PowerShell 1-3s).
pub(super) const STARTUP_RESIZE_WINDOW: Duration = Duration::from_secs(3);

/// What [`StartupResizeGate::resize`] wants the caller to do with a resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartupResizeDecision {
    /// Safe to call `MasterPty::resize` now — no ConPTY repaint race.
    Apply,
    /// Hold the resize; the gate merges it into its pending slot and the
    /// flush timer applies it once output goes quiet.
    Defer,
}

/// Merges startup-window resizes on Windows so ConPTY's whole-screen repaint
/// never interleaves with the shell's opening output.
///
/// ConPTY re-serializes its entire screen buffer on every
/// `ResizePseudoConsole`. When that lands mid-banner the terminal renders the
/// snapshot and the live stream as two copies (an orphan glyph on the first
/// row, a full banner/prompt duplicate below) — the classic "M"/"d" artifact.
/// Spawn-time sizing and same-size dedup only remove the resizes the plugin
/// itself causes; the host keeps reshaping the panel after spawn (dock/tab
/// layout settling, window resize), and those real geometry changes still
/// race PowerShell's slow first prompt. The gate holds such resizes while
/// output is in flight and applies the merged geometry once the shell has
/// been quiet — a repaint over a static screen is an idempotent re-layout,
/// exactly what a user-initiated resize looks like later in the session.
///
/// All timestamps are elapsed-since-spawn `Duration`s so tests drive time
/// directly. Non-Windows platforms always get [`StartupResizeDecision::Apply`]:
/// Unix PTYs do not repaint on resize.
pub(super) struct StartupResizeGate {
    pub(super) platform: Platform,
    /// Elapsed-since-spawn of the last PTY output; `None` before the first
    /// byte (an empty screen repaints harmlessly, so no gating then).
    pub(super) last_output_at: Option<Duration>,
    /// Latest deferred geometry; each new resize overwrites the previous one.
    pub(super) pending: Option<PtySize>,
}
