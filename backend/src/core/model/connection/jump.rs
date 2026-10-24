//! Jump-host chain parsing and validation.

use super::*;

/// One hop of a `external_config.jump_hosts` chain. Credentials live inline
/// because the DBX connection model only provides a single secrets store for
/// the target host.
#[derive(Debug, Clone, Default)]
pub struct JumpHost {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub authentication: String,
    pub private_key_path: String,
    pub private_key_passphrase: String,
    pub agent_socket: String,
    pub totp_secret: String,
    pub password_prompt_hint: String,
    pub totp_prompt_hint: String,
    pub auth_flow_mode: String,
}

impl JumpHost {
    /// Parses one jump entry from a JSON object (also used by the MCP
    /// `jumpHosts` parameter). Field names are snake_case, matching the
    /// `external_config.jump_hosts` form field.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let Some(config) = value.as_object() else {
            return Err("Jump host must be a JSON object".to_string());
        };
        Self::from_config(config)
    }

    fn from_config(value: &serde_json::Map<String, Value>) -> Result<Self, String> {
        let host = validate_host_field(string_field(value, "host")?)?;
        let port = match value.get("port") {
            None => 22,
            Some(port) => port
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or("Jump host port must be between 1 and 65535")?,
        };
        let authentication = value
            .get("authentication")
            .and_then(Value::as_str)
            .unwrap_or("password")
            .to_string();
        if !matches!(
            authentication.as_str(),
            "password" | "private-key" | "private-key-password" | "agent"
        ) {
            return Err(format!(
                "Unsupported jump host authentication method '{authentication}'"
            ));
        }
        Ok(Self {
            host,
            port,
            username: string_field(value, "username")?,
            // Credentials must survive verbatim: trimming would silently
            // change the password/passphrase the server sees.
            password: credential_string(value, "password"),
            authentication,
            private_key_path: optional_string(Some(value), "private_key_path"),
            private_key_passphrase: credential_string(value, "private_key_passphrase"),
            agent_socket: optional_string(Some(value), "agent_socket"),
            totp_secret: optional_string(Some(value), "totp_secret"),
            password_prompt_hint: optional_string(Some(value), "password_prompt_hint"),
            totp_prompt_hint: optional_string(Some(value), "totp_prompt_hint"),
            auth_flow_mode: optional_string(Some(value), "auth_flow_mode"),
        })
    }

    pub fn validate(&self, position: usize) -> Result<(), String> {
        match self.authentication.as_str() {
            "password" if self.password.is_empty() => Err(format!(
                "Jump host #{} ({}) requires a password",
                position + 1,
                self.host
            )),
            "private-key" | "private-key-password" if self.private_key_path.is_empty() => {
                Err(format!(
                    "Jump host #{} ({}) requires a private key path",
                    position + 1,
                    self.host
                ))
            }
            _ => Ok(()),
        }
    }

    /// Synthesizes a StoredConnection so the shared connect/auth pipeline
    /// (including keyboard-interactive 2FA) applies to jump hops as well.
    pub fn to_connection(
        &self,
        id: &str,
        timeout_secs: u64,
        keepalive_secs: u64,
    ) -> StoredConnection {
        StoredConnection {
            id: id.to_string(),
            // Jump hops are referenced by position in the chain, never by
            // name on the MCP surface.
            name: None,
            host: self.host.clone(),
            port: self.port,
            runtime_host: self.host.clone(),
            runtime_port: self.port,
            username: self.username.clone(),
            protocol: "ssh".to_string(),
            password: self.password.clone(),
            authentication: AuthenticationMethod::from_method_name(&self.authentication),
            private_key_path: self.private_key_path.clone(),
            private_key: String::new(),
            private_key_passphrase: self.private_key_passphrase.clone(),
            agent_socket: self.agent_socket.clone(),
            connect_timeout_secs: timeout_secs.max(1),
            // 跳数合成的连接其超时由调用方显式传入，按显式处理。
            connect_timeout_explicit: true,
            keepalive_interval_secs: keepalive_secs,
            // Jump hops carry no interactive terminal, so no activity
            // keepalive either.
            terminal_keepalive_secs: 0,
            read_only: false,
            sudo_password: String::new(),
            totp_secret: self.totp_secret.clone(),
            sudo_source: SudoSource::Custom,
            sudo_profile_ref: String::new(),
            sudo_use_pty: false,
            sudo_whitelist: Vec::new(),
            password_prompt_hint: self.password_prompt_hint.clone(),
            totp_prompt_hint: self.totp_prompt_hint.clone(),
            auth_flow_mode: self.auth_flow_mode.clone(),
            // Jump hops never inject SetEnv or exec a RemoteCommand: those
            // features target the final session only (deliberate
            // simplification, see StoredConnection docs).
            set_env: Vec::new(),
            remote_command: String::new(),
            // Jump hops never run interactive trigger stages.
            triggers_enabled: false,
            // Jump hops fetch no credentials locally: their inline credential
            // fields are the whole story.
            triggers: None,
            password_command: String::new(),
            passphrase_command: String::new(),
            jump_hosts: Vec::new(),
        }
    }
}

pub(in crate::core::model) fn parse_jump_hosts(
    external_config: Option<&serde_json::Map<String, Value>>,
) -> Result<Vec<JumpHost>, String> {
    let Some(list) = external_config
        .and_then(|config| config.get("jump_hosts"))
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };
    if list.len() > 3 {
        return Err("At most 3 jump hosts are supported".to_string());
    }
    let mut hosts: Vec<JumpHost> = Vec::with_capacity(list.len());
    for (position, value) in list.iter().enumerate() {
        let Some(config) = value.as_object() else {
            return Err(format!("Jump host #{} must be an object", position + 1));
        };
        let host = JumpHost::from_config(config)?;
        host.validate(position)?;
        // A repeated hop can only be a misconfiguration: the same
        // host:port dialled twice never advances the chain.
        if hosts
            .iter()
            .any(|existing| existing.host == host.host && existing.port == host.port)
        {
            return Err(format!(
                "Jump host #{} ({}) repeats an earlier hop",
                position + 1,
                host.host
            ));
        }
        hosts.push(host);
    }
    Ok(hosts)
}
