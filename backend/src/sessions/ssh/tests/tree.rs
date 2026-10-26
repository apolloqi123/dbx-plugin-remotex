use super::*;

#[test]
fn classify_entry_kind_maps_wire_types() {
    // Directories stay directories; only `kind === "directory"` renders a
    // folder icon in the UI.
    assert_eq!(classify_entry_kind(FileType::Dir), "directory");
    assert_eq!(classify_entry_kind(FileType::File), "file");
    assert_eq!(classify_entry_kind(FileType::Symlink), "symlink");
    // Missing/unusable type bits (no PERMISSIONS flag, or permissions
    // without S_IFMT bits) must degrade to a file icon, never a folder.
    assert_eq!(classify_entry_kind(FileType::Other), "file");
}

/// Multi-chunk folder download over the local-staging (gzip) lane: the tree
/// state is shared across chunk calls, so the pump must advance the queue,
/// switch files, rename each `.part` and still report aggregate offsets
/// without copying the remaining tree per chunk.
#[tokio::test]
async fn tree_download_streams_multiple_files_across_chunks() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let root = data_dir.path().join("tree-root");
    std::fs::create_dir_all(root.join("sub")).expect("root");
    let big: Vec<u8> = (0..TRANSFER_CHUNK_SIZE + 1024)
        .map(|index| (index % 251) as u8)
        .collect();
    let small = b"tail-bytes".to_vec();
    std::fs::write(root.join("big.bin"), &big).expect("big");
    std::fs::write(root.join("sub/small.bin"), &small).expect("small");
    std::fs::write(root.join("empty.bin"), b"").expect("empty");
    let files = vec![
        sftp_tree::TreeFile {
            relative: "big.bin".to_string(),
            remote_path: "/remote/big.bin".to_string(),
            size: big.len() as u64,
        },
        sftp_tree::TreeFile {
            relative: "sub/small.bin".to_string(),
            remote_path: "/remote/sub/small.bin".to_string(),
            size: small.len() as u64,
        },
        sftp_tree::TreeFile {
            relative: "empty.bin".to_string(),
            remote_path: "/remote/empty.bin".to_string(),
            size: 0,
        },
    ];
    let total = big.len() as u64 + small.len() as u64;
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let task_id = "t-tree-chunks";
    runtime.downloads.lock().unwrap().insert(
        task_id.to_string(),
        DownloadState {
            session_id: "s-tree".to_string(),
            remote_path: "/remote".to_string(),
            file_name: "tree-root".to_string(),
            size: total,
            next_offset: 0,
            sink: None,
            tree: Some(Arc::new(TreeDownloadState::new(
                root.clone(),
                3,
                0,
                false,
                TreeDownloadProgress {
                    files: VecDeque::from(files),
                    current: None,
                    current_offset: 0,
                    sink: None,
                    files_done: 0,
                    failures: Vec::new(),
                    read_handle: None,
                    handle_path: None,
                },
            ))),
            sudo_tmp: None,
            latin1: false,
            throttle: Throttle::new(0),
            progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
            compression: CompressionMode::Gzip,
            ready: true,
            serving_plain: false,
            staged_plain: None,
            gz_staging: None,
            remote_temp: None,
            final_dir: None,
            overwrite: false,
            read_handle: Arc::new(AsyncMutex::new(None)),
            raw_client: Arc::new(AsyncMutex::new(None)),
        },
    );
    let (emitter, sink) = framed_transfer_emitter();
    let mut offset = 0_u64;
    let mut calls = 0_u32;
    loop {
        let response = runtime
            .download_chunk(task_id, offset, &emitter)
            .await
            .expect("tree chunk");
        calls += 1;
        let length = response["length"].as_u64().expect("length");
        assert_eq!(response["offset"].as_u64(), Some(offset));
        offset += length;
        if response["eof"].as_bool().expect("eof") {
            break;
        }
        assert!(calls < 10, "tree chunk loop did not terminate: {response}");
    }
    assert_eq!(offset, total, "aggregate offset must cover every byte");
    assert!(
        calls >= 3,
        "300 KiB + tail files must take at least three chunk calls"
    );

    let messages = framed_messages(&sink);
    assert!(
        messages.iter().any(|message| matches!(
            message,
            FramedMessage::Json(value) if value["method"] == "sftp/transfer/progress"
        )),
        "tree chunk progress events must still be emitted"
    );
    let binary: Vec<Vec<u8>> = messages
        .into_iter()
        .filter_map(|message| match message {
            FramedMessage::Binary { channel, data }
                if channel == format!("sftp/download/{task_id}") =>
            {
                Some(data)
            }
            _ => None,
        })
        .collect();
    assert_eq!(binary.len(), calls as usize);
    let mut streamed = Vec::new();
    for (index, frame) in binary.iter().enumerate() {
        let frame_offset = u64::from_be_bytes(frame[..8].try_into().expect("frame offset"));
        let expected_offset = streamed.len() as u64;
        assert_eq!(
            frame_offset, expected_offset,
            "binary frame {index} carries a stale offset"
        );
        streamed.extend_from_slice(&frame[8..]);
    }
    assert_eq!(streamed.len() as u64, total);
    assert_eq!(&streamed[..big.len()], big.as_slice());
    assert_eq!(&streamed[big.len()..], small.as_slice());

    // Every file landed under its mirrored relative path and the shared tree
    // state has no queued/current file left.
    assert_eq!(std::fs::read(root.join("big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(root.join("sub/small.bin")).unwrap(), small);
    assert_eq!(std::fs::read(root.join("empty.bin")).unwrap(), b"");
    let state = runtime.downloads.lock().unwrap().remove(task_id).unwrap();
    let progress = state.tree.as_ref().unwrap().progress.lock().await;
    assert_eq!(progress.files_done, 3);
    assert!(progress.files.is_empty());
    assert!(progress.current.is_none());
    assert!(progress.failures.is_empty());
    assert!(progress.read_handle.is_none());
    assert!(progress.handle_path.is_none());
    drop(progress);
    assert_eq!(state.next_offset, total);
}

/// Cancelling a folder download tears down the shared tree, releases the
/// cached read handle and still emits the regular cancellation event.
#[tokio::test]
async fn cancel_tree_download_removes_partial_root_and_reports_cancel() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let root = data_dir.path().join("partial-tree");
    std::fs::create_dir_all(root.join("sub")).expect("root");
    std::fs::write(root.join("sub/file.bin"), b"partial").expect("partial");
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let task_id = "t-tree-cancel";
    // An already-open handle must be released by cancel even though the task
    // still has an in-flight file (the pump moves the handle out of the
    // progress lock while reading).
    let open_handle = tokio::fs::File::open(root.join("sub/file.bin"))
        .await
        .expect("open cached handle");
    let tree_state = Arc::new(TreeDownloadState::new(
        root.clone(),
        1,
        0,
        false,
        TreeDownloadProgress {
            files: VecDeque::new(),
            current: Some(sftp_tree::TreeFile {
                relative: "sub/file.bin".to_string(),
                remote_path: "/remote/sub/file.bin".to_string(),
                size: 7,
            }),
            current_offset: 3,
            sink: None,
            files_done: 0,
            failures: Vec::new(),
            read_handle: Some(Box::new(open_handle)),
            handle_path: Some("/remote/sub/file.bin".to_string()),
        },
    ));
    runtime.downloads.lock().unwrap().insert(
        task_id.to_string(),
        DownloadState {
            session_id: "s-tree".to_string(),
            remote_path: "/remote".to_string(),
            file_name: "partial-tree".to_string(),
            size: 7,
            next_offset: 3,
            sink: None,
            tree: Some(tree_state.clone()),
            sudo_tmp: None,
            latin1: false,
            throttle: Throttle::new(0),
            progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
            compression: CompressionMode::None,
            ready: true,
            serving_plain: false,
            staged_plain: None,
            gz_staging: None,
            remote_temp: None,
            final_dir: None,
            overwrite: false,
            read_handle: Arc::new(AsyncMutex::new(None)),
            raw_client: Arc::new(AsyncMutex::new(None)),
        },
    );
    let (emitter, sink) = transfer_event_emitter();
    runtime
        .cancel_transfer(task_id, Some("user"), &emitter)
        .await
        .expect("tree cancel");
    assert!(
        !root.exists(),
        "the partial tree root must be removed on cancel"
    );
    let events = emitted_events(&sink);
    let cancel = events
        .iter()
        .find(|event| {
            event["method"] == "sftp/transfer/progress"
                && event["params"]["taskId"] == task_id
                && event["params"]["status"] == "cancelled"
        })
        .expect("cancellation event");
    assert_eq!(cancel["params"]["fileName"], "partial-tree");
    assert_eq!(cancel["params"]["transferred"], 3);
    let progress = tree_state.progress.lock().await;
    assert!(
        progress.read_handle.is_none() && progress.handle_path.is_none(),
        "cancel must release a cached read handle"
    );
}

/// Non-gzip tree over the high-level lane (local test source): the handle is
/// reused across chunks of the same file and reopened only on file switch.
#[tokio::test]
async fn tree_download_reuses_and_reopens_high_level_handles() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let source = data_dir.path().join("source");
    let root = data_dir.path().join("tree-root");
    std::fs::create_dir_all(source.join("sub")).expect("source");
    std::fs::create_dir_all(&root).expect("root");
    let big: Vec<u8> = (0..TRANSFER_CHUNK_SIZE + 1024)
        .map(|index| (index % 253) as u8)
        .collect();
    let small = b"tail-bytes".to_vec();
    std::fs::write(source.join("big.bin"), &big).expect("big");
    std::fs::write(source.join("sub/small.bin"), &small).expect("small");
    let files = vec![
        sftp_tree::TreeFile {
            relative: "big.bin".to_string(),
            remote_path: "/remote/big.bin".to_string(),
            size: big.len() as u64,
        },
        sftp_tree::TreeFile {
            relative: "sub/small.bin".to_string(),
            remote_path: "/remote/sub/small.bin".to_string(),
            size: small.len() as u64,
        },
        sftp_tree::TreeFile {
            relative: "empty.bin".to_string(),
            remote_path: "/remote/empty.bin".to_string(),
            size: 0,
        },
    ];
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let task_id = "t-tree-handles";
    runtime.downloads.lock().unwrap().insert(
        task_id.to_string(),
        tree_chunk_state(
            root.clone(),
            files,
            CompressionMode::None,
            Some(source.clone()),
        ),
    );
    let state = runtime
        .downloads
        .lock()
        .unwrap()
        .get(task_id)
        .cloned()
        .unwrap();
    let tree = state.tree.clone().unwrap();
    let (emitter, sink) = framed_transfer_emitter();

    // Chunk 1: open + first 300 KiB. The handle stays cached for the file.
    let response = runtime
        .download_chunk(task_id, 0, &emitter)
        .await
        .expect("first chunk");
    assert_eq!(
        response["length"].as_u64(),
        Some(TRANSFER_CHUNK_SIZE as u64)
    );
    assert_eq!(
        tree.opened_handles.load(Ordering::Relaxed),
        1,
        "first chunk must open exactly one handle"
    );
    {
        let progress = tree.progress.lock().await;
        assert_eq!(
            progress.handle_path.as_deref(),
            Some("/remote/big.bin"),
            "handle must stay cached for the next chunk"
        );
        assert!(progress.read_handle.is_some());
    }

    // Chunk 2: tail of the same file reuses the cached handle (no reopen).
    let response = runtime
        .download_chunk(task_id, TRANSFER_CHUNK_SIZE as u64, &emitter)
        .await
        .expect("second chunk");
    assert_eq!(response["length"].as_u64(), Some(1024));
    assert_eq!(
        tree.opened_handles.load(Ordering::Relaxed),
        1,
        "same file must not be reopened"
    );
    {
        let progress = tree.progress.lock().await;
        assert!(
            progress.read_handle.is_none(),
            "finished file drops its handle"
        );
        assert!(progress.handle_path.is_none());
        assert_eq!(progress.files_done, 1);
    }

    // Chunk 3: file switch reopens once for the next file.
    runtime
        .download_chunk(task_id, (TRANSFER_CHUNK_SIZE + 1024) as u64, &emitter)
        .await
        .expect("third chunk");
    assert_eq!(
        tree.opened_handles.load(Ordering::Relaxed),
        2,
        "switching files must reopen the handle"
    );
    {
        let progress = tree.progress.lock().await;
        assert!(
            progress.read_handle.is_none() && progress.handle_path.is_none(),
            "the small file completed in this chunk and drops its handle"
        );
        assert_eq!(progress.files_done, 2);
    }

    // Final call: zero-byte file lands, then the empty eof frame drains it.
    let streamed = drain_tree_frames(&runtime, task_id, &emitter, &sink).await;
    assert_eq!(streamed.len(), (big.len() + small.len()) as usize);
    assert_eq!(std::fs::read(root.join("big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(root.join("sub/small.bin")).unwrap(), small);
    assert_eq!(std::fs::read(root.join("empty.bin")).unwrap(), b"");
    let progress = tree.progress.lock().await;
    assert_eq!(progress.files_done, 3);
    assert!(progress.files.is_empty() && progress.current.is_none());
    assert!(progress.failures.is_empty());
}

/// A premature finish racing the final in-flight chunk must wait for the pump
/// commit (rename) before removing the registry entry: otherwise the chunk
/// aborts with a fully staged last file, the finish deletes that `.part` and
/// still reports completed. With the barrier the last file survives and the
/// completed summary is only produced after the chunk commit.
#[tokio::test]
async fn finish_tree_download_waits_for_the_in_flight_final_chunk() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let root = data_dir.path().join("tree-root");
    std::fs::create_dir_all(&root).expect("root");
    let part_path = root.join("last.bin.part");
    let part = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&part_path)
        .await
        .expect("part file");
    let data = b"last-file-bytes".to_vec();
    let started = Arc::new(tokio::sync::Notify::new());
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let tree_state = Arc::new(TreeDownloadState::new(
        root.clone(),
        1,
        0,
        false,
        TreeDownloadProgress {
            files: VecDeque::new(),
            current: Some(sftp_tree::TreeFile {
                relative: "last.bin".to_string(),
                remote_path: "/remote/last.bin".to_string(),
                size: data.len() as u64,
            }),
            current_offset: 0,
            sink: Some(Arc::new(DownloadSink {
                part_path: part_path.clone(),
                // Tree sinks rename within the mirrored directory, exactly as
                // `open_tree_sink` sets it up.
                final_dir: root.clone(),
                overwrite: false,
                file: AsyncMutex::new(part),
            })),
            files_done: 0,
            failures: Vec::new(),
            read_handle: Some(Box::new(GatedTreeRead {
                started: started.clone(),
                release: Some(release_rx),
                announced: false,
                data: data.clone(),
                pos: 0,
            })),
            handle_path: Some("/remote/last.bin".to_string()),
        },
    ));
    let runtime = Arc::new(SshRuntime::new(data_dir.path().to_path_buf()));
    let task_id = "t-tree-finish-inflight";
    let mut state = tree_chunk_state(root.clone(), Vec::new(), CompressionMode::None, None);
    state.size = data.len() as u64;
    state.tree = Some(tree_state.clone());
    runtime
        .downloads
        .lock()
        .unwrap()
        .insert(task_id.to_string(), state);

    let (emitter, _sink) = framed_transfer_emitter();
    let chunk_runtime = runtime.clone();
    let chunk_emitter = emitter.clone();
    let id = task_id.to_string();
    let chunk = tokio::spawn(async move {
        chunk_runtime
            .download_chunk(&id, 0, &chunk_emitter)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
        .await
        .expect("the chunk must enter its read");

    let finish_runtime = runtime.clone();
    let finish_emitter = emitter.clone();
    let id = task_id.to_string();
    let finish = tokio::spawn(async move {
        finish_runtime.complete_download(&id, &finish_emitter).await
    });
    // The finish must block on the pump, not tear the staged bytes down.
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(
        !finish.is_finished(),
        "finish must not complete before the in-flight chunk commits"
    );
    assert!(
        part_path.exists(),
        "the staged final file must survive a premature finish"
    );

    release_tx.send(()).expect("release the read");
    let chunk = tokio::time::timeout(std::time::Duration::from_secs(2), chunk)
        .await
        .expect("the chunk must finish")
        .expect("join chunk")
        .expect("chunk result");
    assert_eq!(chunk["eof"], true);
    let finished = tokio::time::timeout(std::time::Duration::from_secs(2), finish)
        .await
        .expect("finish must finish")
        .expect("join finish")
        .expect("finish result");
    assert_eq!(finished["success"], true);
    assert_eq!(
        std::fs::read(root.join("last.bin")).unwrap(),
        data,
        "the fully staged last file must be renamed, not deleted"
    );
    assert!(
        !part_path.exists(),
        "the staging .part must be renamed away by the chunk commit"
    );
    let progress = tree_state.progress.lock().await;
    assert_eq!(progress.files_done, 1);
    assert!(progress.current.is_none() && progress.sink.is_none());
    assert!(progress.failures.is_empty());
}

/// A failed binary emission must not advance the shared offset: the retry of
/// the same offset re-reads the same range and the staging write is rolled
/// back so the final local file is byte-identical (no skipped/duplicated
/// bytes).
#[tokio::test]
async fn tree_download_retries_offset_after_emission_failure() {
    struct FailingSink;
    impl std::io::Write for FailingSink {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("bridge closed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let data_dir = tempfile::tempdir().expect("tempdir");
    let source = data_dir.path().join("source");
    let root = data_dir.path().join("tree-root");
    std::fs::create_dir_all(&source).expect("source");
    std::fs::create_dir_all(&root).expect("root");
    let data: Vec<u8> = (0..TRANSFER_CHUNK_SIZE + 777)
        .map(|index| (index % 241) as u8)
        .collect();
    std::fs::write(source.join("big.bin"), &data).expect("source file");
    let files = vec![sftp_tree::TreeFile {
        relative: "big.bin".to_string(),
        remote_path: "/remote/big.bin".to_string(),
        size: data.len() as u64,
    }];
    let runtime = SshRuntime::new(data_dir.path().to_path_buf());
    let task_id = "t-tree-retry";
    runtime.downloads.lock().unwrap().insert(
        task_id.to_string(),
        tree_chunk_state(
            root.clone(),
            files,
            CompressionMode::None,
            Some(source.clone()),
        ),
    );
    let state = runtime
        .downloads
        .lock()
        .unwrap()
        .get(task_id)
        .cloned()
        .unwrap();
    let tree = state.tree.clone().unwrap();
    let failing = PluginEmitter::for_tests(
        Arc::new(Mutex::new(Box::new(FailingSink))),
        dbx_plugin_sdk::PluginTransport::Framed,
    );
    let error = runtime
        .download_chunk(task_id, 0, &failing)
        .await
        .expect_err("failed emission must surface");
    assert!(error.contains("bridge closed"), "unexpected error: {error}");
    // Neither offset advanced, and the staged bytes were rolled back.
    {
        let state = runtime
            .downloads
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .unwrap();
        assert_eq!(state.next_offset, 0, "registry offset must not advance");
        let progress = state.tree.as_ref().unwrap().progress.lock().await;
        assert_eq!(
            progress.current_offset, 0,
            "shared file offset must not advance"
        );
        let sink = progress.sink.as_ref().expect("staging sink stays open");
        assert_eq!(
            std::fs::metadata(&sink.part_path).unwrap().len(),
            0,
            "staged bytes must be rolled back for the retry"
        );
    }
    let (emitter, sink) = framed_transfer_emitter();
    let streamed = drain_tree_frames(&runtime, task_id, &emitter, &sink).await;
    assert_eq!(streamed, data, "retry must stream exactly the same bytes");
    assert_eq!(std::fs::read(root.join("big.bin")).unwrap(), data);
    assert_eq!(
        tree.opened_handles.load(Ordering::Relaxed),
        1,
        "the retry reuses the handle opened by the failed attempt"
    );
}

/// `sftp/transfer/status` keeps a stable shape: while a chunk holds the tree
/// state, status waits for the (short) snapshot instead of omitting fields.
#[tokio::test]
async fn tree_transfer_status_keeps_dynamic_fields_while_state_is_locked() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let root = data_dir.path().join("tree-root");
    std::fs::create_dir_all(&root).expect("root");
    let files = vec![
        sftp_tree::TreeFile {
            relative: "current.bin".to_string(),
            remote_path: "/remote/current.bin".to_string(),
            size: 4,
        },
        sftp_tree::TreeFile {
            relative: "queued.bin".to_string(),
            remote_path: "/remote/queued.bin".to_string(),
            size: 4,
        },
    ];
    let runtime = Arc::new(SshRuntime::new(data_dir.path().to_path_buf()));
    let task_id = "t-tree-status";
    let state = tree_chunk_state(root, files, CompressionMode::None, None);
    if let Some(tree) = state.tree.as_ref() {
        let mut progress = tree.progress.lock().await;
        progress.current = Some(sftp_tree::TreeFile {
            relative: "current.bin".to_string(),
            remote_path: "/remote/current.bin".to_string(),
            size: 4,
        });
    }
    runtime
        .downloads
        .lock()
        .unwrap()
        .insert(task_id.to_string(), state);
    let tree = runtime
        .downloads
        .lock()
        .unwrap()
        .get(task_id)
        .unwrap()
        .tree
        .clone()
        .unwrap();
    let guard = tree.progress.lock().await;
    let this = runtime.clone();
    let id = task_id.to_string();
    let status_task = tokio::spawn(async move { this.transfer_status(&id).await });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(
        !status_task.is_finished(),
        "status must wait for the snapshot instead of dropping dynamic fields"
    );
    drop(guard);
    let status = tokio::time::timeout(std::time::Duration::from_secs(1), status_task)
        .await
        .expect("status must unblock once the snapshot lock is released")
        .expect("join")
        .expect("status");
    assert_eq!(status["filesRemaining"], 3);
    assert_eq!(status["currentFile"], "current.bin");
    assert_eq!(status["fileCount"], 2);
}

/// Cancel must return promptly even while a chunk is blocked in a read: the
/// pump moves the handle out of `progress` before I/O, so cleanup acquires the
/// state lock immediately instead of waiting for the whole chunk.
#[tokio::test]
async fn cancel_tree_download_does_not_wait_for_an_in_flight_read() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let root = data_dir.path().join("tree-root");
    std::fs::create_dir_all(&root).expect("root");
    let staging = data_dir.path().join("staging.part");
    let part = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&staging)
        .await
        .expect("part file");
    let started = Arc::new(tokio::sync::Notify::new());
    let slow = SlowTreeRead {
        started: started.clone(),
        announced: false,
        delay: Box::pin(tokio::time::sleep(std::time::Duration::from_secs(10))),
        data: vec![7_u8; 16],
        pos: 0,
    };
    let tree_state = Arc::new(TreeDownloadState::new(
        root.clone(),
        1,
        0,
        false,
        TreeDownloadProgress {
            files: VecDeque::new(),
            current: Some(sftp_tree::TreeFile {
                relative: "slow.bin".to_string(),
                remote_path: "/remote/slow.bin".to_string(),
                size: 16,
            }),
            current_offset: 0,
            sink: Some(Arc::new(DownloadSink {
                part_path: staging.clone(),
                final_dir: data_dir.path().to_path_buf(),
                overwrite: false,
                file: AsyncMutex::new(part),
            })),
            files_done: 0,
            failures: Vec::new(),
            read_handle: Some(Box::new(slow)),
            handle_path: Some("/remote/slow.bin".to_string()),
        },
    ));
    let runtime = Arc::new(SshRuntime::new(data_dir.path().to_path_buf()));
    let task_id = "t-tree-cancel-inflight";
    let mut state = tree_chunk_state(root.clone(), Vec::new(), CompressionMode::None, None);
    state.size = 16;
    state.file_name = "slow".to_string();
    state.tree = Some(tree_state.clone());
    runtime
        .downloads
        .lock()
        .unwrap()
        .insert(task_id.to_string(), state);
    let (emitter, _sink) = transfer_event_emitter();
    let chunk_runtime = runtime.clone();
    let chunk_emitter = emitter.clone();
    let id = task_id.to_string();
    let chunk =
        tokio::spawn(async move { chunk_runtime.download_chunk(&id, 0, &chunk_emitter).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
        .await
        .expect("the chunk must enter its read");
    // The read is parked for 10s; cancel must still return promptly.
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        runtime.cancel_transfer(task_id, Some("user"), &emitter),
    )
    .await
    .expect("cancel must not wait for the in-flight read")
    .expect("cancel");
    chunk.abort();
    let _ = chunk.await;
    let progress = tree_state.progress.lock().await;
    assert!(progress.read_handle.is_none());
    assert!(progress.handle_path.is_none());
}

#[test]
fn sftp_client_config_compat_serializes_pipelines_and_widens_timeout() {
    // compat（M14-B）：请求/响应不做流水线并发（读写各 1 路）。
    let config = sftp_client_config(true);
    assert_eq!(config.max_concurrent_reads, 1);
    assert_eq!(config.max_concurrent_writes, 1);
    // #206：russh-sftp 默认 10s 单请求超时在慢链路/跳板下误杀大文件上传。
    assert_eq!(config.request_timeout_secs, 60);
}

#[test]
fn sftp_client_config_non_compat_keeps_default_pipeline_concurrency() {
    // 非 compat 保持 crate 默认流水线并发，只放宽每请求超时（#206）。
    let config = sftp_client_config(false);
    assert_eq!(
        config.max_concurrent_reads,
        SftpConfig::default().max_concurrent_reads
    );
    assert_eq!(
        config.max_concurrent_writes,
        SftpConfig::default().max_concurrent_writes
    );
    assert_eq!(config.request_timeout_secs, 60);
}
