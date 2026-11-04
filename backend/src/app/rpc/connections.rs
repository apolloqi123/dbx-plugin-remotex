//! Connections RPC handlers.

use super::*;
use crate::platform::diagnostics::*;

impl Plugin {
    pub(super) fn handle_connections(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "connection/test" => {
                // 本地 Shell 连接目标（io.dbx.sshx.local）：无 TCP 语义，可达性
                // 无从探测；Shell 路径合法性留到任务真正 spawn 时校验（此时
                // 才做默认 Shell 解析），test 阶段如实放行并回显登记结果。
                if let Some(target) =
                    crate::sessions::local::target::LocalTarget::from_lifecycle_params(&params)?
                {
                    let shell = if target.shell.is_empty() {
                        "the DBX default shell (resolved when the task runs)".to_string()
                    } else {
                        format!("shell {}", target.shell)
                    };
                    return Ok(json!({
                        "success": true,
                        "message": format!("Local shell target registered ({})", shell),
                    }));
                }
                let endpoint = RuntimeEndpoint::from_lifecycle_params(&params)?;
                // 串口无 TCP 语义（M32-B）：本地设备的可达性只能真开串口时校验，
                // 测试阶段直接放行提示；rdp 与 telnet/vnc 同走 TCP probe。
                if endpoint.protocol == "serial" {
                    return Ok(json!({
                        "success": true,
                        "message": "serial device: reachability is validated when the connection opens",
                    }));
                }
                if endpoint.protocol == "telnet"
                    || endpoint.protocol == "vnc"
                    || endpoint.protocol == "rdp"
                {
                    let reachable = self
                        .runtime
                        .block_on(tcp_probe(&endpoint.runtime_host, endpoint.runtime_port));
                    return Ok(match reachable {
                        Ok(()) => json!({
                            "success": true,
                            "message": format!(
                                "{} TCP endpoint {}:{} is reachable",
                                endpoint.protocol, endpoint.host, endpoint.port
                            ),
                        }),
                        Err(error) => json!({
                            "success": false,
                            "message": format!(
                                "{} TCP endpoint {}:{} is unreachable: {error}",
                                endpoint.protocol, endpoint.host, endpoint.port
                            ),
                        }),
                    });
                }
                let connection = StoredConnection::from_lifecycle_params(&params)?;
                let operation_id = operation_id(&params);
                self.runtime.block_on(self.ssh.test_connection(
                    &connection,
                    &operation_id,
                    emitter.clone(),
                ))?;
                Ok(
                    json!({ "success": true, "message": "SSH handshake, host-key verification, and authentication succeeded" }),
                )
            }

            "connection/connect" => {
                // 本地 Shell 连接目标（io.dbx.sshx.local）：无凭据可存，登记
                // 进 SshRuntime 的 local_targets 注册表（调度任务执行时按
                // connection id 判定走本地分支的依据）。
                if let Some(target) =
                    crate::sessions::local::target::LocalTarget::from_lifecycle_params(&params)?
                {
                    self.ssh.store_local_target(target)?;
                    return Ok(json!({ "success": true }));
                }
                let endpoint = RuntimeEndpoint::from_lifecycle_params(&params)?;
                // 非 SSH 会话协议（telnet/vnc/serial/rdp）不走 SSH 连接存储：
                // 会话生命周期由各自的 * / start 方法族与前端路由驱动。表单
                // 密码（binding:"password" → connection.password）暂存进凭据
                // 注册表，*\/start 合并（表单存了密码 = 自动登录/免密认证的
                // 用户意图；没存则回落各自的连接弹窗）。
                if endpoint.protocol != "ssh" {
                    let connection = params.get("connection").cloned().unwrap_or(json!({}));
                    self.non_ssh_creds.store(
                        &endpoint.connection_id,
                        connection
                            .get("username")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        connection
                            .get("password")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        &endpoint.protocol,
                    );
                    return Ok(json!({ "success": true }));
                }
                let connection = StoredConnection::from_lifecycle_params(&params)?;
                // A host re-push can arrive right after a deferred disconnect.
                // Keep the live PTY when the transport identity is unchanged;
                // changed host/port/credentials still take the fresh path.
                if let Some(previous) = self.cancel_pending_ssh_disconnect(&connection.id) {
                    let same = super::super::lifecycle_grace::same_ssh_transport(&previous, &connection);
                    crate::platform::diagnostics::log_lifecycle_debug(format!(
                        "connection-repush id={} same_transport={same}",
                        connection.id
                    ));
                    if !same {
                        self.runtime
                            .block_on(self.ssh.disconnect_connection(&connection.id))?;
                    }
                }
                self.ssh.store_connection(connection)?;
                Ok(json!({ "success": true }))
            }

            // webview 路由前查询权威协议：宿主推给 webview 的 connection 上下文
            // 不含 external_config.protocol（只在 lifecycle connection/connect
            // 推送里），前端据此把 RDP/telnet/vnc 连接路由到各自的 * /start，
            // 而不是误拨 SSH。本地 Shell 目标（io.dbx.sshx.local）同理：返回
            // protocol="local" 与连接表单的 shell（空 = 默认 Shell 解析链），
            // 前端据此路由到 local/terminal/start 而非 SSH 拨号。SSH 连接（登记
            // 于 SSH 连接注册表）返回 protocol="ssh"：宿主 lifecycle 推送先于
            // webview 创建，首个查询即可命中，调用方不必把重试窗口跑满去等一个
            // 永远为空的答案。三者都没有 = 宿主尚未推送该连接，返回空串由调用方
            // 短暂重试（推送与 webview 创建存在固有竞态）。
            "connection/protocol" => {
                let connection_id = params
                    .get("connectionId")
                    .and_then(Value::as_str)
                    .ok_or("Missing connectionId")?;
                let payload = connection_protocol_payload(
                    self.runtime.block_on(self.ssh.local_target(connection_id)),
                    self.non_ssh_creds.protocol_of(connection_id),
                    self.ssh.has_connection(connection_id),
                );
                log_rdp_debug(format!(
                    "connection/protocol query id={connection_id} -> {payload}"
                ));
                Ok(payload)
            }

            "connection/disconnect" => {
                let connection_id = params
                    .get("connection")
                    .and_then(|value| value.get("id"))
                    .and_then(Value::as_str)
                    .ok_or("Missing connection id")?;
                self.non_ssh_creds.drop_connection(connection_id);
                // 本地 Shell 目标的登记随断开清除；SSH/telnet/vnc 连接 id 不在
                // local_targets 里，此调用为空操作。
                self.ssh.drop_local_target(connection_id);
                // 宿主在编辑连接/重建工作台时会断开并立刻重推同一条 SSH
                // 连接；有存活 PTY 时先延迟几秒，让紧随其后的 connect 取消
                // 断开并保留终端（生命周期见 app/lifecycle_grace.rs）。
                let deferred = self.schedule_ssh_disconnect_if_live(connection_id);
                let ssh_result = if deferred {
                    Ok(())
                } else {
                    self.runtime
                        .block_on(self.ssh.disconnect_connection(connection_id))
                };
                self.runtime
                    .block_on(self.telnet.close_connection(connection_id));
                self.runtime
                    .block_on(self.vnc.close_connection(connection_id));
                self.runtime
                    .block_on(self.rdp.close_connection(connection_id));
                ssh_result?;
                Ok(json!({ "success": true }))
            }

            "ssh/host-key/resolve" | "connection/challenge/resolve" => {
                let challenge_id = required_string(&params, "challengeId")?;
                let operation_id = required_string(&params, "operationId")?;
                let accept = params
                    .get("accept")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let remember = params
                    .get("remember")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                // RDP 证书确认（kind=rdp-certificate）有自己的注册表；先按
                // id 路由，未命中再进 SSH host-key/agent 的共享 resolve。
                if self.rdp.has_certificate_challenge(challenge_id) {
                    self.rdp
                        .resolve_certificate(challenge_id, accept, remember)?;
                } else {
                    self.runtime.block_on(self.ssh.prompts.resolve(
                        challenge_id,
                        operation_id,
                        PromptDecision { accept, remember },
                    ))?;
                }
                Ok(json!({ "success": true }))
            }

            "connection/action" => {
                let action = connection_action_id(&params)?;
                match action {
                    // 「从文件导入私钥」：桌面端弹系统文件选择框，读取校验后
                    // 回填表单 private_key 字段；取消安静返回；web/docker
                    // sidecar 不在本机，指引粘贴内容。
                    "import-private-key" => {
                        if !crate::filesystem::local::downloads::can_save_local(|key| {
                            std::env::var_os(key)
                        }) {
                            return Err(
                                "File import is only available on desktop — paste the key content instead"
                                    .to_string(),
                            );
                        }
                        action_import_private_key(&mut || crate::filesystem::local::fs::pick_file())
                    }
                    // 「导入连接」：先弹原生来源选择（MobaXterm/Xshell/…/
                    // OpenSSH config，与工作台向导同 8 类），再按所选类型弹带
                    // 扩展名过滤的文件选择框，按扩展名/JSON 形态嗅探来源后复用
                    // 导入预览的解析器，把首条会话回填表单字段。凭据结构性缺位
                    // ——密码永不回填、私钥只回路径。表单模型是一次一条连接，
                    // 批量选择仍走工作台向导；WindTerm 加密导出需要主密码，
                    // 无 UI 的动作问不了，指回向导。可选 source/path 参数跳过
                    // 两段系统对话框，供冒烟/自测注入（宿主从不携带；载荷仍
                    // 只有脱敏字段，与预览同边界）。任一步取消整体 null 安静
                    // 返回。
                    "import-sessions" => {
                        if !crate::filesystem::local::downloads::can_save_local(|key| {
                            std::env::var_os(key)
                        }) {
                            return Err(
                                "File import is only available on desktop — use the import wizard in the workbench"
                                    .to_string(),
                            );
                        }
                        action_import_sessions(
                            &params,
                            &mut crate::filesystem::local::fs::pick_import_source,
                            &mut crate::filesystem::local::fs::pick_import_file,
                        )
                    }
                    other => Err(format!("Unknown connection action: {other}")),
                }
            }

            "keys/discover" => {
                let keys = crate::core::keys::discover()?;
                Ok(json!({ "keys": keys }))
            }

            // 连接表单 private_key_path 的 options_action 数据源：宿主拉取后
            // 渲染动态下拉；无该扩展能力的宿主保持文本框（见 sudo_profile 先例）。
            "keys/discover/options" => crate::core::keys::discover_options(),

            "ssh/knownHosts/list" => Ok(crate::core::keys::list_known_hosts(&plugin_data_dir())?),

            "ssh/knownHosts/remove" => {
                let host = required_string(&params, "host")?;
                let port = params
                    .get("port")
                    .and_then(Value::as_u64)
                    .and_then(|value| u16::try_from(value).ok())
                    .filter(|value| *value > 0)
                    .unwrap_or(22);
                let removed = crate::core::keys::remove_known_host(&plugin_data_dir(), host, port)?;
                Ok(json!({ "success": true, "removed": removed }))
            }

            "ssh/host-key/check" => {
                let connection_id = required_string(&params, "connectionId")?;
                self.runtime
                    .block_on(self.ssh.check_host_key(connection_id))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
