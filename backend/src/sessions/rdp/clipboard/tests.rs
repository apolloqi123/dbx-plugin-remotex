use super::*;
use dbx_plugin_sdk::PluginTransport;
use ironrdp::cliprdr::pdu::FormatDataResponse;

/// Captures what the backend hands to the client proxy so tests can
/// assert on the CLIPRDR messages without a live channel.
#[derive(Debug, Clone, Default)]
struct CapturingProxy {
    messages: Arc<StdMutex<Vec<ClipboardMessage>>>,
}

impl ClipboardMessageProxy for CapturingProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        self.messages.lock().expect("proxy mutex").push(message);
    }
}

fn test_emitter() -> PluginEmitter {
    let output: Arc<StdMutex<Box<dyn std::io::Write + Send>>> =
        Arc::new(StdMutex::new(Box::new(Vec::<u8>::new())));
    PluginEmitter::for_tests(output, PluginTransport::JsonLines)
}

fn enable_files(backend: &TextClipboardBackend) {
    backend.files.set_negotiated_capabilities(
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA,
    );
    backend.files.set_ready();
}

fn test_backend(
    stage: Arc<ClipboardStage>,
) -> (TextClipboardBackend, Arc<StdMutex<Vec<ClipboardMessage>>>) {
    let proxy = CapturingProxy::default();
    let messages = proxy.messages.clone();
    let files = Arc::new(FileClipboardStage::new("s1".to_string()));
    // Text callbacks are only valid after Ready; file tests additionally
    // negotiate STREAM_FILECLIP_ENABLED via `enable_files`.
    files.set_ready();
    let backend = TextClipboardBackend {
        session_id: "s1".to_string(),
        stage,
        files,
        proxy: Arc::new(StdMutex::new(
            Box::new(proxy) as Box<dyn ClipboardMessageProxy>
        )),
        emitter: test_emitter(),
        text_after_file_list: false,
        file_advertise_retries: 0,
        channel_ready: true,
        reinit_attempts: 0,
        reinit_next_at: None,
    };
    (backend, messages)
}

fn only_format_data_response(
    messages: &Arc<StdMutex<Vec<ClipboardMessage>>>,
) -> FormatDataResponse<'static> {
    let messages = messages.lock().expect("messages mutex");
    assert_eq!(messages.len(), 1, "{messages:?}");
    match &messages[0] {
        ClipboardMessage::SendFormatData(response) => response.clone(),
        other => panic!("unexpected clipboard message: {other:?}"),
    }
}

#[test]
fn request_format_list_always_advertises_unicode_text() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    backend.on_request_format_list();
    let messages = messages.lock().expect("messages mutex");
    assert_eq!(messages.len(), 1, "{messages:?}");
    match &messages[0] {
        ClipboardMessage::SendInitiateCopy(formats) => {
            assert_eq!(formats.len(), 1);
            assert_eq!(formats[0].id, ClipboardFormatId::CF_UNICODETEXT);
        }
        other => panic!("unexpected clipboard message: {other:?}"),
    }
}

#[test]
fn ready_does_not_re_advertise_without_staged_text() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    backend.on_ready();
    assert!(messages.lock().expect("messages mutex").is_empty());
}

#[test]
fn ready_re_advertises_text_staged_before_the_channel_joined() {
    let stage = Arc::new(ClipboardStage::default());
    stage.set_text("staged".to_string());
    let (mut backend, messages) = test_backend(stage);
    backend.channel_ready = false;
    backend.on_ready();
    let messages = messages.lock().expect("messages mutex");
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(matches!(messages[0], ClipboardMessage::SendInitiateCopy(_)));
}

#[test]
fn remote_copy_before_ready_reinitializes_once_per_interval() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    backend.channel_ready = false;
    backend.files.reset_channel();
    let formats = [ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)];
    backend.on_remote_copy(&formats);
    backend.on_remote_copy(&formats);
    let messages = messages.lock().expect("messages mutex");
    // initiate_copy in Initialization bundles Capabilities, Temporary
    // Directory and FormatList, but the test proxy sees one message.
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(matches!(messages[0], ClipboardMessage::SendInitiateCopy(_)));
}

#[test]
fn format_data_request_before_ready_reinitializes_instead_of_dropping() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    backend.channel_ready = false;
    backend.on_format_data_request(FormatDataRequest {
        format: ClipboardFormatId::CF_UNICODETEXT,
    });
    let messages = messages.lock().expect("messages mutex");
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(matches!(messages[0], ClipboardMessage::SendInitiateCopy(_)));
}

#[test]
fn rejected_initial_format_list_reinitializes_text() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    backend.channel_ready = false;
    backend.on_format_list_response(false);
    let messages = messages.lock().expect("messages mutex");
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(matches!(messages[0], ClipboardMessage::SendInitiateCopy(_)));
}

#[test]
fn format_data_request_without_staged_text_returns_empty_unicode_string() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    backend.on_format_data_request(FormatDataRequest {
        format: ClipboardFormatId::CF_UNICODETEXT,
    });
    let response = only_format_data_response(&messages);
    assert!(!response.is_error());
    assert_eq!(response.to_unicode_string().expect("valid unicode"), "");
}

#[test]
fn format_data_request_with_staged_text_returns_that_text() {
    let stage = Arc::new(ClipboardStage::default());
    stage.set_text("hello RDP 剪贴板".to_string());
    let (mut backend, messages) = test_backend(stage);
    backend.on_format_data_request(FormatDataRequest {
        format: ClipboardFormatId::CF_UNICODETEXT,
    });
    let response = only_format_data_response(&messages);
    assert!(!response.is_error());
    assert_eq!(
        response.to_unicode_string().expect("valid unicode"),
        "hello RDP 剪贴板"
    );
}

#[test]
fn format_data_request_for_other_format_is_an_error() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    backend.on_format_data_request(FormatDataRequest {
        format: ClipboardFormatId::CF_TEXT,
    });
    let response = only_format_data_response(&messages);
    assert!(response.is_error());
}

#[test]
fn remote_copy_requests_only_unicode_text() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    backend.on_remote_copy(&[
        ClipboardFormat::new(ClipboardFormatId::CF_TEXT),
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
    ]);
    let messages = messages.lock().expect("messages mutex");
    assert_eq!(messages.len(), 1, "{messages:?}");
    match &messages[0] {
        ClipboardMessage::SendInitiatePaste(format) => {
            assert_eq!(*format, ClipboardFormatId::CF_UNICODETEXT);
        }
        other => panic!("unexpected clipboard message: {other:?}"),
    }
}

#[test]
fn file_clipboard_capabilities_are_advertised() {
    let (backend, _) = test_backend(Arc::new(ClipboardStage::default()));
    let flags = backend.client_capabilities();
    assert!(flags.contains(ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES));
    assert!(flags.contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED));
    assert!(flags.contains(ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS));
    assert!(flags.contains(ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA));
}

#[test]
fn remote_copy_with_file_list_pastes_only_the_file_list_first() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    enable_files(&backend);
    let file_list_id = ClipboardFormatId::new(0xC0FE);
    backend.on_remote_copy(&[
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ClipboardFormat::new(file_list_id)
            .with_name(ironrdp::cliprdr::pdu::ClipboardFormatName::FILE_LIST),
    ]);
    let messages = messages.lock().expect("messages mutex");
    let pasted: Vec<ClipboardFormatId> = messages
        .iter()
        .filter_map(|message| match message {
            ClipboardMessage::SendInitiatePaste(format) => Some(*format),
            _ => None,
        })
        .collect();
    assert_eq!(pasted, vec![file_list_id], "{messages:?}");
}

#[test]
fn file_list_metadata_chains_the_text_paste() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    enable_files(&backend);
    let file_list_id = ClipboardFormatId::new(0xC0FE);
    backend.on_remote_copy(&[
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ClipboardFormat::new(file_list_id)
            .with_name(ironrdp::cliprdr::pdu::ClipboardFormatName::FILE_LIST),
    ]);
    messages.lock().expect("messages mutex").clear();
    backend.on_remote_file_list(
        &[ironrdp::cliprdr::pdu::FileDescriptor::new("sub")
            .with_attributes(ironrdp::cliprdr::pdu::ClipboardFileAttributes::DIRECTORY)],
        None,
    );
    let messages = messages.lock().expect("messages mutex");
    let pasted: Vec<ClipboardFormatId> = messages
        .iter()
        .filter_map(|message| match message {
            ClipboardMessage::SendInitiatePaste(format) => Some(*format),
            _ => None,
        })
        .collect();
    assert_eq!(
        pasted,
        vec![ClipboardFormatId::CF_UNICODETEXT],
        "{messages:?}"
    );
}

#[test]
fn file_list_is_ignored_without_negotiated_stream_clipboard() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    let file_list_id = ClipboardFormatId::new(0xC0FE);
    backend.on_remote_copy(&[
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ClipboardFormat::new(file_list_id)
            .with_name(ironrdp::cliprdr::pdu::ClipboardFormatName::FILE_LIST),
    ]);
    let messages = messages.lock().expect("messages mutex");
    let pasted: Vec<ClipboardFormatId> = messages
        .iter()
        .filter_map(|message| match message {
            ClipboardMessage::SendInitiatePaste(format) => Some(*format),
            _ => None,
        })
        .collect();
    // No STREAM_FILECLIP_ENABLED: never request file metadata; text wins.
    assert_eq!(
        pasted,
        vec![ClipboardFormatId::CF_UNICODETEXT],
        "{messages:?}"
    );
}

#[test]
fn malformed_file_list_response_still_requests_text() {
    let (mut backend, messages) = test_backend(Arc::new(ClipboardStage::default()));
    enable_files(&backend);
    let file_list_id = ClipboardFormatId::new(0xC0FE);
    backend.on_remote_copy(&[
        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ClipboardFormat::new(file_list_id)
            .with_name(ironrdp::cliprdr::pdu::ClipboardFormatName::FILE_LIST),
    ]);
    messages.lock().expect("messages mutex").clear();
    backend.on_format_data_response(FormatDataResponse::new_data(vec![0xff]));
    let messages = messages.lock().expect("messages mutex");
    let pasted: Vec<ClipboardFormatId> = messages
        .iter()
        .filter_map(|message| match message {
            ClipboardMessage::SendInitiatePaste(format) => Some(*format),
            _ => None,
        })
        .collect();
    assert_eq!(
        pasted,
        vec![ClipboardFormatId::CF_UNICODETEXT],
        "{messages:?}"
    );
}
