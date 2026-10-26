//! Local container shell over `portable-pty`.
//!
//! `docker exec -it` needs a TTY on both ends; the sidecar process owns the
//! master, a reader thread feeds output chunks and a waiter thread reports the
//! child exit code. Input and resize are applied on the async pump.

use super::*;

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::{mpsc, watch};

pub(crate) const SHELL_INNER: &str =
    "if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi";
const OUTPUT_QUEUE: usize = 64;
const CLOSE_GRACE: Duration = Duration::from_secs(5);

/// One running local shell: the PTY master plus the child handle. Input goes
/// through `writer`, output through `reader`; `killer` is the cancel fallback.
pub(crate) struct LocalPty {
    master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    child: Box<dyn Child + Send + Sync>,
}

pub(crate) fn local_shell_argv(container_id: &str) -> Result<Vec<String>, String> {
    validate_container_id(container_id)?;
    Ok(vec![
        "exec".to_string(),
        "-i".to_string(),
        "-t".to_string(),
        "-e".to_string(),
        "TERM=xterm-256color".to_string(),
        container_id.to_string(),
        "sh".to_string(),
        "-c".to_string(),
        SHELL_INNER.to_string(),
    ])
}

pub(crate) fn spawn_local_pty(
    engine: &EngineConfig,
    container_id: &str,
    cols: u32,
    rows: u32,
) -> Result<LocalPty, String> {
    let cli = find_local_cli(&engine.cli)
        .ok_or_else(|| format!("{} CLI not found on this machine", engine.cli))?;
    let argv = local_shell_argv(container_id)?;
    let env = engine
        .endpoint
        .as_ref()
        .map(|endpoint| {
            vec![
                ("DOCKER_HOST", endpoint.clone()),
                ("CONTAINER_HOST", endpoint.clone()),
            ]
        })
        .unwrap_or_default();
    spawn_local_program(Path::new(&cli), &argv, &env, cols, rows)
}

pub(crate) fn spawn_local_program(
    program: &Path,
    args: &[String],
    env: &[(&str, String)],
    cols: u32,
    rows: u32,
) -> Result<LocalPty, String> {
    let pair = native_pty_system()
        .openpty(pty_size(cols, rows))
        .map_err(|error| format!("Failed to open a pseudo terminal: {error}"))?;
    let mut command = CommandBuilder::new(program.as_os_str());
    command.args(args);
    for (key, value) in env {
        command.env(*key, value.as_str());
    }
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("Failed to start {}: {error}", program.display()))?;
    drop(pair.slave);
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("Failed to read the pseudo terminal: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("Failed to write the pseudo terminal: {error}"))?;
    let killer = child.clone_killer();
    Ok(LocalPty {
        master: pair.master,
        reader,
        writer: Arc::new(Mutex::new(writer)),
        killer,
        child,
    })
}

fn pty_size(cols: u32, rows: u32) -> PtySize {
    PtySize {
        rows: rows.clamp(2, u16::MAX as u32) as u16,
        cols: cols.clamp(2, u16::MAX as u32) as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Why the multiplex loop stopped forwarding: the caller cancelled the
/// terminal or the output sink went away. Both paths drop the master and let
/// the shell exit (or kill it after [`CLOSE_GRACE`]).
enum Shutdown {
    Cancelled,
    SinkClosed,
}

pub(crate) async fn drive_local_pty(
    pty: LocalPty,
    sink: mpsc::Sender<Vec<u8>>,
    mut input: mpsc::Receiver<Vec<u8>>,
    mut resize: mpsc::Receiver<(u32, u32)>,
    mut cancel: watch::Receiver<bool>,
) -> Result<Option<i32>, String> {
    let LocalPty {
        master,
        reader,
        writer,
        mut killer,
        child,
    } = pty;
    // Reader thread: one bounded chunk queue per PTY read, so a slow consumer
    // applies backpressure instead of growing memory without bound.
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(OUTPUT_QUEUE);
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if out_tx.blocking_send(buffer[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
    // Wait thread: the child's exit code is reported once, then the thread ends.
    let (exit_tx, mut exit_rx) = mpsc::unbounded_channel::<Option<i32>>();
    std::thread::spawn(move || {
        let mut child = child;
        let code = child.wait().ok().map(|status| status.exit_code() as i32);
        let _ = exit_tx.send(code);
    });

    let mut master = Some(master);
    let mut exit_code = None;
    let mut exit_done = false;
    let mut reader_done = false;
    let mut input_open = true;
    let mut resize_open = true;
    let mut shutdown: Option<Shutdown> = None;

    while shutdown.is_none() && !(reader_done && exit_done) {
        // A close that arrived before the pump started must not be missed:
        // `changed()` only reports changes not yet observed.
        if *cancel.borrow() {
            shutdown = Some(Shutdown::Cancelled);
            master = None;
            continue;
        }
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    shutdown = Some(Shutdown::Cancelled);
                }
            }
            chunk = out_rx.recv(), if !reader_done => match chunk {
                Some(bytes) => {
                    if sink.send(bytes).await.is_err() {
                        shutdown = Some(Shutdown::SinkClosed);
                        reader_done = true;
                    }
                }
                None => reader_done = true,
            },
            data = input.recv(), if input_open && shutdown.is_none() => match data {
                Some(bytes) => {
                    let writer = Arc::clone(&writer);
                    let _ = tokio::task::spawn_blocking(move || {
                        let mut writer = writer.lock().unwrap_or_else(|poison| poison.into_inner());
                        writer.write_all(&bytes).and_then(|_| writer.flush())
                    })
                    .await;
                }
                None => input_open = false,
            },
            size = resize.recv(), if resize_open && shutdown.is_none() => match size {
                Some((cols, rows)) => {
                    if let Some(master) = master.as_ref() {
                        let _ = master.resize(pty_size(cols, rows));
                    }
                }
                None => resize_open = false,
            },
            status = exit_rx.recv(), if !exit_done => {
                exit_code = status.flatten();
                exit_done = true;
            },
        }
        if shutdown.is_some() || exit_done {
            // Dropping the master sends EOF/hangup to the shell; the reader's
            // own descriptor still drains the tail before it hits EOF.
            master = None;
        }
    }

    if !exit_done {
        match tokio::time::timeout(CLOSE_GRACE, exit_rx.recv()).await {
            Ok(Some(code)) => exit_code = code.or(exit_code),
            Ok(None) => {}
            Err(_) => {
                let _ = killer.kill();
                if let Ok(Some(code)) = tokio::time::timeout(CLOSE_GRACE, exit_rx.recv()).await {
                    exit_code = code.or(exit_code);
                }
            }
        }
    }
    Ok(exit_code)
}
