use super::*;

impl GateOutcome {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::WriteDenied => "write-denied",
            Self::WhitelistDenied => "whitelist-denied",
            Self::SensitivePath => "sensitive-path",
            Self::DestructiveUnconfirmed => "destructive-unconfirmed",
            Self::SudoAllowlistDenied => "sudo-allowlist-denied",
            Self::ReadOnlyServer => "read-only-server",
        }
    }
}

impl ApprovalTrail {
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Prompt => "prompt",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Timeout => "timeout",
            Self::Remembered => "remembered",
        }
    }
}

impl EntryOutcome {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }
}

impl ExecMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Embedded => "embedded",
            Self::Terminal => "terminal",
        }
    }
}

// The serde impls are hand-rolled around the `name()` accessors instead of
// `#[serde(rename)]` so the wire strings live in exactly one place per enum
// (the protocol contract strings must never drift between the two paths).

impl serde::Serialize for GateOutcome {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> serde::Deserialize<'de> for GateOutcome {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_name(&text).ok_or_else(|| {
            serde::de::Error::unknown_variant(
                &text,
                &[
                    "pass",
                    "write-denied",
                    "whitelist-denied",
                    "sensitive-path",
                    "destructive-unconfirmed",
                    "sudo-allowlist-denied",
                    "read-only-server",
                ],
            )
        })
    }
}

impl serde::Serialize for ApprovalTrail {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> serde::Deserialize<'de> for ApprovalTrail {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_name(&text).ok_or_else(|| {
            serde::de::Error::unknown_variant(
                &text,
                &[
                    "none",
                    "prompt",
                    "approved",
                    "denied",
                    "timeout",
                    "remembered",
                ],
            )
        })
    }
}

impl serde::Serialize for EntryOutcome {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> serde::Deserialize<'de> for EntryOutcome {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_name(&text)
            .ok_or_else(|| serde::de::Error::unknown_variant(&text, &["ok", "error"]))
    }
}

impl serde::Serialize for ExecMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> serde::Deserialize<'de> for ExecMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_name(&text).ok_or_else(|| {
            serde::de::Error::unknown_variant(&text, &["stdio", "embedded", "terminal"])
        })
    }
}

impl GateOutcome {
    fn from_name(text: &str) -> Option<Self> {
        Some(match text {
            "pass" => Self::Pass,
            "write-denied" => Self::WriteDenied,
            "whitelist-denied" => Self::WhitelistDenied,
            "sensitive-path" => Self::SensitivePath,
            "destructive-unconfirmed" => Self::DestructiveUnconfirmed,
            "sudo-allowlist-denied" => Self::SudoAllowlistDenied,
            "read-only-server" => Self::ReadOnlyServer,
            _ => return None,
        })
    }
}

impl ApprovalTrail {
    fn from_name(text: &str) -> Option<Self> {
        Some(match text {
            "none" => Self::None,
            "prompt" => Self::Prompt,
            "approved" => Self::Approved,
            "denied" => Self::Denied,
            "timeout" => Self::Timeout,
            "remembered" => Self::Remembered,
            _ => return None,
        })
    }
}

impl EntryOutcome {
    fn from_name(text: &str) -> Option<Self> {
        Some(match text {
            "ok" => Self::Ok,
            "error" => Self::Error,
            _ => return None,
        })
    }
}

impl ExecMode {
    fn from_name(text: &str) -> Option<Self> {
        Some(match text {
            "stdio" => Self::Stdio,
            "embedded" => Self::Embedded,
            "terminal" => Self::Terminal,
            _ => return None,
        })
    }
}

/// Serialize by hand (explicit camelCase keys, `None` → `null`) so the wire
/// shape is pinned in one place instead of depending on derive attributes.
impl serde::Serialize for AuditEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AuditEntry", 12)?;
        state.serialize_field("tsMs", &self.ts_ms)?;
        state.serialize_field("tool", &self.tool)?;
        state.serialize_field("connectionId", &self.connection_id)?;
        state.serialize_field("gate", &self.gate)?;
        state.serialize_field("approval", &self.approval)?;
        state.serialize_field("outcome", &self.outcome)?;
        state.serialize_field("exitCode", &self.exit_code)?;
        state.serialize_field("durationMs", &self.duration_ms)?;
        state.serialize_field("mode", &self.mode)?;
        state.serialize_field("command", &self.command)?;
        state.serialize_field("output", &self.output)?;
        state.serialize_field("error", &self.error)?;
        state.end()
    }
}

/// Deserialize through a camelCase-named bridge struct; all fields required
/// (missing `exitCode`/`error` is a bad line, skipped by `tail`). The newer
/// `command`/`output` fields default to `None` so pre-0.4.77 lines (which
/// never carried them) keep replaying.
impl<'de> serde::Deserialize<'de> for AuditEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            ts_ms: u64,
            tool: String,
            connection_id: String,
            gate: GateOutcome,
            approval: ApprovalTrail,
            outcome: EntryOutcome,
            exit_code: Option<i64>,
            duration_ms: u64,
            mode: ExecMode,
            #[serde(default)]
            command: Option<String>,
            #[serde(default)]
            output: Option<String>,
            error: Option<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            ts_ms: wire.ts_ms,
            tool: wire.tool,
            connection_id: wire.connection_id,
            gate: wire.gate,
            approval: wire.approval,
            outcome: wire.outcome,
            exit_code: wire.exit_code,
            duration_ms: wire.duration_ms,
            mode: wire.mode,
            command: wire.command,
            output: wire.output,
            error: wire.error,
        })
    }
}
