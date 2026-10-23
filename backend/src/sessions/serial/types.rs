//! Serial session layer.

use super::*;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct SerialStartRequest {
    pub port_name: String,
    pub baud_rate: u32,
    pub data_bits: Option<String>,
    pub parity: Option<String>,
    pub stop_bits: Option<String>,
    /// New in the line-parameter validation pass; absent on legacy clients,
    /// which keeps the driver default (`none`).
    pub flow_control: Option<String>,
    pub backspace_mode: Option<String>,
    pub workbench_id: String,
}

impl Default for SerialStartRequest {
    fn default() -> Self {
        Self {
            port_name: String::new(),
            baud_rate: 115_200,
            data_bits: None,
            parity: None,
            stop_bits: None,
            flow_control: None,
            backspace_mode: None,
            workbench_id: String::new(),
        }
    }
}

pub(crate) struct SerialSession {
    /// Owning workbench used for reattachment and backend-side cleanup when a
    /// webview cannot deliver its best-effort close notification.
    pub(crate) workbench_id: String,
    pub(crate) port_name: String,
    pub(crate) baud_rate: u32,
    pub(crate) created_at_secs: u64,
    pub(crate) write: Arc<Mutex<Box<dyn serialport::SerialPort>>>,
    pub(crate) cmd_tx: mpsc::UnboundedSender<SerialCommand>,
    pub(crate) backspace: BackspaceMode,
    /// 进行中的文件上传（每会话至多一个；上传期间的键入由前端拦下）。
    pub(crate) upload: Mutex<Option<SerialUploadJob>>,
    /// 输出回放环形缓冲（设计稿 §3）：读线程在会话生命周期内逐帧保存，
    /// `serial/replay` 据此重发。std 互斥锁（读线程与 JSON 侧都是同步上下文，
    /// 临界区只做入队/拷贝，无 IO）。
    pub(crate) replay: Arc<std::sync::Mutex<crate::sessions::ssh::ReplayBuffer>>,
    /// 有界写队列（设计稿「写序列化与回压」）：专用写线程独占端口写方向，
    /// 异步侧只入队不碰锁。
    pub(crate) write_queue: Arc<WriteQueue>,
}

impl SerialSession {
    /// 上传互斥开关（B1）：上传 job 存在即为活动期，二进制写帧被 sidecar
    /// 直接拒绝，作为单一前端闸门的后盾；引擎取消/完成清掉 job 后自动释放。
    pub(crate) fn upload_active(&self) -> bool {
        self.upload
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}

/// 一次串口文件上传的运行时状态：引擎 + 进度限流。
pub(crate) struct SerialUploadJob {
    pub(crate) engine: UploadEngine,
    pub(crate) gate: ProgressGate,
}

/// 进度事件限流：Running 态按字节增量/时间窗口折叠，状态变化强制上报。
pub(crate) struct ProgressGate {
    pub(crate) last_emit: Option<(Instant, u64)>,
}

impl ProgressGate {
    pub(crate) fn allows(&mut self, progress: &TransferProgress) -> bool {
        if progress.state != ProgressState::Running {
            self.last_emit = Some((Instant::now(), progress.sent));
            return true;
        }
        let now = Instant::now();
        match self.last_emit {
            Some((at, sent))
                if progress.sent.saturating_sub(sent) < serial_xmodem::PROGRESS_DELTA_BYTES
                    && now.duration_since(at) < serial_xmodem::PROGRESS_INTERVAL =>
            {
                false
            }
            _ => {
                self.last_emit = Some((now, progress.sent));
                true
            }
        }
    }
}

pub(crate) enum SerialCommand {
    Close,
}

pub struct SerialSessionRuntime {
    pub(crate) sessions: Arc<RwLock<HashMap<String, Arc<SerialSession>>>>,
}

impl Default for SerialSessionRuntime {
    fn default() -> Self {
        Self::new()
    }
}
