//! Lifecycle connection payload parsing.

use super::*;

impl StoredConnection {
    pub fn from_lifecycle_params(params: &Value) -> Result<Self, String> {
        let connection = params
            .get("connection")
            .and_then(Value::as_object)
            .ok_or("Missing connection payload")?;
        let id = string_field(connection, "id")?;
        // Display name is optional (older payloads omit it); trimmed so the
        // MCP connectionName lookup matches what the workbench shows.
        let name = connection
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let host = validate_host_field(string_field(connection, "host")?)?;
        let port = connection
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or("SSH port must be between 1 and 65535")?;
        let runtime = params.get("runtime").and_then(Value::as_object);
        let runtime_host = optional_string(runtime, "host");
        let runtime_port = runtime
            .and_then(|value| value.get("port"))
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(port);
        let username = string_field(connection, "username")?;
        let protocol = optional_string(
            connection.get("external_config").and_then(Value::as_object),
            "protocol",
        );
        let protocol = match protocol.as_str() {
            "telnet" | "vnc" | "serial" | "rdp" => protocol,
            _ => "ssh".to_string(),
        };
        let mut password = connection
            .get("password")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let authentication = AuthenticationMethod::from_connection(connection)?;
        let external_config = connection.get("external_config").and_then(Value::as_object);
        let connection_secrets = connection
            .get("connection_secrets")
            .and_then(Value::as_object);
        let private_key_path = optional_string(external_config, "private_key_path");
        // 私钥内容与口令同为凭据：原样读取（PEM/PPK 是多行文本，禁止 trim）。
        let private_key = connection_secrets
            .map(|secrets| credential_string(secrets, "private_key"))
            .unwrap_or_default();
        let private_key_passphrase = connection_secrets
            .map(|secrets| credential_string(secrets, "private_key_passphrase"))
            .unwrap_or_default();
        let agent_socket = optional_string(external_config, "agent_socket");
        let _advanced_options = external_config
            .and_then(|config| config.get("advanced_options"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // sudo_password 是凭据：原样读取（首尾空格合法），空白语义由
        // SudoAuth::new 的「空白即回退登录密码」兜底，不在解析层改写。
        let sudo_password = connection_secrets
            .map(|secrets| credential_string(secrets, "sudo_password"))
            .unwrap_or_default();
        let totp_secret = optional_string(connection_secrets, "totp_secret");
        let password_prompt_hint = optional_string(external_config, "password_prompt_hint");
        let totp_prompt_hint = optional_string(external_config, "totp_prompt_hint");
        let auth_flow_mode = optional_string(external_config, "auth_flow_mode");
        // 会话特性两件套（camelCase 为主；snake_case 别名兼容手改配置/历史
        // 草稿）。setEnv 严格校验，非法条目让连接直接失败（宁可连不上也
        // 不错配）；remoteCommand trim 后非空才生效。
        let set_env = parse_set_env(config_text(external_config, &["setEnv", "set_env"]))?;
        let remote_command = config_text(external_config, &["remoteCommand", "remote_command"])
            .unwrap_or_default()
            .trim()
            .to_string();
        // The form has an independent opt-in switch. Missing/invalid values
        // are off by default so old non-empty trigger text cannot silently
        // start sending replies after an upgrade.
        let triggers_enabled = external_config
            .and_then(|config| config.get("triggers_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Expect 式终端触发器（camelCase/snake_case 均为 manifest 原生 key，
        // triggers 本身无别名）。仅在明确启用时解析；关闭时保留文本但
        // 不因旧文本非法而阻止连接。启用时非法配置仍直接失败（D7）。
        let triggers = triggers_enabled
            .then(|| {
                crate::features::automation::triggers::parse_triggers(
                    external_config.and_then(|config| config.get("triggers")),
                    &|key| {
                        connection_secrets
                            .and_then(|secrets| secrets.get(key))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    },
                )
            })
            .transpose()?
            .flatten();
        // 外部密码管理器（tssh PasswordCommand/PassphraseCommand 对标）：
        // trim 后非空才生效；既有显式凭据优先（D9）。
        let password_command =
            config_text(external_config, &["password_command", "passwordCommand"])
                .unwrap_or_default()
                .trim()
                .to_string();
        let passphrase_command = config_text(
            external_config,
            &["passphrase_command", "passphraseCommand"],
        )
        .unwrap_or_default()
        .trim()
        .to_string();
        // Legacy 0.4.x flag: only consulted when `sudo_source` is absent, so
        // a re-saved connection (stale `quick_sudo` left behind) follows the
        // explicit source chosen on the form.
        let legacy_quick_sudo = external_config
            .and_then(|config| config.get("quick_sudo"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let sudo_source = SudoSource::parse(
            external_config
                .and_then(|config| config.get("sudo_source"))
                .and_then(Value::as_str),
            legacy_quick_sudo,
        );
        let sudo_profile_ref = optional_string(external_config, "sudo_profile");
        let sudo_use_pty = external_config
            .and_then(|config| config.get("sudo_use_pty"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let sudo_whitelist = optional_string(external_config, "sudo_whitelist")
            .split('\n')
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect();
        let jump_hosts = parse_jump_hosts(external_config)?;
        // connect_timeout_secs 显式性单独留存：缺省（未展开高级选项）时宿主
        // connection/test 的 RPC 截止走的是宿主自己的 10s 回退而不是这里的 30s。
        let connect_timeout = config_u64(external_config, connection, "connect_timeout_secs");
        let runtime_host = if runtime_host.is_empty() {
            host.clone()
        } else {
            runtime_host
        };
        // 表单的「密码来源」（password_source）是显式选择，解析层照它执行：
        //   * "direct"  —— 表单里必须填密码（宿主 required_when 已先拦一道，
        //                  这里兜底 MCP/手改配置/导入等非对话框路径）；
        //   * "command" —— 由本地命令取回；此时存量密码必须让位，否则
        //                  「显式密码优先」会让命令永不执行，用户选定的
        //                  来源被静默忽略；
        //   * 缺省      —— 老配置（0.4.x 只有 password/password_command）与
        //                  MCP 内联参数保持历史语义：两者二选一即可。
        // 报错一律点名表单字段，非对话框路径也能直接改对。
        let password_source = config_text(external_config, &["password_source"])
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(source) = password_source {
            match (
                source,
                matches!(
                    authentication,
                    AuthenticationMethod::Password | AuthenticationMethod::PrivateKeyPassword
                ),
            ) {
                ("direct", true) if password.is_empty() => {
                    return Err(
                        "Password source \"Enter in this form\" requires the \"Password\" field \
                         (or choose \"Local command\" as \"Password source\" and provide \
                         \"Password command\")"
                            .to_string(),
                    );
                }
                ("command", true) if password_command.is_empty() => {
                    return Err(
                        "Password source \"Local command\" requires the \"Password command\" field \
                         (or choose \"Enter in this form\" as \"Password source\" and fill \
                         \"Password\")"
                            .to_string(),
                    );
                }
                ("command", true) => password.clear(),
                ("direct", _) | ("command", _) => {}
                (other, _) => return Err(format!("Unsupported password source '{other}'")),
            }
        } else if matches!(
            authentication,
            AuthenticationMethod::Password | AuthenticationMethod::PrivateKeyPassword
        ) && password.is_empty()
            && password_command.is_empty()
        {
            return Err(
                "Password authentication requires a password or a password_command: fill the \
                 \"Password\" field, or set \"Password source\" to \"Local command\" and provide \
                 \"Password command\""
                    .to_string(),
            );
        }
        if matches!(
            authentication,
            AuthenticationMethod::PrivateKey | AuthenticationMethod::PrivateKeyPassword
        ) && private_key_path.is_empty()
            && private_key.is_empty()
        {
            return Err(
                "Private-key authentication requires a private key path or key content: fill \
                 \"Private key path\", or paste the key into \"Private key content\""
                    .to_string(),
            );
        }
        Ok(Self {
            id,
            name,
            host,
            port,
            runtime_host,
            runtime_port,
            username,
            protocol,
            password,
            authentication,
            private_key_path,
            private_key,
            private_key_passphrase,
            agent_socket,
            connect_timeout_secs: connect_timeout.unwrap_or(30).max(1),
            connect_timeout_explicit: connect_timeout.is_some(),
            keepalive_interval_secs: config_u64(
                external_config,
                connection,
                "keepalive_interval_secs",
            )
            .unwrap_or(30),
            terminal_keepalive_secs: clamp_terminal_keepalive(
                config_u64(external_config, connection, "terminal_keepalive_secs").unwrap_or(0),
            ),
            // 只读门禁收敛：连接表单 read_only（插件特定配置项）∥ 宿主标准
            // read_only（ConnectionConfig 通用设置）。
            read_only: external_config
                .and_then(|config| config.get("read_only"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || connection
                    .get("read_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            triggers_enabled,
            sudo_password,
            totp_secret,
            sudo_source,
            sudo_profile_ref,
            sudo_use_pty,
            sudo_whitelist,
            password_prompt_hint,
            totp_prompt_hint,
            auth_flow_mode,
            set_env,
            remote_command,
            triggers,
            password_command,
            passphrase_command,
            jump_hosts,
        })
    }
}
