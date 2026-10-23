//! Metrics RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_metrics(
        &self,
        method: &str,
        params: Value,
        _emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "ssh/metrics" => {
                let session_id = required_string(&params, "sessionId")?;
                let cached = params
                    .get("cached")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.runtime.block_on(self.ssh.metrics(session_id, cached))
            }

            "ssh/metrics/history" => {
                let session_id = required_string(&params, "sessionId")?;
                let limit = params
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(720)
                    .clamp(1, crate::features::metrics::history::MAX_SAMPLES as u64)
                    as usize;
                self.runtime
                    .block_on(self.ssh.metrics_history(session_id, limit))
            }

            "ssh/processes/list" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime.block_on(self.ssh.processes_list(session_id))
            }

            "ssh/processes/kill" => {
                let session_id = required_string(&params, "sessionId")?;
                let pid = params
                    .get("pid")
                    .and_then(Value::as_u64)
                    .ok_or("Missing pid")?;
                let signal = params
                    .get("signal")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .unwrap_or(15);
                self.runtime
                    .block_on(self.ssh.kill_process(session_id, pid, signal))
            }

            // 本地终端的系统信息带（dock 面板左上角）：同一采集脚本走本机
            // shell（POSIX sh；Windows 无 sh 时结构化报错，前端整条隐藏）。
            // 无会话参数——采样对象是 sidecar 宿主机本身；GPU/NPU 探测保持
            // SSH-only，本地文档缺省两段。
            "local/metrics" => self
                .runtime
                .block_on(crate::features::metrics::collect_local_metrics_async()),
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
