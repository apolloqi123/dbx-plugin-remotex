//! Serial session layer.

use super::*;

/// Dedicated blocking read thread: forwards port bytes to the pump channel
/// and honours `Close` by simply exiting (dropping its port clone closes the
/// handle on the writer side too — the OS closes the last reference).
///
/// While a file upload is active the same read chunk drives the upload
/// engine (peer responses feed the protocol AND still render on the
/// terminal, NyaTerm semantics); idle cycles tick the engine's retry
/// deadlines.
pub(crate) fn spawn_reader(
    session_id: String,
    session: Arc<SerialSession>,
    sessions: Arc<RwLock<HashMap<String, Arc<SerialSession>>>>,
    port: Arc<Mutex<Box<dyn serialport::SerialPort>>>,
    mut cmd_rx: mpsc::UnboundedReceiver<SerialCommand>,
    emitter: PluginEmitter,
) {
    std::thread::spawn(move || {
        let mut buffer = [0u8; READ_BUFFER];
        loop {
            if matches!(cmd_rx.try_recv(), Ok(SerialCommand::Close)) {
                let _ = emitter.event(
                    "serial/session/state",
                    json!({ "sessionId": session_id, "state": "closed" }),
                );
                return;
            }
            let read = {
                let mut port = port.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                port.read(&mut buffer)
            };
            match read {
                Ok(0) => {
                    std::thread::sleep(READ_TIMEOUT);
                    SerialSessionRuntime::pump_upload_tick(&session, &session_id, &emitter);
                }
                Ok(n) => {
                    // 序号由回放缓冲统一分配（replay 与在线帧共用同一序列）。
                    let frame = session
                        .replay
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(TerminalStream::Stdout, buffer[..n].to_vec());
                    let _ = emitter.binary(
                        &format!("serial/terminal/out/{session_id}"),
                        &frame.encode(),
                    );
                    SerialSessionRuntime::pump_upload_feed(
                        &session,
                        &session_id,
                        &buffer[..n],
                        &emitter,
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                    SerialSessionRuntime::pump_upload_tick(&session, &session_id, &emitter);
                }
                Err(error) => {
                    retire_session(&sessions, &session_id, &session, false);
                    let _ = emitter.event(
                        "serial/session/state",
                        json!({ "sessionId": session_id, "state": "error", "error": error.to_string() }),
                    );
                    return;
                }
            }
        }
    });
}

/// 专用写线程（设计稿「写序列化与回压」）：每会话一条，独占端口写方向；
/// 键入与上传引擎输出只入队不碰锁，队列按 256 KiB 字节预算有界。单块持锁
/// 时间被分帧上限约束（4 KiB @9600 波特约 4 秒），键入与引擎 ACK 不再被
/// 64 KiB 大块持锁 66 秒饿死。
///
/// 写失败镜像为 `serial/write/error` 事件（二进制/JSON 调用方在入队时早已
/// 拿到应答，错误无法回传）：清空队列（后续块大概率同样失败）后继续服务
/// 后续写入；端口消失时读线程会以 error 状态收场。在途块（阻塞在 write_all
/// 系统调用里）无法中断——取消/关闭保证的是其后队列内容不再写出。
pub(crate) fn spawn_writer(
    session_id: String,
    session: Arc<SerialSession>,
    emitter: PluginEmitter,
) {
    std::thread::spawn(move || {
        while let Some(chunk) = session.write_queue.pop() {
            let result = {
                let mut port = session
                    .write
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                port.write_all(&chunk).and_then(|_| port.flush())
            };
            if let Err(error) = result {
                let _ = emitter.event(
                    "serial/write/error",
                    json!({
                        "sessionId": session_id,
                        "error": format!("serial write failed: {error}"),
                    }),
                );
                session.write_queue.clear();
            }
        }
    });
}
