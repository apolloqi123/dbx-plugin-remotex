//! CLIPRDR 桥：CF_UNICODETEXT 文本（16 MiB 整包拒绝，入站按 JSON 预算分片为
//! `rdp/clipboard` 事件）+ 文件/文件夹传输（状态机在 `file_clipboard.rs`，
//! 本文件只是薄适配层）。所有文件回调只返回消息或发错误事件，绝不把错误
//! 穿回引擎泵（剪贴板失败降级，不结束会话）。
use super::file_clipboard::FileClipboardStage;
use super::limits::{clipboard_chunks, validate_clipboard_payload_len, validate_clipboard_text};
use crate::platform::diagnostics::log_rdp_debug;
use dbx_plugin_sdk::PluginEmitter;
use ironrdp::cliprdr::backend::{
    ClipboardMessage, ClipboardMessageProxy, CliprdrBackend, CliprdrBackendFactory,
};
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FileDescriptor, FormatDataRequest, FormatDataResponse, LockDataId,
    OwnedFormatDataResponse,
};
use ironrdp::core::impl_as_any;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

/// Clipboard lifecycle diagnostics go to `rdp-debug.log` in the field; unit
/// tests must not touch the user's plugin data directory.
fn diag(message: String) {
    if !cfg!(test) {
        log_rdp_debug(message);
    }
}

/// Bounded re-initialization when the server has not completed the CLIPRDR
/// handshake yet: `initiate_copy` in Initialization re-sends Capabilities +
/// TemporaryDirectory + FormatList, which is the only recovery the vendored
/// client offers (it never leaves Initialization on its own).
const CLIPBOARD_REINIT_ATTEMPTS: u8 = 3;
const CLIPBOARD_REINIT_INTERVAL: Duration = Duration::from_secs(2);

/// Per-event JSON budget for `rdp/clipboard` chunk events (C2,
/// rdp-security-review): the SDK transport silently drops JSON events over
/// 8 MiB (`MAX_JSON_BYTES`), so oversized text is split into chunks whose
/// JSON-escaped size stays under this bound.
pub(crate) const CLIPBOARD_CHUNK_JSON_BUDGET: usize = 7 * 1024 * 1024;

// —— clipboard (text-only bridge) ——

/// Local-side clipboard text staged by `rdp/set-clipboard`. The CLIPRDR
/// backend answers the server's format-data requests from here; the OS
/// clipboard itself stays with the workbench frontend (the sidecar is
/// headless — same split as VNC). Never logged, never audited.
#[derive(Default)]
pub(crate) struct ClipboardStage {
    text: StdMutex<Option<String>>,
}

impl ClipboardStage {
    pub(crate) fn set_text(&self, text: String) {
        if let Ok(mut slot) = self.text.lock() {
            *slot = Some(text);
        }
    }

    pub(crate) fn text(&self) -> Option<String> {
        self.text.lock().ok().and_then(|slot| slot.clone())
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut slot) = self.text.lock() {
            *slot = None;
        }
    }
}

/// Text-first CLIPRDR backend. CF_UNICODETEXT always stays advertised (empty
/// text is a valid response); file/folder transfer is delegated to
/// [`FileClipboardStage`], which returns messages to send or emits an error
/// event. No clipboard failure may terminate the session.
struct TextClipboardBackend {
    session_id: String,
    stage: Arc<ClipboardStage>,
    files: Arc<FileClipboardStage>,
    proxy: Arc<StdMutex<Box<dyn ClipboardMessageProxy>>>,
    emitter: PluginEmitter,
    /// The remote offered CF_UNICODETEXT alongside its file list; the text
    /// request is chained after the file-list metadata (the vendored Cliprdr
    /// correlates only one FormatDataRequest at a time).
    text_after_file_list: bool,
    /// One bounded re-advertisement after a transient CB_RESPONSE_FAIL.
    file_advertise_retries: u8,
    /// This CLIPRDR channel completed the initialization handshake. Text
    /// callbacks are gated on this (never on the file-transfer stage).
    channel_ready: bool,
    /// Bounded handshake recovery counters (see CLIPBOARD_REINIT_*).
    reinit_attempts: u8,
    reinit_next_at: Option<Instant>,
}

impl std::fmt::Debug for TextClipboardBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TextClipboardBackend") // never render staged text
    }
}

impl_as_any!(TextClipboardBackend);

impl TextClipboardBackend {
    fn send(&self, message: ClipboardMessage) {
        if let Ok(proxy) = self.proxy.lock() {
            proxy.send_clipboard_message(message);
        }
    }

    /// Advertises CF_UNICODETEXT; this is also the mandatory CLIPRDR
    /// initialization step (Capabilities + TemporaryDirectory + FormatList),
    /// so it must be sent even when no text is staged.
    fn advertise_text_format(&mut self) {
        self.send(ClipboardMessage::SendInitiateCopy(vec![
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ]));
    }

    /// Recovery for a channel that never reached Ready (server answered Fail,
    /// was slow, or the initial advertisement raced the channel join). In
    /// Initialization the vendored client bundles Capabilities + Temporary
    /// Directory + FormatList again; after Ready this is a normal text
    /// advertisement. Bounded so a rejecting server cannot be spammed.
    fn maybe_reinit(&mut self) {
        if self.channel_ready || self.reinit_attempts >= CLIPBOARD_REINIT_ATTEMPTS {
            return;
        }
        let now = Instant::now();
        if let Some(next) = self.reinit_next_at {
            if now < next {
                return;
            }
        }
        self.reinit_attempts += 1;
        self.reinit_next_at = Some(now + CLIPBOARD_REINIT_INTERVAL);
        diag(format!(
            "rdp clipboard session={} reinit attempt={}",
            self.session_id, self.reinit_attempts
        ));
        self.advertise_text_format();
    }
}

impl CliprdrBackend for TextClipboardBackend {
    fn temporary_directory(&self) -> &str {
        // Downloads are staged by `FileClipboardStage` instead.
        ".dbx-ssh-rdp-cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // Streamed files + no source paths + clipDataId locks (MS-RDPECLIP 2.2.2.1).
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
    }

    fn on_ready(&mut self) {
        self.channel_ready = true;
        self.reinit_attempts = 0;
        self.reinit_next_at = None;
        self.files.set_ready();
        diag(format!(
            "rdp clipboard session={} ready file_copy={}",
            self.session_id,
            self.files.supports_file_copy()
        ));
        if let Err(error) = self.emitter.event(
            "rdp/clipboard-status",
            json!({
                "sessionId": self.session_id,
                "state": "ready",
                // Remote file clipboard capability: GNOME Remote Desktop and
                // other non-Windows servers negotiate text only.
                "fileCopy": self.files.supports_file_copy(),
            }),
        ) {
            diag(format!(
                "rdp clipboard session={} status event dropped: {}",
                self.session_id, error.message
            ));
        }
        // Announce text staged before the channel joined (set-clipboard during
        // dial). A text copy clears any staged file list, so skip it then.
        if self.stage.text().is_some() && self.files.outgoing_descriptors().is_none() {
            self.advertise_text_format();
        }
    }

    fn on_request_format_list(&mut self) {
        // Initialization advertisement — see `advertise_text_format`.
        diag(format!(
            "rdp clipboard session={} format-list request",
            self.session_id
        ));
        self.advertise_text_format();
    }

    fn on_process_negotiated_capabilities(
        &mut self,
        capabilities: ClipboardGeneralCapabilityFlags,
    ) {
        self.files.set_negotiated_capabilities(capabilities);
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        // A remote FormatList starts a new clipboard generation; a text pull
        // chained for the previous one must not fire against this one.
        self.text_after_file_list = false;
        let text_offered = available_formats
            .iter()
            .any(|format| format.id == ClipboardFormatId::CF_UNICODETEXT);
        diag(format!(
            "rdp clipboard session={} remote-copy formats={} text={} ready={}",
            self.session_id,
            available_formats.len(),
            text_offered,
            self.channel_ready
        ));
        // initiate_paste hard-fails (and kills the session) before Ready.
        if !self.channel_ready {
            self.maybe_reinit();
            return;
        }
        // Pre-Ready or without STREAM_FILECLIP_ENABLED the engine would treat a
        // file paste/contents request as fatal.
        let file_list = if self.files.supports_file_copy() {
            self.files.note_remote_copy(available_formats)
        } else {
            None
        };
        // Cliprdr has one pending FormatDataRequest: metadata first, text chained
        // in `on_remote_file_list`.
        if let Some(file_list) = file_list {
            self.text_after_file_list = text_offered;
            self.send(ClipboardMessage::SendInitiatePaste(file_list));
            return;
        }
        if text_offered {
            self.send(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        diag(format!(
            "rdp clipboard session={} format-data request format={} ready={}",
            self.session_id, request.format.0, self.channel_ready
        ));
        if !self.channel_ready {
            self.maybe_reinit();
            return;
        }
        // Backend-enforced text-only: every other format is an error reply,
        // never an empty/implicit success.
        let response = match request.format {
            // A remote may request text before the user ever copied locally;
            // CLIPRDR still expects a valid (empty) Unicode string, not an
            // error, so the paste succeeds with no content.
            ClipboardFormatId::CF_UNICODETEXT => match self.stage.text() {
                Some(text) if validate_clipboard_text(&text).is_ok() => {
                    OwnedFormatDataResponse::new_unicode_string(&text)
                }
                _ => OwnedFormatDataResponse::new_unicode_string(""),
            },
            _ => OwnedFormatDataResponse::new_error(),
        };
        self.send(ClipboardMessage::SendFormatData(response));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        // Any response while the file-list request was pending is that file
        // list (the crate only forwards parse/error failures here). Chain the
        // text pull now so a malformed file-list payload cannot lose text.
        if std::mem::take(&mut self.text_after_file_list) {
            self.send(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
        diag(format!(
            "rdp clipboard session={} format-data response len={} error={}",
            self.session_id,
            response.data().len(),
            response.is_error()
        ));
        if response.is_error() {
            return;
        }
        // C1 entry gate (rdp-security-review): length-check the raw payload
        // BEFORE the UTF-16 → String materialization. 缓解：vendored 层整包
        // 物化不可避免（ironrdp-cliprdr 0.7.0 无流式 API），见
        // rdp-security-review C1 —— 本插件层只能在进入后端回调后尽早拒绝。
        if let Err(error) = validate_clipboard_payload_len(response.data().len()) {
            // Whole-package rejection; the oversized payload is never kept
            // or logged.
            eprintln!("[dbx-plugin-sshx] rdp clipboard inbound rejected: {error}");
            return;
        }
        let Ok(text) = response.to_unicode_string() else {
            return;
        };
        if let Err(error) = validate_clipboard_text(&text) {
            // Whole-package rejection; the oversized payload is never kept
            // or logged.
            eprintln!("[dbx-plugin-sshx] rdp clipboard inbound rejected: {error}");
            return;
        }
        // Explorer advertises the file names as text alongside a file list.
        // Once the native download is on the macOS pasteboard, writing that
        // text would replace the files Finder is about to paste.
        if self.files.pasteboard_holds_download() {
            diag(format!(
                "rdp clipboard session={} inbound text skipped: pasteboard holds downloaded files",
                self.session_id
            ));
            return;
        }
        // Forward to the workbench (the frontend bridges to the OS
        // clipboard); the text only ever rides the local IPC event. Oversized
        // text is split into budget-sized chunks (C2): a single >8 MiB JSON
        // event is silently dropped by the SDK transport.
        let chunks = clipboard_chunks(&text);
        let total = chunks.len();
        diag(format!(
            "rdp clipboard session={} inbound text len={} chunks={}",
            self.session_id,
            text.len(),
            total
        ));
        for (index, chunk) in chunks.iter().enumerate() {
            let mut payload = json!({ "sessionId": self.session_id, "text": chunk });
            if total > 1 {
                payload["chunkIndex"] = Value::from(index);
                payload["chunkTotal"] = Value::from(total);
            }
            if let Err(error) = self.emitter.event("rdp/clipboard", payload) {
                // Never silent (C2): a dropped clipboard event is at least
                // logged. No clipboard text in the log line.
                eprintln!(
                    "[dbx-plugin-sshx] rdp clipboard event dropped: {}",
                    error.message
                );
            }
        }
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        if !self.files.is_ready() {
            return;
        }
        // Outbound file: the stage answers SIZE / clamped RANGE from the
        // staged disk files; anything unknown (directory, index) is an error.
        let response = self.files.file_contents_response(&request);
        self.send(ClipboardMessage::SendFileContentsResponse(response));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        // Inbound file: the stage advances its download and returns the next
        // request (or nothing). Malformed/stale responses are swallowed.
        for message in self
            .files
            .on_file_contents_response(response, &self.emitter)
        {
            self.send(message);
        }
    }

    fn on_format_list_response(&mut self, ok: bool) {
        diag(format!(
            "rdp clipboard session={} format-list response ok={} ready={}",
            self.session_id, ok, self.channel_ready
        ));
        if ok {
            self.file_advertise_retries = 0;
            return;
        }
        // A rejected *initialization* FormatList leaves the channel in
        // Initialization; re-advertise the text format to complete the
        // handshake instead of retrying a file copy the engine would refuse.
        if !self.channel_ready {
            self.maybe_reinit();
            return;
        }
        let Some(descriptors) = self.files.outgoing_descriptors() else {
            return;
        };
        if self.file_advertise_retries == 0 {
            // A remote whose window is momentarily inactive may reject the
            // first file FormatList; retry exactly once, then fail visibly.
            self.file_advertise_retries = 1;
            self.send(ClipboardMessage::SendInitiateFileCopy(descriptors));
            return;
        }
        self.file_advertise_retries = 0;
        self.files.clear_outgoing();
        if let Err(error) = self.emitter.event(
            "rdp/clipboard-files/error",
            json!({
                "sessionId": self.session_id,
                "message": "the remote rejected the file clipboard",
            }),
        ) {
            eprintln!(
                "[dbx-plugin-sshx] rdp clipboard files error event dropped: {}",
                error.message
            );
        }
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        // File metadata arrived; the stage emits `available`, downloads every
        // regular file and finally emits `ready`/`error`. Never panics.
        for message in self
            .files
            .on_remote_file_list(files, clip_data_id, &self.emitter)
        {
            self.send(message);
        }
        // The file-list FormatDataResponse was just correlated; the chained
        // text pull now has the single pending slot to itself.
        if std::mem::take(&mut self.text_after_file_list) {
            self.send(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}
}

/// Factory handed to the vendored client: the client invokes it once per
/// run with its own `ClipboardMessageProxy` (which forwards messages into
/// the client's input queue) and we bind that proxy to the backend.
pub(crate) struct TextClipboardBackendFactory {
    pub(crate) session_id: String,
    pub(crate) stage: Arc<ClipboardStage>,
    pub(crate) files: Arc<FileClipboardStage>,
    pub(crate) emitter: PluginEmitter,
    pub(crate) proxy: Arc<StdMutex<Box<dyn ClipboardMessageProxy>>>,
}

impl CliprdrBackendFactory for TextClipboardBackendFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        // A new CLIPRDR channel starts un-negotiated: files may only be
        // advertised after this channel's capabilities/Ready handshake.
        self.files.reset_channel();
        diag(format!(
            "rdp clipboard session={} backend built",
            self.session_id
        ));
        Box::new(TextClipboardBackend {
            session_id: self.session_id.clone(),
            stage: self.stage.clone(),
            files: self.files.clone(),
            proxy: self.proxy.clone(),
            emitter: self.emitter.clone(),
            text_after_file_list: false,
            file_advertise_retries: 0,
            channel_ready: false,
            reinit_attempts: 0,
            reinit_next_at: None,
        })
    }
}

#[cfg(test)]
mod tests;
