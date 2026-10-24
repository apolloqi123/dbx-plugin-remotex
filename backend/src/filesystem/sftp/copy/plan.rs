//! Copy/move planning phase: request parsing, target naming, command building
//! and the `overwrite=false` existence probe. Everything here is pure (no I/O,
//! no session access), so the execution phase can be reasoned about on its own.

use super::*;

use serde_json::json;

use crate::core::exec;
use crate::core::model::normalize_remote_path;
use crate::filesystem::sftp::name as sftp_name;

/// Whether an operation copies or moves its sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyOp {
    Copy,
    Move,
}

/// latin-1 模式下源/目标路径的字节还原口径：工作台 RPC（`sftp/copy`）传
/// 列表回传的 wire 形式（%XX 转义，[`sftp_name::unescape_wire`] 还原）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathForm {
    WireEscaped,
}

impl PathForm {
    /// wire 文本 → 服务器原始字节。
    pub fn decode(self, path: &str) -> Vec<u8> {
        match self {
            PathForm::WireEscaped => sftp_name::unescape_wire(path),
        }
    }
}

impl CopyOp {
    pub(super) fn label(self) -> &'static str {
        match self {
            CopyOp::Copy => "copy",
            CopyOp::Move => "move",
        }
    }
}

/// Parsed `sftp/copy` / `sftp/move` request:
/// `{ connectionId, from: string|string[], toDir: string, overwrite?: bool }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyMoveRequest {
    pub from: Vec<String>,
    pub to_dir: String,
    pub overwrite: bool,
}

/// Per-source outcome of a copy/move batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemOutcome {
    pub from: String,
    pub to: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// Batch outcome: `{ success, results: [{ from, to, ok, error? }] }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyMoveOutcome {
    pub success: bool,
    pub results: Vec<ItemOutcome>,
}

impl CopyMoveOutcome {
    /// Contract shape consumed by the SFTP panel: `error` is present only
    /// on failed items.
    pub fn into_json(self) -> Value {
        json!({
            "success": self.success,
            "results": self.results.iter().map(ItemOutcome::to_json).collect::<Vec<_>>(),
        })
    }
}

impl ItemOutcome {
    fn to_json(&self) -> Value {
        let mut item = json!({ "from": self.from, "to": self.to, "ok": self.ok });
        if let Some(error) = &self.error {
            item["error"] = json!(error);
        }
        item
    }

    pub(super) fn ok(from: &str, to: &str) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
            ok: true,
            error: None,
        }
    }

    pub(super) fn failed(from: &str, to: &str, error: String) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
            ok: false,
            error: Some(error),
        }
    }
}

// ---------------------------------------------------------------------------
// Request parsing (pure)
// ---------------------------------------------------------------------------

/// Parses the copy/move parameters shared by the plugin RPC and the MCP tool.
pub fn parse_request(params: &Value) -> Result<CopyMoveRequest, String> {
    let from_value = params.get("from").ok_or("Missing from")?;
    let raw_sources = match from_value {
        Value::String(single) => vec![single.clone()],
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "from must be a string or an array of strings".to_string())
            })
            .collect::<Result<Vec<String>, String>>()?,
        _ => return Err("from must be a string or an array of strings".to_string()),
    };
    let mut from = Vec::with_capacity(raw_sources.len());
    for source in raw_sources {
        let trimmed = source.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = normalize_remote_path(trimmed)?;
        if normalized == "/" {
            return Err("Refusing to copy or move the filesystem root".to_string());
        }
        from.push(normalized);
    }
    if from.is_empty() {
        return Err("At least one source path is required".to_string());
    }
    let to_dir = params
        .get("toDir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("Missing toDir")?;
    let to_dir = normalize_remote_path(to_dir)?;
    // Boolean tolerance lives with the MCP argument helpers: a string
    // "true" must not silently read as overwrite=false.
    let overwrite = crate::features::mcp::arg_bool(params, "overwrite")?.unwrap_or(false);
    Ok(CopyMoveRequest {
        from,
        to_dir,
        overwrite,
    })
}

/// Target path for a source copied into `to_dir`: the source's own name
/// underneath the destination directory.
pub fn target_path(to_dir: &str, from: &str) -> String {
    let name = from
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    if name.is_empty() {
        return format!("{}/copy", to_dir.trim_end_matches('/'));
    }
    format!("{}/{}", to_dir.trim_end_matches('/'), name)
}

/// True when a move can take the SFTP-rename fast path: source and
/// destination directory are the same.
pub fn same_directory(from: &str, to_dir: &str) -> bool {
    let parent = from
        .trim_end_matches('/')
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let parent = if parent.is_empty() { "/" } else { parent };
    let dir = to_dir.trim_end_matches('/');
    let dir = if dir.is_empty() { "/" } else { dir };
    parent == dir
}

// ---------------------------------------------------------------------------
// Command builders (pure)
// ---------------------------------------------------------------------------

/// `cp -a -- <src> <toDir/name>` — archive mode keeps permissions, timestamps
/// and copies directories recursively.
pub fn build_copy_command(source: &str, target: &str) -> String {
    format!(
        "cp -a -- {} {}",
        exec::shell_quote(source),
        exec::shell_quote(target)
    )
}

/// `mv -f -- <src> <toDir/name>`.
pub fn build_move_command(source: &str, target: &str) -> String {
    format!(
        "mv -f -- {} {}",
        exec::shell_quote(source),
        exec::shell_quote(target)
    )
}

/// One-shot existence probe for every target: echoes the index of each path
/// that already exists; the trailing `true` keeps the overall exit status 0.
pub fn build_exists_probe_command(targets: &[String]) -> String {
    let mut command = String::new();
    for (index, target) in targets.iter().enumerate() {
        if index > 0 {
            command.push_str("; ");
        }
        command.push_str(&format!(
            "test -e {} && echo {}",
            exec::shell_quote(target),
            index
        ));
    }
    command.push_str("; true");
    command
}

/// Reduces probe output to a per-index "already exists" table.
pub fn existing_targets(output: &str, count: usize) -> Vec<bool> {
    let mut existing = vec![false; count];
    for line in output.lines() {
        if let Ok(index) = line.trim().parse::<usize>() {
            if index < count {
                existing[index] = true;
            }
        }
    }
    existing
}
