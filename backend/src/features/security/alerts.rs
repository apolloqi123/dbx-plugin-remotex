//! Alert triage: structured normalization + keyword classification +
//! whitelist-safe diagnostic command suggestions.
//!
//! Positioning is deliberately narrow: this module performs NO LLM analysis.
//! Triage = (1) normalize a heterogeneous alert payload into a fixed shape,
//! (2) classify it into one of eight categories via bilingual keyword scoring,
//! (3) emit read-only diagnostic commands. The reasoning about *why* the alert
//! fired is left to the external caller (ZCode, an operator); the plugin is a
//! tool surface, not an agent.
//!
//! Compatibility semantics come from openocta's `/hooks/alert` standardization:
//! a payload that parses as JSON is read field-wise (`alertId`/`alert_id`,
//! `title`, `message`, `severity`, `source`, object `data`); when the parsed
//! `message` is empty — or the payload does not parse as JSON at all, which
//! covers plain-text alerts — the entire payload text becomes the message.
//!
//! Hard constraint (decision D6, pinned by unit tests): every command emitted
//! by [`playbook`] — including the service group with a substituted unit name
//! — must satisfy `mcp_safety::assess_command(cmd) == CommandRisk::ReadOnly`,
//! so the suggestions can pass the plugin's own read-only gate untouched.
//! Suggestions never contain redirection, command substitution, `sudo`, or
//! write verbs. The initial draft's `top -b -n 1 | head -20` was dropped
//! because `top` is not a whitelisted verb (its information is covered by
//! `ps aux --sort=-%cpu` and `vmstat`). To keep D6 true for hostile inputs,
//! [`extract_service`] rejects unit candidates that reference a sensitive
//! path, which would otherwise downgrade `systemctl status <svc>` to
//! `Unknown` (falling back to the generic command set).

use serde_json::Value;

use crate::features::mcp::safety::is_sensitive_path;

/// Character clamp for `title` / `message` / `alert_id` / `source`.
const MAX_FIELD_CHARS: usize = 2048;
/// Character clamp for the pretty-printed `data` JSON blob.
const MAX_DATA_JSON_CHARS: usize = 16384;

/// Normalized shape of an alert payload.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NormalizedAlert {
    pub alert_id: String,
    pub title: String,
    pub message: String,
    pub severity: String,
    pub source: String,
    pub data_json: String,
}

/// Alert category derived from keyword scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Cpu,
    Memory,
    Disk,
    Inode,
    Network,
    Oom,
    Service,
    Generic,
}

impl Category {
    /// Stable wire name (`ssh/alert/triage` response `category` field).
    pub fn name(self) -> &'static str {
        match self {
            Category::Cpu => "cpu",
            Category::Memory => "memory",
            Category::Disk => "disk",
            Category::Inode => "inode",
            Category::Network => "network",
            Category::Oom => "oom",
            Category::Service => "service",
            Category::Generic => "generic",
        }
    }
}

/// One suggested diagnostic command. `purpose_key` is a stable contract key
/// the frontend maps to i18n labels; it must never be renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub command: String,
    pub purpose_key: &'static str,
}

/// Triage result: normalized alert + category + whitelist-safe suggestions.
#[derive(Debug, Clone)]
pub struct TriageResult {
    pub normalized: NormalizedAlert,
    pub category: Category,
    pub suggestions: Vec<Suggestion>,
}

/// Normalizes a raw alert payload (JSON or plain text) into the fixed shape.
///
/// JSON path: string fields are picked (`alertId` with an `alert_id`
/// fallback); an object `data` becomes pretty-printed `data_json`. When the
/// parsed `message` is empty the entire payload becomes the message.
/// Non-JSON path (plain-text alerts): the entire payload becomes the message.
/// Strings are char-clamped; `severity` / `source` are lowercased and a
/// missing severity becomes `"unknown"`.
pub fn normalize(payload: &str) -> NormalizedAlert {
    let trimmed = payload.trim();
    let mut alert_id = String::new();
    let mut title = String::new();
    let mut severity = String::new();
    let mut source = String::new();
    let mut data_json = String::new();
    let mut message;

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        alert_id = string_field(&value, &["alertId", "alert_id"]);
        title = string_field(&value, &["title"]);
        message = string_field(&value, &["message"]);
        severity = string_field(&value, &["severity"]).to_lowercase();
        source = string_field(&value, &["source"]).to_lowercase();
        if let Some(data) = value.get("data").filter(|value| value.is_object()) {
            data_json = serde_json::to_string_pretty(data).unwrap_or_default();
        }
        // openocta /hooks/alert compatibility: an alert whose `message` is
        // empty degrades to the whole payload text.
        if message.is_empty() {
            message = trimmed.to_string();
        }
    } else {
        // Plain-text alert (or any non-JSON payload).
        message = trimmed.to_string();
    }

    if severity.is_empty() {
        severity = "unknown".to_string();
    }

    NormalizedAlert {
        alert_id: clamp_chars(&alert_id, MAX_FIELD_CHARS),
        title: clamp_chars(&title, MAX_FIELD_CHARS),
        message: clamp_chars(&message, MAX_FIELD_CHARS),
        severity,
        source,
        data_json: clamp_chars(&data_json, MAX_DATA_JSON_CHARS),
    }
}

/// First present string field among `keys`; non-string values count as
/// missing. Returns an empty string when no key matches.
fn string_field(value: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            return text.to_string();
        }
    }
    String::new()
}

fn clamp_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Bilingual (English + Chinese) keyword sets per category, lowercased.
const KEYWORDS: &[(Category, &[&str])] = &[
    (
        Category::Oom,
        &[
            "oom",
            "out of memory",
            "oom-kill",
            "killed process",
            "内存耗尽",
        ],
    ),
    (Category::Inode, &["inode"]),
    (
        Category::Cpu,
        &[
            "cpu",
            "processor",
            "load average",
            "loadavg",
            "处理器",
            "负载",
        ],
    ),
    (Category::Memory, &["memory", "mem usage", "内存", "swap"]),
    (
        Category::Disk,
        &["disk", "no space", "disk space", "磁盘", "剩余空间"],
    ),
    (
        Category::Network,
        &["network", "bandwidth", "packet loss", "丢包", "网卡"],
    ),
    (
        Category::Service,
        &["systemd", "service", "unit", "服务", "restart"],
    ),
];

/// Tie-break precedence. Declaration order applies (first listed wins on
/// equal scores) with one deliberate override: `Oom` precedes `Memory`, so
/// "内存耗尽" / "out of memory" alerts — where the OOM keyword also scores a
/// Memory keyword — always land on Oom. Pinned by `classify` tests.
const SCORE_ORDER: &[Category] = &[
    Category::Oom,
    Category::Cpu,
    Category::Memory,
    Category::Disk,
    Category::Inode,
    Category::Network,
    Category::Service,
];

/// Classifies an alert by keyword scoring over `title + message + data_json`
/// (lowercased). Highest score wins; all-zero scores yield
/// [`Category::Generic`]; ties are resolved by [`SCORE_ORDER`].
pub fn classify(alert: &NormalizedAlert) -> Category {
    let text = format!("{} {} {}", alert.title, alert.message, alert.data_json).to_lowercase();
    let mut best = Category::Generic;
    let mut best_score = 0usize;
    for category in SCORE_ORDER {
        let Some((_, keywords)) = KEYWORDS.iter().find(|(owner, _)| owner == category) else {
            continue;
        };
        let score: usize = keywords
            .iter()
            .map(|keyword| text.matches(keyword).count())
            .sum();
        if score > best_score {
            best_score = score;
            best = *category;
        }
    }
    best
}

/// Extracts the systemd unit name for the Service category.
///
/// Order: first `<name>.service` token, then `unit=<name>`, then
/// `service=<name>`. Candidates are restricted to `[A-Za-z0-9@._-]+` and are
/// rejected when they reference a sensitive path (defense for decision D6 —
/// such a name would keep `systemctl status <svc>` out of the read-only
/// class, so the caller falls back to the generic command set instead).
pub fn extract_service(alert: &NormalizedAlert) -> Option<String> {
    let haystack = format!("{} {}", alert.message, alert.data_json).to_lowercase();
    service_token(&haystack)
        .or_else(|| keyed_service(&haystack, "unit="))
        .or_else(|| keyed_service(&haystack, "service="))
}

/// Valid unit-name characters (`systemctl status`/`journalctl -u` argument).
fn is_unit_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '-')
}

/// Longest valid-unit-character run starting at `start`.
fn unit_run_at(text: &str, start: usize) -> String {
    text[start..]
        .chars()
        .take_while(|c| is_unit_char(*c))
        .collect()
}

/// First maximal `[A-Za-z0-9@._-]+` run that ends with `.service` and has a
/// non-empty name part.
fn service_token(text: &str) -> Option<String> {
    let mut start = 0;
    while start < text.len() {
        if !text[start..].starts_with(|c: char| is_unit_char(c)) {
            start += 1;
            continue;
        }
        let run = unit_run_at(text, start);
        if let Some(name) = run.strip_suffix(".service") {
            if !name.is_empty() && !is_sensitive_path(&run) {
                return Some(run);
            }
        }
        start += run.len().max(1);
    }
    None
}

/// Value after a `key=` marker (e.g. `unit=nginx`).
fn keyed_service(text: &str, key: &str) -> Option<String> {
    let mut rest = text;
    while let Some(position) = rest.find(key) {
        let value = unit_run_at(rest, position + key.len());
        if !value.is_empty() && !is_sensitive_path(&value) {
            return Some(value);
        }
        rest = &rest[position + key.len()..];
    }
    None
}

/// Generic command set used as the fallback for [`Category::Generic`] and for
/// [`Category::Service`] when no unit name could be extracted (test-pinned).
fn generic_suggestions() -> Vec<Suggestion> {
    vec![
        suggestion("uptime", "loadSnapshot"),
        suggestion("df -h", "diskUsage"),
        suggestion("free -m", "memFree"),
        suggestion("ps aux --sort=-%cpu | head -10", "cpuTopProcesses"),
    ]
}

fn suggestion(command: &str, purpose_key: &'static str) -> Suggestion {
    Suggestion {
        command: command.to_string(),
        purpose_key,
    }
}

/// Static playbook per category. Every emitted command is pinned by the
/// `playbook` tests to satisfy `mcp_safety::assess_command == ReadOnly`
/// (decision D6).
///
/// Dropped from the initial draft: `top -b -n 1 | head -20` (`cpuTop`) —
/// `top` is not in `mcp_safety::READ_ONLY_VERBS`, so the pipeline degrades to
/// `Unknown`; its information (hot processes + instantaneous CPU state) is
/// already covered by `cpuTopProcesses` and `cpuVmstat`.
pub fn playbook(category: Category, alert: &NormalizedAlert) -> Vec<Suggestion> {
    match category {
        Category::Cpu => vec![
            suggestion("uptime", "loadSnapshot"),
            suggestion("ps aux --sort=-%cpu | head -15", "cpuTopProcesses"),
            suggestion("vmstat 1 3", "cpuVmstat"),
        ],
        Category::Memory => vec![
            suggestion("free -m", "memFree"),
            suggestion("ps aux --sort=-%mem | head -15", "memTopProcesses"),
        ],
        Category::Disk => vec![
            suggestion("df -h", "diskUsage"),
            suggestion("du -x -d 1 / | sort -rh | head -15", "diskDu"),
        ],
        Category::Inode => vec![suggestion("df -i", "diskInode")],
        Category::Network => vec![
            suggestion("ss -s", "netSummary"),
            suggestion("ip -s link", "netLinks"),
        ],
        Category::Oom => vec![
            suggestion("dmesg -T | tail -100", "oomDmesg"),
            suggestion("journalctl -k -n 200 --no-pager", "oomJournal"),
        ],
        Category::Service => match extract_service(alert) {
            Some(service) => vec![
                suggestion(&format!("systemctl status {service}"), "serviceStatus"),
                suggestion(
                    &format!("journalctl -u {service} -n 100 --no-pager"),
                    "serviceJournal",
                ),
            ],
            // No unit name found: fall back to the generic command set.
            None => generic_suggestions(),
        },
        Category::Generic => generic_suggestions(),
    }
}

/// Runs the full pipeline: normalize → classify → playbook.
pub fn triage(payload: &str) -> TriageResult {
    let normalized = normalize(payload);
    let category = classify(&normalized);
    let suggestions = playbook(category, &normalized);
    TriageResult {
        normalized,
        category,
        suggestions,
    }
}

/// Protocol response shape for `ssh/alert/triage` (camelCase).
pub fn triage_view(result: &TriageResult) -> Value {
    serde_json::json!({
        "normalized": {
            "alertId": result.normalized.alert_id,
            "title": result.normalized.title,
            "message": result.normalized.message,
            "severity": result.normalized.severity,
            "source": result.normalized.source,
            "dataJson": result.normalized.data_json,
        },
        "category": result.category.name(),
        "suggestions": result
            .suggestions
            .iter()
            .map(|item| {
                serde_json::json!({
                    "command": item.command,
                    "purposeKey": item.purpose_key,
                })
            })
            .collect::<Vec<Value>>(),
    })
}

#[cfg(test)]
mod tests;
