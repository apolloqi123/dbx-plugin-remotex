//! Inbound RFB clipboard forwarding.
//!
//! VNC servers only speak text, but some forward a remote file copy as its
//! `file://` URI list. That must not enter the host text clipboard: the
//! transfer window downloads the real files instead (`vnc/clipboard-files`).

use super::*;
use crate::platform::diagnostics::log_rdp_debug;

pub(super) fn forward_text(session_id: &str, text: String, emitter: &PluginEmitter) {
    let files = crate::core::clipboard_uri::parse_uri_list(&text);
    if !files.is_empty() {
        log_rdp_debug(format!(
            "vnc clipboard-files session={session_id} count={}",
            files.len()
        ));
        let _ = emitter.event(
            "vnc/clipboard-files",
            json!({ "sessionId": session_id, "paths": files }),
        );
        return;
    }
    log_rdp_debug(format!(
        "vnc clipboard session={session_id} inbound len={}",
        text.len()
    ));
    let _ = emitter.event(
        "vnc/clipboard",
        json!({ "sessionId": session_id, "text": text }),
    );
}
