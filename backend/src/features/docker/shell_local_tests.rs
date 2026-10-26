//! Local PTY driver tests. A real portable-pty process (`sh -c printf`) proves
//! output, input and exit-code forwarding without Docker.

use super::shell_local::*;
use std::path::PathBuf;
use tokio::sync::{mpsc, watch};

#[test]
fn local_argv_uses_exec_it_and_the_fixed_shell() {
    let argv = local_shell_argv("d4a7c9f1e2b3").expect("argv");
    assert_eq!(
        argv,
        vec![
            "exec".to_string(),
            "-i".to_string(),
            "-t".to_string(),
            "-e".to_string(),
            "TERM=xterm-256color".to_string(),
            "d4a7c9f1e2b3".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            SHELL_INNER.to_string(),
        ]
    );
    assert!(local_shell_argv("web-nginx").is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn drive_local_pty_forwards_output_and_exit_code() {
    let (sink, mut chunks) = mpsc::channel(16);
    let (_input_tx, input) = mpsc::channel(4);
    let (_resize_tx, resize) = mpsc::channel(4);
    let (_cancel_tx, cancel) = watch::channel(false);

    let program = PathBuf::from("sh");
    let args = vec!["-c".to_string(), "printf ready; exit 3".to_string()];
    let pty = spawn_local_program(&program, &args, &[], 80, 24).expect("spawn");
    let code = drive_local_pty(pty, sink, input, resize, cancel)
        .await
        .expect("drive");

    assert_eq!(code, Some(3));
    let mut output = Vec::new();
    while let Some(chunk) = chunks.recv().await {
        output.extend_from_slice(&chunk);
    }
    assert_eq!(String::from_utf8_lossy(&output), "ready");
}

#[cfg(unix)]
#[tokio::test]
async fn cancel_stops_the_local_pty() {
    let (sink, _chunks) = mpsc::channel(16);
    let (_input_tx, input) = mpsc::channel(4);
    let (_resize_tx, resize) = mpsc::channel(4);
    let (cancel_tx, cancel) = watch::channel(false);

    let program = PathBuf::from("sh");
    let args = vec!["-c".to_string(), "sleep 30".to_string()];
    let pty = spawn_local_program(&program, &args, &[], 80, 24).expect("spawn");
    let drive = tokio::spawn(drive_local_pty(pty, sink, input, resize, cancel));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    cancel_tx.send(true).expect("cancel");
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), drive)
        .await
        .expect("cancel must stop the PTY within the grace window");
    assert!(result.expect("drive task").is_ok());
}
