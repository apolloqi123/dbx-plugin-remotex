//! MCP state layer.

use super::*;

pub(crate) struct McpConnection {
    pub(crate) handle: Arc<Handle<SshClient>>,
    #[allow(dead_code)]
    pub(crate) jumps: Vec<Arc<Handle<SshClient>>>,
    pub(crate) sftp: Option<Arc<AsyncMutex<SftpSession>>>,
}

impl McpConnection {
    pub(crate) async fn sftp(&mut self) -> Result<Arc<AsyncMutex<SftpSession>>, String> {
        if let Some(sftp) = self.sftp.as_ref() {
            return Ok(sftp.clone());
        }
        let channel = crate::sessions::ssh::open_session_channel_untracked(&self.handle)
            .await
            .map_err(|error| format!("Failed to open SFTP channel: {error}"))?;
        if let Err(error) = channel.request_subsystem(true, "sftp").await {
            // Open channel, refused subsystem: close it so the server session
            // is released instead of leaking a `MaxSessions` slot.
            let _ = channel.eof().await;
            let _ = channel.close().await;
            return Err(format!("Failed to start SFTP: {error}"));
        }
        let sftp = Arc::new(AsyncMutex::new(
            SftpSession::new_with_config(channel.into_stream(), sftp_client_config(false))
                .await
                .map_err(|error| format!("Failed to start SFTP: {error}"))?,
        ));
        self.sftp = Some(sftp.clone());
        Ok(sftp)
    }
}

/// 在 `handle` 上独立打开一条 sftp 子系统通道跑裸包客户端（严格串行请求/
/// 响应）。独立函数化：MCP 工具在释放 connections 写锁之后仍要按需新建
/// 裸包客户端（latin-1 车道），不能依赖 &mut McpConnection。
pub(crate) async fn open_raw_sftp(handle: &Handle<SshClient>) -> Result<RawSftpClient, String> {
    let channel = crate::sessions::ssh::open_session_channel_untracked(handle)
        .await
        .map_err(|error| format!("Failed to open SFTP channel: {error}"))?;
    if let Err(error) = channel.request_subsystem(true, "sftp").await {
        let _ = channel.eof().await;
        let _ = channel.close().await;
        return Err(format!("Failed to start SFTP: {error}"));
    }
    sftp_raw::RawSftp::init(channel.into_stream()).await
}

pub struct McpState {
    pub(crate) runtime: Arc<SshRuntime>,
    pub(crate) connections: AsyncRwLock<HashMap<String, McpConnection>>,
    /// StoredConnection payloads registered through `mcp/call` (DBX
    /// connections), kept so a dropped pooled handle can be reconnected
    /// without asking the caller for credentials again.
    pub(crate) dbx_connections: AsyncRwLock<HashMap<String, StoredConnection>>,
    /// MCP size preferences, mirrored to `limits_path` on every change.
    pub(crate) limits: RwLock<McpLimits>,
    /// §1.3 permission settings, sharing `limits_path`'s
    /// `mcp-settings.json` (separate read/write guard).
    pub(crate) permission: RwLock<McpPermission>,
    pub(crate) limits_path: PathBuf,
    /// Operator-level kill switch: `DBX_SSH_MCP_READ_ONLY` forces every
    /// tool call (bridge and standalone alike) through the read-only gates.
    pub(crate) global_read_only: bool,
    /// L1 stdio bridge fallback switch: forwards unregistered-`connectionId`
    /// calls to the running DBX app through the local TCP bridge. Always on
    /// in production; tests flip it off to keep decision paths hermetic (no
    /// real app bridge on the box).
    pub(crate) bridge_fallback: bool,
}

/// Truthy values accepted for `DBX_SSH_MCP_READ_ONLY`.
fn env_read_only() -> bool {
    matches!(
        std::env::var("DBX_SSH_MCP_READ_ONLY").ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "on")
    )
}

impl McpState {
    pub fn new(data_dir: PathBuf) -> Self {
        let limits_path = data_dir.join("mcp-settings.json");
        let limits = McpLimits::load(&limits_path);
        let permission = McpPermission::load(&limits_path);
        Self {
            runtime: Arc::new(SshRuntime::new(data_dir).with_auto_trust_keys()),
            connections: AsyncRwLock::new(HashMap::new()),
            dbx_connections: AsyncRwLock::new(HashMap::new()),
            limits: RwLock::new(limits),
            permission: RwLock::new(permission),
            limits_path,
            global_read_only: env_read_only(),
            bridge_fallback: true,
        }
    }

    /// Wraps the workbench sidecar's shared runtime so `mcp/call` reuses the
    /// plugin's connection registry, known_hosts store, and settings.
    pub fn shared(runtime: Arc<SshRuntime>) -> Self {
        let limits_path = runtime.data_dir().join("mcp-settings.json");
        let limits = McpLimits::load(&limits_path);
        let permission = McpPermission::load(&limits_path);
        Self {
            runtime,
            connections: AsyncRwLock::new(HashMap::new()),
            dbx_connections: AsyncRwLock::new(HashMap::new()),
            limits: RwLock::new(limits),
            permission: RwLock::new(permission),
            limits_path,
            global_read_only: env_read_only(),
            bridge_fallback: true,
        }
    }
}
