//! Model connection types.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationMethod {
    Password,
    PrivateKey,
    PrivateKeyPassword,
    Agent,
    /// Tabby-style ordered fallback: try password → private key →
    /// keyboard-interactive (incl. TOTP) → ssh-agent, in that fixed order,
    /// until one succeeds or every attempt has failed. Stage prerequisites
    /// (no password, no key material, unreachable agent) are recorded as
    /// skipped attempts instead of aborting the chain.
    Auto,
    None,
}

impl AuthenticationMethod {
    fn from_connection(connection: &serde_json::Map<String, Value>) -> Result<Self, String> {
        let value = connection
            .get("external_config")
            .and_then(Value::as_object)
            .and_then(|config| config.get("authentication"))
            .and_then(Value::as_str)
            .unwrap_or("password");
        match value {
            "password" | "private-key" | "private-key-password" | "agent" | "none" | "auto" => {
                Ok(Self::from_method_name(value))
            }
            _ => Err(format!("Unsupported SSH authentication method '{value}'")),
        }
    }

    pub(super) fn from_method_name(value: &str) -> Self {
        match value {
            "private-key" => Self::PrivateKey,
            "private-key-password" => Self::PrivateKeyPassword,
            "agent" => Self::Agent,
            "none" => Self::None,
            "auto" => Self::Auto,
            _ => Self::Password,
        }
    }

    /// Canonical method name (the `external_config.authentication` spelling)
    /// for display in read-only payloads such as `ssh/sessions/list`.
    /// Deliberately excludes any credential material — names only.
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::PrivateKey => "private-key",
            Self::PrivateKeyPassword => "private-key-password",
            Self::Agent => "agent",
            Self::None => "none",
            Self::Auto => "auto",
            Self::Password => "password",
        }
    }
}

/// Where sudo credentials come from (connection form `sudo_source`): the
/// connection's own values ("custom"), a global Quick Sudo profile
/// ("global", resolved from `sudo_profile_ref` or the workbench binding),
/// or disabled ("off"). Legacy 0.4.x connections without the field map from
/// the old `quick_sudo` boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SudoSource {
    Off,
    Custom,
    Global,
}

impl SudoSource {
    /// `sudo_source` wins whenever it is present and recognized; absent,
    /// empty, or unknown values fall back to the legacy `quick_sudo` flag so
    /// existing connections keep their behavior.
    pub fn parse(value: Option<&str>, legacy_quick_sudo: bool) -> Self {
        match value.map(str::trim) {
            Some("off") => Self::Off,
            Some("global") => Self::Global,
            Some("custom") => Self::Custom,
            _ => {
                if legacy_quick_sudo {
                    Self::Custom
                } else {
                    Self::Off
                }
            }
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Custom => "custom",
            Self::Global => "global",
        }
    }
}

/// Logical connection identity plus the host-provided runtime dial target.
/// Non-SSH saved connections use this small lifecycle shape instead of the
/// SSH credential parser; the logical endpoint remains the UI identity while
/// TCP traffic always targets `runtime_host:runtime_port`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEndpoint {
    pub connection_id: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub runtime_host: String,
    pub runtime_port: u16,
}

impl RuntimeEndpoint {
    pub fn from_lifecycle_params(params: &Value) -> Result<Self, String> {
        let connection = params
            .get("connection")
            .and_then(Value::as_object)
            .ok_or("Missing connection payload")?;
        let connection_id = string_field(connection, "id")?;
        let host = validate_host_field(string_field(connection, "host")?)?;
        let port = connection
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or("Connection port must be between 1 and 65535")?;
        let runtime = params.get("runtime").and_then(Value::as_object);
        let runtime_host = optional_string(runtime, "host");
        let runtime_port = runtime
            .and_then(|value| value.get("port"))
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(port);
        let protocol = optional_string(
            connection.get("external_config").and_then(Value::as_object),
            "protocol",
        );
        Ok(Self {
            connection_id,
            protocol: match protocol.as_str() {
                // M32-B：serial/rdp 连接记录与 telnet/vnc 同走非 SSH 生命周期
                // （test 探测 / connect 直通），不进 SSH 凭据解析。
                "telnet" | "vnc" | "serial" | "rdp" => protocol,
                _ => "ssh".to_string(),
            },
            host: host.clone(),
            port,
            runtime_host: if runtime_host.is_empty() {
                host
            } else {
                runtime_host
            },
            runtime_port,
        })
    }
}

#[derive(Debug, Clone)]
pub struct StoredConnection {
    pub id: String,
    /// Display name from the lifecycle payload (`connection.name`); `None`
    /// for payloads without one (older hosts, inline MCP dials). Powers the
    /// `connectionName` lookup on the MCP surface and the metadata-only
    /// `ssh_list_connections` view. Never a credential.
    pub name: Option<String>,
    pub host: String,
    pub port: u16,
    pub runtime_host: String,
    pub runtime_port: u16,
    pub username: String,
    /// Retained for the SSH/MCP parser's forward-compatible lifecycle model;
    /// non-SSH routing is handled by [`RuntimeEndpoint`] before SSH parsing.
    #[allow(dead_code)]
    pub protocol: String,
    pub password: String,
    pub authentication: AuthenticationMethod,
    pub private_key_path: String,
    pub private_key_passphrase: String,
    /// Inline private key contents (`connection_secrets.private_key`), the
    /// pasted-key alternative to `private_key_path`. Non-empty content wins
    /// over the path (see `ssh.rs` key resolution); like every secret here it
    /// only travels inside the connect pipeline and is never echoed back.
    pub private_key: String,
    #[cfg_attr(windows, allow(dead_code))]
    pub agent_socket: String,
    pub connect_timeout_secs: u64,
    /// Whether `connect_timeout_secs` was explicitly present in the lifecycle
    /// params (vs defaulted to 30). `connection/test` needs this to align its
    /// dial budget with the host-side RPC deadline: a stored/absent value of
    /// 0 materializes as the host's own fallback (10s), not the manifest
    /// default, so an absent field means the host will kill the call at 10s.
    pub connect_timeout_explicit: bool,
    pub keepalive_interval_secs: u64,
    /// Interactive-terminal activity keepalive: interval in seconds for
    /// injecting space+backspace into the PTY so server-side idle policies
    /// (TMOUT, bastion keystroke audits) never fire. 0 = off; the parser
    /// clamps enabled values into 5..=3600.
    pub terminal_keepalive_secs: u64,
    pub read_only: bool,
    /// Quick Sudo orchestration: sudo password override, TOTP secret, and
    /// prompt hints. Secrets come from `connection_secrets`, tuning from
    /// `external_config`. `sudo_source` selects the credential source and
    /// `sudo_profile_ref` names the global profile when the source is global.
    pub sudo_password: String,
    pub totp_secret: String,
    pub sudo_source: SudoSource,
    pub sudo_profile_ref: String,
    pub sudo_use_pty: bool,
    /// sudoers-style per-connection sudo command allowlist
    /// (`external_config.sudo_whitelist`); empty = gate off.
    pub sudo_whitelist: Vec<String>,
    pub password_prompt_hint: String,
    pub totp_prompt_hint: String,
    pub auth_flow_mode: String,
    /// Client-specified remote environment — the `SetEnv` half of ssh's
    /// SetEnv/SendEnv pair (values come from this connection's config, never
    /// from the local process environment). Parsed and validated eagerly so
    /// malformed entries fail the connection instead of misconfiguring
    /// remote commands; applied per channel before shell/exec.
    pub set_env: Vec<(String, String)>,
    /// `ssh RemoteCommand`: exec this command instead of a shell on the
    /// interactive terminal session (PTY stays on). Empty = normal shell.
    pub remote_command: String,
    /// Independent connection-form switch for expect-style terminal triggers.
    /// It defaults to false, including for existing connections that still
    /// contain text in `external_config.triggers`.
    pub triggers_enabled: bool,
    /// Expect-style terminal triggers (tssh parity, contract §2.1): parsed and
    /// validated eagerly only when `triggers_enabled` is true. `None` = disabled.
    /// Secret answers are resolved from `connection_secrets` at parse time.
    pub triggers: Option<crate::features::automation::triggers::TriggersConfig>,
    /// Local command fetching the login password when none is stored
    /// (`external_config.password_command`; tssh PasswordCommand parity).
    /// Runs only when `password` is empty — explicit credentials win.
    pub password_command: String,
    /// Local command fetching the private-key passphrase when none is stored
    /// (`external_config.passphrase_command`; tssh PassphraseCommand parity).
    pub passphrase_command: String,
    /// ProxyJump chain: each entry is dialed before the target, the final hop
    /// reaching `host:port` directly (the runtime tunnel endpoint is skipped).
    pub jump_hosts: Vec<JumpHost>,
}

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

impl StoredConnection {
    /// Quick Sudo is active unless the source is explicitly off; read-only
    /// connections gate it separately (see `ssh.rs`).
    pub fn sudo_enabled(&self) -> bool {
        self.sudo_source != SudoSource::Off
    }

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

/// First present string among `keys` in the config object.
pub(super) fn config_text<'a>(
    config: Option<&'a serde_json::Map<String, Value>>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| config.and_then(|config| config.get(*key)))
        .and_then(Value::as_str)
}

/// Terminal activity keepalive: 0 disables, anything else is bounded so a
/// typo can neither hammer the PTY (5s floor) nor idle for days (1h ceiling).
pub(super) fn clamp_terminal_keepalive(raw: u64) -> u64 {
    if raw == 0 {
        0
    } else {
        raw.clamp(5, 3600)
    }
}

/// Numeric `binding: config` field: the connection form writes these into
/// `external_config`; the top-level connection object stays as fallback for
/// hand-edited configs and older payloads (same convergence as `read_only`,
/// which the timeout/keepalive fields previously lacked — the form values
/// never reached the parser and the defaults always won).
pub(super) fn config_u64(
    external_config: Option<&serde_json::Map<String, Value>>,
    connection: &serde_json::Map<String, Value>,
    key: &str,
) -> Option<u64> {
    external_config
        .and_then(|config| config.get(key))
        .or_else(|| connection.get(key))
        .and_then(Value::as_u64)
}

/// Parses the `setEnv` connection field: one `KEY=VALUE` entry per line,
/// semicolons tolerated as separators (mirrors the `totp_secret` input
/// convention), blank entries ignored, entries trimmed. Strict by design —
/// "prefer failing the connection to silently misconfiguring remote
/// commands": every invalid entry is reported with its content in one
/// aggregated error. Duplicate keys follow `ssh SetEnv` semantics: the last
/// entry wins. This is the client-specified half of ssh's SetEnv/SendEnv —
/// no local environment is ever transmitted.
pub fn parse_set_env(raw: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut vars: Vec<(String, String)> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for entry in raw.split(['\n', ';']) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some((key, value)) = entry.split_once('=') else {
            errors.push(format!("'{entry}' (missing '=' separator)"));
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() {
            errors.push(format!("'{entry}' (empty key)"));
            continue;
        }
        if key.chars().any(char::is_whitespace) {
            errors.push(format!("'{entry}' (key must not contain whitespace)"));
            continue;
        }
        if key.contains('\0') {
            errors.push(format!("'{entry}' (NUL byte in key)"));
            continue;
        }
        if value.contains('\0') {
            errors.push(format!("'{entry}' (NUL byte in value)"));
            continue;
        }
        match vars.iter_mut().find(|(existing, _)| existing == key) {
            Some(slot) => slot.1 = value.to_string(),
            None => vars.push((key.to_string(), value.to_string())),
        }
    }
    if errors.is_empty() {
        Ok(vars)
    } else {
        Err(format!("Invalid setEnv entries: {}", errors.join("; ")))
    }
}

pub(super) fn parse_jump_hosts(
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

/// Host fields must be a dialable hostname/IP: no internal whitespace and no
/// URI scheme syntax (`ssh://…`) that would silently become a bogus TCP
/// target instead of failing fast at parse time.
pub(super) fn validate_host_field(host: String) -> Result<String, String> {
    if host.chars().any(char::is_whitespace) {
        return Err(format!(
            "Invalid SSH host '{host}': must not contain whitespace"
        ));
    }
    if host.contains("://") {
        return Err(format!(
            "Invalid SSH host '{host}': use a bare hostname, not a URI"
        ));
    }
    Ok(host)
}

/// Reads a credential field verbatim (no trimming): passwords and
/// passphrases may legitimately start or end with spaces.
pub(super) fn credential_string(object: &serde_json::Map<String, Value>, key: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub(super) fn optional_string(
    object: Option<&serde_json::Map<String, Value>>,
    key: &str,
) -> String {
    object
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub(super) fn string_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("Missing SSH {key}"))
}
