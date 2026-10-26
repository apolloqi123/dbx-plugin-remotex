# Container Manager Default Split + PTY Terminal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open the container manager with list+logs split by default, and replace the one-shot command console with a real PTY shell that matches the host terminal's input/display experience.

**Architecture:** The SSH plane opens a second PTY channel on the already-authenticated SSH transport and runs `docker exec -it`; the local plane spawns `portable-pty` with the same argv. A bounded sidecar registry multiplexes input/resize/cancel and emits raw PTY bytes on a binary channel; the frontend renders them with xterm using the workbench theme/font. The container pane auto-opens the Logs tab for the first selected container and reopens on explicit row clicks after the user dismisses it.

**Tech Stack:** Rust (russh, portable-pty, tokio), Vue 3 `<script setup>`, xterm.js (`@xterm/xterm`, `@xterm/addon-fit`), Vitest + happy-dom.

**Spec:** `docs/superpowers/specs/2026-10-26-container-pty-terminal-design.md`

## Global Constraints

- Frontend source files ≤600 lines; test files ≤1,200 lines. Backend source files ≤600; test files ≤1,200.
- Layering: `app → features → shared`; `shared` must never import `features` or `app`. Feature→feature imports are allowed.
- Container id must pass `validate_container_id`; terminal command is fixed, user input is never interpolated into it.
- SSH read-only connections must be rejected by the backend (`ensure_writable`) and disabled in the UI (`canWrite === false`).
- Audit rows record only the fixed command and container/terminal identity; never keystrokes, PTY output, or secrets.
- No new dependencies: `portable-pty 0.9`, `russh 0.62`, `@xterm/xterm` and `@xterm/addon-fit` are already in the workspace.
- Existing UX contracts (list-only row highlight, resizable divider 25–70%, icon tabs, logs follow/pause) stay intact.
- Do not commit generated artifacts or `node_modules`; run gates before each commit.
- Local commits only; never push, merge, install into DBX, or restart DBX from an agent.

---

### Task 1: SSH PTY channel driver

**Files:**
- Create: `backend/src/sessions/ssh/container_terminal.rs`
- Create: `backend/src/sessions/ssh/container_terminal_tests.rs`
- Modify: `backend/src/sessions/ssh/mod.rs` (add `mod container_terminal;` and `#[cfg(test)] mod container_terminal_tests;`)
- Test: `backend/src/sessions/ssh/container_terminal_tests.rs`

**Interfaces:**
- Consumes: `SshRuntime::session()` (`pub(super)`, same module tree), `russh::client::Handle`, `russh::ChannelMsg`, `crate::sessions::input_queue::enqueue_bounded` is NOT used here (input arrives via an mpsc receiver).
- Produces:
  - `pub(crate) enum PtyMessage { Data(Vec<u8>), ExitStatus(i32), Closed }`
  - `pub(crate) trait PtyChannel { async fn send_data(&mut self, data: &[u8]) -> Result<(), String>; async fn resize(&mut self, cols: u32, rows: u32) -> Result<(), String>; async fn close(&mut self) -> Result<(), String>; async fn next(&mut self) -> Option<PtyMessage>; }`
  - `pub(crate) async fn drive_pty_channel<C: PtyChannel>(channel: &mut C, sink: &tokio::sync::mpsc::Sender<Vec<u8>>, input: &mut tokio::sync::mpsc::Receiver<Vec<u8>>, resize: &mut tokio::sync::mpsc::Receiver<(u32, u32)>, cancel: &mut tokio::sync::watch::Receiver<bool>) -> Result<Option<i32>, String>`
  - `impl SshRuntime { pub async fn run_container_terminal(&self, session_id: &str, command: &str, cols: u32, rows: u32, sink: Sender<Vec<u8>>, input: Receiver<Vec<u8>>, resize: Receiver<(u32, u32)>, cancel: watch::Receiver<bool>) -> Result<Option<i32>, String> }`

- [ ] **Step 1: Write the failing test**

Create `backend/src/sessions/ssh/container_terminal_tests.rs`:

```rust
//! PTY driver tests: a fake channel proves the select-loop forwards output,
//! input, resize and cancellation without touching russh.

use super::*;
use tokio::sync::{mpsc, watch};

#[derive(Default)]
struct FakeChannel {
    sent_data: Vec<Vec<u8>>,
    resizes: Vec<(u32, u32)>,
    closed: bool,
    incoming: Vec<PtyMessage>,
}

impl PtyChannel for FakeChannel {
    async fn send_data(&mut self, data: &[u8]) -> Result<(), String> {
        self.sent_data.push(data.to_vec());
        Ok(())
    }

    async fn resize(&mut self, cols: u32, rows: u32) -> Result<(), String> {
        self.resizes.push((cols, rows));
        Ok(())
    }

    async fn close(&mut self) -> Result<(), String> {
        self.closed = true;
        Ok(())
    }

    async fn next(&mut self) -> Option<PtyMessage> {
        if self.incoming.is_empty() {
            std::future::pending::<()>().await;
        }
        Some(self.incoming.remove(0))
    }
}

#[tokio::test]
async fn drive_forwards_output_input_resize_and_exit() {
    let (sink, mut chunks) = mpsc::channel(8);
    let (input_tx, mut input) = mpsc::channel(8);
    let (resize_tx, mut resize) = mpsc::channel(8);
    let (_cancel_tx, mut cancel) = watch::channel(false);
    let mut channel = FakeChannel {
        incoming: vec![
            PtyMessage::Data(b"hello".to_vec()),
            PtyMessage::Data(b" world".to_vec()),
            PtyMessage::ExitStatus(7),
            PtyMessage::Closed,
        ],
        ..Default::default()
    };

    input_tx.send(b"ls\r".to_vec()).await.expect("input");
    resize_tx.send((120, 40)).await.expect("resize");

    let code = drive_pty_channel(&mut channel, &sink, &mut input, &mut resize, &mut cancel)
        .await
        .expect("driver");

    assert_eq!(code, Some(7));
    assert_eq!(channel.sent_data, vec![b"ls\r".to_vec()]);
    assert_eq!(channel.resizes, vec![(120, 40)]);
    assert_eq!(chunks.recv().await.as_deref(), Some(b"hello".as_slice()));
    assert_eq!(chunks.recv().await.as_deref(), Some(b" world".as_slice()));
}

#[tokio::test]
async fn cancel_closes_the_channel() {
    let (sink, _chunks) = mpsc::channel(8);
    let (_input_tx, mut input) = mpsc::channel(8);
    let (_resize_tx, mut resize) = mpsc::channel(8);
    let (cancel_tx, mut cancel) = watch::channel(false);
    let mut channel = FakeChannel::default();

    let run = drive_pty_channel(&mut channel, &sink, &mut input, &mut resize, &mut cancel);
    let cancel_task = async move {
        cancel_tx.send(true).expect("cancel");
    };
    let (code, ()) = tokio::join!(run, cancel_task);

    assert_eq!(code.expect("cancelled").map(|_| ()), None);
    assert!(channel.closed);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd backend && cargo test --locked container_terminal -- --nocapture`
Expected: compile error `unresolved import` / `PtyMessage` not found.

- [ ] **Step 3: Implement the driver**

Create `backend/src/sessions/ssh/container_terminal.rs`:

```rust
//! Interactive PTY channel for container terminals.
//!
//! One channel per container shell, opened on the session's already
//! authenticated transport. Output is forwarded as raw bytes; input and
//! resize arrive on bounded queues; cancellation closes the channel.

use super::*;

use tokio::sync::{mpsc, watch};

/// `connection::open::pty`'s alias is only `pub(super)` to its own module, so
/// the driver names russh's concrete channel type directly.
type TerminalChannel = russh::Channel<client::Msg>;

/// One message the driver cares about. russh's full message set is mapped in
/// [`ChannelPty`] so this module stays testable without a network peer.
pub(crate) enum PtyMessage {
    Data(Vec<u8>),
    ExitStatus(i32),
    Closed,
}

pub(crate) trait PtyChannel {
    async fn send_data(&mut self, data: &[u8]) -> Result<(), String>;
    async fn resize(&mut self, cols: u32, rows: u32) -> Result<(), String>;
    async fn close(&mut self) -> Result<(), String>;
    async fn next(&mut self) -> Option<PtyMessage>;
}

/// Multiplexes one PTY channel until it closes or is cancelled. Returns the
/// remote exit status when the peer reported one.
pub(crate) async fn drive_pty_channel<C: PtyChannel>(
    channel: &mut C,
    sink: &mpsc::Sender<Vec<u8>>,
    input: &mut mpsc::Receiver<Vec<u8>>,
    resize: &mut mpsc::Receiver<(u32, u32)>,
    cancel: &mut watch::Receiver<bool>,
) -> Result<Option<i32>, String> {
    let mut exit_code = None;
    loop {
        if *cancel.borrow() {
            let _ = channel.close().await;
            return Ok(exit_code);
        }
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    let _ = channel.close().await;
                    return Ok(exit_code);
                }
            }
            data = input.recv() => match data {
                Some(bytes) => channel.send_data(&bytes).await?,
                None => {
                    let _ = channel.close().await;
                    return Ok(exit_code);
                }
            },
            size = resize.recv() => if let Some((cols, rows)) = size {
                channel.resize(cols, rows).await?;
            },
            message = channel.next() => match message {
                Some(PtyMessage::Data(bytes)) => {
                    if sink.send(bytes).await.is_err() {
                        let _ = channel.close().await;
                        return Ok(exit_code);
                    }
                }
                Some(PtyMessage::ExitStatus(code)) => exit_code = Some(code),
                Some(PtyMessage::Closed) | None => {
                    let _ = channel.close().await;
                    return Ok(exit_code);
                }
            },
        }
    }
}

/// russh-backed channel adapter.
struct ChannelPty {
    channel: TerminalChannel,
}

impl PtyChannel for ChannelPty {
    async fn send_data(&mut self, data: &[u8]) -> Result<(), String> {
        self.channel
            .data(data)
            .await
            .map_err(|error| format!("Failed to write to the container terminal: {error}"))
    }

    async fn resize(&mut self, cols: u32, rows: u32) -> Result<(), String> {
        self.channel
            .window_change(cols.max(1), rows.max(1), 0, 0)
            .await
            .map_err(|error| format!("Failed to resize the container terminal: {error}"))
    }

    async fn close(&mut self) -> Result<(), String> {
        self.channel
            .close()
            .await
            .map_err(|error| format!("Failed to close the container terminal: {error}"))
    }

    async fn next(&mut self) -> Option<PtyMessage> {
        match self.channel.wait().await? {
            ChannelMsg::Data { data } => Some(PtyMessage::Data(data.to_vec())),
            ChannelMsg::ExtendedData { data, .. } => Some(PtyMessage::Data(data.to_vec())),
            ChannelMsg::ExitStatus { exit_status } => Some(PtyMessage::ExitStatus(exit_status as i32)),
            ChannelMsg::Eof => Some(PtyMessage::Closed),
            ChannelMsg::Close => Some(PtyMessage::Closed),
            _ => Some(PtyMessage::Data(Vec::new())),
        }
    }
}

impl SshRuntime {
    /// Opens a PTY channel and runs `command` in it (an `exec` request after
    /// `pty-req`, exactly like `ssh -t host <command>`).
    async fn open_container_channel(
        &self,
        session_id: &str,
        command: &str,
        cols: u32,
        rows: u32,
    ) -> Result<TerminalChannel, String> {
        let entry = self.session(session_id).await?;
        let mut channel = entry
            .handle
            .channel_open_session()
            .await
            .map_err(|error| format!("Failed to open the container terminal channel: {error}"))?;
        channel
            .request_pty(true, "xterm-256color", cols.max(1), rows.max(1), 0, 0, &[])
            .await
            .map_err(|error| format!("The host refused the terminal PTY request: {error}"))?;
        channel
            .exec(true, command.as_bytes())
            .await
            .map_err(|error| format!("Failed to start the container shell: {error}"))?;
        Ok(channel)
    }

    /// Runs one container shell: opens the channel, then drives it until the
    /// shell exits, the peer closes, or the caller cancels.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_container_terminal(
        &self,
        session_id: &str,
        command: &str,
        cols: u32,
        rows: u32,
        sink: mpsc::Sender<Vec<u8>>,
        mut input: mpsc::Receiver<Vec<u8>>,
        mut resize: mpsc::Receiver<(u32, u32)>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<Option<i32>, String> {
        let channel = tokio::time::timeout(
            Duration::from_secs(30),
            self.open_container_channel(session_id, command, cols, rows),
        )
        .await
        .map_err(|_| "Timed out opening the container terminal".to_string())??;
        let mut channel = ChannelPty { channel };
        drive_pty_channel(&mut channel, &sink, &mut input, &mut resize, &mut cancel).await
    }
}
```

Modify `backend/src/sessions/ssh/mod.rs`:

```rust
mod container_terminal;
#[cfg(test)]
mod container_terminal_tests;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd backend && cargo test --locked container_terminal -- --nocapture`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add backend/src/sessions/ssh/container_terminal.rs backend/src/sessions/ssh/container_terminal_tests.rs backend/src/sessions/ssh/mod.rs
git commit -m "feat(docker): drive container PTY channels over SSH"
```

---

### Task 2: Local PTY process driver

**Files:**
- Create: `backend/src/features/docker/shell_local.rs`
- Create: `backend/src/features/docker/shell_local_tests.rs`
- Modify: `backend/src/features/docker.rs` (`mod shell_local;` + `#[cfg(test)] mod shell_local_tests;`)
- Test: `backend/src/features/docker/shell_local_tests.rs`

**Interfaces:**
- Consumes: `EngineConfig`, `find_local_cli()` (existing `local.rs`), `portable_pty`, `SHELL_INNER` from `shell.rs` (Task 3 defines it; this task defines its own `pub(crate) const SHELL_INNER` and Task 3 reuses it).
- Produces:
  - `pub(crate) const SHELL_INNER: &str`
  - `pub(crate) fn local_shell_argv(container_id: &str) -> Result<Vec<String>, String>`
  - `pub(crate) struct LocalPty`
  - `pub(crate) fn spawn_local_pty(engine: &EngineConfig, container_id: &str, cols: u32, rows: u32) -> Result<LocalPty, String>`
  - `pub(crate) async fn drive_local_pty(pty: LocalPty, sink, input, resize, cancel) -> Result<Option<i32>, String>`
  - `fn spawn_local_program(program: &Path, args: &[String], env: &[(&str, String)], cols: u32, rows: u32) -> Result<LocalPty, String>` (test seam; `pub(crate)`)

- [ ] **Step 1: Write the failing test**

Create `backend/src/features/docker/shell_local_tests.rs`:

```rust
//! Local PTY driver tests. A real portable-pty process (`sh -c printf`) proves
//! output, input and exit-code forwarding without Docker.

use super::*;
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd backend && cargo test --locked shell_local -- --nocapture`
Expected: compile error `local_shell_argv` not found.

- [ ] **Step 3: Implement the local PTY driver**

Create `backend/src/features/docker/shell_local.rs`:

```rust
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
        .openpty(PtySize {
            rows: rows.clamp(2, u16::MAX as u32) as u16,
            cols: cols.clamp(2, u16::MAX as u32) as u16,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("Failed to open a pseudo terminal: {error}"))?;
    let mut command = CommandBuilder::new(program.as_os_str());
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
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
    let (exit_tx, mut exit_rx) = mpsc::unbounded_channel::<Option<i32>>();
    std::thread::spawn(move || {
        let mut child = child;
        let code = child.wait().ok().map(|status| status.exit_code() as i32);
        let _ = exit_tx.send(code);
    });

    let mut master = Some(master);
    let mut closing = false;
    let mut exit_code = None;
    loop {
        if closing {
            match tokio::time::timeout(CLOSE_GRACE, exit_rx.recv()).await {
                Ok(code) => exit_code = code.flatten(),
                Err(_) => {
                    let _ = killer.kill();
                    if let Ok(code) = tokio::time::timeout(CLOSE_GRACE, exit_rx.recv()).await {
                        exit_code = code.flatten();
                    }
                }
            }
            break;
        }
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    closing = true;
                    master = None; // dropping the master sends EOF to the shell
                }
            }
            chunk = out_rx.recv() => match chunk {
                Some(bytes) => {
                    if sink.send(bytes).await.is_err() {
                        closing = true;
                        master = None;
                    }
                }
                None => {
                    closing = true;
                    master = None;
                }
            },
            data = input.recv() => {
                if let Some(bytes) = data {
                    let writer = Arc::clone(&writer);
                    let _ = tokio::task::spawn_blocking(move || {
                        let mut writer = writer.lock().unwrap_or_else(|poison| poison.into_inner());
                        writer.write_all(&bytes).and_then(|_| writer.flush())
                    }).await;
                }
            }
            size = resize.recv() => if let (Some((cols, rows)), Some(master)) = (size, master.as_ref()) {
                let _ = master.resize(PtySize {
                    rows: rows.clamp(2, u16::MAX as u32) as u16,
                    cols: cols.clamp(2, u16::MAX as u32) as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            },
            status = exit_rx.recv() => {
                exit_code = status.flatten();
                closing = true;
                master = None;
            }
        }
    }
    Ok(exit_code)
}
```

Modify `backend/src/features/docker.rs`:

```rust
mod shell_local;
#[cfg(test)]
mod shell_local_tests;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd backend && cargo test --locked shell_local -- --nocapture`
Expected: PASS (2 tests on macOS/Linux; Windows may need `sh` on PATH — skip locally only if absent, with a `cfg(unix)` gate on the test).

- [ ] **Step 5: Commit**

```bash
git add backend/src/features/docker/shell_local.rs backend/src/features/docker/shell_local_tests.rs backend/src/features/docker.rs
git commit -m "feat(docker): spawn local container shells on a real PTY"
```

---

### Task 3: Terminal registry, RPC surface, input channel, and removal of `docker/exec`

**Files:**
- Create: `backend/src/features/docker/shell.rs`
- Create: `backend/src/features/docker/shell_tests.rs`
- Modify: `backend/src/features/docker.rs`
- Modify: `backend/src/app/mod.rs`, `backend/src/app/bootstrap.rs`
- Modify: `backend/src/app/rpc/docker.rs`
- Modify: `backend/src/app/handler.rs`
- Modify: `backend/src/features/docker/local.rs` (remove `audit_exec_intent` / `audit_exec_failure`; add `audit_terminal_intent`)
- Delete: `backend/src/features/docker/terminal.rs`
- Test: `backend/src/features/docker/shell_tests.rs`

**Interfaces:**
- Consumes: Task 1 `SshRuntime::run_container_terminal`, Task 2 `spawn_local_pty` / `drive_local_pty` / `SHELL_INNER`, existing `parse_engine`, `parse_target`, `validate_container_id`, `audit_intent`.
- Produces:
  - `pub const MAX_ACTIVE_TERMINALS: usize = 4`
  - `pub const TERMINAL_TTL: Duration = Duration::from_secs(2 * 60 * 60)`
  - `pub fn shell_ssh_command(engine: &EngineConfig, container_id: &str) -> Result<String, String>`
  - `pub fn shell_audit_text(engine: &EngineConfig, container_id: &str) -> Result<String, String>`
  - `pub fn requested_terminal_id(requested: Option<&str>) -> Result<String, String>`
  - `pub struct DockerTerminals` with:
    - `pub fn new() -> Self`
    - `pub async fn start_ssh(self: &Arc<Self>, runtime: Arc<SshRuntime>, emitter: PluginEmitter, session_id: String, container_id: &str, engine: EngineConfig, terminal_id: Option<&str>, cols: u32, rows: u32) -> Result<Value, String>`
    - `pub fn start_local(self: &Arc<Self>, emitter: PluginEmitter, container_id: &str, engine: EngineConfig, terminal_id: Option<&str>, cols: u32, rows: u32) -> Result<Value, String>`
    - `pub fn write_input(&self, terminal_id: &str, data: Vec<u8>) -> Result<(), String>`
    - `pub fn resize(&self, terminal_id: &str, cols: u32, rows: u32) -> bool`
    - `pub fn close(&self, terminal_id: &str) -> bool`
    - `pub fn stop_all(&self)`
  - `pub fn audit_terminal_intent(data_dir: &Path, command: &str)`
  - RPC: `docker/terminal/open|resize|close`; binary channel `docker/terminal/in/<terminalId>`; events `docker/terminal/out/<terminalId>` (binary) and `docker/terminal/state` (JSON).

- [ ] **Step 1: Write the failing tests**

Create `backend/src/features/docker/shell_tests.rs`:

```rust
//! Command rendering, id validators and registry limits.

use super::*;
use std::time::Duration;

#[test]
fn ssh_command_is_fixed_and_quotes_only_the_shell_body() {
    let engine = test_engine("docker", Some("unix:///tmp/d.sock"));
    let command = shell_ssh_command(&engine, "d4a7c9f1e2b3").expect("command");
    assert_eq!(
        command,
        "export DOCKER_HOST='unix:///tmp/d.sock' CONTAINER_HOST='unix:///tmp/d.sock'; \
         docker exec -i -t -e TERM=xterm-256color d4a7c9f1e2b3 sh -c \
         'if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi'"
    );
    assert_eq!(
        shell_audit_text(&engine, "d4a7c9f1e2b3").expect("audit"),
        "docker exec -it d4a7c9f1e2b3 sh"
    );
}

#[test]
fn container_id_gate_rejects_names_and_uppercase() {
    assert!(shell_ssh_command(&EngineConfig::default(), "web-nginx").is_err());
    assert!(shell_ssh_command(&EngineConfig::default(), "D4A7C9F1E2B3").is_err());
}

#[test]
fn requested_terminal_id_is_validated_or_generated() {
    assert_eq!(
        requested_terminal_id(Some(" 123e4567-e89b-12d3-a456-426614174000 ")).expect("id"),
        "123e4567-e89b-12d3-a456-426614174000"
    );
    assert!(requested_terminal_id(Some("../bad")).is_err());
    let generated = requested_terminal_id(None).expect("generated");
    assert!(generated.len() >= 8 && generated.is_ascii());
}

#[tokio::test]
async fn registry_enforces_the_concurrency_cap() {
    let registry = DockerTerminals::new();
    for index in 0..MAX_ACTIVE_TERMINALS {
        let id = format!("terminal-{index}-0000");
        registry.insert_for_test(id).await;
    }
    let error = registry.insert_for_test("terminal-9-0000".to_string()).await;
    assert!(error.contains("Too many active container terminals"));
}

#[tokio::test]
async fn close_and_resize_are_idempotent() {
    let registry = DockerTerminals::new();
    assert!(!registry.close("missing"));
    assert!(!registry.resize("missing", 80, 24));
    registry.insert_for_test("terminal-1-0000".to_string()).await;
    assert!(registry.resize("terminal-1-0000", 80, 24));
    assert!(registry.close("terminal-1-0000"));
    registry.remove_for_test("terminal-1-0000");
    assert!(!registry.close("terminal-1-0000"));
}
```

Add a test-only helper to `shell.rs`:

```rust
#[cfg(test)]
impl DockerTerminals {
    pub(crate) async fn insert_for_test(&self, terminal_id: String) -> String { /* reserve and ignore error on success */ }
    pub(crate) fn remove_for_test(&self, terminal_id: &str) { self.lock().remove(terminal_id); }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd backend && cargo test --locked shell_tests -- --nocapture`
Expected: compile error `DockerTerminals` not found.

- [ ] **Step 3: Implement `shell.rs`**

Create `backend/src/features/docker/shell.rs` with the registry. Key code:

```rust
//! Live container terminals: one bounded registry per sidecar, command
//! rendering, event emission and TTL reaping. SSH and local shells share the
//! same registry and event surface.

use super::*;
use super::shell_local::{drive_local_pty, spawn_local_pty, SHELL_INNER};

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use dbx_plugin_sdk::PluginEmitter;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::sessions::ssh::SshRuntime;

pub const MAX_ACTIVE_TERMINALS: usize = 4;
pub const TERMINAL_TTL: Duration = Duration::from_secs(2 * 60 * 60);
const TTL_TICK: Duration = Duration::from_secs(30);
const OUTPUT_QUEUE: usize = 64;
const INPUT_QUEUE: usize = 256;
const RESIZE_QUEUE: usize = 32;
const STOP_GRACE: Duration = Duration::from_secs(5);

struct TerminalEntry {
    cancel: watch::Sender<bool>,
    input: mpsc::Sender<Vec<u8>>,
    resize: mpsc::Sender<(u32, u32)>,
    started: Instant,
}

pub struct DockerTerminals {
    inner: Mutex<HashMap<String, TerminalEntry>>,
}

pub fn shell_ssh_command(engine: &EngineConfig, container_id: &str) -> Result<String, String> {
    validate_container_id(container_id)?;
    Ok(format!(
        "{} exec -i -t -e TERM=xterm-256color {container_id} sh -c {}",
        engine.env_prefix(),
        exec::shell_quote(SHELL_INNER),
    ))
}

pub fn shell_audit_text(engine: &EngineConfig, container_id: &str) -> Result<String, String> {
    validate_container_id(container_id)?;
    Ok(format!("{} exec -it {container_id} sh", engine.cli))
}

pub fn requested_terminal_id(requested: Option<&str>) -> Result<String, String> {
    let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Uuid::new_v4().to_string());
    };
    let valid = (8..=64).contains(&requested.len())
        && requested
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !valid {
        return Err(format!(
            "Invalid terminalId '{requested}': expected 8-64 letters, digits or '-'"
        ));
    }
    Ok(requested.to_string())
}

impl DockerTerminals {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, TerminalEntry>> {
        self.inner.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn reserve(
        &self,
        terminal_id: &str,
    ) -> Result<
        (
            watch::Sender<bool>,
            watch::Receiver<bool>,
            mpsc::Receiver<Vec<u8>>,
            mpsc::Receiver<(u32, u32)>,
        ),
        String,
    > {
        let mut inner = self.lock();
        if inner.len() >= MAX_ACTIVE_TERMINALS {
            return Err(format!(
                "Too many active container terminals ({MAX_ACTIVE_TERMINALS}). Close one before opening another."
            ));
        }
        if inner.contains_key(terminal_id) {
            return Err(format!("terminalId '{terminal_id}' is already in use"));
        }
        let (cancel, cancel_rx) = watch::channel(false);
        let (input_tx, input_rx) = mpsc::channel(INPUT_QUEUE);
        let (resize_tx, resize_rx) = mpsc::channel(RESIZE_QUEUE);
        inner.insert(
            terminal_id.to_string(),
            TerminalEntry {
                cancel: cancel.clone(),
                input: input_tx,
                resize: resize_tx,
                started: Instant::now(),
            },
        );
        Ok((cancel, cancel_rx, input_rx, resize_rx))
    }

    pub async fn start_ssh(
        self: &Arc<Self>,
        runtime: Arc<SshRuntime>,
        emitter: PluginEmitter,
        session_id: String,
        container_id: &str,
        engine: EngineConfig,
        terminal_id: Option<&str>,
        cols: u32,
        rows: u32,
    ) -> Result<Value, String> {
        let command = shell_ssh_command(&engine, container_id)?;
        let terminal_id = requested_terminal_id(terminal_id)?;
        runtime.ensure_writable(&session_id).await?;
        // Open the channel before admission so a missing session/denied PTY is
        // an RPC error with no registry entry to leak.
        let (emit_id, cancel, cancel_rx, input_rx, resize_rx) = {
            let (cancel, cancel_rx, input_rx, resize_rx) = self.reserve(&terminal_id)?;
            (terminal_id.clone(), cancel, cancel_rx, input_rx, resize_rx)
        };
        let registry = Arc::clone(self);
        let runtime_for_task = Arc::clone(&runtime);
        let emitter_for_task = emitter.clone();
        let runner_id = terminal_id.clone();
        let (sink, sink_rx) = mpsc::channel(OUTPUT_QUEUE);
        let handle = tokio::spawn(async move {
            runtime_for_task
                .run_container_terminal(&session_id, &command, cols, rows, sink, input_rx, resize_rx, cancel_rx)
                .await
        });
        tokio::spawn(async move {
            registry.forward(runner_id, emitter_for_task, emit_id, cancel, handle, sink_rx).await;
        });
        Ok(json!({ "terminalId": terminal_id }))
    }

    pub fn start_local(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        container_id: &str,
        engine: EngineConfig,
        terminal_id: Option<&str>,
        cols: u32,
        rows: u32,
    ) -> Result<Value, String> {
        let pty = spawn_local_pty(&engine, container_id, cols, rows)?;
        let terminal_id = requested_terminal_id(terminal_id)?;
        let (cancel, cancel_rx, input_rx, resize_rx) = self.reserve(&terminal_id)?;
        let registry = Arc::clone(self);
        let runner_id = terminal_id.clone();
        let emit_id = terminal_id.clone();
        let (sink, sink_rx) = mpsc::channel(OUTPUT_QUEUE);
        let handle = tokio::spawn(async move {
            drive_local_pty(pty, sink, input_rx, resize_rx, cancel_rx).await
        });
        tokio::spawn(async move {
            registry.forward(runner_id, emitter, emit_id, cancel, handle, sink_rx).await;
        });
        Ok(json!({ "terminalId": terminal_id }))
    }

    async fn forward(
        self: Arc<Self>,
        runner_id: String,
        emitter: PluginEmitter,
        terminal_id: String,
        cancel: watch::Sender<bool>,
        mut runner: JoinHandle<Result<Option<i32>, String>>,
        mut sink: mpsc::Receiver<Vec<u8>>,
    ) {
        let started = self.started(terminal_id.as_str());
        let mut ticker = tokio::time::interval(TTL_TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let result = loop {
            tokio::select! {
                biased;
                _ = ticker.tick() => {
                    if started.is_some_and(|started| started.elapsed() >= TERMINAL_TTL) {
                        let _ = cancel.send(true);
                        break stop_runner(&mut runner, STOP_GRACE).await;
                    }
                }
                chunk = sink.recv() => match chunk {
                    Some(bytes) if !bytes.is_empty() => {
                        let _ = emitter.binary(&format!("docker/terminal/out/{terminal_id}"), &bytes);
                    }
                    Some(_) => {}
                    None => break stop_runner(&mut runner, STOP_GRACE).await,
                },
                status = &mut runner => break status,
            }
        };
        while let Ok(bytes) = sink.try_recv() {
            if !bytes.is_empty() {
                let _ = emitter.binary(&format!("docker/terminal/out/{terminal_id}"), &bytes);
            }
        }
        self.lock().remove(&runner_id);
        match result {
            Ok(Ok(code)) => {
                let _ = emitter.event(
                    "docker/terminal/state",
                    json!({ "terminalId": terminal_id, "state": "exited", "exitCode": code.unwrap_or(-1) }),
                );
            }
            Ok(Err(error)) => {
                let _ = emitter.event(
                    "docker/terminal/state",
                    json!({ "terminalId": terminal_id, "state": "error", "error": error }),
                );
            }
            Err(_) => {}
        }
    }

    fn started(&self, terminal_id: &str) -> Option<Instant> {
        self.lock().get(terminal_id).map(|entry| entry.started)
    }

    pub fn write_input(&self, terminal_id: &str, data: Vec<u8>) -> Result<(), String> {
        let sender = {
            let inner = self.lock();
            inner
                .get(terminal_id)
                .map(|entry| entry.input.clone())
                .ok_or("Container terminal was not found or expired")?
        };
        crate::sessions::input_queue::enqueue_bounded(&sender, data, "Container terminal")
    }

    pub fn resize(&self, terminal_id: &str, cols: u32, rows: u32) -> bool {
        let inner = self.lock();
        inner
            .get(terminal_id)
            .is_some_and(|entry| entry.resize.try_send((cols.max(1), rows.max(1))).is_ok())
    }

    pub fn close(&self, terminal_id: &str) -> bool {
        let inner = self.lock();
        match inner.get(terminal_id) {
            Some(entry) => entry.cancel.send(true).is_ok(),
            None => false,
        }
    }

    pub fn stop_all(&self) {
        let entries: Vec<TerminalEntry> = self.lock().drain().map(|(_, entry)| entry).collect();
        for entry in entries {
            let _ = entry.cancel.send(true);
        }
    }
}

async fn stop_runner(
    runner: &mut JoinHandle<Result<Option<i32>, String>>,
    grace: Duration,
) -> Result<Result<Option<i32>, String>, tokio::task::JoinError> {
    match tokio::time::timeout(grace, &mut *runner).await {
        Ok(result) => result,
        Err(_) => {
            runner.abort();
            match runner.await {
                Ok(result) => Ok(result),
                Err(error) => Err(error),
            }
        }
    }
}

impl Default for DockerTerminals {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DockerTerminals {
    fn drop(&mut self) {
        self.stop_all();
    }
}

pub fn audit_terminal_intent(data_dir: &Path, command: &str) {
    super::local::audit_intent(data_dir, "docker/terminal", command);
}
```

- [ ] **Step 4: Wire the plugin state and RPC**

`backend/src/app/mod.rs`:

```rust
    /// Live container terminal PTYs (`docker/terminal/*`): ≤4 per sidecar,
    /// 2-hour TTL, binary output on `docker/terminal/out/{id}`.
    docker_terminals: Arc<crate::features::docker::DockerTerminals>,
```

`backend/src/app/bootstrap.rs`:

```rust
            docker_terminals: Arc::new(crate::features::docker::DockerTerminals::new()),
```

`backend/src/app/rpc/docker.rs`: replace the whole `"docker/exec"` arm with:

```rust
            "docker/terminal/open" => {
                let container_id = required_string(&params, "containerId")?;
                crate::features::docker::validate_container_id(container_id)?;
                let engine = crate::features::docker::parse_engine(&params)?;
                let cols = required_u32(&params, "cols")?;
                let rows = required_u32(&params, "rows")?;
                let terminal_id = params.get("terminalId").and_then(Value::as_str);
                let audit_text = crate::features::docker::shell_audit_text(&engine, container_id)?;
                if crate::features::docker::parse_target(&params)?
                    == crate::features::docker::DockerTarget::Local
                {
                    crate::features::docker::audit_terminal_intent(&self.ssh.data_dir(), &audit_text);
                    return self.docker_terminals.start_local(
                        emitter.clone(),
                        container_id,
                        engine,
                        terminal_id,
                        cols,
                        rows,
                    );
                }
                let session_id = required_string(&params, "sessionId")?;
                // Reject read-only before audit: a refused open is not an
                // executed command.
                self.runtime
                    .block_on(self.ssh.ensure_writable(session_id))?;
                crate::features::docker::audit_terminal_intent(&self.ssh.data_dir(), &audit_text);
                self.runtime.block_on(self.docker_terminals.start_ssh(
                    self.ssh.clone(),
                    emitter.clone(),
                    session_id.to_string(),
                    container_id,
                    engine,
                    terminal_id,
                    cols,
                    rows,
                ))
            }
            "docker/terminal/resize" => {
                let terminal_id = required_string(&params, "terminalId")?;
                let cols = required_u32(&params, "cols")?;
                let rows = required_u32(&params, "rows")?;
                Ok(json!({ "resized": self.docker_terminals.resize(terminal_id, cols, rows) }))
            }
            "docker/terminal/close" => {
                let terminal_id = required_string(&params, "terminalId")?;
                Ok(json!({ "stopped": self.docker_terminals.close(terminal_id) }))
            }
```

Remove the `"docker/exec"` arm and its now-unused imports. Keep `docker/terminal/*` before `docker/files/*`.

`backend/src/app/handler.rs`: after the `ssh/terminal/in/` branch add:

```rust
        if let Some(terminal_id) = channel.strip_prefix("docker/terminal/in/") {
            let (_sequence, payload) = match decode_sequenced_input(&data) {
                Ok(split) => split,
                Err(error) => return Err(to_plugin_error(error)),
            };
            if let Err(error) = self.docker_terminals.write_input(terminal_id, payload) {
                let _ = emitter.event(
                    "docker/terminal/state",
                    json!({ "terminalId": terminal_id, "state": "error", "error": error }),
                );
                return Err(to_plugin_error(error));
            }
            return Ok(());
        }
```

- [ ] **Step 5: Remove the one-shot terminal**

- Delete `backend/src/features/docker/terminal.rs` and its `mod terminal;` / `pub use terminal::*;` lines in `backend/src/features/docker.rs`.
- Delete `audit_exec_intent` / `audit_exec_failure` from `backend/src/features/docker/local.rs` and their tests.
- Delete every `docker/exec` test in `backend/src/features/docker/tests.rs` (search `exec_` and `docker/exec`).
- Keep `docker/files` and `docker/logs` untouched.

- [ ] **Step 6: Run backend tests**

Run: `cd backend && cargo test --locked shell_tests -- --nocapture && cargo test --locked`
Expected: shell tests PASS; full suite PASS (the removed exec tests no longer compile).

- [ ] **Step 7: Commit**

```bash
git add -A backend
git commit -m "feat(docker): add PTY terminal registry and RPC, remove one-shot exec"
```

---

### Task 4: Frontend terminal transport composable

**Files:**
- Create: `frontend/src/features/docker/composables/useContainerTerminal.ts`
- Create: `frontend/src/features/docker/composables/useContainerTerminal.spec.ts`

**Interfaces:**
- Consumes: `window.dbxPlugin.invoke/notify/onBinary/decodeBase64`, `bridgeBinaryBytes` (`shared/frontend/binaryEvent`), `createTerminalInputQueue` (`features/terminal/lib/emulator/terminalInputQueue`), `randomUUID` (`features/terminal/shared/lib/uuid`), `dockerEngineParams` / `loadDockerEngineSettings`.
- Produces:

```ts
export type ContainerTerminalStatus = "idle" | "connecting" | "connected" | "exited" | "error" | "disabled";

export interface UseContainerTerminalOptions {
  containerId: () => string;
  sessionId: () => string | undefined;
  connectionId: () => string | undefined;
  canWrite: () => boolean;
  running: () => boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
  onData: (data: Uint8Array) => void;
  onState?: (state: ContainerTerminalStatus, detail?: { exitCode?: number; error?: string }) => void;
  createTerminalId?: () => string;
}

export interface UseContainerTerminal {
  status: Ref<ContainerTerminalStatus>;
  errorMessage: Ref<string>;
  open(cols: number, rows: number): Promise<void>;
  close(): void;
  retry(cols: number, rows: number): Promise<void>;
  sendInput(data: string): void;
  resize(cols: number, rows: number): void;
}
```

- [ ] **Step 1: Write the failing test**

Create `frontend/src/features/docker/composables/useContainerTerminal.spec.ts` with a harness mirroring `SplitTerminalPane.spec.ts`:

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useContainerTerminal } from "./useContainerTerminal";

interface Harness {
  invoke: ReturnType<typeof vi.fn>;
  notify: ReturnType<typeof vi.fn>;
  sendBinary: ReturnType<typeof vi.fn>;
  binaryListeners: Array<(event: { channel: string; data?: Uint8Array; dataBase64?: string }) => void>;
  emitOutput(terminalId: string, text: string): void;
}

function installHarness(openResult: () => Promise<unknown> = async () => ({ terminalId: "term-12345678" })): Harness {
  const harness = {
    invoke: vi.fn(async (method: string, params: Record<string, unknown>) => {
      if (method === "docker/terminal/open") return openResult();
      if (method === "docker/terminal/resize") return { resized: true };
      if (method === "docker/terminal/close") return { stopped: true };
      return {};
    }),
    notify: vi.fn(async () => undefined),
    sendBinary: vi.fn(async () => undefined),
    binaryListeners: [] as Harness["binaryListeners"],
    emitOutput(terminalId: string, text: string) {
      for (const listener of this.binaryListeners) {
        listener({ channel: `docker/terminal/out/${terminalId}`, data: new TextEncoder().encode(text) });
      }
    },
  };
  (window as unknown as { dbxPlugin: unknown }).dbxPlugin = {
    invoke: harness.invoke,
    notify: harness.notify,
    sendBinary: harness.sendBinary,
    onBinary: (listener: Harness["binaryListeners"][number]) => {
      harness.binaryListeners.push(listener);
      return () => {
        const index = harness.binaryListeners.indexOf(listener);
        if (index >= 0) harness.binaryListeners.splice(index, 1);
      };
    },
    decodeBase64: (value: string) => Uint8Array.from(atob(value), (character) => character.charCodeAt(0)),
  };
  return harness;
}

function options(overrides: Partial<Parameters<typeof useContainerTerminal>[0]> = {}) {
  return {
    containerId: () => "d4a7c9f1e2b3",
    sessionId: () => "session-1",
    connectionId: () => "conn-1",
    canWrite: () => true,
    running: () => true,
    t: (key: string) => key,
    onData: vi.fn(),
    createTerminalId: () => "term-12345678",
    ...overrides,
  };
}

describe("useContainerTerminal", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => { delete (window as unknown as { dbxPlugin?: unknown }).dbxPlugin; });

  it("subscribes before open and forwards output bytes", async () => {
    const harness = installHarness();
    const onData = vi.fn();
    const terminal = useContainerTerminal(options({ onData }));
    await terminal.open(80, 24);
    expect(harness.invoke).toHaveBeenCalledWith(
      "docker/terminal/open",
      expect.objectContaining({ terminalId: "term-12345678", containerId: "d4a7c9f1e2b3", cols: 80, rows: 24 }),
      expect.anything(),
    );
    harness.emitOutput("term-12345678", "hello");
    expect(onData).toHaveBeenCalledWith(new TextEncoder().encode("hello"));
    expect(terminal.status.value).toBe("connected");
  });

  it("sends input on the docker channel with a sequence prefix", async () => {
    const harness = installHarness();
    const terminal = useContainerTerminal(options());
    await terminal.open(80, 24);
    terminal.sendInput("ls\r");
    await vi.waitFor(() => expect(harness.sendBinary).toHaveBeenCalled());
    const [channel, payload] = harness.sendBinary.mock.calls[0];
    expect(channel).toBe("docker/terminal/in/term-12345678");
    expect(new TextDecoder().decode(payload.slice(8))).toBe("ls\r");
  });

  it("deduplicates unchanged resize requests", async () => {
    const harness = installHarness();
    const terminal = useContainerTerminal(options());
    await terminal.open(80, 24);
    terminal.resize(100, 30);
    terminal.resize(100, 30);
    await vi.waitFor(() => expect(harness.notify).toHaveBeenCalledTimes(1));
    expect(harness.notify).toHaveBeenCalledWith("docker/terminal/resize", { terminalId: "term-12345678", cols: 100, rows: 30 });
  });

  it("reports errors and retries with a fresh terminal id", async () => {
    let attempt = 0;
    const harness = installHarness(async () => {
      attempt += 1;
      if (attempt === 1) throw new Error("no shell");
      return { terminalId: "term-12345678" };
    });
    const terminal = useContainerTerminal(options());
    await terminal.open(80, 24);
    expect(terminal.status.value).toBe("error");
    expect(terminal.errorMessage.value).toContain("no shell");
    await terminal.retry(80, 24);
    expect(terminal.status.value).toBe("connected");
    expect(harness.invoke).toHaveBeenCalledTimes(2);
  });

  it("maps state events for the current terminal only", async () => {
    const harness = installHarness();
    const terminal = useContainerTerminal(options());
    await terminal.open(80, 24);
    terminal.handleStateEvent({ terminalId: "other", state: "exited", exitCode: 1 });
    expect(terminal.status.value).toBe("connected");
    terminal.handleStateEvent({ terminalId: "term-12345678", state: "exited", exitCode: 0 });
    expect(terminal.status.value).toBe("exited");
    expect(harness.invoke).toHaveBeenCalledTimes(1);
  });

  it("does not open for read-only connections or stopped containers", async () => {
    const harness = installHarness();
    const readOnly = useContainerTerminal(options({ canWrite: () => false, t: (key) => `ro:${key}` }));
    await readOnly.open(80, 24);
    expect(harness.invoke).not.toHaveBeenCalled();
    expect(readOnly.status.value).toBe("disabled");

    const stopped = useContainerTerminal(options({ running: () => false, t: (key) => `stopped:${key}` }));
    await stopped.open(80, 24);
    expect(harness.invoke).not.toHaveBeenCalled();
    expect(stopped.status.value).toBe("disabled");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd frontend && pnpm vitest run src/features/docker/composables/useContainerTerminal.spec.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the composable**

Create `frontend/src/features/docker/composables/useContainerTerminal.ts`:

```ts
// 容器 PTY 终端的宿主桥接线：open/close/resize、二进制输入输出、状态事件。
// xterm 实例与视图在组件里；这里只管协议与生命周期，便于单测。
import { ref, type Ref } from "vue";
import { bridgeBinaryBytes } from "../../../shared/frontend/binaryEvent";
import { createTerminalInputQueue } from "../../terminal/lib/emulator/terminalInputQueue";
import { randomUUID } from "../../terminal/shared/lib/uuid";
import { dockerEngineParams, loadDockerEngineSettings } from "../lib/dockerEngine";

export type ContainerTerminalStatus = "idle" | "connecting" | "connected" | "exited" | "error" | "disabled";

const OPEN_TIMEOUT_MS = 35_000;
const INPUT_SEND_TIMEOUT_MS = 8_000;
const OUTPUT_CHANNEL_PREFIX = "docker/terminal/out/";
const INPUT_CHANNEL_PREFIX = "docker/terminal/in/";

export interface UseContainerTerminalOptions {
  containerId: () => string;
  sessionId: () => string | undefined;
  connectionId: () => string | undefined;
  canWrite: () => boolean;
  running: () => boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
  onData: (data: Uint8Array) => void;
  onState?: (state: ContainerTerminalStatus, detail?: { exitCode?: number; error?: string }) => void;
  createTerminalId?: () => string;
}

export function useContainerTerminal(options: UseContainerTerminalOptions) {
  const status = ref<ContainerTerminalStatus>("idle");
  const errorMessage = ref("");
  let terminalId = "";
  let generation = 0;
  let unsubscribeBinary: (() => void) | undefined;
  let lastResizeCols = 0;
  let lastResizeRows = 0;
  let stopped = true;
  const encoder = new TextEncoder();

  const inputQueue = createTerminalInputQueue({
    send: (id, payload) => window.dbxPlugin.sendBinary(`${INPUT_CHANNEL_PREFIX}${id}`, payload),
    sendTimeoutMs: INPUT_SEND_TIMEOUT_MS,
    onError: () => inputQueue.reset(),
  });

  function newTerminalId(): string {
    return options.createTerminalId ? options.createTerminalId() : randomUUID();
  }

  function setStatus(next: ContainerTerminalStatus, detail?: { exitCode?: number; error?: string }) {
    status.value = next;
    options.onState?.(next, detail);
  }

  function stopListening() {
    unsubscribeBinary?.();
    unsubscribeBinary = undefined;
  }

  function handleBinary(event: DbxPluginBinaryEvent) {
    if (stopped || !terminalId) return;
    if (event.channel !== `${OUTPUT_CHANNEL_PREFIX}${terminalId}`) return;
    try {
      const bytes = bridgeBinaryBytes(event, window.dbxPlugin.decodeBase64);
      if (bytes.byteLength) options.onData(bytes);
    } catch {
      // A malformed frame must not kill the session; the next frame recovers.
    }
  }

  async function open(cols: number, rows: number): Promise<void> {
    generation += 1;
    const current = generation;
    close();
    stopped = false;
    errorMessage.value = "";
    if (!options.canWrite()) {
      setStatus("disabled", { error: options.t("docker.terminalReadOnly") });
      return;
    }
    if (!options.running()) {
      setStatus("disabled", { error: options.t("docker.terminalNotRunning") });
      return;
    }
    const api = window.dbxPlugin;
    if (!api) {
      errorMessage.value = options.t("docker.terminalOpenFailed", { error: "host bridge unavailable" });
      setStatus("error", { error: errorMessage.value });
      return;
    }
    terminalId = newTerminalId();
    const spawnCols = Math.max(1, Math.floor(cols));
    const spawnRows = Math.max(1, Math.floor(rows));
    lastResizeCols = spawnCols;
    lastResizeRows = spawnRows;
    setStatus("connecting");
    unsubscribeBinary = api.onBinary(handleBinary);
    try {
      const params: Record<string, unknown> = {
        terminalId,
        containerId: options.containerId(),
        cols: spawnCols,
        rows: spawnRows,
        ...dockerEngineParams(loadDockerEngineSettings(options.connectionId() || "")),
      };
      if (options.sessionId()) params.sessionId = options.sessionId();
      else params.target = "local";
      const result = await api.invoke<{ terminalId?: string }>("docker/terminal/open", params, { timeoutMs: OPEN_TIMEOUT_MS });
      if (current !== generation || stopped) return;
      if (!result?.terminalId) throw new Error(options.t("docker.terminalOpenFailed", { error: "missing terminalId" }));
      const authoritative = result.terminalId;
      if (authoritative !== terminalId) {
        terminalId = authoritative;
        unsubscribeBinary?.();
        unsubscribeBinary = api.onBinary(handleBinary);
      }
      setStatus("connected");
    } catch (cause) {
      if (current !== generation) return;
      const message = cause instanceof Error ? cause.message : String(cause);
      errorMessage.value = options.t("docker.terminalOpenFailed", { error: message });
      setStatus("error", { error: errorMessage.value });
      stopListening();
    }
  }

  function close(): void {
    generation += 1;
    stopped = true;
    stopListening();
    inputQueue.reset();
    const id = terminalId;
    terminalId = "";
    if (id) void window.dbxPlugin?.invoke("docker/terminal/close", { terminalId: id }).catch(() => undefined);
    if (status.value !== "idle") status.value = "idle";
  }

  async function retry(cols: number, rows: number): Promise<void> {
    await open(cols, rows);
  }

  function sendInput(data: string): void {
    if (status.value !== "connected" || !terminalId || !data) return;
    inputQueue.enqueue(terminalId, encoder.encode(data));
  }

  function resize(cols: number, rows: number): void {
    if (status.value !== "connected" || !terminalId) return;
    const nextCols = Math.max(1, Math.floor(cols));
    const nextRows = Math.max(1, Math.floor(rows));
    if (nextCols === lastResizeCols && nextRows === lastResizeRows) return;
    lastResizeCols = nextCols;
    lastResizeRows = nextRows;
    void window.dbxPlugin
      ?.notify("docker/terminal/resize", { terminalId, cols: nextCols, rows: nextRows })
      .catch(() => undefined);
  }

  function handleStateEvent(event: { terminalId?: string; state?: string; exitCode?: number; error?: string }) {
    if (!terminalId || event.terminalId !== terminalId) return;
    if (event.state === "error") {
      errorMessage.value = event.error || options.t("docker.terminalEnded");
      setStatus("error", { error: errorMessage.value });
      stopListening();
    } else if (event.state === "exited") {
      setStatus("exited", { exitCode: event.exitCode });
      stopListening();
    }
  }

  return { status, errorMessage, open, close, retry, sendInput, resize, handleStateEvent };
}
```

`handleStateEvent` is wired by the component through the workbench event bus (the docker pane does not own a global event listener). Document that in the component.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd frontend && pnpm vitest run src/features/docker/composables/useContainerTerminal.spec.ts`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/docker/composables/useContainerTerminal.ts frontend/src/features/docker/composables/useContainerTerminal.spec.ts
git commit -m "feat(docker): add container terminal transport composable"
```

---

### Task 5: xterm terminal tab and theme/font threading

**Files:**
- Modify: `frontend/src/features/docker/components/ContainerTerminalTab.vue` (full rewrite)
- Modify: `frontend/src/features/docker/components/ContainerTerminalTab.spec.ts` (rewrite)
- Modify: `frontend/src/features/docker/components/ContainerDetailPane.vue`
- Modify: `frontend/src/features/docker/components/ContainerDetailPane.spec.ts`
- Modify: `frontend/src/features/docker/components/ContainerManagerPane.vue` (pass theme/font)
- Modify: `frontend/src/app/components/WorkbenchWorkspace.vue`
- Modify: `frontend/src/app/components/WorkbenchWorkspace.spec.ts` if it asserts the docker pane props

**Interfaces:**
- Consumes: Task 4 `useContainerTerminal` / `ContainerTerminalStatus`; `@xterm/xterm`, `@xterm/addon-fit`, `@xterm/addon-unicode11`, `@xterm/addon-web-links`.
- Produces props on `ContainerDetailPane`:
  - `theme?: Record<string, unknown>`
  - `fontFamily?: string`
  - `fontSize?: number`
  - `events?: { onDockerTerminalState: (handler: (event: { terminalId?: string; state?: string; exitCode?: number; error?: string }) => void) => () => void }`
- Produces on `ContainerManagerPane` the same optional props, forwarded unchanged.
- `ContainerTerminalTab` emits none.

- [ ] **Step 1: Write the failing test**

Rewrite `ContainerTerminalTab.spec.ts` around a stubbed xterm. Use this shape:

```ts
import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";
import ContainerTerminalTab from "./ContainerTerminalTab.vue";
import type { DockerContainer } from "../lib/dockerPanelModel";

const terminalWrites: Uint8Array[] = [];
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80; rows = 24;
    constructor() { this.cols = 80; this.rows = 24; }
    loadAddon() {}
    open(host: HTMLElement) { host.dataset.xterm = "ready"; }
    write(data: Uint8Array) { terminalWrites.push(data); }
    onData(handler: (data: string) => void) { (this as unknown as { handler: typeof handler }).handler = handler; return { dispose() {} }; }
    onResize(handler: (size: { cols: number; rows: number }) => void) { (this as unknown as { resizer: typeof handler }).resizer = handler; return { dispose() {} }; }
    attachCustomKeyEventHandler() {}
    dispose() {}
    focus() {}
    hasSelection() { return false; }
    get buffer() { return { active: { type: "normal" } }; }
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} proposeDimensions() { return { cols: 80, rows: 24 }; } } }));
vi.mock("@xterm/addon-unicode11", () => ({ Unicode11Addon: class {} }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));

const container: DockerContainer = {
  id: "d4a7c9f1e2b3", name: "web", image: "nginx:latest", state: "running",
  status: "Up 2 hours", ports: "0.0.0.0:80->80/tcp",
};

function mountTab(overrides: Record<string, unknown> = {}) {
  return mount(ContainerTerminalTab, {
    attachTo: document.body,
    props: {
      t: (key: string) => key,
      container,
      sessionId: "session-1",
      connectionId: "conn-1",
      canWrite: true,
      visible: true,
      theme: { background: "#000000" },
      fontFamily: "Menlo",
      fontSize: 13,
      terminalStateEvents: () => () => undefined,
      ...overrides,
    },
  });
}

describe("ContainerTerminalTab", () => {
  it("opens only when visible and renders a connected xterm", async () => {
    const invoke = vi.fn(async () => ({ terminalId: "term-12345678" }));
    (window as unknown as { dbxPlugin: unknown }).dbxPlugin = {
      invoke, notify: vi.fn(async () => undefined), sendBinary: vi.fn(async () => undefined),
      onBinary: () => () => undefined, decodeBase64: (value: string) => Uint8Array.from(atob(value), (c) => c.charCodeAt(0)),
    };
    const wrapper = mountTab();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(invoke).toHaveBeenCalledWith("docker/terminal/open", expect.objectContaining({ containerId: container.id }), expect.anything());
    expect(wrapper.find("[data-xterm-host]").attributes("data-xterm")).toBe("ready");
    wrapper.unmount();
  });

  it("shows a disabled state for read-only connections without opening", async () => {
    const invoke = vi.fn();
    (window as unknown as { dbxPlugin: unknown }).dbxPlugin = { invoke } as never;
    const wrapper = mountTab({ canWrite: false });
    await vi.waitFor(() => expect(wrapper.text()).toContain("docker.terminalReadOnly"));
    expect(invoke).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("shows a stopped-container hint without opening", async () => {
    const invoke = vi.fn();
    (window as unknown as { dbxPlugin: unknown }).dbxPlugin = { invoke } as never;
    const wrapper = mountTab({ container: { ...container, state: "exited", status: "Exited (0)" } });
    await vi.waitFor(() => expect(wrapper.text()).toContain("docker.terminalNotRunning"));
    expect(invoke).not.toHaveBeenCalled();
    wrapper.unmount();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd frontend && pnpm vitest run src/features/docker/components/ContainerTerminalTab.spec.ts`
Expected: FAIL — the old component renders a command input, not xterm.

- [ ] **Step 3: Rewrite the component**

Implement `ContainerTerminalTab.vue` with:

- Props: `t`, `container`, `sessionId?`, `connectionId?`, `canWrite?`, `visible`, `theme?`, `fontFamily?`, `fontSize?`, `terminalStateEvents?`.
- `const running = computed(() => isDockerContainerRunning(props.container))`.
- `useContainerTerminal({ containerId: () => props.container.id, sessionId: () => props.sessionId, connectionId: () => props.connectionId, canWrite: () => props.canWrite !== false, running: () => running.value, t: props.t, onData: (bytes) => terminal?.write(bytes) })`.
- xterm creation in `onMounted` with `terminalOptionPatch`-free explicit options:

```ts
terminal = new XTerm({
  convertEol: false,
  cursorBlink: true,
  fontFamily: props.fontFamily || "Menlo, Monaco, 'Courier New', monospace",
  fontSize: props.fontSize || 13,
  scrollback: 5_000,
  allowProposedApi: true,
  theme: (props.theme ?? {}) as ITheme,
});
fit = new FitAddon();
terminal.loadAddon(fit);
terminal.loadAddon(new Unicode11Addon());
terminal.unicode.activeVersion = "11";
terminal.loadAddon(new WebLinksAddon());
terminal.open(host);
const dataDisposable = terminal.onData((data) => sendInput(data));
const resizeDisposable = terminal.onResize(({ cols, rows }) => resize(cols, rows));
```

- `startIfNeeded()`: if `visible && running && canWrite !== false && status === "idle"` → `nextTick` → `fit.fit()` → `open(cols, rows)` → focus.
- `watch(() => props.visible, (visible) => { if (visible) { nextTick(() => { fit.fit(); focus(); startIfNeeded(); }); } })`.
- `watch(() => props.container.id, () => { terminal?.reset(); startIfNeeded(); })` and watch `running`/`canWrite`.
- `onBeforeUnmount`: dispose subscriptions, close(), terminal.dispose().
- State subscription: `props.terminalStateEvents?.(handleStateEvent)` stored and unsubscribed on unmount.
- ResizeObserver on host → `fit.fit()` (debounced via requestAnimationFrame).
- Overlay:

```html
<div data-xterm-host class="container-terminal-host" />
<div v-if="status !== 'connected'" class="container-terminal-overlay">
  <p v-if="status === 'disabled'">{{ errorMessage }}</p>
  <p v-else-if="status === 'connecting'">{{ t("docker.terminalConnecting") }}</p>
  <p v-else-if="status === 'exited'">{{ t("docker.terminalExited", { code: exitCode ?? "?" }) }}</p>
  <p v-else-if="status === 'error'" :title="errorMessage">{{ errorMessage }}</p>
  <button v-if="status === 'exited' || status === 'error'" type="button" @click="retry">{{ t("docker.terminalRetry") }}</button>
</div>
```

- Styles: host fills the pane, padding 4px, background `var(--background)`; overlay centered with translucent backdrop; xterm font smoothing.

- [ ] **Step 4: Thread props**

`ContainerDetailPane.vue`: add optional `theme`, `fontFamily`, `fontSize`, `terminalStateEvents`; pass to `ContainerTerminalTab`, which also needs `:visible="activeTab === 'terminal'"`.

`ContainerManagerPane.vue`: add the same optional props and forward them to `ContainerDetailPane`.

`WorkbenchWorkspace.vue`:

```vue
<ContainerManagerPane
  class="docker-pane-surface"
  :style="dockerPaneBasis"
  :t="t"
  :session-id="session?.sessionId"
  :connection-id="connectionId || undefined"
  :can-write="canWrite"
  :theme="splitTerminalTheme"
  :font-family="splitTerminalFont.fontFamily"
  :font-size="splitTerminalFont.fontSize"
  :terminal-state-events="dockerTerminalStateEvents"
  @close="closeDockerPane"
/>
```

`dockerTerminalStateEvents` is a subscription helper bound to the workbench event bus:

```ts
function dockerTerminalStateEvents(handler: (event: { terminalId?: string; state?: string; exitCode?: number; error?: string }) => void): () => void {
  const off = window.dbxPlugin?.onEvent?.((event) => {
    if (event.method === "docker/terminal/state") handler(event.params as never);
  });
  return () => off?.();
}
```

If the workbench event bus exposes an existing typed subscription hook, use that instead of a raw `window.dbxPlugin.onEvent`; keep the same signature.

- [ ] **Step 5: Run the component tests**

Run: `cd frontend && pnpm vitest run src/features/docker/components/ContainerTerminalTab.spec.ts src/features/docker/components/ContainerDetailPane.spec.ts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/docker/components/ContainerTerminalTab.vue frontend/src/features/docker/components/ContainerTerminalTab.spec.ts frontend/src/features/docker/components/ContainerDetailPane.vue frontend/src/features/docker/components/ContainerDetailPane.spec.ts frontend/src/features/docker/components/ContainerManagerPane.vue frontend/src/app/components/WorkbenchWorkspace.vue
git commit -m "feat(docker): render container shell in xterm with workbench appearance"
```

---

### Task 6: Default list+logs split and explicit selection semantics

**Files:**
- Modify: `frontend/src/features/docker/components/ContainerListPane.vue`
- Modify: `frontend/src/features/docker/components/ContainerListPane.spec.ts`
- Modify: `frontend/src/features/docker/components/ContainerManagerPane.vue`
- Modify: `frontend/src/features/docker/components/ContainerManagerPane.spec.ts`

**Interfaces:**
- `ContainerListPane` emit becomes:
  - `select: [container: DockerContainer | null, explicit?: boolean]`
  - `logs: [container: DockerContainer]`
  - `files: [container: DockerContainer]`
- `ContainerManagerPane` behavior:
  - `autoOpened` starts false; first non-null `select` with `explicit !== true` opens the detail with `logs`.
  - Explicit `select` always opens/reopens with `logs` when the detail is closed; keeps the active tab when already open.
  - `select(null)` closes the detail.
  - Poll/refresh with the detail closed never reopens.
  - Connection change resets everything.

- [ ] **Step 1: Write the failing tests**

Add to `ContainerManagerPane.spec.ts`:

```ts
it("opens the logs detail for the first selected container", async () => {
  const wrapper = mount(ContainerManagerPane, { props: baseProps() });
  const list = wrapper.findComponent(ContainerListPane);
  list.vm.$emit("select", runningContainer);
  await nextTick();
  expect(wrapper.findComponent(ContainerDetailPane).exists()).toBe(true);
  expect(wrapper.findComponent(ContainerDetailPane).props("activeTab")).toBe("logs");
  expect(wrapper.find(".container-manager-divider").exists()).toBe(true);
});

it("does not reopen after the user closes the detail on a poll update", async () => {
  const wrapper = mount(ContainerManagerPane, { props: baseProps() });
  const list = wrapper.findComponent(ContainerListPane);
  list.vm.$emit("select", runningContainer);
  await nextTick();
  await wrapper.findComponent(ContainerDetailPane).vm.$emit("close");
  await nextTick();
  expect(wrapper.findComponent(ContainerDetailPane).exists()).toBe(false);
  list.vm.$emit("select", { ...runningContainer, status: "Up 3 hours" });
  await nextTick();
  expect(wrapper.findComponent(ContainerDetailPane).exists()).toBe(false);
});

it("reopens logs when the user clicks a row after closing", async () => {
  const wrapper = mount(ContainerManagerPane, { props: baseProps() });
  const list = wrapper.findComponent(ContainerListPane);
  list.vm.$emit("select", runningContainer);
  await nextTick();
  await wrapper.findComponent(ContainerDetailPane).vm.$emit("close");
  await nextTick();
  list.vm.$emit("select", runningContainer, true);
  await nextTick();
  expect(wrapper.findComponent(ContainerDetailPane).exists()).toBe(true);
  expect(wrapper.findComponent(ContainerDetailPane).props("activeTab")).toBe("logs");
});
```

Add to `ContainerListPane.spec.ts`:

```ts
it("marks row clicks as explicit selections", async () => {
  const wrapper = mountList({ containers: [runningContainer] });
  await nextTick();
  await wrapper.find(".container-row").trigger("click");
  expect(wrapper.emitted("select")?.at(-1)).toEqual([runningContainer, true]);
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd frontend && pnpm vitest run src/features/docker/components/ContainerManagerPane.spec.ts src/features/docker/components/ContainerListPane.spec.ts`
Expected: FAIL — first test sees no detail; row click emits only `[container]`.

- [ ] **Step 3: Implement selection semantics**

`ContainerListPane.vue`:

```ts
const emit = defineEmits<{
  (e: "select", container: DockerContainer | null, explicit?: boolean): void;
  (e: "logs", container: DockerContainer): void;
  (e: "files", container: DockerContainer): void;
}>();

function emitSelection(container: DockerContainer | null, force = false): void {
  const signature = identitySignature(container);
  if (!force && signature === lastSignature) return;
  lastSignature = signature;
  emit("select", container, force || undefined);
}

function selectRow(container: DockerContainer): void {
  selectedId.value = container.id;
  emitSelection(container, true);
}
```

`ContainerManagerPane.vue`:

```ts
const autoOpened = ref(false);
const selected = ref<DockerContainer>();
const activeTab = ref<DetailTab>("logs");
const detailOpen = computed(() => selected.value !== undefined);

function select(container: DockerContainer | null, explicit = false): void {
  if (!container) {
    selected.value = undefined;
    return;
  }
  if (explicit) {
    const wasOpen = detailOpen.value;
    selected.value = container;
    if (!wasOpen) activeTab.value = "logs";
    autoOpened.value = true;
    return;
  }
  if (detailOpen.value) {
    selected.value = container;
    return;
  }
  if (!autoOpened.value) {
    autoOpened.value = true;
    selected.value = container;
    activeTab.value = "logs";
  }
}
```

`watch([sessionId, connectionId])` resets `autoOpened = false`, `selected = undefined`, `activeTab = "logs"`.

`ContainerListPane` row click already calls `selectRow`, so no template change. Row menu Logs/Files emit unchanged.

- [ ] **Step 4: Run the tests**

Run: `cd frontend && pnpm vitest run src/features/docker/components/ContainerManagerPane.spec.ts src/features/docker/components/ContainerListPane.spec.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/docker/components/ContainerListPane.vue frontend/src/features/docker/components/ContainerListPane.spec.ts frontend/src/features/docker/components/ContainerManagerPane.vue frontend/src/features/docker/components/ContainerManagerPane.spec.ts
git commit -m "feat(docker): default to list+logs split and reopen on explicit clicks"
```

---

### Task 7: Mock, i18n, docs, cleanup, and full verification

**Files:**
- Modify: `frontend/src/app/mock/host/docker.ts` (or create `dockerTerminal.ts` and register it)
- Modify: `frontend/src/app/mockDbxHost.ts` (binary routing)
- Modify: `frontend/src/shared/lib/i18n/messages/base/{en,zh-CN,zh-TW,ja,es,it,pt-BR}.ts`
- Modify: `frontend/src/features/docker/README.md`
- Modify: `docs/superpowers/specs/2026-10-26-container-manager-pane-design.md` (note the default split supersedes list-only)
- Modify: `CHANGELOG.md`, `manifest.json`, `backend/Cargo.toml`, `backend/Cargo.lock` (version bump)

**Interfaces:**
- Mock methods: `docker/terminal/open` → `{ terminalId }`, `docker/terminal/resize` → `{ resized: true }`, `docker/terminal/close` → `{ stopped: true }`.
- Mock binary channel: `docker/terminal/in/<id>` echoes input; Enter emits `\r\nroot@<id>:/# `; `?docker=off` open throws `docker CLI not found on this host`.
- i18n keys to add:
  - `docker.terminalConnecting`
  - `docker.terminalExited`
  - `docker.terminalRetry`
  - `docker.terminalReadOnly`
  - `docker.terminalNotRunning`
  - `docker.terminalOpenFailed`
  - `docker.terminalEnded`
  - `docker.terminalHintPty`
- i18n keys to remove: `execHint`, `execPlaceholder`, `execRun`, `execClear`, `execEmpty`, `execRunning`, `execFailed`, `execExitCode`, `execReadOnly`.

- [ ] **Step 1: Implement mock terminal handling**

In `frontend/src/app/mock/host/docker.ts` replace the `docker/exec` block with:

```ts
  if (method === "docker/terminal/open") {
    const cli = cliName(input);
    requireContainerId(input);
    if (dockerMode() === "off") throw new Error(`${cli} CLI not found on this host`);
    const terminalId = String(input.terminalId || "mock-terminal");
    emitHostEvent({
      channel: `docker/terminal/out/${terminalId}`,
      data: new TextEncoder().encode(`root@${String(input.containerId).slice(0, 12)}:/# `),
    });
    return { terminalId };
  }
  if (method === "docker/terminal/resize") return { resized: true };
  if (method === "docker/terminal/close") return { stopped: true };
```

Export a helper `handleDockerTerminalInput(channel: string, bytes: Uint8Array)` from the same file:

```ts
export function handleDockerTerminalInput(channel: string, bytes: Uint8Array): void {
  const terminalId = channel.slice("docker/terminal/in/".length);
  const text = new TextDecoder().decode(bytes.subarray(8));
  const payload = text.endsWith("\r")
    ? `${text.slice(0, -1)}\r\nroot@mock:/# `
    : text;
  emitHostEvent({ channel: `docker/terminal/out/${terminalId}`, data: new TextEncoder().encode(payload) });
}
```

In `frontend/src/app/mockDbxHost.ts` `sendBinary`, route `docker/terminal/in/` to that helper before the SSH/local branches.

- [ ] **Step 2: Add i18n keys (all 7 locales) and remove exec keys**

Add exact copy (translate per locale, keep placeholders):

- en: `terminalConnecting: "Connecting to the container shell…"`, `terminalExited: "Shell exited (code {code})."`, `terminalRetry: "Reconnect"`, `terminalReadOnly: "This connection is read-only; the container terminal is disabled."`, `terminalNotRunning: "Start the container before opening a terminal."`, `terminalOpenFailed: "Cannot open the container terminal: {error}"`, `terminalEnded: "Container shell ended."`, `terminalHintPty: "Interactive shell in the container (uses sh, or bash when available)."`.
- zh-CN: `terminalConnecting: "正在连接容器 shell…"`, `terminalExited: "Shell 已退出（退出码 {code}）。"`, `terminalRetry: "重新连接"`, `terminalReadOnly: "此连接为只读，容器终端不可用。"`, `terminalNotRunning: "容器未运行，请先启动容器再打开终端。"`, `terminalOpenFailed: "无法打开容器终端：{error}"`, `terminalEnded: "容器 shell 已结束。"`, `terminalHintPty: "容器内交互式 shell（使用 sh，有 bash 时优先 bash）。"`。
- zh-TW / ja / es / it / pt-BR: translate the same set; keep `{code}` / `{error}` untouched.
- Delete the nine `exec*` keys.

- [ ] **Step 3: Update docs and version**

- `frontend/src/features/docker/README.md`: replace the command-console paragraph with the PTY architecture (SSH channel + local portable-pty, registry limits 4/TTL 2h, input/output channels, default split).
- `docs/superpowers/specs/2026-10-26-container-manager-pane-design.md`: add a dated note that the default is now list+logs split and row clicks reopen Logs.
- Bump `manifest.json`, `backend/Cargo.toml`, `backend/Cargo.lock`, `CHANGELOG.md` to `0.7.9-beta.35`; CHANGELOG section:

```markdown
## 0.7.9-beta.35

- 容器管理默认显示「列表 + 日志」分栏；收起详情后点击容器行会重新以日志打开。
- 容器终端改为真正的 PTY 交互式 shell：SSH 走现有连接上的独立 PTY 通道，本机走本地伪终端；沿用工作台字体与配色，支持方向键/历史、Ctrl+C、全屏程序与自适应尺寸。
- 移除逐条命令的旧代码终端与 `docker/exec` 接口。
```

- [ ] **Step 4: Run the full gates**

```bash
cd frontend && pnpm typecheck && pnpm vitest run
cd ../backend && cargo test --locked
cd .. && scripts/test.sh --skip-host
```

Expected: all PASS; Docker smoke PASS or SKIP when Docker is unavailable.

- [ ] **Step 5: Browser fixture check**

Run the existing Vite fixture (`frontend` visual server on `:5299`) and verify:

1. Container manager opens with list + logs split; logs streaming visible.
2. Close detail → wide list; click a row → logs reopen.
3. Terminal tab connects; type `echo hi`; output and prompt render; resize divider and confirm no xterm corruption.
4. `?docker=off` shows the open-failed message and retry.

- [ ] **Step 6: Package**

```bash
scripts/build.sh
shasum -a 256 dist/io.dbx.sshx-0.7.9-beta.35-darwin-arm64.dbxp
```

Record the artifact size and SHA-256 in the final report. Do not install into DBX.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "chore: release beta.35"
```

---

## Execution order for parallel subagents

**Wave A (parallel, disjoint files):** Task 1 (SSH PTY), Task 2 (local PTY), Task 4 (frontend composable), Task 6 (selection semantics).
**Wave B (after Wave A):** Task 3 (backend registry/RPC; depends on 1+2), Task 5 (terminal UI; depends on 4 + 6 for ManagerPane props).
**Wave C:** Task 7 (mock/i18n/docs/gates/package).

No subagent commits; the controller commits per task after review. Task 5 and Task 6 both touch `ContainerManagerPane.vue`, so they must not run in the same wave.

## Self-review

- **Spec coverage:** default split (Task 6), PTY SSH (Tasks 1/3), local PTY (Task 2), xterm + theme/font (Task 5), transport (Task 4), protocol/limits/audit (Task 3), removal of old exec (Task 3/7), mock/i18n/docs/verification (Task 7). No gaps.
- **Placeholders:** no TBD/TODO; each code step has concrete content. The only intentionally deferred detail is the exact workbench event-bus helper name in Task 5 Step 4, with an explicit fallback (`window.dbxPlugin.onEvent`) so the implementer is never blocked.
- **Type consistency:** `terminalId`, `docker/terminal/in|out|state`, `ContainerTerminalStatus`, `select(container, explicit?)`, `detailOpen`, `autoOpened` are used consistently across tasks.
