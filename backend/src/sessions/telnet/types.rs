//! Telnet layer.

use super::*;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelnetStartRequest {
    pub connection_id: Option<String>,
    pub workbench_id: String,
    /// Logical endpoint retained for identity and display.
    pub host: String,
    /// Logical endpoint port. Defaults to 23.
    pub port: Option<u16>,
    /// Runtime endpoint supplied by the host transport; it is the only target
    /// dialed by the TCP pump. Missing fields fall back to the logical endpoint.
    pub runtime_host: Option<String>,
    pub runtime_port: Option<u16>,
    pub enter_mode: Option<EnterMode>,
    pub backspace_mode: Option<BackspaceMode>,
    /// Initial window size for the first NAWS frame; the workbench sends its
    /// real size right after start anyway.
    pub cols: Option<u32>,
    pub rows: Option<u32>,
    /// Optional auto-login, in exactly one of two forms (both validated
    /// before the dial — D7: invalid configuration fails the start with a
    /// readable error and never degrades silently).
    pub auto_login: Option<AutoLoginSpec>,
}

/// Auto-login request: either expect `rules` (tssh/JSON grammar) or the
/// NyaTerm-style `declarative` prompt/credential form — mutually exclusive.
/// Secrets travel as request values only; `Debug` redacts them so no log or
/// panic message can ever carry a credential.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoLoginSpec {
    /// tssh text form, JSON string form or raw JSON object form — exactly the
    /// shapes `triggers/validate` accepts.
    pub rules: Option<Value>,
    /// Slot values for `trigger_answer_1` / `trigger_answer_2`, in order.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Declarative form (P0-1): prompt regexes + credentials + success /
    /// failure detection + retry budget.
    pub declarative: Option<DeclarativeAutoLogin>,
}

impl fmt::Debug for AutoLoginSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AutoLoginSpec")
            .field("rules", &self.rules)
            .field("secrets", &"<redacted>")
            .field("declarative", &self.declarative)
            .finish()
    }
}

/// NyaTerm-parity declarative login config (`TelnetAutoLoginConfig` minus the
/// knobs the dialog does not expose). Regexes are optional — blank falls back
/// to the built-in prompt vocabulary ([`DECL_USERNAME_PROMPT_DEFAULT`] and
/// friends). `Debug` redacts the password.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclarativeAutoLogin {
    pub username: Option<String>,
    pub password: Option<String>,
    pub username_prompt_regex: Option<String>,
    pub password_prompt_regex: Option<String>,
    pub success_regex: Option<String>,
    pub failure_regex: Option<String>,
    pub max_retries: Option<u8>,
}

impl fmt::Debug for DeclarativeAutoLogin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeclarativeAutoLogin")
            .field("username", &self.username.as_ref().map(|_| "<set>"))
            .field("password", &"<redacted>")
            .field("usernamePromptRegex", &self.username_prompt_regex)
            .field("passwordPromptRegex", &self.password_prompt_regex)
            .field("successRegex", &self.success_regex)
            .field("failureRegex", &self.failure_regex)
            .field("maxRetries", &self.max_retries)
            .finish()
    }
}

/// A parsed auto-login request: the expect-engine configuration (both forms
/// lower onto it) plus, for the declarative form, the success/failure/retry
/// supervision the engine itself does not model.
#[derive(Debug)]
pub struct AutoLoginPlan {
    pub triggers: triggers::TriggersConfig,
    pub watch: Option<DeclarativeWatch>,
}

/// Per-connection auto-login runtime owned by the read loop.
pub(crate) struct AutoLoginRuntime {
    pub(crate) engine: triggers::TriggerEngine,
    pub(crate) watch: Option<DeclarativeWatch>,
}

pub(crate) enum TelnetCommand {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Close,
}

pub(crate) struct TelnetSession {
    pub(crate) connection_id: Option<String>,
    pub(crate) workbench_id: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) runtime_host: String,
    pub(crate) runtime_port: u16,
    pub(crate) created_at_secs: u64,
    pub(crate) cmd_tx: mpsc::Sender<TelnetCommand>,
    pub(crate) replay: Arc<tokio::sync::Mutex<ReplayBuffer>>,
}

pub struct TelnetSessionRuntime {
    pub(crate) sessions: Arc<RwLock<HashMap<String, Arc<TelnetSession>>>>,
}
