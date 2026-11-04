//! `rdp/clipboard-files/status` payload: remote file list, download progress
//! and the host/remote file-clipboard capabilities used by the toolbar hint.

use super::{pasteboard_references_dir, FileClipboardStage};
use serde_json::{json, Value};

pub(super) fn status(stage: &FileClipboardStage) -> Value {
    let session_id = stage.session_id.clone();
    let Ok(inner) = stage.lock() else {
        return json!({ "sessionId": session_id, "remoteFiles": [] });
    };
    let mut payload = json!({
        "sessionId": session_id,
        "remoteFiles": [],
        "remoteTotalBytes": 0,
        "archiveReady": false,
        "downloading": false,
        "filesDone": 0,
        "filesTotal": 0,
        "bytesDone": 0,
        "bytesTotal": 0,
        // Host-side file clipboard capability (macOS NSPasteboard, Windows
        // CF_HDROP, Linux xclip/wl-clipboard). The toolbar hint uses it to
        // choose between Ctrl+C/V wording and the download/send buttons.
        "nativeFiles": crate::platform::pasteboard::supported(),
        // Remote file clipboard capability: CLIPRDR reached Ready and the
        // server accepted STREAM_FILECLIP_ENABLED. Linux servers such as
        // GNOME Remote Desktop only do text, so the UI offers SSH/SFTP.
        "serverFiles": stage.supports_file_copy(),
        "fileClipboardReady": stage.is_ready(),
    });
    if let Some(remote) = inner.remote.as_ref() {
        payload["remoteFiles"] = Value::Array(remote.files_payload());
        payload["remoteTotalBytes"] = json!(remote.bytes_total);
        payload["archiveReady"] = json!(remote.archive.is_some());
        payload["downloading"] = json!(remote.downloading);
        payload["filesDone"] = json!(remote.files_done);
        payload["filesTotal"] = json!(remote.files_total);
        payload["bytesDone"] = json!(remote.bytes_done);
        payload["bytesTotal"] = json!(remote.bytes_total.max(remote.bytes_done));
        payload["fileCount"] = json!(remote.files.iter().filter(|file| !file.directory).count());
        payload["completed"] = json!(
            remote.files_total > 0
                && !remote.downloading
                && remote.files_done >= remote.files_total
        );
        payload["pasteboard"] =
            json!(remote.completed_pasteboard && pasteboard_references_dir(&remote.dir));
        if let Some(archive) = remote.archive.as_ref() {
            payload["archiveName"] = json!(archive.name);
            payload["archiveSize"] = json!(archive.size);
        }
    }
    if let Some(error) = inner.last_error.as_ref() {
        payload["error"] = json!(error);
    }
    payload
}
