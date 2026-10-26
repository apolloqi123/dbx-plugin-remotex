//! Plugin construction and the sidecar serve loop.

use super::diagnostics::{log_sidecar_exit, spawn_terminal_input_counter};
use crate::platform::paths::plugin_data_dir;
use super::*;
use crate::core::plugin_ids::PLUGIN_ID;

impl Plugin {
    fn new() -> Result<Self, String> {
        let data_dir = plugin_data_dir();
        let runtime =
            Runtime::new().map_err(|error| format!("Failed to create async runtime: {error}"))?;
        crate::core::otp_store::init_data_dir(&data_dir);
        crate::features::import::remove_legacy_store(&data_dir)?;
        // Sidecar 启动即同步 X11 快速标志（重启会丢进程内状态）。
        let prefs = crate::core::preferences::load_preferences(&data_dir);
        crate::features::forwarding::x11::set_enabled(
            prefs.get("x11_forwarding").and_then(Value::as_bool) == Some(true),
        );
        // 会话自动录制（M14）沿用 X11 快速标志模式：open_session 读进程内
        // 原子量，不重复解析 preferences.json。
        crate::features::recording::set_auto_record(
            prefs.get("auto_record").and_then(Value::as_bool) == Some(true),
        );
        let ssh = Arc::new(SshRuntime::new(data_dir));
        Ok(Self {
            runtime,
            mcp: Arc::new(crate::features::mcp::McpState::shared(ssh.clone())),
            ssh,
            local: Arc::new(crate::sessions::local::LocalTerminalRuntime::new()),
            telnet: Arc::new(crate::sessions::telnet::TelnetSessionRuntime::new()),
            serial: Arc::new(crate::sessions::serial::SerialSessionRuntime::new()),
            vnc: Arc::new(crate::sessions::vnc::VncSessionRuntime::new()),
            rdp: Arc::new(crate::sessions::rdp::RdpSessionRuntime::new()),
            watcher: Arc::new(crate::filesystem::watch::WatchRuntime::new()),
            docker_streams: Arc::new(crate::features::docker::DockerStreams::new()),
            docker_uploads: std::sync::Mutex::new(
                crate::features::docker::files::UploadRegistry::new(std::env::temp_dir()),
            ),
            connection_import: crate::features::import::ImportStream::default(),
            non_ssh_creds: crate::core::non_ssh_credentials::NonSshCredentials::new(),
            tasks: Arc::new(crate::features::automation::task_provider::TaskProvider::new()),
        })
    }
}

pub(crate) fn run() -> std::io::Result<()> {
    // MCP stdio mode: expose SSH/SFTP tools to MCP clients over JSON-RPC
    // instead of running the DBX plugin server.
    if std::env::args().any(|arg| arg == "--mcp") {
        return crate::features::mcp::run_mcp_stdio(plugin_data_dir());
    }
    // Kernel teardown net: every child (local-terminal ConPTY conhost + shell
    // trees included) dies with the process, so a crashed/killed sidecar can
    // no longer orphan shells that burn a core in their headless conhosts.
    // No-op outside Windows; best-effort by design (never blocks startup).
    #[cfg(windows)]
    crate::platform::job_object::setup_process_job();
    log_sidecar_exit("serve-start".to_string());
    spawn_terminal_input_counter();
    let plugin = Plugin::new().map_err(std::io::Error::other)?;
    let metadata = PluginMetadata::new(PLUGIN_ID, env!("CARGO_PKG_VERSION"))
        .with_capability("connections")
        .with_capability("events")
        .with_capability("binary")
        .with_capability("filesystem");
    let result = PluginServer::new(metadata, plugin)
        .transport(PluginTransport::Framed)
        .worker_threads(4)
        .serve();
    // serve() only returns when stdin reaches EOF (host closed the pipe) or a
    // read fails; everything else — SIGTERM/SIGKILL, a crash — ends the
    // process without any trace. #33/#71 debugging showed mid-session sidecar
    // exits that leave zero output in the host log, so the exit cause is
    // appended here to tell "host closed stdin" (line present) apart from
    // "host signalled the process" (no line).
    log_sidecar_exit(match &result {
        Ok(()) => "serve-ok stdin-eof".to_string(),
        Err(error) => format!("serve-err {error}"),
    });
    result
}
