//! Plugin application shell: shared state, lifecycle helpers, and RPC routing.

pub(crate) mod actions;
mod bootstrap;
pub(crate) mod diagnostics;
mod handler;
mod lifecycle_grace;
pub(crate) mod params;
pub(crate) mod rpc;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use dbx_plugin_sdk::{
    PluginEmitter, PluginError, PluginHandler, PluginMetadata, PluginServer, PluginTransport,
    RequestContext,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::runtime::Runtime;

use crate::core::model::{
    path_from_sftp_uri, RuntimeEndpoint, SessionOpenRequest, StoredConnection, MAX_TRANSFER_SIZE,
};
use crate::sessions::ssh::{connection_id_param, filesystem_path, PromptDecision, SshRuntime};

use crate::platform::paths::plugin_data_dir;

struct Plugin {
    runtime: Runtime,
    ssh: Arc<SshRuntime>,
    local: Arc<crate::sessions::local::LocalTerminalRuntime>,
    telnet: Arc<crate::sessions::telnet::TelnetSessionRuntime>,
    serial: Arc<crate::sessions::serial::SerialSessionRuntime>,
    vnc: Arc<crate::sessions::vnc::VncSessionRuntime>,
    rdp: Arc<crate::sessions::rdp::RdpSessionRuntime>,
    /// Host ↔ remote SFTP transfer tasks for the remote desktop toolbar.
    remote_files: Arc<crate::features::remote_files::RemoteFilesRuntime>,
    mcp: Arc<crate::features::mcp::McpState>,
    watcher: Arc<crate::filesystem::watch::WatchRuntime>,
    /// Live `docker logs -f` streams (`docker/logs/start|stop`): bounded at
    /// four per sidecar, reaped by TTL, emitting `docker/logs/chunk|end`.
    docker_streams: Arc<crate::features::docker::DockerStreams>,
    /// Live container terminal PTYs (`docker/terminal/*`): ≤4 per sidecar,
    /// 2-hour TTL, binary output on `docker/terminal/out/{id}`.
    docker_terminals: Arc<crate::features::docker::DockerTerminals>,
    /// 本机分片上传表（`docker/files/upload/*`）：≤4 活跃、单文件
    /// 512 MiB、30 分钟过期；finished 条目保留到 cleanup。
    docker_uploads: std::sync::Mutex<crate::features::docker::files::UploadRegistry>,
    connection_import: crate::features::import::ImportStream,
    /// 非 SSH 连接（telnet/vnc/rdp）的已存凭据暂存：connection/connect 生命周期
    /// 存、*\/start 合并、connection/disconnect 清（见 non_ssh_credentials 模块）。
    non_ssh_creds: crate::core::non_ssh_credentials::NonSshCredentials,
    /// Short grace windows for host-initiated teardown/re-push. DBX can
    /// disconnect and reconnect an unchanged SSH connection while editing a
    /// different connection; keeping the live PTY across that recycle avoids a
    /// visible terminal reconnect. See `app/lifecycle_grace.rs`.
    pending_ssh_disconnects: Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
    pending_workbench_closes: Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
    /// Session ids whose webview-initiated close is waiting out a host
    /// workbench re-init; an attach to the same session cancels it.
    pending_ssh_session_closes: Arc<std::sync::Mutex<std::collections::HashMap<String, u64>>>,
    lifecycle_grace_seq: AtomicU64,
    /// Scheduler Task Provider（io.dbx.remotex.tasks）：固定 task/* RPC 面
    /// （validate/execute/start/stop/status）与常驻会话监管，见
    /// src/task_provider.rs 与冻结契约 ADR §6。
    tasks: Arc<crate::features::automation::task_provider::TaskProvider>,
}

impl Plugin {
    /// 连接级 SFTP 文件名编码判定（M16）：sessionId → connectionId →
    /// 连接覆盖 > 全局偏好 > 缺省 auto。会话未知/已断开按未覆盖处理
    /// （跟随全局），判定绝不因会话状态失败。
    fn resolve_sftp_encoding(
        &self,
        session_id: &str,
    ) -> crate::filesystem::sftp::name::NameEncoding {
        self.resolve_sftp_encoding_opt(Some(session_id))
    }

    /// 同上，供 watchId/taskId 链上查不到所属会话的入口使用：None 时
    /// 跳过连接覆盖直接回退全局（与"会话未知按未覆盖"语义一致）。
    fn resolve_sftp_encoding_opt(
        &self,
        session_id: Option<&str>,
    ) -> crate::filesystem::sftp::name::NameEncoding {
        let connection_id = match session_id {
            Some(session_id) => self
                .runtime
                .block_on(self.ssh.connection_id_for_session(session_id)),
            None => None,
        };
        crate::core::preferences::sftp_name_encoding_for(
            &plugin_data_dir(),
            connection_id.as_deref(),
        )
    }

    /// Async liveness probe for the file watchers: an emission only prompts
    /// when the owning SSH session still exists. Built from `list_sessions`
    /// because the session table itself stays inside ssh.rs; a listing
    /// failure must never kill watches, so it reports "alive".
    fn session_probe(&self) -> crate::filesystem::watch::SessionProbe {
        let ssh = self.ssh.clone();
        Arc::new(move |session_id: String| {
            let ssh = ssh.clone();
            Box::pin(async move {
                ssh.list_sessions()
                    .await
                    .get("sessions")
                    .and_then(Value::as_array)
                    .map(|rows| {
                        rows.iter().any(|row| {
                            row.get("sessionId").and_then(Value::as_str)
                                == Some(session_id.as_str())
                        })
                    })
                    .unwrap_or(true)
            })
        })
    }
}

pub(crate) use bootstrap::run;
