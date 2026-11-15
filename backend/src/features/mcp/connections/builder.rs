//! Inline-dial connection builder.

use super::*;

pub(in crate::features::mcp) fn stored_connection_from_arguments(
    arguments: &Value,
) -> Result<StoredConnection, String> {
    let jump_hosts = parse_jump_hosts(arguments)?;
    let host = required_str(arguments, "host")?;
    let username = required_str(arguments, "username")?;
    let port = arg_port(arguments)?;
    let password = arguments
        .get("password")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let private_key_path = arguments
        .get("privateKeyPath")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let private_key_content = arguments
        .get("privateKeyContent")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let has_key = !private_key_path.is_empty() || !private_key_content.is_empty();
    let authentication = match arguments
        .get("authentication")
        .and_then(Value::as_str)
        .unwrap_or("password")
    {
        "private-key" if has_key => AuthenticationMethod::PrivateKey,
        "private-key-password" if has_key => AuthenticationMethod::PrivateKeyPassword,
        "agent" => AuthenticationMethod::Agent,
        _ => {
            if has_key {
                AuthenticationMethod::PrivateKey
            } else {
                AuthenticationMethod::Password
            }
        }
    };
    if matches!(authentication, AuthenticationMethod::Password) && password.is_empty() {
        let password_command = arguments
            .get("passwordCommand")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if password_command.is_empty() {
            return Err("Password authentication requires a password".to_string());
        }
    }
    if matches!(
        authentication,
        AuthenticationMethod::PrivateKey | AuthenticationMethod::PrivateKeyPassword
    ) && !has_key
    {
        return Err(
            "Private-key authentication requires privateKeyPath or privateKeyContent".to_string(),
        );
    }
    // Expect 式触发器（JSON 字符串形态，同 §2.1 schema）与外部密码管理器
    // 命令：解析/校验与存储路径同一套代码（triggers::parse_triggers），非法
    // 即拨号报错。内联拨号没有 secret binding，sendSecretKey 引用的槽位
    // 无法填充，引用即报错。
    let triggers = crate::features::automation::triggers::parse_triggers(
        arguments.get("triggers"),
        &|_key| None,
    )?;
    let password_command = arguments
        .get("passwordCommand")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let passphrase_command = arguments
        .get("passphraseCommand")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let connect_timeout = arg_u64(arguments, "connectTimeoutSecs")?;
    Ok(StoredConnection {
        sudo_whitelist: Vec::new(),
        id: connection_pool_id(arguments),
        // Inline MCP dials carry no display name.
        name: None,
        host: host.to_string(),
        port,
        runtime_host: host.to_string(),
        runtime_port: port,
        username: username.to_string(),
        protocol: "ssh".to_string(),
        password,
        authentication,
        private_key_path,
        private_key: private_key_content,
        private_key_passphrase: arguments
            .get("privateKeyPassphrase")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        agent_socket: arguments
            .get("agentSocket")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        connect_timeout_secs: connect_timeout.unwrap_or(30).max(1),
        connect_timeout_explicit: connect_timeout.is_some(),
        keepalive_interval_secs: 30,
        // MCP drives exec channels, never the user's interactive PTY —
        // activity injection doesn't apply here.
        terminal_keepalive_secs: 0,
        read_only: false,
        sudo_password: arguments
            .get("sudoPassword")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        totp_secret: arguments
            .get("totpSecret")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        // MCP keeps its own quickSudoProfile resolution (explicit arguments
        // win), so the stored connection carries the plain on/off source.
        sudo_source: if arg_bool(arguments, "quickSudo")?.unwrap_or(true) {
            SudoSource::Custom
        } else {
            SudoSource::Off
        },
        sudo_profile_ref: String::new(),
        sudo_use_pty: false,
        password_prompt_hint: arguments
            .get("passwordPromptHint")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        totp_prompt_hint: arguments
            .get("totpPromptHint")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        auth_flow_mode: arguments
            .get("authFlowMode")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        // MCP tool calls carry their own env/command semantics; connection
        // SetEnv / RemoteCommand are workbench connection-form features.
        set_env: Vec::new(),
        remote_command: String::new(),
        triggers_enabled: triggers.is_some(),
        triggers,
        password_command,
        passphrase_command,
        jump_hosts,
    })
}

/// Parses the optional `jumpHosts` array (snake_case fields, same shape as
/// `external_config.jump_hosts`) into the ProxyJump chain.
pub(in crate::features::mcp) fn parse_jump_hosts(
    arguments: &Value,
) -> Result<Vec<JumpHost>, String> {
    let Some(list) = arguments.get("jumpHosts") else {
        return Ok(Vec::new());
    };
    let Some(list) = list.as_array() else {
        return Err("jumpHosts must be an array of jump host objects".to_string());
    };
    if list.len() > 3 {
        return Err("At most 3 jump hosts are supported".to_string());
    }
    let mut hosts = Vec::with_capacity(list.len());
    for (position, value) in list.iter().enumerate() {
        let host = JumpHost::from_json(value)
            .map_err(|error| format!("jumpHosts[{position}]: {error}"))?;
        host.validate(position)
            .map_err(|error| format!("jumpHosts[{position}]: {error}"))?;
        hosts.push(host);
    }
    Ok(hosts)
}
