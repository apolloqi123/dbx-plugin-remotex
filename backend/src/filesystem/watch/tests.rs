use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[derive(Default)]
struct CollectingPublisher {
    count: AtomicUsize,
    payloads: std::sync::Mutex<Vec<Value>>,
}

impl EventPublisher for CollectingPublisher {
    fn publish(&self, payload: Value) {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.payloads
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(payload);
    }
}

impl CollectingPublisher {
    fn payload(&self) -> Value {
        self.payloads
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .last()
            .cloned()
            .unwrap_or(Value::Null)
    }
}

fn always_alive() -> SessionProbe {
    Arc::new(|_| Box::pin(async { true }) as Pin<Box<dyn Future<Output = bool> + Send>>)
}

fn always_dead() -> SessionProbe {
    Arc::new(|_| Box::pin(async { false }) as Pin<Box<dyn Future<Output = bool> + Send>>)
}

fn write_file(path: &Path, content: &[u8]) {
    std::fs::write(path, content).expect("write test file");
}

// ---- fingerprint classification (pure) ----

#[test]
fn fingerprint_ignores_mtime_only_touches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("watched.txt");
    write_file(&path, b"stable");
    let first = file_fingerprint(&path).expect("fingerprint");
    // Same bytes, later mtime (writer runs after the first snapshot).
    std::thread::sleep(Duration::from_millis(20));
    write_file(&path, b"stable");
    let second = file_fingerprint(&path).expect("fingerprint");
    assert_eq!(first.len, second.len);
    assert_eq!(first.sha256, second.sha256);
    // Classification must say "same" even if the mtime ticked.
    assert_eq!(
        classify_change(&Some(first), &Some(second)),
        ChangeVerdict::Same
    );
}

#[test]
fn fingerprint_detects_content_change() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("watched.txt");
    // 两次写入刻意用不同长度：同毫秒内完成时 mtime 短路无法区分，
    // 长度差异保证 classify_change 判定稳定（不依赖文件系统时间精度）。
    write_file(&path, b"before");
    let baseline = file_fingerprint(&path).expect("fingerprint");
    // The gap keeps the two snapshots on distinct millisecond mtimes so
    // the classification exercises the sha branch, not the mtime one.
    std::thread::sleep(Duration::from_millis(10));
    write_file(&path, b"after!");
    let current = file_fingerprint(&path).expect("fingerprint");
    assert_eq!(
        classify_change(&Some(baseline), &Some(current)),
        ChangeVerdict::ContentChanged
    );
}

#[test]
fn fingerprint_same_content_rewritten_has_same_sha() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("watched.txt");
    write_file(&path, b"identical bytes");
    let first = file_fingerprint(&path).expect("fingerprint");
    std::thread::sleep(Duration::from_millis(20));
    write_file(&path, b"identical bytes");
    let second = file_fingerprint(&path).expect("fingerprint");
    assert_eq!(first.sha256, second.sha256);
}

#[test]
fn fingerprint_is_undeterminable_for_missing_and_directories() {
    assert_eq!(classify_change(&None, &None), ChangeVerdict::Undeterminable);
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(file_fingerprint(&dir.path().join("gone.txt")).is_none());
    // A directory is not a watchable file.
    assert!(file_fingerprint(dir.path()).is_none());
}

#[test]
fn oversized_file_keeps_size_mtime_fingerprint() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("big.txt");
    // 6 bytes over a 4-byte cap: exercises the oversized branch without a
    // 64 MiB fixture. Production passes MAX_HASH_BYTES here.
    write_file(&path, b"123456");
    let baseline = file_fingerprint_with_limit(&path, 4).expect("fingerprint");
    assert_eq!(baseline.len, 6);
    assert_eq!(baseline.sha256, None);
    // Same bytes rewritten later: mtime ticks, no content proof — the
    // coarse signal must still classify (ContentChanged), not stall.
    std::thread::sleep(Duration::from_millis(10));
    write_file(&path, b"123456");
    let current = file_fingerprint_with_limit(&path, 4).expect("fingerprint");
    assert_eq!(
        classify_change(&Some(baseline), &Some(current.clone())),
        ChangeVerdict::ContentChanged
    );
    // Unchanged file re-snapshotted: against the NEW baseline (current,
    // what the pump stores after an emit) the verdict is Same.
    let stable = file_fingerprint_with_limit(&path, 4).expect("fingerprint");
    assert_eq!(
        classify_change(&Some(current), &Some(stable)),
        ChangeVerdict::Same
    );
}

#[test]
fn classify_change_treats_missing_snapshot_as_undeterminable() {
    let baseline = Some(FileFingerprint {
        len: 3,
        modified_ms: 1,
        sha256: Some([0; 32]),
    });
    assert_eq!(
        classify_change(&baseline, &None),
        ChangeVerdict::Undeterminable
    );
    assert_eq!(
        classify_change(&None, &baseline),
        ChangeVerdict::Undeterminable
    );
}

// ---- dedup table (registry, no real filesystem events) ----

fn tunables(base: &Path) -> Tunables {
    Tunables {
        debounce: Duration::from_millis(20),
        suppress: Duration::from_millis(10),
        desktop_gate_override: Some(true),
        // Pin the remote-edit origin root to the test tempdir: files
        // under `<base>/remote-edit/` pass, everything else is refused —
        // CI runners have no desktop Downloads layout to lean on.
        remote_edit_downloads_override: Some(base.to_path_buf()),
    }
}

/// Fixture stand-in for the plugin's `remote-edit/` download directory.
fn remote_edit_file(dir: &tempfile::TempDir, file_name: &str) -> PathBuf {
    let subdir = dir.path().join("remote-edit");
    std::fs::create_dir_all(&subdir).expect("create remote-edit dir");
    subdir.join(file_name)
}

async fn start_watch(
    runtime: &WatchRuntime,
    dir: &tempfile::TempDir,
    file_name: &str,
    session_id: &str,
    remote_path: &str,
    publisher: &Arc<CollectingPublisher>,
) -> String {
    let path = remote_edit_file(dir, file_name);
    write_file(&path, b"start");
    let response = runtime
        .start(
            WatchStartRequest {
                session_id: session_id.to_string(),
                remote_path: remote_path.to_string(),
                local_path: path.to_string_lossy().into_owned(),
            },
            publisher.clone() as Arc<dyn EventPublisher>,
            always_alive(),
            dir.path(),
        )
        .await
        .expect("watch start");
    response["watchId"].as_str().expect("watchId").to_string()
}

/// M15 多文件并行：同一会话同时挂两个远端文件的 watcher，互不顶替，
/// 各自保存各自触发（事件按 watchId/路径逐文件区分，无串扰）。
#[tokio::test]
async fn two_files_same_session_watch_in_parallel() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    let _a = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-multi",
        "/remote/a.txt",
        &publisher,
    )
    .await;
    let _b = start_watch(
        &runtime,
        &dir,
        "other.txt",
        "sess-multi",
        "/remote/b.txt",
        &publisher,
    )
    .await;
    assert_eq!(
        runtime.live_count().await,
        2,
        "same session keeps one watcher per file"
    );

    // Past the suppression window: edit both files; each watcher emits
    // its own event with its own identity.
    tokio::time::sleep(Duration::from_millis(150)).await;
    write_file(&remote_edit_file(&dir, "watched.txt"), b"first-file-edit");
    write_file(&remote_edit_file(&dir, "other.txt"), b"second-file-edit");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && publisher.count.load(Ordering::SeqCst) < 2 {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        publisher.count.load(Ordering::SeqCst),
        2,
        "both watchers fire independently"
    );
    let payloads = publisher
        .payloads
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let mut remotes: Vec<String> = payloads
        .iter()
        .map(|payload| {
            payload["remotePath"]
                .as_str()
                .expect("remotePath")
                .to_string()
        })
        .collect();
    remotes.sort();
    assert_eq!(remotes, ["/remote/a.txt", "/remote/b.txt"]);
    let mut ids: Vec<String> = payloads
        .iter()
        .map(|payload| payload["watchId"].as_str().expect("watchId").to_string())
        .collect();
    ids.sort();
    assert_ne!(ids[0], ids[1], "each file carries its own watchId");
    // 同一会话两个 watcher 都还活着（一个文件触发不影响另一个）。
    assert_eq!(runtime.live_count().await, 2);
}

/// 编辑其中一个文件不会顶替同会话另一个文件的监听（per-path 粒度去重，
/// 而非 per-session）。
#[tokio::test]
async fn editing_one_file_keeps_the_others_watch_alive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    let _watch_a = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-keep",
        "/remote/a.txt",
        &publisher,
    )
    .await;
    let watch_b = start_watch(
        &runtime,
        &dir,
        "other.txt",
        "sess-keep",
        "/remote/b.txt",
        &publisher,
    )
    .await;
    // 重新打开文件 a：dedup 只替换 a 的 watcher（返回新 watchId），b 原样保留。
    let watch_a2 = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-keep",
        "/remote/a.txt",
        &publisher,
    )
    .await;
    assert_eq!(runtime.live_count().await, 2, "b survives a's re-open");
    runtime.stop(&watch_a2).await.expect("stop a replacement");
    assert_eq!(runtime.live_count().await, 1, "only b remains");
    runtime.stop(&watch_b).await.expect("stop b");
    assert_eq!(runtime.live_count().await, 0);
}

#[tokio::test]
async fn start_replaces_dedup_watch_for_the_same_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    let first = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-1",
        "/remote/a.txt",
        &publisher,
    )
    .await;
    let second = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-1",
        "/remote/a.txt",
        &publisher,
    )
    .await;
    assert_ne!(first, second);
    assert_eq!(runtime.live_count().await, 1, "dedup keeps one watcher");
    // A different session owns a separate watcher for the same file.
    let _third = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-2",
        "/remote/a.txt",
        &publisher,
    )
    .await;
    assert_eq!(runtime.live_count().await, 2);
}

#[tokio::test]
async fn stop_and_stop_session_clear_the_registry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    let watch_id = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-1",
        "/remote/a.txt",
        &publisher,
    )
    .await;
    runtime.stop(&watch_id).await.expect("stop");
    assert_eq!(runtime.live_count().await, 0);
    assert!(runtime.stop(&watch_id).await.is_err(), "double stop errors");

    let _a = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-1",
        "/remote/a.txt",
        &publisher,
    )
    .await;
    let _b = start_watch(
        &runtime,
        &dir,
        "other.txt",
        "sess-1",
        "/remote/b.txt",
        &publisher,
    )
    .await;
    let _c = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-2",
        "/remote/a.txt",
        &publisher,
    )
    .await;
    assert_eq!(runtime.stop_session("sess-1").await, 2);
    assert_eq!(runtime.live_count().await, 1, "other sessions survive");
    assert_eq!(runtime.stop_session("sess-1").await, 0, "idempotent");
}

#[tokio::test]
async fn start_validates_desktop_and_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    // Missing file is refused before any watcher is created.
    let missing = runtime
        .start(
            WatchStartRequest {
                session_id: "s".to_string(),
                remote_path: "/r".to_string(),
                local_path: "/definitely/not/here.dbx-watch-test".to_string(),
            },
            publisher.clone() as Arc<dyn EventPublisher>,
            always_alive(),
            dir.path(),
        )
        .await;
    // The origin check canonicalizes, so a missing path surfaces as
    // "does not exist" (or the absolute-path rejection on Windows where
    // a leading `/` is not absolute) — accept either rejection text.
    let missing_error = missing.unwrap_err();
    assert!(
        missing_error.contains("does not exist") || missing_error.contains("absolute"),
        "unexpected rejection: {missing_error}"
    );
    // Relative local paths are refused (desktop detection happens first;
    // when the desktop gate is off, that error wins — both are valid on
    // CI, so accept either rejection text).
    let relative = runtime
        .start(
            WatchStartRequest {
                session_id: "s".to_string(),
                remote_path: "/r".to_string(),
                local_path: "relative/path.txt".to_string(),
            },
            publisher.clone() as Arc<dyn EventPublisher>,
            always_alive(),
            dir.path(),
        )
        .await;
    let error = relative.unwrap_err();
    assert!(
        error.contains("absolute path") || error.contains("desktop"),
        "unexpected error: {error}"
    );
    assert_eq!(runtime.live_count().await, 0, "no watcher leaked");
}

// ---- path-origin gate (a watchId is a bearer token; the real boundary
// is the path origin, the same check as sftp/upload-local) ----

#[tokio::test]
async fn start_accepts_files_inside_remote_edit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    let inside = remote_edit_file(&dir, "watched.txt");
    write_file(&inside, b"start");
    runtime
        .start(
            WatchStartRequest {
                session_id: "sess-origin".to_string(),
                remote_path: "/remote/a.txt".to_string(),
                local_path: inside.to_string_lossy().into_owned(),
            },
            publisher.clone() as Arc<dyn EventPublisher>,
            always_alive(),
            dir.path(),
        )
        .await
        .expect("remote-edit file must be watchable");
    assert_eq!(runtime.live_count().await, 1);
}

#[tokio::test]
async fn start_rejects_files_outside_remote_edit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    // Plain sibling, plus the `remote-edit-evil` prefix-confusion name
    // (starts_with is component-wise, so it must not pass).
    for name in ["outside.txt", "remote-edit-evil.txt"] {
        let path = dir.path().join(name);
        write_file(&path, b"secret");
        let error = runtime
            .start(
                WatchStartRequest {
                    session_id: "sess-origin".to_string(),
                    remote_path: "/remote/a.txt".to_string(),
                    local_path: path.to_string_lossy().into_owned(),
                },
                publisher.clone() as Arc<dyn EventPublisher>,
                always_alive(),
                dir.path(),
            )
            .await
            .unwrap_err();
        assert!(
            error.contains("remote-edit"),
            "'{name}' must be refused, got: {error}"
        );
    }
    assert_eq!(runtime.live_count().await, 0, "no watcher leaked");
}

#[tokio::test]
async fn start_rejects_remote_edit_traversal_escape() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    let secret = dir.path().join("secret.txt");
    write_file(&secret, b"secret");
    // `..` inside the request leaves remote-edit once canonicalized.
    let traversal = dir.path().join("remote-edit").join("..").join("secret.txt");
    let error = runtime
        .start(
            WatchStartRequest {
                session_id: "sess-origin".to_string(),
                remote_path: "/remote/a.txt".to_string(),
                local_path: traversal.to_string_lossy().into_owned(),
            },
            publisher.clone() as Arc<dyn EventPublisher>,
            always_alive(),
            dir.path(),
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("remote-edit"),
        "traversal escape must be refused, got: {error}"
    );
    assert_eq!(runtime.live_count().await, 0, "no watcher leaked");
}

/// A symlink inside remote-edit pointing outside resolves (canonicalize)
/// to a path outside the root and must be refused.
#[cfg(unix)]
#[tokio::test]
async fn start_rejects_symlink_escaping_remote_edit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    let secret = dir.path().join("secret.txt");
    write_file(&secret, b"secret");
    let link = remote_edit_file(&dir, "link.txt");
    std::os::unix::fs::symlink(&secret, &link).expect("symlink");
    let error = runtime
        .start(
            WatchStartRequest {
                session_id: "sess-origin".to_string(),
                remote_path: "/remote/a.txt".to_string(),
                local_path: link.to_string_lossy().into_owned(),
            },
            publisher.clone() as Arc<dyn EventPublisher>,
            always_alive(),
            dir.path(),
        )
        .await
        .unwrap_err();
    assert!(
        error.contains("remote-edit"),
        "symlink escape must be refused, got: {error}"
    );
    assert_eq!(runtime.live_count().await, 0, "no watcher leaked");
}

/// End-to-end: a real editor-style rewrite past the suppression window
/// arms the debounce and fires exactly one confirmed event; a mtime-only
/// rewrite of identical bytes never fires.
#[tokio::test]
async fn pump_emits_once_per_confirmed_change() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    let _watch_id = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-e2e",
        "/remote/e2e.txt",
        &publisher,
    )
    .await;

    // Past the 10ms suppression window: rewrite with new content.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let path = remote_edit_file(&dir, "watched.txt");
    write_file(&path, b"edited-by-external-editor");

    let wait_for = |count: usize| {
        let publisher = publisher.clone();
        async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                if publisher.count.load(Ordering::SeqCst) >= count {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            false
        }
    };
    assert!(wait_for(1).await, "confirmed change did not emit");
    let payload = publisher.payload();
    assert_eq!(payload["sessionId"], "sess-e2e");
    assert_eq!(payload["remotePath"], "/remote/e2e.txt");
    assert!(payload["watchId"].as_str().is_some());
    assert!(payload["localPath"]
        .as_str()
        .unwrap()
        .ends_with("watched.txt"));

    // Editor saving identical bytes (mtime-only) must not re-fire.
    tokio::time::sleep(Duration::from_millis(60)).await;
    write_file(&path, b"edited-by-external-editor");
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        publisher.count.load(Ordering::SeqCst),
        1,
        "mtime-only rewrite must not emit"
    );
}

/// A watcher whose owning session died stops itself instead of prompting
/// into a dead workbench.
#[tokio::test]
async fn dead_session_stops_the_watch_without_emitting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    let _watch_id = start_watch_with_probe(
        &runtime,
        &dir,
        "watched.txt",
        "sess-dead",
        "/remote/dead.txt",
        &publisher,
        always_dead(),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let path = remote_edit_file(&dir, "watched.txt");
    write_file(&path, b"change-into-the-void");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && runtime.live_count().await > 0 {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(runtime.live_count().await, 0, "dead session self-cleans");
    assert_eq!(publisher.count.load(Ordering::SeqCst), 0);
}

async fn start_watch_with_probe(
    runtime: &WatchRuntime,
    dir: &tempfile::TempDir,
    file_name: &str,
    session_id: &str,
    remote_path: &str,
    publisher: &Arc<CollectingPublisher>,
    probe: SessionProbe,
) -> String {
    let path = remote_edit_file(dir, file_name);
    write_file(&path, b"start");
    let response = runtime
        .start(
            WatchStartRequest {
                session_id: session_id.to_string(),
                remote_path: remote_path.to_string(),
                local_path: path.to_string_lossy().into_owned(),
            },
            publisher.clone() as Arc<dyn EventPublisher>,
            probe,
            dir.path(),
        )
        .await
        .expect("watch start");
    response["watchId"].as_str().expect("watchId").to_string()
}

/// Deleting the watched file self-cleans the registry.
#[tokio::test]
async fn deleting_the_file_stops_the_watch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = WatchRuntime::with_tunables(tunables(dir.path()));
    let publisher = Arc::new(CollectingPublisher::default());
    let _watch_id = start_watch(
        &runtime,
        &dir,
        "watched.txt",
        "sess-del",
        "/remote/del.txt",
        &publisher,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let watched = remote_edit_file(&dir, "watched.txt");
    std::fs::remove_file(&watched).expect("remove");
    // Touch the parent so watchers that only see directory-level events
    // (FSEvents) wake up and notice the removal.
    write_file(&watched, b"resurrect");
    std::fs::remove_file(&watched).expect("remove again");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && runtime.live_count().await > 0 {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(runtime.live_count().await, 0, "deleted file self-cleans");
    assert_eq!(publisher.count.load(Ordering::SeqCst), 0);
}
