//! Container stats RPCs (`docker/stats`, `docker/stats/limits`): one
//! `stats --no-stream` snapshot per poll plus a one-shot CPU total lookup.
//! Split out of `docker.rs` to keep each handler file under the 600-line cap;
//! dispatch stays in `handle_docker`.

use super::*;

impl Plugin {
    pub(super) fn handle_docker_stats(&self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            // 容器监控快照（CPU / 内存 / 网络 / 块 IO / PIDs）：一次
            // `stats --no-stream`，前端按可见页签每 3s 轮询。payload 恒良构：
            // running=false 表示容器已停/已删，needsSudo=true 走列表同款
            // Quick Sudo 提权重采，其它失败带 error 文本供面板提示。
            "docker/stats" => {
                let container_id = required_string(&params, "containerId")?;
                let engine = crate::features::docker::parse_engine(&params)?;
                if crate::features::docker::parse_target(&params)?
                    == crate::features::docker::DockerTarget::Local
                {
                    return self
                        .runtime
                        .block_on(crate::features::docker::collect_stats_local(
                            &engine,
                            container_id,
                        ));
                }
                let session_id = required_string(&params, "sessionId")?;
                let script = crate::features::docker::stats_script(&engine, container_id)?;
                let response = self.runtime.block_on(self.ssh.exec(
                    session_id,
                    None,
                    &script,
                    false,
                    Some(crate::features::docker::STATS_TIMEOUT.as_secs()),
                ))?;
                let output = response
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let payload = crate::features::docker::stats_payload(output);
                if payload["needsSudo"] != true {
                    return Ok(payload);
                }
                let body = crate::features::docker::sudo_stats_body(&engine, container_id)?;
                if let Ok(sudo_body) = self
                    .runtime
                    .block_on(self.ssh.ensure_sudo_allowed(session_id, &body))
                {
                    if let Ok(sudo_response) = self.runtime.block_on(self.ssh.exec(
                        session_id,
                        None,
                        &sudo_body,
                        true,
                        Some(crate::features::docker::STATS_TIMEOUT.as_secs()),
                    )) {
                        let sudo_output = sudo_response
                            .get("output")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let sudo_payload = crate::features::docker::stats_payload(sudo_output);
                        if sudo_payload["available"] == true {
                            return Ok(sudo_payload);
                        }
                    }
                }
                Ok(payload)
            }
            // 容器监控的 CPU 总量（一次性）：daemon 主机核数 + 容器 CPU 上限。
            // 3s 的 docker/stats 轮询不重复跑 info/inspect；前端按容器缓存。
            "docker/stats/limits" => {
                let container_id = required_string(&params, "containerId")?;
                crate::features::docker::validate_container_id(container_id)?;
                let engine = crate::features::docker::parse_engine(&params)?;
                if crate::features::docker::parse_target(&params)?
                    == crate::features::docker::DockerTarget::Local
                {
                    return self.runtime.block_on(
                        crate::features::docker::collect_stats_limits_local(&engine, container_id),
                    );
                }
                let session_id = required_string(&params, "sessionId")?;
                let script = crate::features::docker::stats_limits_script(&engine, container_id)?;
                let response = self.runtime.block_on(self.ssh.exec(
                    session_id,
                    None,
                    &script,
                    false,
                    Some(crate::features::docker::STATS_TIMEOUT.as_secs()),
                ))?;
                let output = response
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Ok(crate::features::docker::stats_limits_payload(output))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
