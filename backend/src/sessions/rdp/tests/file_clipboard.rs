//! 文件/文件夹剪贴板状态机单测：纯 stage API，不需要活的 RDP 服务端。
use crate::sessions::rdp::file_clipboard::*;
use crate::sessions::rdp::wire::ClipboardFileEntry;
use dbx_plugin_sdk::{PluginEmitter, PluginTransport};
use ironrdp::cliprdr::backend::ClipboardMessage;
use ironrdp::cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName,
    ClipboardGeneralCapabilityFlags, FileContentsFlags, FileContentsRequest, FileContentsResponse,
    FileDescriptor,
};
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tempfile::TempDir;

#[derive(Clone, Default)]
struct SharedSink(Arc<StdMutex<Vec<u8>>>);

impl Write for SharedSink {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut sink) = self.0.lock() {
            sink.extend_from_slice(buffer);
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl SharedSink {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("sink mutex")).to_string()
    }
}

fn start_download(stage: &FileClipboardStage) -> FileContentsRequest {
    stage
        .start_download()
        .expect("start download")
        .expect("first messaging request")
}

fn enable_file_clipboard(stage: &FileClipboardStage) {
    stage.set_negotiated_capabilities(
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA,
    );
    stage.set_ready();
}

/// Small remote copies auto-download only where the host pasteboard accepts
/// file URLs (macOS, Windows, or Linux with xclip/wl-copy). Headless Linux CI
/// has no clipboard backend, so `on_remote_file_list` stays metadata-only and
/// tests that follow the automatic download stream have nothing to assert.
fn require_auto_pasteboard() -> bool {
    crate::platform::pasteboard::supported()
}

fn test_emitter() -> (PluginEmitter, SharedSink) {
    let sink = SharedSink::default();
    let output: Arc<StdMutex<Box<dyn Write + Send>>> =
        Arc::new(StdMutex::new(Box::new(sink.clone())));
    (
        PluginEmitter::for_tests(output, PluginTransport::JsonLines),
        sink,
    )
}

fn entry(name: &str, relative_path: Option<&str>, size: u64) -> ClipboardFileEntry {
    ClipboardFileEntry {
        name: name.to_string(),
        relative_path: relative_path.map(str::to_string),
        size,
    }
}

fn request_of(message: &ClipboardMessage) -> &FileContentsRequest {
    match message {
        ClipboardMessage::SendFileContentsRequest(request) => request,
        other => panic!("unexpected clipboard message: {other:?}"),
    }
}

fn remote_file(name: &str, relative_path: Option<&str>, size: u64) -> FileDescriptor {
    let descriptor = FileDescriptor::new(name)
        .with_attributes(ClipboardFileAttributes::ARCHIVE)
        .with_file_size(size);
    match relative_path {
        Some(path) => descriptor.with_relative_path(path),
        None => descriptor,
    }
}

fn remote_dir(name: &str) -> FileDescriptor {
    FileDescriptor::new(name).with_attributes(ClipboardFileAttributes::DIRECTORY)
}

// —— 路径校验 ——

#[test]
fn path_validation_rejects_traversal_absolute_and_reserved_names() {
    assert!(validate_file_path("a.txt", None).is_ok());
    assert!(validate_file_path("a.txt", Some("dir/sub")).is_ok());
    assert!(validate_file_path("..", None).is_err());
    assert!(validate_file_path(".", None).is_err());
    assert!(validate_file_path("a.txt", Some("../etc")).is_err());
    assert!(validate_file_path("a.txt", Some("dir/./x")).is_err());
    assert!(validate_file_path("a.txt", Some("dir//x")).is_err());
    assert!(validate_file_path("a.txt", Some("/etc")).is_err());
    assert!(validate_file_path("a.txt", Some("\\etc")).is_err());
    assert!(validate_file_path("a.txt", Some("C:\\Users")).is_err());
    assert!(validate_file_path("", None).is_err());
    assert!(validate_file_path("bad\u{0007}name", None).is_err());
    assert!(validate_file_path("CON", None).is_err());
    assert!(validate_file_path("nul.txt", None).is_err());
    assert!(validate_file_path("LPT9.log", None).is_err());
    assert!(validate_file_path("dir", Some("COM1\\x")).is_err());
    assert!(validate_file_path("name.", None).is_err());
    assert!(validate_file_path("..", None).is_err());
    assert!(validate_file_path("/etc/passwd", None).is_err());
    assert!(validate_file_path("\\etc\\passwd", None).is_err());
    assert!(validate_file_path("a//b", None).is_err());
    assert!(validate_file_path("a/", None).is_err());
}

#[test]
fn path_validation_splits_linux_wire_names_into_relative_paths() {
    // GNOME Remote Desktop (WinPR) sends "folder/file.txt" as one cFileName.
    let (relative, name) = validate_file_path("folder/a.txt", None).expect("nested name");
    assert_eq!(relative.as_deref(), Some("folder"));
    assert_eq!(name, "a.txt");
    let (relative, name) = validate_file_path("folder/sub/a.txt", None).expect("deep nested name");
    assert_eq!(relative.as_deref(), Some("folder\\sub"));
    assert_eq!(name, "a.txt");
    // Windows-style separators in cFileName are treated the same way.
    let (relative, name) = validate_file_path("folder\\a.txt", None).expect("nested name");
    assert_eq!(relative.as_deref(), Some("folder"));
    assert_eq!(name, "a.txt");
    // An explicit relativePath and a nested cFileName combine.
    let (relative, name) = validate_file_path("sub/a.txt", Some("outer")).expect("combined name");
    assert_eq!(relative.as_deref(), Some("outer\\sub"));
    assert_eq!(name, "a.txt");
    // A directory entry keeps its relative path so the tree materializes.
    let (relative, name) = validate_file_path("folder/sub", None).expect("nested dir");
    assert_eq!(relative.as_deref(), Some("folder"));
    assert_eq!(name, "sub");
}

#[test]
fn path_validation_normalizes_separators_and_bounds_the_wire_name() {
    let (relative, name) = validate_file_path("a.txt", Some("dir\\sub")).expect("valid");
    assert_eq!(relative.as_deref(), Some("dir\\sub"));
    assert_eq!(name, "a.txt");
    let (relative, _) = validate_file_path("a.txt", Some("dir/sub")).expect("valid");
    assert_eq!(relative.as_deref(), Some("dir\\sub"));

    // 259 characters is the wire maximum (relative path + '\' + name).
    let prefix = "d".repeat(MAX_CLIPBOARD_WIRE_NAME_CHARS - 1 - "a.txt".len());
    assert!(validate_file_path("a.txt", Some(&prefix)).is_ok());
    let too_long = "d".repeat(MAX_CLIPBOARD_WIRE_NAME_CHARS - "a.txt".len());
    assert!(validate_file_path("a.txt", Some(&too_long)).is_err());
}

// —— 出站上传 ——

#[test]
fn upload_begin_chunk_commit_verifies_sizes() {
    let stage = FileClipboardStage::new("upload-session".to_string());
    let files = vec![entry("a.txt", Some("dir"), 5), entry("b.bin", None, 0)];
    let (upload_id, chunk_bytes) = stage.begin_upload(&files).expect("begin");
    assert_eq!(chunk_bytes, CLIPBOARD_FILE_CHUNK_BYTES);

    assert_eq!(
        stage
            .write_chunk(&upload_id, 0, 0, b"hello")
            .expect("chunk"),
        5
    );
    let (descriptors, total) = stage.commit_upload(&upload_id).expect("commit");
    assert_eq!(total, 5);
    assert_eq!(descriptors.len(), 2);
    assert_eq!(descriptors[0].name, "a.txt");
    assert_eq!(descriptors[0].relative_path.as_deref(), Some("dir"));
    assert_eq!(descriptors[0].file_size, Some(5));
    assert!(descriptors[0]
        .attributes
        .expect("attributes")
        .contains(ClipboardFileAttributes::ARCHIVE));
    assert!(stage.abort_upload(&upload_id));
    stage.clear();
}

#[test]
fn upload_rejects_bad_manifests_and_size_mismatches() {
    let stage = FileClipboardStage::new("upload-session".to_string());
    assert!(stage.begin_upload(&[]).is_err());
    assert!(stage.begin_upload(&[entry("..", None, 1)]).is_err());
    assert!(stage
        .begin_upload(&[entry("a.txt", None, MAX_CLIPBOARD_FILE_BYTES + 1)])
        .is_err());
    let duplicated = vec![entry("a.txt", None, 1), entry("A.TXT", None, 1)];
    assert!(stage.begin_upload(&duplicated).is_err());

    let files = vec![entry("a.txt", None, 5)];
    let (upload_id, _) = stage.begin_upload(&files).expect("begin");
    // Chunk beyond the declared size is rejected up front.
    assert!(stage.write_chunk(&upload_id, 0, 0, b"too-long").is_err());
    assert!(stage.write_chunk("unknown-upload", 0, 0, b"x").is_err());
    // Short file: commit fails and drops the upload.
    stage.write_chunk(&upload_id, 0, 0, b"hel").expect("chunk");
    assert!(stage.commit_upload(&upload_id).is_err());
    assert!(!stage.abort_upload(&upload_id));
    stage.clear();
}

#[test]
fn stale_upload_ids_do_not_destroy_the_active_upload() {
    let stage = FileClipboardStage::new("upload-session".to_string());
    let (upload_id, _) = stage
        .begin_upload(&[entry("a.txt", None, 1)])
        .expect("begin");
    assert!(!stage.abort_upload("other-upload"));
    assert!(stage.commit_upload("other-upload").is_err());
    // The active upload survived both stale calls and still accepts chunks.
    assert_eq!(stage.write_chunk(&upload_id, 0, 0, b"x").expect("chunk"), 1);
    stage.abort_upload(&upload_id);
}

#[test]
fn upload_chunks_over_one_mib_are_rejected() {
    let stage = FileClipboardStage::new("upload-session".to_string());
    let files = vec![entry(
        "big.bin",
        None,
        (CLIPBOARD_FILE_CHUNK_BYTES * 2) as u64,
    )];
    let (upload_id, _) = stage.begin_upload(&files).expect("begin");
    let oversized = vec![0u8; CLIPBOARD_FILE_CHUNK_BYTES + 1];
    assert!(stage.write_chunk(&upload_id, 0, 0, &oversized).is_err());
    stage.abort_upload(&upload_id);
}

// —— 出站文件内容应答 ——

fn staged_file(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("write staged file");
    path
}

#[test]
fn outgoing_size_and_range_responses_are_clamped() {
    let dir = TempDir::new().expect("temp dir");
    let big = vec![7u8; CLIPBOARD_FILE_CHUNK_BYTES + 1024];
    let path = staged_file(&dir, "big.bin", &big);
    let stage = FileClipboardStage::new("outgoing-session".to_string());
    stage.stage_for_tests(
        dir.path().to_path_buf(),
        vec![(path, big.len() as u64, false)],
    );

    let size_response = stage.file_contents_response(&FileContentsRequest {
        stream_id: 1,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    });
    assert!(!size_response.is_error());
    assert_eq!(
        size_response.data_as_size().expect("size"),
        big.len() as u64
    );

    let first = stage.file_contents_response(&FileContentsRequest {
        stream_id: 2,
        index: 0,
        flags: FileContentsFlags::RANGE,
        position: 0,
        requested_size: 4 * 1024 * 1024,
        data_id: None,
    });
    assert!(!first.is_error());
    assert_eq!(first.data().len(), CLIPBOARD_FILE_CHUNK_BYTES);
    assert_eq!(first.data(), &big[..CLIPBOARD_FILE_CHUNK_BYTES]);

    let second = stage.file_contents_response(&FileContentsRequest {
        stream_id: 3,
        index: 0,
        flags: FileContentsFlags::RANGE,
        position: CLIPBOARD_FILE_CHUNK_BYTES as u64,
        requested_size: 4 * 1024 * 1024,
        data_id: None,
    });
    assert_eq!(second.data().len(), 1024);
    assert_eq!(second.data(), &big[CLIPBOARD_FILE_CHUNK_BYTES..]);

    // Position at EOF → error: MS-RDPECLIP expects CB_RESPONSE_FAIL, not an
    // empty success that some servers retry forever.
    let eof = stage.file_contents_response(&FileContentsRequest {
        stream_id: 4,
        index: 0,
        flags: FileContentsFlags::RANGE,
        position: big.len() as u64,
        requested_size: 1024,
        data_id: None,
    });
    assert!(eof.is_error());
}

#[test]
fn outgoing_directory_and_out_of_range_index_return_errors() {
    let dir = TempDir::new().expect("temp dir");
    let path = staged_file(&dir, "a.txt", b"abc");
    let stage = FileClipboardStage::new("outgoing-session".to_string());
    stage.stage_for_tests(
        dir.path().to_path_buf(),
        vec![(path, 3, false), (dir.path().to_path_buf(), 0, true)],
    );

    let directory = stage.file_contents_response(&FileContentsRequest {
        stream_id: 1,
        index: 1,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    });
    assert!(directory.is_error());

    let out_of_range = stage.file_contents_response(&FileContentsRequest {
        stream_id: 2,
        index: 9,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    });
    assert!(out_of_range.is_error());

    // No staged files at all → error, never a panic.
    let empty = FileClipboardStage::new("outgoing-session".to_string());
    let response = empty.file_contents_response(&FileContentsRequest {
        stream_id: 3,
        index: 0,
        flags: FileContentsFlags::SIZE,
        position: 0,
        requested_size: 8,
        data_id: None,
    });
    assert!(response.is_error());
}

// —— 入站 ——

#[test]
fn remote_file_list_caps_are_enforced() {
    let (emitter, sink) = test_emitter();
    let stage = FileClipboardStage::new("incoming-session".to_string());
    enable_file_clipboard(&stage);

    let too_many: Vec<FileDescriptor> = (0..=MAX_CLIPBOARD_FILES)
        .map(|index| remote_file(&format!("f{index}.txt"), None, 1))
        .collect();
    assert!(stage
        .on_remote_file_list(&too_many, None, &emitter)
        .is_empty());
    assert!(sink.text().contains("rdp/clipboard-files/error"));
    assert!(!stage.status()["downloading"].as_bool().unwrap_or(true));
    assert!(stage.status()["error"].is_string());

    let oversized = vec![remote_file("huge.bin", None, MAX_CLIPBOARD_FILE_BYTES + 1)];
    assert!(stage
        .on_remote_file_list(&oversized, None, &emitter)
        .is_empty());
    assert!(stage.status()["error"].is_string());

    let total: Vec<FileDescriptor> = (0..2)
        .map(|index| remote_file(&format!("f{index}.bin"), None, MAX_CLIPBOARD_FILE_BYTES))
        .collect();
    assert!(stage.on_remote_file_list(&total, None, &emitter).is_empty());
    assert!(stage.status()["error"].is_string());
    stage.clear();
}

#[test]
fn remote_file_list_downloads_and_zips_relative_paths() {
    if !require_auto_pasteboard() {
        return;
    }
    use base64::Engine as _;

    let (emitter, sink) = test_emitter();
    let stage = FileClipboardStage::new("incoming-session".to_string());
    enable_file_clipboard(&stage);
    let files = vec![
        remote_dir("sub"),
        remote_file("a.txt", None, 5),
        remote_file("b.txt", Some("sub"), 3),
    ];

    // Small remote copies (≤8 files / ≤32 MiB) auto-download so they can be
    // placed on the macOS pasteboard; the first SIZE request comes back here.
    let messages = stage.on_remote_file_list(&files, Some(7), &emitter);
    assert_eq!(messages.len(), 1);
    let dir = stage.remote_dir_for_tests().expect("remote dir");
    assert!(dir.exists());
    assert_eq!(stage.status()["downloading"], Value::Bool(true));
    let size_request = request_of(&messages[0]);
    assert_eq!(size_request.index, 1);
    assert_eq!(size_request.flags, FileContentsFlags::SIZE);
    assert_eq!(size_request.data_id, Some(7));
    let first_stream = size_request.stream_id;

    let messages = stage.on_file_contents_response(
        FileContentsResponse::new_size_response(first_stream, 5),
        &emitter,
    );
    assert_eq!(messages.len(), 1);
    let range = request_of(&messages[0]);
    assert_eq!(range.index, 1);
    assert_eq!(range.flags, FileContentsFlags::RANGE);
    assert_eq!(range.position, 0);
    assert_eq!(range.requested_size, 5);

    let messages = stage.on_file_contents_response(
        FileContentsResponse::new_data_response(range.stream_id, b"hello".to_vec()),
        &emitter,
    );
    assert_eq!(messages.len(), 1);
    let size_request = request_of(&messages[0]);
    assert_eq!(size_request.index, 2);
    assert_eq!(size_request.flags, FileContentsFlags::SIZE);

    let messages = stage.on_file_contents_response(
        FileContentsResponse::new_size_response(size_request.stream_id, 3),
        &emitter,
    );
    assert_eq!(messages.len(), 1);
    let range = request_of(&messages[0]);
    assert_eq!(range.index, 2);
    assert_eq!(range.position, 0);

    let messages = stage.on_file_contents_response(
        FileContentsResponse::new_data_response(range.stream_id, b"abc".to_vec()),
        &emitter,
    );
    assert!(messages.is_empty());

    let status = stage.status();
    assert_eq!(status["completed"], Value::Bool(true));
    // Tests never touch the developer clipboard, so the live revalidation is false.
    assert_eq!(status["pasteboard"], Value::Bool(false));
    // The zip is built on demand: the raw files stay for the pasteboard.
    assert_eq!(status["archiveReady"], Value::Bool(false));
    assert_eq!(status["downloading"], Value::Bool(false));
    assert_eq!(status["filesDone"], serde_json::json!(2));
    assert_eq!(status["bytesDone"], serde_json::json!(8));
    assert_eq!(status["remoteTotalBytes"], serde_json::json!(8));
    assert_eq!(status["remoteFiles"].as_array().map(Vec::len), Some(3));
    let built = stage.build_archive().expect("build archive");
    assert_eq!(stage.status()["archiveReady"], Value::Bool(true));
    let archive_name = built["archiveName"].as_str().expect("archive name");
    assert!(archive_name.starts_with("remote-clipboard-"));
    assert!(archive_name.ends_with(".zip"));
    let archive_size = built["archiveSize"].as_u64().expect("archive size");
    assert!(archive_size > 0);

    let (data, eof, total, _archive) = stage.read_archive(0, 10).expect("read archive");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .expect("base64");
    assert_eq!(bytes.len(), 10);
    assert!(!eof);
    assert_eq!(total, archive_size);

    // Stream the remaining archive and inspect the zip entries.
    let mut archive_bytes = bytes;
    let mut offset = 10u64;
    loop {
        let (data, eof, _, _) = stage.read_archive(offset, 1024).expect("continue");
        let chunk = base64::engine::general_purpose::STANDARD
            .decode(data)
            .expect("base64");
        offset += chunk.len() as u64;
        archive_bytes.extend_from_slice(&chunk);
        if eof {
            break;
        }
    }
    assert_eq!(archive_bytes.len() as u64, archive_size);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive_bytes)).expect("zip");
    assert_eq!(zip.len(), 2);
    let mut names: Vec<String> = (0..zip.len())
        .map(|index| zip.by_index(index).expect("entry").name().to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["a.txt".to_string(), "sub/b.txt".to_string()]);
    let mut content = String::new();
    {
        let mut entry = zip.by_name("sub/b.txt").expect("b");
        std::io::Read::read_to_string(&mut entry, &mut content).expect("read entry");
    }
    assert_eq!(content, "abc");

    assert!(sink.text().contains("rdp/clipboard-files/available"));
    assert!(sink.text().contains("rdp/clipboard-files/ready"));
    assert!(sink.text().contains("rdp/clipboard-files/progress"));

    stage.clear();
    assert_eq!(stage.status()["archiveReady"], Value::Bool(false));
    assert!(stage.read_archive(0, 10).is_err());
}

#[test]
fn linux_nested_wire_names_keep_their_folder_tree() {
    // GNOME Remote Desktop via WinPR encodes nested paths in cFileName with
    // '/' separators (e.g. "sub/deep/c.txt") instead of a relativePath field.
    let (emitter, _sink) = test_emitter();
    let stage = FileClipboardStage::new("linux-nested".to_string());
    enable_file_clipboard(&stage);
    let files = vec![
        remote_dir("sub"),
        remote_dir("sub/deep"),
        remote_file("sub/b.txt", None, 3),
        remote_file("sub/deep/c.txt", None, 2),
    ];

    let _ = stage.on_remote_file_list(&files, None, &emitter);
    let status = stage.status();
    let rows = status["remoteFiles"].as_array().expect("rows");
    assert!(rows.iter().any(|row| {
        row["name"].as_str() == Some("deep")
            && row["relativePath"].as_str() == Some("sub")
            && row["directory"] == Value::Bool(true)
    }));
    assert!(rows.iter().any(|row| {
        row["name"].as_str() == Some("b.txt")
            && row["relativePath"].as_str() == Some("sub")
            && row["directory"] == Value::Bool(false)
    }));
    assert!(rows.iter().any(|row| {
        row["name"].as_str() == Some("c.txt")
            && row["relativePath"].as_str() == Some("sub\\deep")
            && row["size"].as_u64() == Some(2)
    }));

    stage.clear();
}

#[test]
fn malformed_file_contents_responses_do_not_panic() {
    if !require_auto_pasteboard() {
        return;
    }
    let (emitter, sink) = test_emitter();
    let stage = FileClipboardStage::new("incoming-session".to_string());
    enable_file_clipboard(&stage);
    let files = vec![remote_file("a.txt", None, 4)];
    let messages = stage.on_remote_file_list(&files, None, &emitter);
    assert_eq!(messages.len(), 1);
    let stream = request_of(&messages[0]).stream_id;

    // Unknown/stale stream id → ignored.
    assert!(stage
        .on_file_contents_response(
            FileContentsResponse::new_data_response(stream.wrapping_add(99), vec![1, 2, 3, 4]),
            &emitter,
        )
        .is_empty());
    assert_eq!(stage.status()["downloading"], Value::Bool(true));

    // Malformed SIZE payload (not 8 bytes) → error event, transfer stops.
    assert!(stage
        .on_file_contents_response(
            FileContentsResponse::new_data_response(stream, vec![1, 2, 3]),
            &emitter,
        )
        .is_empty());
    assert!(stage.status()["error"].is_string());
    assert!(sink.text().contains("rdp/clipboard-files/error"));

    // Further responses are stale and still must not panic.
    assert!(stage
        .on_file_contents_response(FileContentsResponse::new_error(stream), &emitter,)
        .is_empty());
    assert!(stage
        .on_file_contents_response(
            FileContentsResponse::new_data_response(u32::MAX, vec![]),
            &emitter,
        )
        .is_empty());
    stage.clear();
}

#[test]
fn failed_download_can_be_retried_without_a_stale_directory() {
    if !require_auto_pasteboard() {
        return;
    }
    let (emitter, _sink) = test_emitter();
    let stage = FileClipboardStage::new("retry-session".to_string());
    enable_file_clipboard(&stage);
    let files = vec![remote_file("a.txt", None, 3)];
    let messages = stage.on_remote_file_list(&files, None, &emitter);
    assert_eq!(messages.len(), 1);
    let dir = stage.remote_dir_for_tests().expect("remote dir");

    let first = request_of(&messages[0]);
    assert!(dir.exists());
    assert!(stage
        .on_file_contents_response(FileContentsResponse::new_error(first.stream_id), &emitter)
        .is_empty());
    assert_eq!(stage.status()["downloading"], Value::Bool(false));

    // A retry gets a fresh SIZE request and a recreated directory.
    let retry = start_download(&stage);
    assert_eq!(retry.flags, FileContentsFlags::SIZE);
    let messages = stage.on_file_contents_response(
        FileContentsResponse::new_size_response(retry.stream_id, 3),
        &emitter,
    );
    let range = request_of(&messages[0]);
    assert!(stage
        .on_file_contents_response(
            FileContentsResponse::new_data_response(range.stream_id, b"abc".to_vec()),
            &emitter,
        )
        .is_empty());
    assert_eq!(stage.status()["completed"], Value::Bool(true));
    stage.build_archive().expect("archive");
    assert_eq!(stage.status()["archiveReady"], Value::Bool(true));
    stage.clear();
}

#[test]
fn remote_error_and_oversized_range_responses_fail_safely() {
    if !require_auto_pasteboard() {
        return;
    }
    let (emitter, _sink) = test_emitter();
    let stage = FileClipboardStage::new("incoming-session".to_string());
    enable_file_clipboard(&stage);
    let files = vec![remote_file("a.txt", None, 4)];
    let messages = stage.on_remote_file_list(&files, None, &emitter);
    assert_eq!(messages.len(), 1);
    let stream = request_of(&messages[0]).stream_id;
    assert!(stage
        .on_file_contents_response(FileContentsResponse::new_error(stream), &emitter)
        .is_empty());
    assert!(stage.status()["error"].is_string());

    // Range larger than the advertised file size → error, no partial archive.
    let messages = stage.on_remote_file_list(&files, None, &emitter);
    assert_eq!(messages.len(), 1);
    let size_stream = request_of(&messages[0]).stream_id;
    let messages = stage.on_file_contents_response(
        FileContentsResponse::new_size_response(size_stream, 4),
        &emitter,
    );
    let range_stream = request_of(&messages[0]).stream_id;
    assert!(stage
        .on_file_contents_response(
            FileContentsResponse::new_data_response(range_stream, vec![0u8; 8]),
            &emitter,
        )
        .is_empty());
    assert_eq!(stage.status()["archiveReady"], Value::Bool(false));
    assert!(stage.status()["error"].is_string());
    stage.clear();
}

#[test]
fn file_copy_gate_requires_negotiated_stream_and_ready() {
    let stage = FileClipboardStage::new("gate-session".to_string());
    assert!(!stage.supports_file_copy());
    stage.set_negotiated_capabilities(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED);
    // Capabilities alone are not enough: the channel must also be Ready.
    assert!(!stage.supports_file_copy());
    stage.set_ready();
    assert!(stage.supports_file_copy());
    // A fresh CLIPRDR channel (reconnect) starts un-negotiated again.
    stage.reset_channel();
    assert!(!stage.supports_file_copy());
}

#[test]
fn note_remote_copy_remembers_the_file_list_format() {
    let stage = FileClipboardStage::new("incoming-session".to_string());
    let file_list = ClipboardFormatId::new(0xC0FE);
    assert_eq!(
        stage.note_remote_copy(&[ClipboardFormat::new(file_list)]),
        None
    );
    assert_eq!(
        stage.note_remote_copy(&[
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
            ClipboardFormat::new(file_list).with_name(ClipboardFormatName::FILE_LIST),
        ]),
        Some(file_list)
    );
}

#[test]
fn large_remote_file_lists_wait_for_an_explicit_download() {
    let (emitter, _sink) = test_emitter();
    let stage = FileClipboardStage::new("large-remote".to_string());
    enable_file_clipboard(&stage);
    let files: Vec<FileDescriptor> = (0..=super::AUTO_PASTEBOARD_FILES)
        .map(|index| remote_file(&format!("f{index}.txt"), None, 1))
        .collect();
    let messages = stage.on_remote_file_list(&files, None, &emitter);
    assert!(messages.is_empty());
    assert_eq!(stage.status()["downloading"], Value::Bool(false));
    let dir = stage.remote_dir_for_tests().expect("remote dir");
    assert!(!dir.exists());
    let request = start_download(&stage);
    assert_eq!(request.flags, FileContentsFlags::SIZE);
    stage.clear();
}

#[test]
fn plugin_temp_paths_are_recognized_after_canonicalization() {
    let root = std::env::temp_dir().join("dbx-sshx-rdp-clipboard");
    std::fs::create_dir_all(&root).expect("temp root");
    let dir = root.join(format!("canon-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    assert!(super::is_plugin_temp_path(&dir));
    assert!(!super::is_plugin_temp_path(&std::env::temp_dir()));
    let _ = std::fs::remove_dir_all(&dir);
}
