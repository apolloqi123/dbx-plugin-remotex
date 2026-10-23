//! Trigger types layer.

use super::*;

/// How secret/command answers are paced onto the wire (tssh `ExpectPassSleep`).
/// Plain `sendText` answers are never paced by this knob (contract §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PassSleep {
    /// Whole answer plus Enter in one write (default).
    #[default]
    None,
    /// One character per write, `sleep_ms` between characters.
    Each,
    /// Answer first, then Enter after `sleep_ms`.
    Enter,
}

impl PassSleep {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "each" => Some(Self::Each),
            "enter" => Some(Self::Enter),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Each => "each",
            Self::Enter => "enter",
        }
    }
}

/// One stage's answer, with secret slots already resolved to their values.
/// Variants other than `Secret`/`Totp` carry user-authored configuration
/// (plaintext by definition); `Secret` holds a fetched slot value or a
/// decrypted tssh blob and `Totp` holds a raw TOTP key — both are redacted in
/// `Debug` so a connection dump can never leak them.
#[derive(Clone, PartialEq, Eq)]
pub enum StageAnswer {
    Text(String),
    Secret(String),
    /// Raw RFC 4226 HMAC key (base32-decoded); the code is generated at match
    /// time so a long-pending stage answers with a fresh one.
    Totp(Vec<u8>),
    Command(String),
}

impl StageAnswer {
    /// Wire kind name (same vocabulary as `TriggerKind::name`); exercised by
    /// the parser tests, kept public for diagnostics.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Secret(_) | Self::Totp(_) => "secret",
            Self::Command(_) => "command",
        }
    }
}

impl fmt::Debug for StageAnswer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => write!(formatter, "Text({text:?})"),
            Self::Secret(_) => formatter.write_str("Secret(\"<redacted>\")"),
            Self::Totp(_) => formatter.write_str("Totp(\"<redacted>\")"),
            Self::Command(command) => write!(formatter, "Command({command:?})"),
        }
    }
}

/// Optional pre-match rule: fires the answer when `pattern` appears while the
/// stage is pending, without advancing the stage cursor. Only text/secret
/// answers are allowed here (validated at parse time).
#[derive(Clone, Debug)]
pub struct CaseRule {
    pub pattern: Regex,
    pub answer: StageAnswer,
}

/// One ordered expect stage: `pattern` hit → answer (three-way), with an
/// optional case pre-match.
#[derive(Clone, Debug)]
pub struct TriggerStage {
    pub pattern: Regex,
    pub answer: StageAnswer,
    pub case: Option<CaseRule>,
}

/// Parsed and validated `external_config.triggers` (contract §2.1). Invalid
/// shapes fail the connection (D7) instead of degrading silently.
#[derive(Clone)]
pub struct TriggersConfig {
    /// Seconds a pending stage may wait for its pattern before resetting.
    pub timeout_secs: u64,
    /// Pause between `\|` text segments (and between paced secret chars).
    pub sleep_ms: u64,
    /// Pacing mode for secret/command answers.
    pub pass_sleep: PassSleep,
    /// Ordered stages, 1..=MAX_STAGES.
    pub stages: Vec<TriggerStage>,
}

impl fmt::Debug for TriggersConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TriggersConfig")
            .field("timeout_secs", &self.timeout_secs)
            .field("sleep_ms", &self.sleep_ms)
            .field("pass_sleep", &self.pass_sleep.name())
            .field("stages", &self.stages)
            .finish()
    }
}

/// What the engine decided to type back after one output chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerDecision {
    /// 1-based stage number the decision belongs to (timeout reports the
    /// stage that gave up).
    pub stage: usize,
    pub kind: TriggerKind,
    /// Segment send plan: `(payload, delay_ms)` — the delay elapses before
    /// the payload is written (0 for the first segment). The read loop owns
    /// the actual pacing via `tokio::time::sleep`.
    pub segments: Vec<(Vec<u8>, u64)>,
    /// Only for `TriggerKind::Command`: the placeholder-substituted command
    /// the caller must run locally before sending its (trimmed) stdout.
    pub command: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    Text,
    Secret,
    Command,
    Timeout,
}

impl TriggerKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Secret => "secret",
            Self::Command => "command",
            Self::Timeout => "timeout",
        }
    }
}

/// `%h`/`%u`/`%p`/`%n` placeholder context for local command execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPlaceholders {
    pub host: String,
    pub username: String,
    pub port: u16,
    /// Connection display name, falling back to the connection id.
    pub name: String,
}

impl CommandPlaceholders {
    pub fn new(host: &str, username: &str, port: u16, name: &str) -> Self {
        Self {
            host: host.to_string(),
            username: username.to_string(),
            port,
            name: name.to_string(),
        }
    }
}

/// Substitutes `%h` host, `%u` username, `%p` port, `%n` connection name and
/// `%%` literal `%`; any other `%x` sequence passes through untouched.
pub fn apply_command_placeholders(command: &str, placeholders: &CommandPlaceholders) -> String {
    let mut output = String::with_capacity(command.len());
    let mut chars = command.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '%' {
            output.push(character);
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                output.push('%');
            }
            Some('h') => {
                chars.next();
                output.push_str(&placeholders.host);
            }
            Some('u') => {
                chars.next();
                output.push_str(&placeholders.username);
            }
            Some('p') => {
                chars.next();
                output.push_str(&placeholders.port.to_string());
            }
            Some('n') => {
                chars.next();
                output.push_str(&placeholders.name);
            }
            _ => output.push('%'),
        }
    }
    output
}
