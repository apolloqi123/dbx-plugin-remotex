//! Serial session layer.

use super::*;

use crate::core::time::unix_now_secs;

impl SerialSessionRuntime {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Lists candidate ports, sorted by device path. `ports` stays a flat
    /// string array of device paths only (descriptions glued into the name
    /// are split off); `portDetails` carries the path/description pairs for
    /// pickers that want to show them. Empty on hosts without serial
    /// support — the UI renders its own "no ports" hint from this.
    pub fn list_ports(&self) -> Value {
        let mut labels: Vec<NormalizedPortLabel> = serialport::available_ports()
            .unwrap_or_default()
            .into_iter()
            .map(|entry| {
                let known = match &entry.port_type {
                    serialport::SerialPortType::UsbPort(info) => Some(usb_port_description(info)),
                    serialport::SerialPortType::PciPort => Some("PCI".to_string()),
                    serialport::SerialPortType::BluetoothPort => Some("Bluetooth".to_string()),
                    serialport::SerialPortType::Unknown => None,
                };
                normalize_port_label(&entry.port_name, known.as_deref())
            })
            .collect();
        labels.sort_by(|a, b| a.path.cmp(&b.path));
        let ports = labels.iter().map(|l| l.path.clone()).collect::<Vec<_>>();
        let port_details = labels
            .iter()
            .map(|l| json!({ "path": l.path, "description": l.description }))
            .collect::<Vec<_>>();
        json!({ "ports": ports, "portDetails": port_details })
    }

    /// Opens the port (blocking call moved off the async workers) and spawns
    /// the read thread + output pump.
    pub async fn start(
        &self,
        request: SerialStartRequest,
        emitter: PluginEmitter,
    ) -> Result<Value, String> {
        let port_name = request.port_name.trim().to_string();
        if port_name.is_empty() {
            return Err("serial/start: portName is required".to_string());
        }
        let baud_rate = checked_baud_rate(request.baud_rate)?;
        let backspace = BackspaceMode::parse(request.backspace_mode.as_ref());
        let line = resolve_line_params(&request)?;

        let session_id = uuid::Uuid::new_v4().to_string();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        // The port open is a blocking syscall; keep it off the async workers.
        let open_name = port_name.clone();
        let port = tokio::task::spawn_blocking(move || {
            serialport::new(&open_name, baud_rate)
                .data_bits(line.data_bits)
                .parity(line.parity)
                .stop_bits(line.stop_bits)
                .flow_control(line.flow_control)
                .timeout(READ_TIMEOUT)
                .open()
                .map_err(|error| format!("serial/start: {error}"))
        })
        .await
        .map_err(|error| format!("serial/start: join error: {error}"))??;

        let port = Arc::new(Mutex::new(port));
        let session = Arc::new(SerialSession {
            workbench_id: request.workbench_id.clone(),
            port_name: port_name.clone(),
            baud_rate,
            created_at_secs: unix_now_secs(),
            write: Arc::clone(&port),
            cmd_tx,
            backspace,
            upload: Mutex::new(None),
            replay: Arc::new(std::sync::Mutex::new(
                crate::sessions::ssh::ReplayBuffer::with_byte_limit(SERIAL_REPLAY_BYTE_LIMIT),
            )),
            write_queue: Arc::new(WriteQueue::new()),
        });
        self.sessions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id.clone(), Arc::clone(&session));

        spawn_reader(
            session_id.clone(),
            Arc::clone(&session),
            Arc::clone(&self.sessions),
            Arc::clone(&port),
            cmd_rx,
            emitter.clone(),
        );
        spawn_writer(session_id.clone(), Arc::clone(&session), emitter);
        Ok(start_response(&session_id, &port_name, baud_rate))
    }

    pub(crate) async fn session(&self, session_id: &str) -> Result<Arc<SerialSession>, String> {
        self.sessions
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session_id)
            .cloned()
            .ok_or_else(|| "Serial session was not found".to_string())
    }

    /// 键入路径（B1 二进制帧与 `serial/write` JSON 共用）：退格改写后有界
    /// 入队；准入丢弃（超限/队满）按契约发 `serial/input/dropped` 事件回报
    /// 前端（丢弃不是传输失败，返回 Ok）；队列已随会话关闭才报错。
    pub fn write_input(
        &self,
        session: &SerialSession,
        session_id: &str,
        data: &[u8],
        emitter: &PluginEmitter,
    ) -> Result<(), String> {
        let payload = session.backspace.rewrite(data);
        match session.write_queue.push(&payload, WriteSource::Keystroke) {
            Ok(()) => Ok(()),
            Err(reject @ (WriteReject::Oversize | WriteReject::QueueFull)) => {
                let _ = emitter.event(
                    "serial/input/dropped",
                    json!({
                        "sessionId": session_id,
                        "bytes": payload.len(),
                        "reason": if reject == WriteReject::Oversize { "oversize" } else { "queue_full" },
                    }),
                );
                Ok(())
            }
            Err(WriteReject::Closed) => Err("Serial session is closed".to_string()),
        }
    }

    pub async fn close(&self, session_id: &str) -> Result<(), String> {
        let session = self.session(session_id).await?;
        retire_session(&self.sessions, session_id, &session, true);
        Ok(())
    }

    pub async fn close_workbench(&self, workbench_id: &str) {
        let session_ids = {
            let sessions = self
                .sessions
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            session_ids_for_workbench(
                sessions
                    .iter()
                    .map(|(id, session)| (id.as_str(), session.workbench_id.as_str())),
                workbench_id,
            )
        };
        for session_id in session_ids {
            let _ = self.close(&session_id).await;
        }
    }

    pub async fn list(&self) -> Value {
        let sessions = self
            .sessions
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let rows: Vec<Value> = sessions
            .iter()
            .map(|(id, session)| {
                json!({
                    "sessionId": id,
                    "workbenchId": session.workbench_id,
                    "port": session.port_name,
                    "baudRate": session.baud_rate,
                    "binaryInput": true,
                    "createdAt": session.created_at_secs,
                })
            })
            .collect();
        json!({ "sessions": rows })
    }

    /// 序号制输出回放（设计稿 §3，与 telnet/local 的 after_sequence 先例
    /// 完全同构）：在 `serial/terminal/out/{id}` 上重发其后帧，返回摘要。
    /// `complete: false` 表示环形缓冲已绕回、回放不完整。会话已关闭时走
    /// `session()` 的 "Serial session was not found" 错误。
    pub async fn replay(
        &self,
        session_id: &str,
        after_sequence: u64,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let session = self.session(session_id).await?;
        let replay = session
            .replay
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let first_available_sequence = replay.first_sequence();
        let tail_sequence = replay.tail_sequence();
        let frames = replay.after(after_sequence);
        drop(replay);
        let channel = format!("serial/terminal/out/{session_id}");
        for frame in &frames {
            emitter
                .binary(&channel, &frame.encode())
                .map_err(|error| error.message)?;
        }
        Ok(json!({
            "frameCount": frames.len(),
            "firstAvailableSequence": first_available_sequence,
            "tailSequence": tail_sequence,
            "complete": after_sequence.saturating_add(1) >= first_available_sequence,
        }))
    }

    // —— 串口文件上传（X/Y/ZMODEM）——————————————————————————————

    /// 注册一次上传并让引擎发出初始输出（ZMODEM 的 ZRQINIT；X/Y 静默等
    /// 握手）。并发第二次 upload 直接拒绝（协议无法仲裁两个发送端）。
    pub async fn upload_start(
        &self,
        session_id: &str,
        protocol: UploadProtocol,
        file_name: String,
        total_size: u64,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        if file_name.is_empty() {
            return Err("serial/upload/start: fileName is required".to_string());
        }
        if file_name.len() > 256 {
            return Err("serial/upload/start: fileName is too long".to_string());
        }
        if total_size > serial_xmodem::MAX_UPLOAD_BYTES {
            return Err(format!(
                "serial/upload/start: totalSize {total_size} exceeds the {} byte limit",
                serial_xmodem::MAX_UPLOAD_BYTES
            ));
        }
        // YMODEM 头块约束：name\0size 必须放进 128 字节块（对齐 NyaTerm）。
        if protocol == UploadProtocol::Ymodem {
            let meta_len = file_name.len() + 1 + total_size.to_string().len();
            if meta_len > 128 {
                return Err(format!(
                    "serial/upload/start: YMODEM file name is too long ({meta_len} bytes of header metadata)"
                ));
            }
        }
        let session = self.session(session_id).await?;
        let mut guard = session
            .upload
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_some() {
            return Err("Serial upload is already in progress".to_string());
        }
        let (engine, initial) =
            UploadEngine::new(protocol, file_name.clone(), total_size, Instant::now());
        let mut job = SerialUploadJob {
            engine,
            gate: ProgressGate { last_emit: None },
        };
        let mut outputs = initial;
        // 初始进度（sent=0）不计入限流窗口，前端立即可显示总大小。
        outputs.push(Output::Progress(TransferProgress {
            protocol,
            file_name,
            file_index: 0,
            sent: 0,
            total: total_size,
            state: ProgressState::Running,
            reason: None,
        }));
        let result = apply_upload_outputs(&session, &mut job, outputs, session_id, emitter);
        *guard = Some(job);
        // 端口写出错时保留 Failed 任务意义有限：直接清掉，让用户可立即重试。
        if result.is_err() {
            *guard = None;
        }
        result?;
        Ok(json!({
            "sessionId": session_id,
            "protocol": protocol.as_str(),
            "totalSize": total_size,
        }))
    }

    /// 前端分块到货；可能解锁引擎挂起的读请求（输出立即写往串口）。
    pub async fn upload_data(
        &self,
        session_id: &str,
        data: Vec<u8>,
        final_chunk: bool,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let session = self.session(session_id).await?;
        let mut guard = session
            .upload
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(job) = guard.as_mut() else {
            return Err("No serial upload is in progress".to_string());
        };
        let outputs = job.engine.append_data(&data, final_chunk)?;
        let received = job.engine.source().received();
        let done = job.engine.is_done();
        apply_upload_outputs(&session, job, outputs, session_id, emitter)?;
        if done {
            *guard = None;
        }
        Ok(json!({ "received": received, "final": final_chunk }))
    }

    /// 用户取消：发取消序列、落 Failed 事件并清掉任务（幂等）。
    pub async fn upload_cancel(
        &self,
        session_id: &str,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let session = self.session(session_id).await?;
        let mut guard = session
            .upload
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(job) = guard.as_mut() else {
            return Ok(json!({ "success": true }));
        };
        let outputs = job.engine.cancel();
        // 可取消（设计稿）：清空尚未写出的队列块后再入队取消序列——取消
        // 字节绝不能排在被取消的数据块之后。在途块（阻塞在 write_all 的
        // 系统调用里）无法中断，属既定边界。
        session.write_queue.clear();
        let result = apply_upload_outputs(&session, job, outputs, session_id, emitter);
        *guard = None;
        result?;
        Ok(json!({ "success": true }))
    }

    /// 读线程喂入对端字节（终端照常上屏，引擎并行消费）。
    pub(crate) fn pump_upload_feed(
        session: &SerialSession,
        session_id: &str,
        bytes: &[u8],
        emitter: &PluginEmitter,
    ) {
        let mut guard = session
            .upload
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(job) = guard.as_mut() else {
            return;
        };
        let outputs = job.engine.feed(bytes);
        let done = job.engine.is_done();
        if let Err(error) = apply_upload_outputs(session, job, outputs, session_id, emitter) {
            // 端口写失败：终止上传（读线程自身也会因 IO 错误退出发 error 态）。
            let _ = emitter.event(
                "serial/session/state",
                json!({ "sessionId": session_id, "state": "error", "error": error }),
            );
            *guard = None;
            return;
        }
        if done {
            *guard = None;
        }
    }

    /// 读线程空闲滴答：驱动引擎的静默重发/失败判定。
    pub(crate) fn pump_upload_tick(
        session: &SerialSession,
        session_id: &str,
        emitter: &PluginEmitter,
    ) {
        let mut guard = session
            .upload
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(job) = guard.as_mut() else {
            return;
        };
        let outputs = job.engine.tick();
        let done = job.engine.is_done();
        if let Err(error) = apply_upload_outputs(session, job, outputs, session_id, emitter) {
            let _ = emitter.event(
                "serial/session/state",
                json!({ "sessionId": session_id, "state": "error", "error": error }),
            );
            *guard = None;
            return;
        }
        if done {
            *guard = None;
        }
    }
}

pub(crate) fn session_ids_for_workbench<'a>(
    sessions: impl IntoIterator<Item = (&'a str, &'a str)>,
    workbench_id: &str,
) -> Vec<String> {
    sessions
        .into_iter()
        .filter(|(_, owner)| *owner == workbench_id)
        .map(|(session_id, _)| session_id.to_string())
        .collect()
}

/// Removes a session only if the registry still points at this exact Arc. A
/// reader from an old session must never retire a later session that reused an
/// identifier. Closing the queue always wakes the writer and rejects new data.
pub(crate) fn retire_session(
    sessions: &Arc<RwLock<HashMap<String, Arc<SerialSession>>>>,
    session_id: &str,
    session: &Arc<SerialSession>,
    notify_reader: bool,
) {
    if notify_reader {
        let _ = session.cmd_tx.send(SerialCommand::Close);
    }
    session.write_queue.shutdown();
    let mut registry = sessions
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if registry
        .get(session_id)
        .is_some_and(|registered| Arc::ptr_eq(registered, session))
    {
        registry.remove(session_id);
    }
}
