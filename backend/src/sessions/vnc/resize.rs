//! Live RFB desktop resize (ExtendedDesktopSize pseudo-encoding).
//!
//! `vnc-rs` sends SetDesktopSize on the client's input channel and confirms
//! via a DesktopUpdate observed by the connection reader. The request must not
//! run on the pump (it would stop frame delivery for up to five seconds), so
//! the worker hands it to a spawned task; the reader keeps observing replies
//! independently.

use super::*;

/// Adopts a server-confirmed desktop layout. The framebuffer restarts blank at
/// the new size; the caller requests a full repaint so it repopulates.
/// Returns whether the geometry changed (same-size updates keep the pixels).
pub(crate) fn apply_desktop_layout(
    framebuffer: &mut Option<VncFramebuffer>,
    width: u16,
    height: u16,
) -> Result<bool, String> {
    if framebuffer
        .as_ref()
        .is_some_and(|desktop| desktop.matches_size(width, height))
    {
        return Ok(false);
    }
    *framebuffer = Some(VncFramebuffer::new(width, height)?);
    Ok(true)
}

/// Queues a SetDesktopSize for the current engine and repaints on success.
/// Unsupported/denied/timeout outcomes only log: the session keeps the old
/// resolution and the frontend stays on its existing fit scaling.
pub(crate) fn spawn_resize(client: VncClient, session_id: String, width: u16, height: u16) {
    tokio::spawn(async move {
        let mut unsupported_retried = false;
        let mut busy_retries = 0u32;
        let result = loop {
            match client.resize_desktop(width, height).await {
                // The capability advertisement can still be in flight right
                // after connect: retry once before declaring the server
                // unsupported.
                Err(VncResizeError::Unsupported) if !unsupported_retried => {
                    unsupported_retried = true;
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
                // Another resize (an earlier sidebar toggle) is still in
                // flight; wait for it, then apply the newest target.
                Err(VncResizeError::Busy) if busy_retries < 20 => {
                    busy_retries += 1;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                other => break other,
            }
        };
        match result {
            Ok(layout) => {
                eprintln!(
                    "[dbx-plugin-sshx] vnc resize {session_id} applied: {}x{}",
                    layout.width, layout.height
                );
                // The DesktopUpdate handler rebuilt the framebuffer blank;
                // force a full repaint at the new dimensions.
                let _ = client.input(X11Event::FullRefresh).await;
            }
            Err(error) => {
                eprintln!(
                    "[dbx-plugin-sshx] vnc resize {session_id} -> {width}x{height} failed: {error}"
                );
            }
        }
    });
}