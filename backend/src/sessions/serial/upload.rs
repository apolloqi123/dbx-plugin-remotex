//! Serial session layer.

use super::*;

/// 引擎输出统一处理：写串口（不带退格改写）+ 进度事件（限流）。
/// 端口/队列写错误只报第一个，剩余写输出丢弃（协议随后会终止）。
pub(crate) fn apply_upload_outputs(
    session: &SerialSession,
    job: &mut SerialUploadJob,
    outputs: Vec<Output>,
    session_id: &str,
    emitter: &PluginEmitter,
) -> Result<(), String> {
    let mut write_error = None;
    for output in outputs {
        match output {
            Output::Write(bytes) => {
                // 引擎输出走 Bulk 源入队：块级原样（不切分），队列满/关闭时
                // 报错终止上传（与原直写失败同语义），绝不阻塞调用线程。
                if write_error.is_none() {
                    if let Err(reject) = session.write_queue.push(&bytes, WriteSource::Bulk) {
                        write_error = Some(reject.message());
                    }
                }
            }
            Output::Progress(progress) => {
                if job.gate.allows(&progress) {
                    emit_upload_progress(emitter, session_id, progress);
                }
            }
        }
    }
    match write_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// 进度事件负载（camelCase，不含文件内容）。
pub(crate) fn emit_upload_progress(
    emitter: &PluginEmitter,
    session_id: &str,
    progress: TransferProgress,
) {
    let mut payload = json!({
        "sessionId": session_id,
        "protocol": progress.protocol.as_str(),
        "fileName": progress.file_name,
        "fileIndex": progress.file_index,
        "sent": progress.sent,
        "total": progress.total,
        "state": progress.state.as_str(),
    });
    if let Some(reason) = progress.reason {
        payload["reason"] = Value::String(reason);
    }
    let _ = emitter.event("serial/upload/progress", payload);
}
