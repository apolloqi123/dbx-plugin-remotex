//! RDP 连接级生命周期：connection/disconnect 回收该连接名下的所有会话。
//! 从 `tests.rs` 拆出（结构守卫 1200 行预算），共享父模块的测试夹具。

use super::*;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

#[tokio::test]
async fn close_connection_reclaims_only_its_own_rdp_sessions() {
    let runtime = RdpSessionRuntime::new();
    let data_dir = tempfile::tempdir().expect("tempdir");
    let owned = uuid::Uuid::new_v4().to_string();
    let other = uuid::Uuid::new_v4().to_string();
    {
        let mut sessions = runtime.sessions.write().await;
        sessions.insert(
            owned.clone(),
            test_session_entry("w-owned", Some("conn-1"), "10.255.255.1", data_dir.path()),
        );
        sessions.insert(
            other.clone(),
            test_session_entry("w-other", Some("conn-2"), "10.255.255.2", data_dir.path()),
        );
        sessions.insert(
            "w-no-connection".to_string(),
            test_session_entry("w-none", None, "10.255.255.3", data_dir.path()),
        );
    }
    runtime.close_connection("conn-1").await;
    let sessions = runtime.sessions.read().await;
    assert!(!sessions.contains_key(&owned));
    assert!(sessions.contains_key(&other));
    assert!(sessions.contains_key("w-no-connection"));
}

#[tokio::test]
async fn idempotent_start_adopts_a_late_connection_id() {
    let runtime = RdpSessionRuntime::new();
    let data_dir = tempfile::tempdir().expect("tempdir");
    let seeded_id = uuid::Uuid::new_v4().to_string();
    runtime.sessions.write().await.insert(
        seeded_id.clone(),
        test_session_entry("w-adopt", None, "10.255.255.1", data_dir.path()),
    );
    let mut request = idempotent_start_request("w-adopt", "10.255.255.1");
    request.connection_id = Some("conn-adopt".to_string());
    let started = runtime
        .start(request, test_emitter(), data_dir.path())
        .await
        .expect("idempotent start");
    assert_eq!(started["sessionId"].as_str(), Some(seeded_id.as_str()));
    assert_eq!(
        runtime
            .sessions
            .read()
            .await
            .get(&seeded_id)
            .unwrap()
            .connection_id
            .lock()
            .unwrap()
            .as_deref(),
        Some("conn-adopt")
    );
    runtime.close_connection("conn-adopt").await;
    assert!(runtime.sessions.read().await.is_empty());
}

#[tokio::test]
async fn resize_queues_a_live_command_without_restarting_the_generation() {
    let runtime = RdpSessionRuntime::new();
    let data_dir = tempfile::tempdir().expect("tempdir");
    let session_id = uuid::Uuid::new_v4().to_string();
    let entry = test_session_entry("w-live", None, "10.255.255.1", data_dir.path());
    entry.generation.store(7, Ordering::Release);
    let (command_sender, mut command_receiver) = mpsc::channel(4);
    *entry.command_sender.lock().await = Some(command_sender);
    runtime
        .sessions
        .write()
        .await
        .insert(session_id.clone(), entry.clone());

    let response = runtime
        .resize(&session_id, 1600, 900)
        .await
        .expect("live resize");
    assert_eq!(response["live"], true);
    match command_receiver.recv().await.expect("resize command") {
        RdpCommand::Resize { width, height } => assert_eq!((width, height), (1600, 900)),
        _ => panic!("expected a live resize command"),
    }
    // The session keeps its generation (no worker restart) and remembers the
    // new size for any later engine-level reconnect.
    assert_eq!(entry.generation.load(Ordering::Acquire), 7);
    assert_eq!(entry.width.load(Ordering::Acquire), 1600);
    assert_eq!(entry.height.load(Ordering::Acquire), 900);
}

#[tokio::test]
async fn resize_without_an_engine_targets_the_next_generation() {
    let runtime = RdpSessionRuntime::new();
    let data_dir = tempfile::tempdir().expect("tempdir");
    let session_id = uuid::Uuid::new_v4().to_string();
    let entry = test_session_entry("w-idle", None, "10.255.255.1", data_dir.path());
    entry.generation.store(3, Ordering::Release);
    runtime
        .sessions
        .write()
        .await
        .insert(session_id.clone(), entry.clone());

    let response = runtime
        .resize(&session_id, 1440, 900)
        .await
        .expect("deferred resize");
    assert_eq!(response["live"], false);
    assert_eq!(entry.generation.load(Ordering::Acquire), 3);
    assert_eq!(entry.width.load(Ordering::Acquire), 1440);
    assert_eq!(entry.height.load(Ordering::Acquire), 900);
    assert!(runtime.sessions.read().await.contains_key(&session_id));
}
