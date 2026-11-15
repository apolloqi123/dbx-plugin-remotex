//! Response payloads, event shapers and pure command/path builders shared by
//! the SSH runtime. No locks are taken here: callers pass plain values in and
//! receive owned payloads out, so this module cannot change guard scope or
//! ordering.

use super::*;

/// sudo 上传车道的暂存件名（登录用户 SFTP home 下）。独立前缀
/// `.dbx-sudo-ul-` 与下载车道的 `.dbx-sudo-dl-` 互不误伤。
pub(crate) fn sudo_upload_staging_name(task_id: &str) -> String {
    format!(".dbx-sudo-ul-{task_id}.part")
}

/// sudo 上传车道的 exec 超时（exec 层钳到 5..300）：chmod 快路径 30s；
/// 提交 mv 走最大档——跨文件系统时 mv 退化为复制，要给足大文件时间。
pub(crate) const SUDO_UPLOAD_QUICK_SECS: u64 = 30;

pub(crate) const SUDO_UPLOAD_COMMIT_SECS: u64 = 300;

/// sudo 提交体（纯函数，供单测）：`target→backup` 让位 → 暂存件 `mv` 到
/// 位 → backup 清理；mv 失败把 backup 滚回 target 后以非零退出。路径一律
/// [`shell_quote`] 单引号转义；整体再经 exec 层 `sanitize_sudo_command`
/// 包进 `sh -c '…'`。跨文件系统时 `mv` 退化为复制+删除（非原子，登记边界）。
pub(crate) fn sudo_commit_body(staging: &str, target: &str, backup: &str) -> String {
    let (staging, target, backup) = (
        exec::shell_quote(staging),
        exec::shell_quote(target),
        exec::shell_quote(backup),
    );
    format!(
        "if [ -e {target} ]; then mv -f -- {target} {backup}; fi; \
         if mv -f -- {staging} {target}; then rm -f -- {backup}; exit 0; fi; \
         if [ -e {backup} ]; then mv -f -- {backup} {target}; fi; exit 1"
    )
}

/// Shared shape of every upload progress event; `fileName` is only present on
/// task-start events (the workbench keeps the name it already displayed).
#[allow(clippy::too_many_arguments)]
pub(crate) fn upload_progress_payload(
    task_id: &str,
    session_id: &str,
    file_name: Option<&str>,
    transferred: u64,
    size: u64,
    phase: UploadPhase,
    status: &str,
    compression: CompressionMode,
) -> Value {
    let mut payload = json!({
        "taskId": task_id,
        "sessionId": session_id,
        "direction": "upload",
        "transferred": transferred,
        "size": size,
        "phase": phase.as_str(),
        "status": status,
        // M33 压缩通道标记与下载事件同语义：gzip=压缩通道，none=普通管线
        // （含预压回退后的后续事件——前端据 none 清掉徽标）。
        "compression": compression.as_str(),
    });
    if let Some(file_name) = file_name {
        payload["fileName"] = json!(file_name);
    }
    payload
}

/// Error text for a cancelled upload. The optional reason slug comes from the
/// workbench so "Upload cancelled by user" reads differently from an
/// error-triggered cleanup ("Upload cancelled (ack-timeout)").
pub(crate) fn upload_cancel_error(reason: Option<&str>) -> String {
    match reason.map(str::trim).filter(|value| !value.is_empty()) {
        Some("user") => "Upload cancelled by user".to_string(),
        Some(reason) => format!("Upload cancelled ({reason})"),
        None => "Upload cancelled".to_string(),
    }
}

/// Download counterpart of [`upload_cancel_error`] (prep-only cancellation
/// events carry no transfer row to borrow a message from).
pub(crate) fn download_cancel_error(reason: Option<&str>) -> String {
    match reason.map(str::trim).filter(|value| !value.is_empty()) {
        Some("user") => "Download cancelled by user".to_string(),
        Some(reason) => format!("Download cancelled ({reason})"),
        None => "Download cancelled".to_string(),
    }
}

/// Builds the response served from the snapshot cache: the stored fields plus
/// a `cachedAt` marker (Unix seconds) so callers can show the age of the data.
/// The stored snapshot itself is never mutated.
pub(crate) fn cached_metrics_payload(snapshot: &Value, collected_at: u64) -> Value {
    let mut payload = snapshot.clone();
    if let Some(object) = payload.as_object_mut() {
        object.insert("cachedAt".to_string(), json!(collected_at));
    }
    payload
}

/// Read-only connection display info resolved from the connections registry
/// for a session row. Empty host/user means the connection is no longer
/// registered (sidecar restarted after the workbench attached).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionEndpoint {
    pub host: String,
    pub port: u16,
    pub username: String,
}

impl ConnectionEndpoint {
    pub fn fallback() -> Self {
        Self {
            host: String::new(),
            port: 22,
            username: String::new(),
        }
    }
}

/// One row of `ssh/sessions/list`. Pure so tests can exercise the payload
/// shape without a live SSH connection.
// 参数与 JSON 行字段一一对应，是纯载荷整形函数的自然形状。
#[allow(clippy::too_many_arguments)]
pub(crate) fn session_info_payload(
    session_id: &str,
    connection_id: &str,
    workbench_id: &str,
    read_only: bool,
    connected: bool,
    sudo_keepalive: bool,
    terminal_keepalive_secs: u64,
    created_at_secs: u64,
    auth_method: &str,
    endpoint: &ConnectionEndpoint,
    recording: bool,
) -> Value {
    json!({
        "sessionId": session_id,
        "connectionId": connection_id,
        "workbenchId": workbench_id,
        "readOnly": read_only,
        "connected": connected,
        "recording": recording,
        "sudoKeepalive": sudo_keepalive,
        "terminalKeepaliveSecs": terminal_keepalive_secs,
        "createdAt": created_at_secs,
        "authMethod": auth_method,
        "host": endpoint.host,
        "port": endpoint.port,
        "username": endpoint.username,
    })
}

/// Total order for `sftp/transfer/history` rows (issue #18). Newest first by
/// `startedAt`; rows without one — live snapshots whose start record never
/// landed on disk — count as the most recent activity and lead the list;
/// `taskId` breaks every tie so the order no longer depends on the HashMap
/// iteration order of the in-memory registries. Pure so tests can exercise
/// it without a runtime.
pub(crate) fn compare_history_rows(a: &Value, b: &Value) -> std::cmp::Ordering {
    pub(crate) fn started(task: &Value) -> Option<u64> {
        task.get("startedAt").and_then(Value::as_u64)
    }
    pub(crate) fn task_id(task: &Value) -> &str {
        task.get("taskId").and_then(Value::as_str).unwrap_or("")
    }
    match (started(a), started(b)) {
        (Some(left), Some(right)) => right.cmp(&left).then_with(|| task_id(a).cmp(task_id(b))),
        // Rows without a timestamp (live snapshots) lead the list as the
        // most recent activity; `Less`/`Greater` place a timestamped row
        // strictly after a timestamp-less one.
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => task_id(a).cmp(task_id(b)),
    }
}
