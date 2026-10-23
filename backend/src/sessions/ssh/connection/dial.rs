//! SSH connection support.

use super::*;

impl SshRuntime {
    /// Dials the ProxyJump chain (if any) and returns the authenticated
    /// target handle plus the jump-host handles that must stay alive for the
    /// target tunnel to keep working. Ported from tiny-rdm's dialThroughJump.
    pub(crate) async fn connect_authenticated(
        &self,
        connection: &StoredConnection,
        operation_id: &str,
        emitter: Option<PluginEmitter>,
    ) -> Result<(Handle<SshClient>, Vec<Handle<SshClient>>), String> {
        let mut jump_handles = Vec::new();
        let mut dial = DialTarget::Tcp((connection.runtime_host.clone(), connection.runtime_port));

        for (position, jump) in connection.jump_hosts.iter().enumerate() {
            let jump_connection = jump.to_connection(
                &format!("jump-{}-{}", position + 1, connection.id),
                connection.connect_timeout_secs,
                connection.keepalive_interval_secs,
            );
            let handle = self
                .dial_and_authenticate(&jump_connection, dial, operation_id, emitter.clone())
                .await
                .map_err(|error| {
                    format!(
                        "Jump host #{} ({}:{}) failed: {error}",
                        position + 1,
                        jump.host,
                        jump.port
                    )
                })?;
            // The next hop dials through a direct-tcpip channel on this jump.
            let next = if position + 1 < connection.jump_hosts.len() {
                let target = &connection.jump_hosts[position + 1];
                (target.host.clone(), target.port)
            } else {
                (connection.host.clone(), connection.port)
            };
            dial = DialTarget::through_jump(&handle, &next.0, next.1).await?;
            jump_handles.push(handle);
        }

        let target = self
            .dial_and_authenticate(connection, dial, operation_id, emitter)
            .await?;
        Ok((target, jump_handles))
    }

    pub(super) async fn dial_and_authenticate(
        &self,
        connection: &StoredConnection,
        dial: DialTarget,
        operation_id: &str,
        emitter: Option<PluginEmitter>,
    ) -> Result<Handle<SshClient>, String> {
        // Match tiny-rdm: after 3 unanswered keepalive probes the connection
        // is declared dead so the workbench can reconnect.
        let config = Arc::new(ssh_client_config(connection));
        let verifier = Arc::new(HostKeyVerifier::new(self.known_hosts_path.clone()));
        let timeout = Duration::from_secs(connection.connect_timeout_secs);
        let dial_deadline = DialDeadline::start(timeout);
        // The forwarded-tcpip handler needs the mapping table at dial time;
        // remote forwards registered later on this connection insert into the
        // same Arc, and the table dies with the connection's last session.
        let remote_forwards = self.remote_table_for(&connection.id);
        // Auto 回退编排也要逐方式发日志事件：emitter 主体随 handler 移走，
        // 这里先留一份克隆给认证分派用（Option<PluginEmitter>，headless 为
        // None 时事件自然省略）。
        let auth_emitter = emitter.clone();
        let handler = SshClient {
            verifier,
            prompts: self.prompts.clone(),
            emitter,
            auto_trust: self.auto_trust,
            host: connection.host.clone(),
            port: connection.port,
            connection_id: connection.id.clone(),
            operation_id: operation_id.to_string(),
            dial_deadline: dial_deadline.clone(),
            connect_timeout: timeout,
            remote_forwards,
        };
        let timeout_message = || {
            format!(
                "SSH connection timed out after {} seconds",
                connection.connect_timeout_secs
            )
        };
        // The deadline is dynamic: a pending host-key challenge suspends the
        // dial timeout while the user studies the fingerprint dialog.
        let mut session = match dial {
            DialTarget::Tcp(address) => {
                let connect = client::connect(config, (address.0.as_str(), address.1), handler);
                tokio::pin!(connect);
                loop {
                    let remaining = dial_deadline.remaining().ok_or_else(timeout_message)?;
                    match tokio::time::timeout_at(
                        tokio::time::Instant::now() + remaining,
                        &mut connect,
                    )
                    .await
                    {
                        Ok(result) => break result.map_err(dial_error_message)?,
                        Err(_elapsed) => continue,
                    }
                }
            }
            DialTarget::JumpStream(stream) => {
                let connect = client::connect_stream(config, stream, handler);
                tokio::pin!(connect);
                loop {
                    let remaining = dial_deadline.remaining().ok_or_else(timeout_message)?;
                    match tokio::time::timeout_at(
                        tokio::time::Instant::now() + remaining,
                        &mut connect,
                    )
                    .await
                    {
                        Ok(result) => break result.map_err(dial_error_message)?,
                        Err(_elapsed) => continue,
                    }
                }
            }
        };

        let none = tokio::time::timeout(timeout, session.authenticate_none(&connection.username))
            .await
            .map_err(|_| "SSH authentication probe timed out".to_string())?
            .map_err(|error| format!("SSH auth probe failed: {error}"))?;
        eprintln!(
            "[ssh-trace] tcp+banner ok, auth none success={}",
            none.success()
        );
        if none.success() {
            return Ok(session);
        }
        if connection.authentication == AuthenticationMethod::None {
            return Err("SSH server rejected unauthenticated access".to_string());
        }

        // D9: password_command 在 orchestration 构建前一次性解析——结果回填
        // 后供密码链（try_password / keyboard-interactive）与 sudo 编排共用，
        // 避免多处执行命令。既有显式密码优先（契约优先级）。
        let connection = &resolve_password_command(connection).await;
        let orchestration = {
            // 登录期 KI 与 sudo 共用同一套生效凭据来源（连接配置，或
            // sudo_source 解析出的全局 Quick Sudo 配置）：global 模式在表单上
            // 隐藏 2FA 四件套、契约写明"凭据来源整体由全局配置接管"，所以登录
            // 提问也必须能读到该配置的 TOTP/提示词/流程模式。
            let store = sudo_profiles::load_store(&self.data_dir);
            let profile = effective_sudo_profile(connection, &store);
            login_sudo_auth(connection, profile.as_ref())
        };

        // #6（none 广告集前置 reconcile，对抗评审第二期）：none 探测拿到的
        // 服务器广告集作为后续 password/KI 方法选择的初始门。广告集拿不到
        // （连接中断等异常形状）时为 None，方法选择回退为逐步广告判定，
        // 绝不 fail-closed 卡死登录。
        let advertised_none = advertised_methods(&none);

        match connection.authentication {
            AuthenticationMethod::Password => {
                authenticate_password_or_interactive(
                    &mut session,
                    connection,
                    &orchestration,
                    advertised_none,
                    &none,
                    &self.prompts,
                )
                .await?;
            }
            AuthenticationMethod::PrivateKey => {
                let key_result = authenticate_private_key_result(&mut session, connection).await?;
                if !key_result.success() {
                    // 公钥被接受、服务器还要求 MFA（koko 的私钥 + 二次认证）：
                    // 继续答 keyboard-interactive 的验证码提问；公钥被直接
                    // 拒绝（非 partial success）时仍按密钥认证失败报错，不
                    // 静默改用密码。
                    if auth_partial_success(&key_result)
                        && method_offered(&key_result, MethodKind::KeyboardInteractive)
                    {
                        eprintln!(
                            "[ssh-trace] auth: publickey partial success, continuing with keyboard-interactive"
                        );
                        authenticate_keyboard_interactive(
                            &mut session,
                            connection,
                            &orchestration,
                            true,
                            &self.prompts,
                        )
                        .await?;
                    } else {
                        return Err("SSH private-key authentication was rejected".to_string());
                    }
                }
            }
            AuthenticationMethod::PrivateKeyPassword => {
                let key_result = authenticate_private_key_result(&mut session, connection).await?;
                if !key_result.success() {
                    authenticate_password_or_interactive(
                        &mut session,
                        connection,
                        &orchestration,
                        advertised_none,
                        &key_result,
                        &self.prompts,
                    )
                    .await?;
                }
            }
            AuthenticationMethod::Agent => {
                match authenticate_agent(&mut session, connection).await? {
                    AgentAuthOutcome::Accepted => {}
                    // agent 身份被接受、服务器还要 MFA：与私钥路径同样续答
                    // keyboard-interactive 验证码提问。
                    AgentAuthOutcome::NeedsKeyboardInteractive => {
                        eprintln!(
                            "[ssh-trace] auth: agent partial success, continuing with keyboard-interactive"
                        );
                        authenticate_keyboard_interactive(
                            &mut session,
                            connection,
                            &orchestration,
                            true,
                            &self.prompts,
                        )
                        .await?;
                    }
                    AgentAuthOutcome::Rejected(reason) => {
                        return Err(reason);
                    }
                }
            }
            AuthenticationMethod::Auto => {
                authenticate_auto(
                    &mut session,
                    connection,
                    &orchestration,
                    &none,
                    &self.prompts,
                    auth_emitter.as_ref(),
                    operation_id,
                )
                .await?;
            }
            AuthenticationMethod::None => unreachable!(),
        }

        Ok(session)
    }
}
