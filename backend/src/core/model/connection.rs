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

impl StoredConnection {
    /// Quick Sudo is active unless the source is explicitly off; read-only
    /// connections gate it separately (see `ssh.rs`).
    pub fn sudo_enabled(&self) -> bool {
        self.sudo_source != SudoSource::Off
    }
}

mod fields;
mod jump;
mod parse;

pub use fields::*;
pub use jump::*;
