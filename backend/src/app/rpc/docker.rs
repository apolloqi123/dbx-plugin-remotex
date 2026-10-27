//! Docker RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_docker(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            // Docker 管理面板（IMPL_PLAN Task P2-4）。列表/日志是只读采集
            // 脚本（探针区分「未装容器 CLI」与「daemon socket 拒绝」）；动作
            // 走白名单动词 + 容器 id 严格校验 + 只读连接直接拒绝 + 执行前
            // 审计，plain 失败且命中 daemon 权限签名时才回落 Quick Sudo
            // 管线（密码只走 stdin，绝不拼进命令行）。
            // `target: "local"` 切换到 sidecar 所在本机 daemon（Docker
            // Desktop/OrbStack/无 SSH 会话场景）：无需 sessionId，动作仍过
            // 审计与同一套白名单/id 门。可选 cli/socket/host 参数切换引擎
            // 与 daemon 端点（Podman / 自定义 socket / tcp host）。
            "docker/list" => {
                let engine = crate::features::docker::parse_engine(&params)?;
                if crate::features::docker::parse_target(&params)?
                    == crate::features::docker::DockerTarget::Local
                {
                    return self
                        .runtime
                        .block_on(crate::features::docker::collect_list_local(&engine));
                }
                let session_id = required_string(&params, "sessionId")?;
                let response = self.runtime.block_on(self.ssh.exec(
                    session_id,
                    None,
                    &crate::features::docker::list_script(&engine),
                    false,
                    Some(crate::features::docker::LIST_TIMEOUT.as_secs()),
                ))?;
                let mut output = response
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                // Windows OpenSSH 默认 shell 没有 sh，heredoc 输不出任何
                // 探针标记：多花一个往返用 cmd 版采集器重试。探测结果同时
                // 决定日志跟随命令能否使用 POSIX 看门狗包装。
                let mut windows_remote = false;
                if crate::features::docker::needs_windows_fallback(&output) {
                    if let Ok(win_response) = self.runtime.block_on(self.ssh.exec(
                        session_id,
                        None,
                        &crate::features::docker::win_list_script(&engine),
                        false,
                        Some(crate::features::docker::LIST_TIMEOUT.as_secs()),
                    )) {
                        let win_output = win_response
                            .get("output")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if !crate::features::docker::needs_windows_fallback(win_output) {
                            output = win_output.to_string();
                            windows_remote = true;
                        }
                    }
                }
                crate::features::docker::mark_remote_platform(session_id, windows_remote);
                let payload = crate::features::docker::list_payload(&output);
                if payload["needsSudo"] != true {
                    return Ok(payload);
                }
                // found-but-denied 且本连接配置了 Quick Sudo：用同一管线
                // （sudo -S sh -c '<body>'，密码走 stdin、脚本体走 argv）
                // 提权重采。只读连接/白名单拒绝/提权失败都退回原 payload，
                // 保留 needsSudo 提示而不是让只读调用报错。
                let body = crate::features::docker::list_body(&engine);
                let sudo_body = match self
                    .runtime
                    .block_on(self.ssh.ensure_sudo_allowed(session_id, &body))
                {
                    Ok(rebuilt) => rebuilt,
                    Err(_) => return Ok(payload),
                };
                if let Ok(sudo_response) = self.runtime.block_on(self.ssh.exec(
                    session_id,
                    None,
                    &sudo_body,
                    true,
                    Some(crate::features::docker::LIST_TIMEOUT.as_secs()),
                )) {
                    let sudo_output = sudo_response
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let sudo_payload = crate::features::docker::list_payload(sudo_output);
                    if sudo_payload["available"] == true {
                        return Ok(sudo_payload);
                    }
                }
                Ok(payload)
            }

            "docker/logs" => {
                let container_id = required_string(&params, "containerId")?;
                let tail = optional_u64(&params, "tail", crate::features::docker::TAIL_DEFAULT);
                let engine = crate::features::docker::parse_engine(&params)?;
                if crate::features::docker::parse_target(&params)?
                    == crate::features::docker::DockerTarget::Local
                {
                    return self
                        .runtime
                        .block_on(crate::features::docker::collect_logs_local(
                            &engine,
                            container_id,
                            tail,
                        ));
                }
                let session_id = required_string(&params, "sessionId")?;
                let script = crate::features::docker::logs_script(&engine, container_id, tail)?;
                let response = self.runtime.block_on(self.ssh.exec(
                    session_id,
                    None,
                    &script,
                    false,
                    Some(crate::features::docker::LOGS_TIMEOUT.as_secs()),
                ))?;
                let mut output = response
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                // Windows 兜底：heredoc 报废时把 inspect/logs 两步拆成裸
                // 命令直跑（cmd 可执行），本地拼回标记文本复用同一解析；
                // 端点经 cmd set 前缀带过（charset 白名单保证 cmd 惰性）。
                if crate::features::docker::needs_windows_fallback(&output) {
                    let tail_clamped = crate::features::docker::clamp_tail(tail).to_string();
                    let inspect = self.runtime.block_on(self.ssh.exec(
                        session_id,
                        None,
                        &format!(
                            "{}{} inspect {container_id}",
                            engine.cmd_env_prefix(),
                            engine.cli
                        ),
                        false,
                        Some(crate::features::docker::LOGS_TIMEOUT.as_secs()),
                    ));
                    let logs = self.runtime.block_on(self.ssh.exec(
                        session_id,
                        None,
                        &format!(
                            "{}{} logs --tail {tail_clamped} {container_id}",
                            engine.cmd_env_prefix(),
                            engine.cli
                        ),
                        false,
                        Some(crate::features::docker::LOGS_TIMEOUT.as_secs()),
                    ));
                    if let (Ok(inspect), Ok(logs)) = (inspect, logs) {
                        output = format!(
                            "DBXDOCKER_INSPECT\n{}\nDBXDOCKER_LOGS\n{}",
                            inspect
                                .get("output")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                            logs.get("output")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        );
                    }
                }
                // daemon socket 权限签名（logs 段 2>&1 合并了错误）→ Quick
                // Sudo 提权重采；任何失败都退回原输出，payload 恒良构。
                if crate::features::docker::is_daemon_permission_failure(1, &output) {
                    let body =
                        crate::features::docker::sudo_logs_body(&engine, container_id, tail)?;
                    if let Ok(sudo_body) = self
                        .runtime
                        .block_on(self.ssh.ensure_sudo_allowed(session_id, &body))
                    {
                        if let Ok(sudo_response) = self.runtime.block_on(self.ssh.exec(
                            session_id,
                            None,
                            &sudo_body,
                            true,
                            Some(crate::features::docker::LOGS_TIMEOUT.as_secs()),
                        )) {
                            let sudo_output = sudo_response
                                .get("output")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            if sudo_output.contains("DBXDOCKER_LOGS") {
                                return Ok(crate::features::docker::logs_payload(sudo_output));
                            }
                        }
                    }
                }
                Ok(crate::features::docker::logs_payload(&output))
            }

            // 真流式日志跟随（容器管理分屏）：start 立即返回 streamId，分片
            // 走 `docker/logs/chunk`，收口走 `docker/logs/end`（自然结束给
            // exitCode，daemon 权限签名时 needsSudo；取消统一
            // `error:"cancelled"`（SSH 与本机同一形状），TTL 到期
            // `error:"stream timed out"`）。注册表上限 4 条、单条 TTL 30
            // 分钟；stop 幂等。sudo 仅 SSH 连接可用且要求连接可写、
            // 已启用 Quick Sudo —— 在开出流之前拒绝，避免前端拿到 streamId
            // 后才在 end 事件里看到权限错误。
            "docker/logs/start" => {
                let container_id = required_string(&params, "containerId")?;
                crate::features::docker::validate_container_id(container_id)?;
                let tail = crate::features::docker::clamp_tail(optional_u64(
                    &params,
                    "tail",
                    crate::features::docker::TAIL_DEFAULT,
                ));
                let engine = crate::features::docker::parse_engine(&params)?;
                let sudo = params.get("sudo").and_then(Value::as_bool).unwrap_or(false);
                if crate::features::docker::parse_target(&params)?
                    == crate::features::docker::DockerTarget::Local
                {
                    // 本机没有 Quick Sudo 管线：显式拒绝而不是静默降级。
                    if sudo {
                        return Err(
                            "Quick Sudo is not available for local docker targets".to_string()
                        );
                    }
                    let stream_id = self.runtime.block_on(async {
                        self.docker_streams.start_local(
                            emitter.clone(),
                            engine,
                            container_id.to_string(),
                            tail,
                        )
                    })?;
                    return Ok(json!({ "streamId": stream_id }));
                }
                let session_id = required_string(&params, "sessionId")?;
                if sudo {
                    self.runtime
                        .block_on(self.ssh.ensure_writable(session_id))?;
                    let settings = self
                        .runtime
                        .block_on(self.ssh.settings_get(session_id, &json!({})))?;
                    if settings.get("quickSudo").and_then(Value::as_bool) != Some(true) {
                        return Err("Quick Sudo is disabled for this connection".to_string());
                    }
                }
                // POSIX remotes get a watchdog supervisor so a stopped stream's
                // `docker logs -f` child dies even when the server never forwards
                // our signal requests; Windows cmd keeps the flat command.
                let command = if crate::features::docker::remote_is_windows(session_id) {
                    crate::features::docker::logs_follow_command(&engine, container_id, tail)
                } else {
                    crate::features::docker::supervised_logs_follow_command(
                        &engine,
                        container_id,
                        tail,
                    )
                };
                let stream_id = self.runtime.block_on(async {
                    self.docker_streams.start_ssh(
                        self.ssh.clone(),
                        emitter.clone(),
                        session_id.to_string(),
                        command,
                        sudo,
                        tail,
                    )
                })?;
                Ok(json!({ "streamId": stream_id }))
            }

            "docker/logs/stop" => {
                // 幂等：未知/已结束的 streamId 返回 stopped:false，不报错。
                // 显式 stop 会短暂等待 forwarder 释放通道，避免「旧 logs -f
                // 还没退出就开新流」把服务器的 MaxSessions 名额堆满。
                let stream_id = required_string(&params, "streamId")?;
                Ok(json!({
                    "stopped": self.runtime.block_on(self.docker_streams.stop_and_wait(stream_id))
                }))
            }

            // 容器文件浏览/传输（`docker/files/*`）拆到 docker_files.rs，
            // 保持单文件 600 行上限；分派仍在这里。
            method if method.starts_with("docker/files/") => {
                self.handle_docker_files(method, params)
            }

            // 容器内嵌终端（容器管理分屏的终端页）：真正的 PTY 会话。
            // `target: "local"` 用本机 portable-pty，否则在既有 SSH 传输上
            // 新开 PTY channel 跑固定的 `docker exec -it ...`；并发上限 4、
            // TTL 2 小时，输出走二进制 `docker/terminal/out/<id>`，输入走
            // `docker/terminal/in/<id>`（handler 分支）。打开前写审计意图
            // （固定命令文本，绝不含按键/输出）；只读连接先拒绝再审计。
            "docker/terminal/open" => {
                let container_id = required_string(&params, "containerId")?;
                crate::features::docker::validate_container_id(container_id)?;
                let engine = crate::features::docker::parse_engine(&params)?;
                let cols = required_u32(&params, "cols")?;
                let rows = required_u32(&params, "rows")?;
                let terminal_id = params.get("terminalId").and_then(Value::as_str);
                let audit_text = crate::features::docker::shell_audit_text(&engine, container_id)?;
                if crate::features::docker::parse_target(&params)?
                    == crate::features::docker::DockerTarget::Local
                {
                    crate::features::docker::audit_terminal_intent(
                        &self.ssh.data_dir(),
                        &audit_text,
                    );
                    // The registry spawns its forwarder/runner onto the plugin
                    // runtime; this RPC runs on an SDK worker thread, so enter
                    // the runtime before starting the local PTY.
                    return self.runtime.block_on(async {
                        self.docker_terminals.start_local(
                            emitter.clone(),
                            container_id,
                            engine,
                            terminal_id,
                            cols,
                            rows,
                        )
                    });
                }
                let session_id = required_string(&params, "sessionId")?;
                // 只读连接直接拒绝：终端能改容器内状态；拒绝发生在审计之前。
                self.runtime
                    .block_on(self.ssh.ensure_writable(session_id))?;
                crate::features::docker::audit_terminal_intent(&self.ssh.data_dir(), &audit_text);
                self.runtime.block_on(self.docker_terminals.start_ssh(
                    self.ssh.clone(),
                    emitter.clone(),
                    session_id.to_string(),
                    container_id,
                    engine,
                    terminal_id,
                    cols,
                    rows,
                ))
            }

            "docker/terminal/resize" => {
                // 幂等：未知/已结束的 terminalId 返回 resized:false，不报错。
                let terminal_id = required_string(&params, "terminalId")?;
                let cols = required_u32(&params, "cols")?;
                let rows = required_u32(&params, "rows")?;
                Ok(json!({ "resized": self.docker_terminals.resize(terminal_id, cols, rows) }))
            }

            "docker/terminal/close" => {
                // 幂等：未知/已结束的 terminalId 返回 stopped:false，不报错。
                // 已存在的终端会短暂等待 runner 释放 SSH channel，避免快速
                // 切换容器时“关闭未完成就重开”触发服务器 MaxSessions 限制。
                let terminal_id = required_string(&params, "terminalId")?;
                Ok(json!({
                    "stopped": self
                        .runtime
                        .block_on(self.docker_terminals.close_and_wait(terminal_id))
                }))
            }

            "docker/action" => {
                let container_id = required_string(&params, "containerId")?;
                crate::features::docker::validate_container_id(container_id)?;
                let action =
                    crate::features::docker::parse_action(required_string(&params, "action")?)?;
                let engine = crate::features::docker::parse_engine(&params)?;
                if crate::features::docker::parse_target(&params)?
                    == crate::features::docker::DockerTarget::Local
                {
                    // 本机动作同样先审计再执行；本机没有只读连接语义，
                    // 破坏性确认由前端/MCP 确认门承担。
                    crate::features::docker::audit_action_intent(
                        &self.ssh.data_dir(),
                        &crate::features::docker::action_command(&engine, action, container_id),
                    );
                    let started = std::time::Instant::now();
                    return match self.runtime.block_on(
                        crate::features::docker::perform_action_local(
                            &engine,
                            container_id,
                            action,
                        ),
                    ) {
                        Ok(payload) => Ok(payload),
                        Err(error) => {
                            crate::features::docker::audit_action_failure(
                                &self.ssh.data_dir(),
                                &crate::features::docker::action_command(
                                    &engine,
                                    action,
                                    container_id,
                                ),
                                &error,
                                started.elapsed().as_millis() as u64,
                            );
                            Err(error)
                        }
                    };
                }
                let session_id = required_string(&params, "sessionId")?;
                // 只读连接直接拒绝：动作会改变远端容器状态。
                self.runtime
                    .block_on(self.ssh.ensure_writable(session_id))?;
                // 审计/展示文本用短形态（cli 动词 id）；实际执行命令额外
                // 带端点 export 前缀（POSIX），sudo 回落重放同一条文本。
                let command =
                    crate::features::docker::action_command(&engine, action, container_id);
                let exec_text =
                    crate::features::docker::exec_command(&engine, action, container_id);
                // 执行前写审计（意图行）：即使 sidecar 中途退出，账本上也留
                // 有一条记录；失败时补一行带错误详情的失败行。
                crate::features::docker::audit_action_intent(&self.ssh.data_dir(), &command);
                let started = std::time::Instant::now();
                let plain = self.runtime.block_on(self.ssh.exec(
                    session_id,
                    None,
                    &exec_text,
                    false,
                    Some(crate::features::docker::ACTION_TIMEOUT.as_secs()),
                ))?;
                let exit_code = plain.get("exitCode").and_then(Value::as_i64).unwrap_or(-1) as i32;
                let output = plain
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if exit_code == 0 {
                    return Ok(json!({ "success": true, "output": output }));
                }
                let failure = |error: String| {
                    crate::features::docker::audit_action_failure(
                        &self.ssh.data_dir(),
                        &command,
                        &error,
                        started.elapsed().as_millis() as u64,
                    );
                    error
                };
                if crate::features::docker::is_daemon_permission_failure(exit_code, &output) {
                    // daemon socket 权限失败是唯一允许回落 sudo 的失败形态：
                    // 其余失败重试可能把半执行的动作应用两次。回落走与
                    // ssh/exec 同一条 Quick Sudo 管线（编排凭据、use_pty、
                    // keepalive 全部复用），连接级 sudoers 白名单同语义生效，
                    // 白名单放行时以重建后的 token 命令提权重跑。
                    let sudo_exec = self
                        .runtime
                        .block_on(self.ssh.ensure_sudo_allowed(session_id, &exec_text))?;
                    return match self.runtime.block_on(self.ssh.exec(
                        session_id,
                        None,
                        &sudo_exec,
                        true,
                        Some(crate::features::docker::ACTION_TIMEOUT.as_secs()),
                    )) {
                        Ok(sudo_response) => {
                            let sudo_output = sudo_response
                                .get("output")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            Ok(json!({ "success": true, "output": sudo_output }))
                        }
                        Err(error) => {
                            Err(failure(crate::features::docker::sudo_fallback_error(error)))
                        }
                    };
                }
                Err(failure(format!(
                    "{} {} failed (exit {}): {}",
                    engine.cli,
                    action.as_str(),
                    exit_code,
                    output
                )))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
