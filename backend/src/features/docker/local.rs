//! Docker feature layer.

use super::*;

/// Where the docker family resolves its daemon: the SSH connection the
/// call is addressed to, or the local daemon on the machine the sidecar
/// runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerTarget {
    Ssh,
    Local,
}

pub fn parse_target(params: &Value) -> Result<DockerTarget, String> {
    match params.get("target") {
        None | Some(Value::Null) => Ok(DockerTarget::Ssh),
        Some(Value::String(target)) => match target.as_str() {
            "ssh" => Ok(DockerTarget::Ssh),
            "local" => Ok(DockerTarget::Local),
            other => Err(format!(
                "Unsupported docker target '{other}'. Supported: ssh (default), local"
            )),
        },
        Some(other) => Err(format!(
            "Invalid docker target {other}: expected \"ssh\" or \"local\""
        )),
    }
}

/// Well-known docker CLI install dirs beyond PATH, per platform. macOS GUI
/// processes (DBX → sidecar) and non-interactive remote shells inherit a
/// skeletal PATH, so Homebrew, Docker Desktop, OrbStack and rootless
/// installs are invisible to a bare lookup — the local plane probes these
/// explicitly, exactly like [`LIST_SCRIPT`]'s PATH line does remotely.
pub fn cli_search_dirs(home: &Path, is_windows: bool) -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = if is_windows {
        vec!["C:\\Program Files\\Docker\\Docker\\resources\\bin".into()]
    } else {
        vec!["/usr/local/bin".into(), "/opt/homebrew/bin".into()]
    };
    dirs.push(home.join(".docker").join("bin"));
    dirs.push(home.join(".orbstack").join("bin"));
    dirs.push(home.join("bin"));
    dirs.push(home.join(".local").join("bin"));
    if is_windows {
        dirs.push("C:\\ProgramData\\DockerDesktop\\version-bin".into());
    } else {
        dirs.push("/snap/bin".into());
        dirs.push("/Applications/Docker.app/Contents/Resources/bin".into());
    }
    dirs
}

pub(crate) fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Resolves the local container CLI: an explicit `cli` that looks like a
/// path (`/usr/local/bin/podman`, `C:\...\docker.exe`) is probed directly;
/// a bare name resolves through PATH first, then [`cli_search_dirs`].
/// Returns the executable path so every local invocation is direct process
/// exec — no shell, nothing to quote, works on Windows too (where the
/// heredoc collector style has no `sh` to run under).
pub fn find_local_cli(cli: &str) -> Option<std::path::PathBuf> {
    let has_dir_part = cli.contains('/') || cli.contains('\\');
    let exe = if cfg!(windows) && !cli.to_ascii_lowercase().ends_with(".exe") {
        format!("{cli}.exe")
    } else {
        cli.to_string()
    };
    if has_dir_part {
        let path = std::path::PathBuf::from(&exe);
        return is_executable_file(&path).then_some(path);
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(&exe);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    for dir in cli_search_dirs(&home, cfg!(windows)) {
        let candidate = dir.join(&exe);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// One local `docker` invocation: direct exec (argv, no shell). Output is
/// `wait_with_output`'s stdout followed by stderr — NOT interleaved in
/// order-of-arrival (docker writes diagnostics to stderr and data to stdout,
/// so appending keeps `logs` 2>&1 semantics at the cost of ordering); bounded
/// by `timeout`. Exit code -1 covers signal deaths like the remote parse path.
/// A configured endpoint rides the engine's own env var (`DOCKER_HOST` /
/// `CONTAINER_HOST`) on the spawned process — never argv, never a shell.
pub(crate) async fn run_local(
    cli: &Path,
    args: &[&str],
    timeout: Duration,
    endpoint: Option<&str>,
) -> Result<(i32, String), String> {
    use std::process::Stdio;
    use tokio::process::Command;
    let mut command = Command::new(cli);
    command.args(args).stdin(Stdio::null());
    if let Some(endpoint) = endpoint {
        command
            .env("DOCKER_HOST", endpoint)
            .env("CONTAINER_HOST", endpoint);
    }
    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn {:?} failed: {error}", cli))?;
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| format!("{cli:?} timed out after {}s", timeout.as_secs()))?
        .map_err(|error| format!("container CLI failed: {error}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.code().unwrap_or(-1), text))
}

/// The unavailable payload shape, shared with the remote probe semantics:
/// `needsSudo` marks CLI-found-but-daemon-denied (the docker-group hint),
/// plain unavailable covers both "CLI missing" and "daemon not running"
/// with the generic empty state.
pub(crate) fn unavailable_payload(needs_sudo: bool) -> Value {
    json!({ "available": false, "needsSudo": needs_sudo, "containers": [] })
}

/// `docker/list` for `target: "local"`. Same payload contract as the
/// remote path — `available` / `needsSudo` / `containers` — assembled into
/// the same marker text so [`list_payload`] stays the single parser.
pub async fn collect_list_local(engine: &EngineConfig) -> Result<Value, String> {
    let Some(cli) = find_local_cli(&engine.cli) else {
        return Ok(unavailable_payload(false));
    };
    let (info_code, info_output) = run_local(
        &cli,
        &["info", "--format", "{{.ServerVersion}}"],
        LIST_TIMEOUT,
        engine.endpoint.as_deref(),
    )
    .await?;
    if info_code != 0 {
        return Ok(unavailable_payload(is_daemon_permission_failure(
            info_code,
            &info_output,
        )));
    }
    let (_, ps_output) = run_local(
        &cli,
        &["ps", "-a", "--no-trunc", "--format", PS_FORMAT],
        LIST_TIMEOUT,
        engine.endpoint.as_deref(),
    )
    .await?;
    // 与远端 heredoc 同一兜底：老 CLI（<1.13）的 .State 模板报错时输出为
    // 空，改跑 legacy 占位符集并打标记（issue #168）。
    let section = if ps_output.trim().is_empty() {
        let (_, legacy_output) = run_local(
            &cli,
            &["ps", "-a", "--no-trunc", "--format", PS_FORMAT_LEGACY],
            LIST_TIMEOUT,
            engine.endpoint.as_deref(),
        )
        .await?;
        format!("{PS_LEGACY_MARKER}\n{legacy_output}")
    } else {
        ps_output
    };
    Ok(list_payload(&format!(
        "DBXDOCKER_PROBE\ndocker-found\ndaemon-ok\nDBXDOCKER_PS\n{section}"
    )))
}

/// `docker/stats` for `target: "local"`: one direct `stats --no-stream`
/// call (no shell, endpoint via env), assembled into the same marker text
/// as the remote collector so [`stats_payload`] stays the only parser.
pub async fn collect_stats_local(
    engine: &EngineConfig,
    container_id: &str,
) -> Result<Value, String> {
    validate_container_id(container_id)?;
    let cli = find_local_cli(&engine.cli)
        .ok_or_else(|| format!("{} CLI not found on this machine", engine.cli))?;
    let (_, output) = run_local(
        &cli,
        &[
            "stats",
            "--no-stream",
            "--format",
            STATS_FORMAT,
            container_id,
        ],
        STATS_TIMEOUT,
        engine.endpoint.as_deref(),
    )
    .await?;
    Ok(stats_payload(&format!("{STATS_MARKER}\n{output}")))
}

/// `docker/logs` for `target: "local"`: inspect first, then the tail, both
/// assembled into the marker text so [`logs_payload`] parses unchanged.
/// A failed inspect (container gone) degrades to `container: null` exactly
/// like the remote script's `2>/dev/null` section.
pub async fn collect_logs_local(
    engine: &EngineConfig,
    container_id: &str,
    tail: u64,
) -> Result<Value, String> {
    validate_container_id(container_id)?;
    let tail = clamp_tail(tail);
    let cli = find_local_cli(&engine.cli)
        .ok_or_else(|| format!("{} CLI not found on this machine", engine.cli))?;
    let (inspect_code, inspect_output) = run_local(
        &cli,
        &["inspect", container_id],
        LOGS_TIMEOUT,
        engine.endpoint.as_deref(),
    )
    .await?;
    if inspect_code != 0 && !inspect_output.trim_start().starts_with('[') {
        // Missing container: keep going into logs, which surface the same
        // "No such container" error — the remote script does the same.
        if is_daemon_permission_failure(inspect_code, &inspect_output) {
            return Err(format!(
                "{} inspect failed: {inspect_output}. Local container access \
                 needs daemon permission: start Docker Desktop/OrbStack/Podman \
                 Desktop, or add your user to the docker group.",
                engine.cli
            ));
        }
    }
    let tail_text = tail.to_string();
    let (_, logs_output) = run_local(
        &cli,
        &["logs", "--tail", &tail_text, container_id],
        LOGS_TIMEOUT,
        engine.endpoint.as_deref(),
    )
    .await?;
    Ok(logs_payload(&format!(
        "DBXDOCKER_INSPECT\n{inspect_output}\nDBXDOCKER_LOGS\n{logs_output}"
    )))
}

/// Read size for streamed log pipes: one chunk per pipe read, coalesced
/// later by the registry's batcher.
const STREAM_CHUNK_BYTES: usize = 8 * 1024;
/// Exit code reported by [`spawn_logs_stream`] when cancellation kills the
/// child. The registry normalizes it into the shared `error:"cancelled"`
/// end payload.
const STREAM_CANCELLED_EXIT: i32 = 130;

/// Why a fragment send stopped early.
enum SendStop {
    Cancelled,
    /// The registry/forwarder is gone; nothing drains the sink anymore.
    ReceiverClosed,
}

impl SendStop {
    /// Terminal outcome of a stopped send: cancellation maps to the cancelled
    /// exit code, a vanished consumer to an error.
    fn into_result(self) -> Result<i32, String> {
        match self {
            Self::Cancelled => Ok(STREAM_CANCELLED_EXIT),
            Self::ReceiverClosed => Err("Log stream consumer went away".to_string()),
        }
    }
}

/// Resolves once cancellation is requested. A dropped sender counts as
/// "never" (the SSH exec helper of the same name): the sender is dropped only
/// after the registry entry is removed, which already implies the stream
/// ended.
async fn wait_cancelled(cancel: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Forwards one fragment, racing the (possibly back-pressured) bounded send
/// against cancellation. `docker logs -f` blocks while its pipe is not
/// drained, so a stalled send would otherwise keep the child — and `stop` —
/// alive forever.
async fn send_or_cancel(
    sink: &tokio::sync::mpsc::Sender<Vec<u8>>,
    bytes: Vec<u8>,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), SendStop> {
    tokio::select! {
        biased;
        _ = wait_cancelled(cancel) => Err(SendStop::Cancelled),
        sent = sink.send(bytes) => sent.map_err(|_| SendStop::ReceiverClosed),
    }
}

/// `docker logs -f` as a local child process: argv-only direct exec, both
/// pipes streamed to `sink`, and cancellation kills the child so no orphan
/// follower survives a stopped pane. The endpoint rides the same two env
/// vars as [`run_local`] (never argv, never a shell). Exit code
/// [`STREAM_CANCELLED_EXIT`] reports a cancelled stream; otherwise the
/// child's own code is returned (-1 for signal deaths, like the remote parse
/// path).
pub async fn spawn_logs_stream(
    engine: &EngineConfig,
    container_id: &str,
    tail: u64,
    sink: tokio::sync::mpsc::Sender<Vec<u8>>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<i32, String> {
    use std::process::Stdio;
    use tokio::process::Command;
    validate_container_id(container_id)?;
    let cli = find_local_cli(&engine.cli)
        .ok_or_else(|| format!("{} CLI not found on this machine", engine.cli))?;
    let mut command = Command::new(cli);
    command
        .arg("logs")
        .arg("-f")
        .arg("--tail")
        .arg(clamp_tail(tail).to_string())
        .arg(container_id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(endpoint) = engine.endpoint.as_deref() {
        command
            .env("DOCKER_HOST", endpoint)
            .env("CONTAINER_HOST", endpoint);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to start {} logs: {error}", engine.cli))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    // The pending read arm cannot borrow the same receiver as the cancel arm,
    // so cancellation is watched through a second handle on the same channel.
    let mut read_cancel = cancel.clone();
    let mut stdout_done = false;
    let mut stderr_done = false;
    while !(stdout_done && stderr_done) {
        tokio::select! {
            biased;
            _ = wait_cancelled(&mut read_cancel) => {
                let _ = child.kill().await;
                return Ok(STREAM_CANCELLED_EXIT);
            }
            read = read_chunk(&mut stdout), if !stdout_done => match read? {
                None => stdout_done = true,
                Some(bytes) => {
                    if let Err(stop) = send_or_cancel(&sink, bytes, &mut cancel).await {
                        let _ = child.kill().await;
                        return stop.into_result();
                    }
                }
            },
            read = read_chunk(&mut stderr), if !stderr_done => match read? {
                None => stderr_done = true,
                Some(bytes) => {
                    if let Err(stop) = send_or_cancel(&sink, bytes, &mut cancel).await {
                        let _ = child.kill().await;
                        return stop.into_result();
                    }
                }
            },
        }
    }
    let status = child.wait().await.map_err(|error| error.to_string())?;
    Ok(status.code().unwrap_or(-1))
}

/// One bounded read from a streamed pipe; `None` marks EOF.
async fn read_chunk<R>(reader: &mut R) -> Result<Option<Vec<u8>>, String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut buffer = vec![0u8; STREAM_CHUNK_BYTES];
    match reader.read(&mut buffer).await {
        Ok(0) => Ok(None),
        Ok(read) => {
            buffer.truncate(read);
            Ok(Some(buffer))
        }
        Err(error) => Err(format!("Failed to read container logs: {error}")),
    }
}

/// `docker/action` for `target: "local"`. Same whitelist, id gate and
/// success contract as the remote plane; the sudo fallback does not exist
/// locally (no Quick Sudo plumbing for the machine itself), so a daemon
/// permission failure returns the actionable guidance instead.
pub async fn perform_action_local(
    engine: &EngineConfig,
    container_id: &str,
    action: DockerAction,
) -> Result<Value, String> {
    validate_container_id(container_id)?;
    let cli = find_local_cli(&engine.cli)
        .ok_or_else(|| format!("{} CLI not found on this machine", engine.cli))?;
    let (exit_code, output) = run_local(
        &cli,
        &[action.as_str(), container_id],
        ACTION_TIMEOUT,
        engine.endpoint.as_deref(),
    )
    .await?;
    if exit_code == 0 {
        return Ok(json!({ "success": true, "output": output }));
    }
    if is_daemon_permission_failure(exit_code, &output) {
        return Err(format!(
            "{} {}: {output}. Local container actions need daemon permission: \
             start Docker Desktop/OrbStack/Podman Desktop, or add your user to \
             the docker group.",
            engine.cli,
            action.as_str()
        ));
    }
    Err(format!(
        "{} {} failed (exit {}): {}",
        engine.cli,
        action.as_str(),
        exit_code,
        output
    ))
}

// —— Audit (workbench docker/action + docker/terminal paths) ————————

/// Audit rows for the workbench docker paths. The tool name is a parameter
/// so `docker/action` and `docker/terminal` share one writer (and one shape)
/// instead of duplicating the entry construction.
pub(crate) fn audit_row(
    data_dir: &Path,
    tool: &str,
    command: &str,
    outcome: audit_log::EntryOutcome,
    error: Option<&str>,
    duration_ms: u64,
) {
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let entry = audit_log::AuditEntry {
        ts_ms,
        tool: tool.to_string(),
        // The workbench path keys on sessionId; the connection column stays
        // empty exactly like MCP rows without a connectionId argument.
        connection_id: String::new(),
        gate: audit_log::GateOutcome::Pass,
        approval: audit_log::ApprovalTrail::None,
        outcome,
        exit_code: None,
        duration_ms,
        mode: audit_log::ExecMode::Stdio,
        command: Some(command.to_string()),
        output: None,
        error: error.map(|error| error.chars().take(256).collect()),
    };
    if let Err(error) = audit_log::append(data_dir, &entry) {
        eprintln!("[docker] audit append failed: {error}");
    }
}

/// Mandatory pre-execution audit row: the intent is on the ledger even if
/// the sidecar dies between the audit write and the remote run.
pub fn audit_intent(data_dir: &Path, tool: &str, command: &str) {
    audit_row(
        data_dir,
        tool,
        command,
        audit_log::EntryOutcome::Ok,
        None,
        0,
    );
}

/// Post-execution failure row (success is already covered by the intent
/// row; only failures add a second entry carrying the error text).
pub fn audit_failure(data_dir: &Path, tool: &str, command: &str, error: &str, duration_ms: u64) {
    audit_row(
        data_dir,
        tool,
        command,
        audit_log::EntryOutcome::Error,
        Some(error),
        duration_ms,
    );
}

pub fn audit_action_intent(data_dir: &Path, command: &str) {
    audit_intent(data_dir, "docker/action", command);
}

pub fn audit_action_failure(data_dir: &Path, command: &str, error: &str, duration_ms: u64) {
    audit_failure(
        data_dir,
        "docker/action",
        command,
        error,
        duration_ms,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_logs_stream_validates_the_container_id_first() {
        let (sink, _chunks) = tokio::sync::mpsc::channel(1);
        let (_cancel, cancel) = tokio::sync::watch::channel(false);
        let engine = crate::features::docker::test_engine("docker", None);
        let error = spawn_logs_stream(&engine, "web-nginx", 200, sink, cancel)
            .await
            .expect_err("invalid id");
        assert!(error.contains("Invalid containerId"), "{error}");
    }

    #[tokio::test]
    async fn spawn_logs_stream_reports_a_missing_cli() {
        let (sink, _chunks) = tokio::sync::mpsc::channel(1);
        let (_cancel, cancel) = tokio::sync::watch::channel(false);
        let engine = crate::features::docker::test_engine("dbx-missing-container-cli", None);
        let error = spawn_logs_stream(&engine, "d4a7c9f1e2b3", 200, sink, cancel)
            .await
            .expect_err("missing cli");
        assert!(error.contains("CLI not found on this machine"), "{error}");
    }

    #[tokio::test]
    async fn send_or_cancel_beats_a_saturated_sink() {
        let (sink, _chunks) = tokio::sync::mpsc::channel(1);
        // Fill the only slot so the next send would block indefinitely.
        sink.send(b"first".to_vec()).await.unwrap();
        let (cancel_tx, mut cancel) = tokio::sync::watch::channel(false);
        let signalled = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = cancel_tx.send(true);
        });

        let started = std::time::Instant::now();
        let stop = send_or_cancel(&sink, b"second".to_vec(), &mut cancel)
            .await
            .expect_err("blocked send must lose to cancel");
        assert!(matches!(stop, SendStop::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(1));
        signalled.await.unwrap();
    }

    #[tokio::test]
    async fn send_or_cancel_reports_a_closed_consumer() {
        let (sink, chunks) = tokio::sync::mpsc::channel(1);
        drop(chunks);
        let (_cancel_tx, mut cancel) = tokio::sync::watch::channel(false);
        let stop = send_or_cancel(&sink, b"lost".to_vec(), &mut cancel)
            .await
            .expect_err("closed receiver");
        assert!(matches!(
            stop.into_result(),
            Err(error) if error.contains("consumer went away")
        ));
    }
}
