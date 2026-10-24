//! SSH support.

use super::*;

mod payload;
mod runtime;

pub use payload::ConnectionEndpoint;
pub(crate) use payload::{
    cached_metrics_payload, compare_history_rows, download_cancel_error, session_info_payload,
    sudo_commit_body, sudo_upload_staging_name, upload_cancel_error, upload_progress_payload,
    SUDO_UPLOAD_COMMIT_SECS, SUDO_UPLOAD_QUICK_SECS,
};
pub(crate) use runtime::{SharedTransportLease, TaskTransport};
#[cfg(test)]
pub(crate) use runtime::TransportLeaseCounter;

/// Remote (-R) port mapping table for one connection, keyed by
/// `(listen_host, bound_port)` and consulted by the SSH client handler when
/// the server hands over a `forwarded-tcpip` channel.
pub type RemoteForwardTable = Mutex<HashMap<(String, u32), forward::RelayTarget>>;

/// `workbench_id` is kept with the session so attach can only restore the
/// session that belongs to the same workbench. A different workbench must open
/// its own SSH session instead of stealing another tab's PTY.
pub(crate) struct SessionEntry {
    pub(crate) connection_id: String,
    pub(crate) workbench_id: RwLock<String>,
    pub(crate) read_only: bool,
    pub(crate) connected: AtomicBool,
    /// Unix seconds when the session was opened (for `ssh/sessions/list`).
    pub(crate) created_at_secs: u64,
    /// Monotonic per-runtime creation counter. created_at_secs has second
    /// granularity, so two workbenches opened in the same second would tie;
    /// the sequence gives `session_id_for_connection` a true creation order.
    pub(crate) created_seq: u64,
    pub(crate) handle: Arc<Handle<SshClient>>,
    /// Reference-counted lifetime for the authenticated transport's jump
    /// chain. Pending copied-session opens reserve a use before awaiting.
    pub(crate) transport_lease: Arc<SharedTransportLease>,
    /// Live Quick Sudo / 2FA orchestration, updatable at runtime through
    /// `ssh/settings/set` and shared by the terminal and exec paths.
    pub(crate) orchestration: Arc<RwLock<SudoAuth>>,
    /// In-terminal Quick Sudo watcher. Shared so `sync_auto_sudo` can attach
    /// or detach it at runtime (a password configured after the session was
    /// opened, or the Quick Sudo toggle, must apply without reconnecting);
    /// the read loop re-locks it on every output chunk.
    pub(crate) auto_sudo: Arc<Mutex<Option<exec::TerminalAutoSudo>>>,
    /// Expect-style trigger engine (tssh parity). Attached when the
    /// connection carries a `triggers` configuration; the read loop feeds it
    /// BEFORE the auto-sudo watcher (contract D1: a chunk answered by one of
    /// the two must never be answered by both).
    pub(crate) triggers: Arc<Mutex<Option<triggers::TriggerEngine>>>,
    pub(crate) terminal_tx: mpsc::Sender<TerminalCommand>,
    pub(crate) replay: Arc<AsyncMutex<ReplayBuffer>>,
    pub(crate) sftp: AsyncMutex<Option<Arc<AsyncMutex<SftpSession>>>>,
    /// Output recorder installed while an agent command runs in this
    /// session's PTY (`exec_in_terminal`); `None` outside such a run. Fed
    /// by the read loop at the same point as the auto-sudo observer.
    pub(crate) agent_recorder: Arc<Mutex<Option<TerminalRecorder>>>,
    /// Session recording (`ssh/recording/start`): captures the terminal
    /// output stream into an asciicast v2 file while active. Fed by the
    /// read loop at the same point as the agent recorder; auto-stopped
    /// when the session's read loop ends.
    pub(crate) session_recorder: Arc<Mutex<Option<session_recording::SessionRecorder>>>,
    /// Serializes agent-terminal executions on this session: two concurrent
    /// MCP commands must not interleave keystrokes on the same PTY or
    /// overwrite each other's recorder. Deliberately per-session (runs on
    /// different connections stay parallel) and deliberately held across the
    /// approval wait too — the wait is part of the serialization so an
    /// un-approved command cannot be raced by a second one. `Arc`-wrapped so
    /// a run can hold an owned guard across awaits.
    pub(crate) agent_exec_lock: Arc<AsyncMutex<()>>,
}

pub(crate) struct UploadState {
    pub(crate) session_id: String,
    pub(crate) remote_path: String,
    pub(crate) expected_size: u64,
    pub(crate) received: u64,
    pub(crate) local_path: PathBuf,
    pub(crate) file: std::fs::File,
    /// Staging push（`sftp/upload/chunk`）每块一发的进度事件节流。
    pub(crate) progress_throttle: crate::features::transfer::progress::ProgressThrottle,
    /// 压缩通道（M33）：start 时决策（偏好/阈值/黑名单/远端 gunzip 探测），
    /// finish 推送前对 spool 预压。latin-1 车道在决策处强制 None。
    pub(crate) compression: CompressionMode,
    /// sudo 上传车道（UploadSudo）：finish 的推送目标从目标目录 `.part`
    /// 改为登录用户可写位置的暂存件（[`sudo_upload_staging_name`]），
    /// 最终提交走 exec `sudo mv`（让位→提交→回滚）。false = 普通
    /// SFTP 车道（目标目录内暂存 + 同目录 rename 原子提交）。
    pub(crate) sudo_lane: bool,
}

/// Local (client-machine) persistence target for a download started with
/// `saveToLocal`: chunks are appended to a staging `.part` file while they
/// stream through, and `complete_download` renames it into the user's
/// Downloads folder once every byte arrived.
pub(crate) struct DownloadSink {
    pub(crate) part_path: PathBuf,
    pub(crate) final_dir: PathBuf,
    /// 冲突策略："overwrite" 直接覆盖同名文件，否则撞名让位（" (n)"）。
    pub(crate) overwrite: bool,
    pub(crate) file: AsyncMutex<tokio::fs::File>,
}

#[derive(Clone)]
pub(crate) struct DownloadState {
    pub(crate) session_id: String,
    /// Single-file download: the file itself. Folder download: the root
    /// directory (`tree` is `Some` in that case).
    pub(crate) remote_path: String,
    pub(crate) file_name: String,
    /// Single-file download: file size. Folder download: aggregate byte total
    /// across the tree (the progress denominator).
    pub(crate) size: u64,
    /// Single-file download: next chunk offset. Folder download: aggregate
    /// transferred bytes across finished files.
    pub(crate) next_offset: u64,
    pub(crate) sink: Option<Arc<DownloadSink>>,
    /// Present only for recursive folder downloads (`sftp/download/tree/start`);
    /// the chunk/finish/cancel paths branch on it while sharing the registry,
    /// progress events and cancel plumbing with plain file downloads.
    ///
    /// Shared (not cloned) ownership: the chunk pump mutates the tree in place
    /// behind `progress`, so a per-chunk `DownloadState` clone stays O(1)
    /// instead of deep-copying the remaining file queue and failure list.
    pub(crate) tree: Option<Arc<TreeDownloadState>>,
    /// Present only for sudo-backed downloads (`sudo/download/start`): the
    /// remote staging temp file that must be removed on finish, cancel,
    /// error and session close (`sudo_download::discard_tmp`).
    pub(crate) sudo_tmp: Option<String>,
    /// 单文件下载生效编码为 latin-1（M28-B）：size 探测与分块读取按
    /// `has_wire_lane` 判裸包车道——latin-1 的 wire 域字面 `%` 已自转义，
    /// `%XX` 唯一解读是转义还原；auto 车道列表 uri 字面 `%` 未自转义
    /// （D-7），一律走高层客户端（字面量语义与列表一致）。树/sudo 任务
    /// 不经此字段分支（树按 `TreeDownloadState.latin1`，sudo 走独立车道）。
    pub(crate) latin1: bool,
    /// 下载限速（issue #66）：start 时对偏好 `transfer_download_limit_kib`
    /// 的一次性快照（0=不限速）。整个任务沿用快照值——改动对下一个下载
    /// 任务生效，进行中的任务不受中途修改影响。sudo 下载车道本期不限速。
    pub(crate) throttle: Throttle,
    /// `download_chunk` 每块一发的进度事件节流（clone 出来的快照不含此
    /// 字段的最新状态；决策只在 registry 锁内做）。
    pub(crate) progress_throttle: crate::features::transfer::progress::ProgressThrottle,
    /// 压缩通道（M33）：start 时决策（偏好/阈值/黑名单/远端工具探测/只读/
    /// latin-1）。Gzip 时分块读取走本地解压后的 `staged_plain`，网络腿在
    /// prep 任务里按压缩流走；None 保持现状管线。
    pub(crate) compression: CompressionMode,
    /// 压缩 prep 是否完成（ready 事件已发、分块可读）。回退（Fallback）
    /// 也置位——status 的 ready 标记是前端等待环的唯一出口。
    pub(crate) ready: bool,
    /// prep 回退标记（M33）：true = 压缩决策曾判 Gzip 但 prep 失败/不划算，
    /// 分块供给走普通管线。`compression` 字段保持原决策值（status 的
    /// compression/ready 字段继续输出），分块泵按本标记分派。
    pub(crate) serving_plain: bool,
    /// 压缩任务：本地解压后的 plain 暂存文件（`download-<taskId>.plain`）。
    /// 分块读取与 saveToLocal 落盘都以它为准；非压缩任务为 None。
    pub(crate) staged_plain: Option<PathBuf>,
    /// 压缩任务：本地压缩流暂存（`download-<taskId>.part.gz`，树为
    /// `tree-<taskId>.tgz`），终态清理。
    pub(crate) gz_staging: Option<PathBuf>,
    /// 压缩任务：远端临时压缩件（`<dir>/.dbx-download-<taskId>.gz`，树为
    /// `.dbx-tree-<taskId>.tgz`），终态 best-effort 删除。
    pub(crate) remote_temp: Option<String>,
    /// 压缩任务（M33）saveToLocal 落盘目标与冲突策略——普通任务存于
    /// DownloadSink，压缩任务无 sink（解压直写 staged_plain），单独记录。
    pub(crate) final_dir: Option<PathBuf>,
    pub(crate) overwrite: bool,
    /// 高层车道复用的远端读句柄（单文件下载）：逐片 OPEN+SEEK+READ+CLOSE
    /// 每片多付 2 个 RTT，高延迟链路吞吐被 RTT 界死。File 的 Drop 发
    /// close_nowait——槽位置 None 即完成关闭，无需显式清理链。文件夹下载
    /// 不复用本槽位：树的在途句柄随 `TreeDownloadProgress.read_handle`
    /// 逐文件复用（换文件时丢弃重开），收益见 tree_download。
    pub(crate) read_handle: Arc<AsyncMutex<Option<russh_sftp::client::fs::File>>>,
    /// 裸包车道（latin-1 转义名）复用的裸包客户端：此前逐片新开一条 SFTP
    /// 子系统通道（open session + request_subsystem + INIT 握手，多付数个
    /// RTT），同一理由缓存在任务槽——单文件与树下载共用（客户端与路径
    /// 无关，逐文件的 open/close 仍按 read_chunk 语义进行）。RawSftp 的
    /// poisoned 语义本就是一次性报废：任何错误置 None 由下一片惰性重建，
    /// 重建只是多一次握手。
    pub(crate) raw_client: Arc<AsyncMutex<Option<RawSftpClient>>>,
}

/// Live state of one recursive folder download. Files stream through the same
/// chunked pipeline one at a time: `current` is in flight into `sink`'s
/// staging `.part` file, which is renamed into place as soon as the file is
/// complete, so a mid-tree failure leaves no partial file behind.
///
/// Immutable fields live directly on the struct (status/list reads them
/// without locking); the mutable queue/current/failures/handle live behind
/// `progress` so the chunk pump owns them in place. This replaces the old
/// clone-and-swap design: each chunk used to deep-copy the remaining
/// `files` queue, `failures` and the open sink (O(n²) across a tree download).
pub(crate) struct TreeDownloadState {
    /// Fresh, collision-free local root directory created by `start`; the
    /// whole tree is removed from disk when the task is cancelled.
    pub(crate) root_local: PathBuf,
    pub(crate) file_count: u64,
    pub(crate) skipped: u64,
    /// latin-1 连接标记：files 的 remote_path 是 wire 形式（raw 扫描），
    /// 逐文件读取必须走裸包车道——高层客户端按 UTF-8 open 字节名路径必
    /// NO_SUCH_FILE（真机 smoke 曝露的第四层 wire 缺口）。
    pub(crate) latin1: bool,
    /// Serializes chunk pumps for this task. `progress` is deliberately NOT
    /// held across SFTP open/seek/read, sink writes, throttle pacing or frame
    /// emission: cancel and finish must be able to read/clear the shared state
    /// promptly even while a whole throttled chunk is in flight. One pump at a
    /// time is still required (the caller-side loop is sequential per task).
    pub(crate) pump: AsyncMutex<()>,
    /// Test-only source root: the high-level lane opens `<root>/<relative>`
    /// from local disk instead of an SFTP session, so handle reuse, seeks and
    /// file switches can be exercised without a server.
    #[cfg(test)]
    pub(crate) local_source_root: Option<PathBuf>,
    /// Test-only count of handle opens (distinct from chunks read), used to
    /// assert reuse across chunks and reopen on file switch.
    #[cfg(test)]
    pub(crate) opened_handles: AtomicUsize,
    pub(crate) progress: AsyncMutex<TreeDownloadProgress>,
}

impl TreeDownloadState {
    pub(crate) fn new(
        root_local: PathBuf,
        file_count: u64,
        skipped: u64,
        latin1: bool,
        progress: TreeDownloadProgress,
    ) -> Self {
        Self {
            root_local,
            file_count,
            skipped,
            latin1,
            pump: AsyncMutex::new(()),
            #[cfg(test)]
            local_source_root: None,
            #[cfg(test)]
            opened_handles: AtomicUsize::new(0),
            progress: AsyncMutex::new(progress),
        }
    }
}

/// Seekable async byte source for one in-flight tree file. Production wraps the
/// high-level SFTP handle; tests wrap a local file through
/// [`TreeDownloadState::local_source_root`]. Dropping the handle closes the
/// remote file (`File`'s Drop sends `close_nowait`).
pub(crate) trait TreeReadSource:
    tokio::io::AsyncRead + tokio::io::AsyncSeek + Send + Unpin
{
}

impl<T> TreeReadSource for T where T: tokio::io::AsyncRead + tokio::io::AsyncSeek + Send + Unpin {}

pub(crate) type TreeReadHandle = Box<dyn TreeReadSource>;

/// Mutable half of [`TreeDownloadState`], locked only for state snapshots and
/// commits. The cached `read_handle` is the high-level SFTP file of the file in
/// `current`; it is reused across chunks (absolute seek per chunk) and only
/// dropped on file switch, failure, finish or cancel — `File`'s Drop sends
/// `close_nowait`, so clearing the slot closes the remote handle.
pub(crate) struct TreeDownloadProgress {
    pub(crate) files: VecDeque<sftp_tree::TreeFile>,
    pub(crate) current: Option<sftp_tree::TreeFile>,
    pub(crate) current_offset: u64,
    pub(crate) sink: Option<Arc<DownloadSink>>,
    pub(crate) files_done: u64,
    pub(crate) failures: Vec<Value>,
    /// 高层车道复用的远端读句柄（当前 `current` 文件）。分块泵在 I/O 期间
    /// 把它临时移出锁外，写回时据此保证同一文件只 open 一次。
    pub(crate) read_handle: Option<TreeReadHandle>,
    /// `read_handle` 对应的 remote_path；切换文件时据此重新 open。
    pub(crate) handle_path: Option<String>,
}

pub(crate) struct FinishingUpload {
    pub(crate) session_id: String,
    pub(crate) remote_path: String,
    pub(crate) size: u64,
    pub(crate) transferred: Arc<AtomicU64>,
    pub(crate) cancelled: Arc<AtomicBool>,
    /// Cancel reason slug supplied by the workbench ("user", "ack-timeout",
    /// ...); read by the background push task so the surfaced error tells a
    /// user abort apart from an error-triggered cleanup.
    pub(crate) cancel_reason: Arc<Mutex<Option<String>>>,
    /// 压缩通道（M33）：status/cancel 载荷标注（进度事件的 phase 分母以
    /// 事件自带字段为准）。
    pub(crate) compression: CompressionMode,
    /// sudo 上传车道（UploadSudo）：推送暂存件路径（登录用户 home 下）。
    /// 取消收尾按它删远端暂存件，而不是目标目录的 `.dbx-upload-<task>.part`。
    pub(crate) sudo_staging: Option<String>,
}

/// 压缩下载 prep 的在途状态（M33）：`start_download`/`start_tree_download`
/// 决策压缩后 spawn 的后台任务（远端 gzip/tar → 拉取 → 本地解压/解包 →
/// ready）。`cancelled` 是取消 RPC 与 prep 循环之间的中断旗标；远端/本地
/// 临时件路径在取消与终态清理时消费。任务注册表条目消失（被取消移除）时
/// prep 循环自行收尾退出。
pub(crate) struct DownloadPrep {
    pub(crate) session_id: String,
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) cancel_reason: Arc<Mutex<Option<String>>>,
    /// prep 当前阶段（"compressing" | "fetching" | "decompressing"），
    /// `transfer_status` 兜底轮询用。
    pub(crate) phase: Arc<Mutex<&'static str>>,
    /// 远端临时压缩件（best-effort 删除）。
    pub(crate) remote_temp: Option<String>,
    /// 本地压缩流暂存（删除）。
    pub(crate) gz_staging: PathBuf,
}

/// 上传 spool 预压结果（M33）：Done(压缩件路径, 压缩后体积)；其余三态都
/// 回落普通推送（Cancelled 以取消错误终止任务）。
pub(crate) enum SpoolCompression {
    Done(PathBuf, u64),
    Cancelled(String),
    NotWorthwhile,
    Failed(String),
}

/// 下载 prep 结果（M33）：Ready 发 ready(gzip) 事件进入分块；Fallback 清
/// 压缩产物后回普通管线并发 ready(none)；Cancelled 走取消清理（事件由
/// cancel RPC 负责）。
pub(crate) enum PrepOutcome {
    Ready,
    Fallback(String),
    Cancelled,
}

/// Upload progress phase: `staging` = bytes buffered into the local spool
/// file, `compressing` = the spool is being gzip-encoded before the push
/// (compressed channel only), `uploading` = bytes actually pushed to the
/// SFTP server. The two counters restart independently, and the workbench
/// needs the marker to keep its progress bar and speed estimate honest
/// (issue #60; `compressing` added by M33).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum UploadPhase {
    Staging,
    Compressing,
    Uploading,
}

impl UploadPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            UploadPhase::Staging => "staging",
            UploadPhase::Compressing => "compressing",
            UploadPhase::Uploading => "uploading",
        }
    }
}

/// Background `sudo -nv` refresh loop keeping a connection's sudo timestamp
/// alive, ported from tiny-rdm's sudoExecService keepalive.
pub(crate) struct SudoKeepalive {
    /// Session whose transport the timestamp validation rides. The entry is
    /// replaced when a different (fresh) session registers and swept when the
    /// bound session is gone, so a re-registered connection — e.g. the host
    /// replaying `connection/connect` after a web refresh (#202) — can never
    /// inherit a keepalive pinned to a dead transport.
    pub(crate) session_id: String,
    pub(crate) task: tokio::task::JoinHandle<()>,
}

/// What `register_sudo_keepalive` should do with the connection's existing
/// entry (see `SudoKeepalive::keepalive_registration_plan`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum KeepaliveRegistration {
    ReuseExisting,
    ReplaceStale,
    InsertFresh,
}

/// Last collected metrics snapshot for one session, timestamped with the
/// collection moment. Mirrors tiny-rdm's `GetLastSnapshot`: the workbench can
/// render instantly from the previous sample before a fresh poll lands.
pub(crate) struct CachedMetrics {
    pub(crate) value: Value,
    pub(crate) collected_at: u64,
}

pub struct SshRuntime {
    pub(crate) connections: RwLock<HashMap<String, StoredConnection>>,
    /// Hydrated `io.dbx.sshx.local` targets (`src/local_target.rs`), keyed by
    /// connection id — the scheduler task provider's "is this connection a
    /// local shell target" source of truth. Same lifecycle as `connections`:
    /// stored on `connection/connect`, dropped on `connection/disconnect`,
    /// in-memory only (the host rehydrates by opening the connection before
    /// dispatching a task).
    pub(crate) local_targets: RwLock<HashMap<String, crate::sessions::local::target::LocalTarget>>,
    pub(crate) sessions: Arc<AsyncRwLock<HashMap<String, Arc<SessionEntry>>>>,
    pub(crate) uploads: Mutex<HashMap<String, UploadState>>,
    pub(crate) finishing_uploads: Mutex<HashMap<String, FinishingUpload>>,
    /// 压缩下载 prep 在途状态（M33）；任务进入 ready 或被取消时移除。
    pub(crate) download_preps: Mutex<HashMap<String, DownloadPrep>>,
    pub(crate) downloads: Mutex<HashMap<String, DownloadState>>,
    pub(crate) transfer_history: Mutex<VecDeque<Value>>,
    pub(crate) sudo_keepalive: Arc<Mutex<HashMap<String, SudoKeepalive>>>,
    /// Last metrics snapshot per session, in-memory only (see `CachedMetrics`).
    pub(crate) metrics_cache: Mutex<HashMap<String, CachedMetrics>>,
    /// In-flight remote command executions, cancellable by exec id.
    pub(crate) exec_tasks: Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
    /// Per-connection AI terminal mode (`agentTerminalMode`), persisted in
    /// `<data_dir>/agent-modes.json` so the toolbar toggle survives sidecar
    /// restarts (an app restart must not silently drop the user's choice
    /// back to `off`).
    pub(crate) agent_modes: Mutex<HashMap<String, AgentTerminalMode>>,
    /// One-shot approval challenges for terminal-routed agent commands;
    /// each entry is removed as soon as it is resolved. The payload carries
    /// the context the resolver needs for the remembered-approval store and
    /// the audit ledger (the resolver only knows the challenge id).
    pub(crate) agent_challenges: Mutex<HashMap<String, PendingChallenge>>,
    /// Monotonic session creation counter (see `SessionEntry::created_seq`).
    pub(crate) session_seq: AtomicU64,
    /// Live user-facing port mappings (Xshell-style 端口映射, -L/-R):
    /// forward id -> mapping row. Runtime-scoped on purpose; rows die with
    /// their session so a closed SSH session cannot leave phantom ports.
    pub(crate) forwards: Arc<forward::ForwardRegistry>,
    /// One authenticated, PTY-free transport per connection with independent
    /// mappings. Serialized creation prevents duplicate dials on concurrent starts.
    pub(crate) forward_transports: Arc<AsyncMutex<HashMap<String, Arc<forward::ForwardTransport>>>>,
    /// Per-connection `(listen_host, bound_port) -> dial target` tables the
    /// forwarded-tcpip handler consults (see `SshClient::remote_forwards`).
    pub(crate) remote_tables: Arc<Mutex<HashMap<String, Arc<RemoteForwardTable>>>>,
    /// Trust-on-first-use for unknown host keys (MCP stdio mode).
    pub(crate) auto_trust: bool,
    /// 会话维度的「建议开启兼容模式」一次性提示标记（M14-B）：SFTP 探测
    /// 失败时提示一次，之后同一会话静默。
    pub(crate) compat_hinted: Mutex<HashSet<String>>,
    /// 远端压缩工具探测结果（M33）按会话缓存：`command -v` 探测在任务
    /// start 时同步执行（5s 上限），目录批量上传逐文件 start 会放大成
    /// N 次 exec。工具存在性在一条连接的生命周期内不变，会话级缓存
    /// 一次即可；连接重建产生新 session id，天然失效。
    pub(crate) compress_tool_probe: Mutex<HashMap<String, bool>>,
    pub prompts: PromptBroker,
    pub(crate) data_dir: PathBuf,
    pub(crate) known_hosts_path: PathBuf,
    pub(crate) transfer_dir: PathBuf,
}

/// A raised approval challenge: the decision channel plus the command
/// context its resolution needs (remembered-approval persistence and the
/// audit ledger).
pub(crate) struct PendingChallenge {
    pub(crate) sender: oneshot::Sender<AgentDecision>,
    pub(crate) connection_id: String,
    pub(crate) tool: String,
    pub(crate) command: String,
    pub(crate) raised_at_ms: u64,
}
