//! Known external editors and "open with a specific app" support.
//!
//! `local/editors/list` returns a per-platform catalog of well-known editors
//! (VS Code, Sublime, WPS, MS Office, …) probed from the filesystem/PATH —
//! never by spawning processes. `local/open-with` launches a catalog entry or
//! a user-defined command line for a path that this plugin previously
//! downloaded (the same transfer-history allowlist as `local/open`), so it can
//! never become an arbitrary "execute anything" primitive.
//!
//! Launch always goes through an argument list (no shell), and user-provided
//! command strings are tokenized POSIX-shlex style with a Windows-friendly
//! twist: backslash is literal unless it escapes a quote or another backslash,
//! so `"C:\Program Files\App\app.exe" {file}` keeps its separators.

use crate::filesystem::local::downloads as local_downloads;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// One well-known editor: how to launch it, plus whether the probe found it.
pub struct CatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub launch: LaunchSpec,
    pub available: bool,
    pub suggested_extensions: &'static [&'static str],
}

/// How an editor is started. `openApp` uses macOS `open -a`; `exe` spawns the
/// binary with an argument template where `{file}` is replaced by the
/// downloaded path (missing placeholder: path appended).
#[derive(Clone, Debug, PartialEq)]
pub enum LaunchSpec {
    OpenApp { app_id: String },
    Exe { exe: String, args: Vec<String> },
}

impl LaunchSpec {
    fn to_json(&self) -> Value {
        match self {
            LaunchSpec::OpenApp { app_id } => json!({ "kind": "openApp", "appId": app_id }),
            LaunchSpec::Exe { exe, args } => json!({ "kind": "exe", "exe": exe, "args": args }),
        }
    }
}

fn open_app(app_id: &str) -> LaunchSpec {
    LaunchSpec::OpenApp {
        app_id: app_id.to_string(),
    }
}

fn exe_spec(exe: Option<&Path>) -> LaunchSpec {
    LaunchSpec::Exe {
        exe: exe
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        args: vec!["{file}".to_string()],
    }
}

/// Everything the catalog probe needs from the machine, injected so tests can
/// fake installs without touching the real filesystem.
pub struct EnvPaths {
    pub local_appdata: Option<PathBuf>,
    pub program_files: Option<PathBuf>,
    pub program_files_x86: Option<PathBuf>,
    pub windows_dir: Option<PathBuf>,
    pub path_dirs: Vec<PathBuf>,
    /// macOS application folders, most specific last (`/Applications`, `~/Applications`).
    pub applications_dirs: Vec<PathBuf>,
}

/// Reads the real machine's environment into `EnvPaths`.
pub fn detect_env_paths() -> EnvPaths {
    let var = |key: &str| std::env::var_os(key).map(PathBuf::from);
    let home_dir = if cfg!(windows) {
        var("USERPROFILE")
    } else {
        var("HOME")
    };
    let mut path_dirs = Vec::new();
    if let Some(raw) = std::env::var_os("PATH") {
        let separator = if cfg!(windows) { ';' } else { ':' };
        for part in raw.to_string_lossy().split(separator) {
            if !part.trim().is_empty() {
                path_dirs.push(PathBuf::from(part));
            }
        }
    }
    let mut applications_dirs = vec![PathBuf::from("/Applications")];
    if let Some(home_dir) = &home_dir {
        applications_dirs.push(home_dir.join("Applications"));
    }
    EnvPaths {
        local_appdata: var("LOCALAPPDATA"),
        program_files: var("ProgramFiles"),
        program_files_x86: var("ProgramFiles(x86)"),
        windows_dir: var("SystemRoot").or_else(|| var("WINDIR")),
        path_dirs,
        applications_dirs,
    }
}

fn probe_app(env: &EnvPaths, probe: &dyn Fn(&Path) -> bool, app_dir_name: &str) -> bool {
    env.applications_dirs
        .iter()
        .any(|dir| probe(&dir.join(format!("{app_dir_name}.app"))))
}

fn probe_on_path(name: &str, env: &EnvPaths, probe: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    env.path_dirs
        .iter()
        .map(|dir| dir.join(name))
        .find(|candidate| probe(candidate))
}

/// Windows installers that nest a version directory (WPS Office lives in
/// `<root>\Kingsoft\WPS Office\<version>\office6\`): iterate the direct
/// children of `base` and return the first `child/tail` that exists.
fn probe_versioned(
    base: &Path,
    tail: &[&str],
    probe: &dyn Fn(&Path) -> bool,
    lister: &dyn Fn(&Path) -> Vec<PathBuf>,
) -> Option<PathBuf> {
    lister(base).into_iter().find_map(|child| {
        let candidate = tail.iter().fold(child, |acc, part| acc.join(part));
        probe(&candidate).then_some(candidate)
    })
}

fn first_existing(
    candidates: &[Option<PathBuf>],
    probe: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    candidates
        .iter()
        .flatten()
        .find(|candidate| probe(candidate))
        .cloned()
}

fn text_extensions() -> &'static [&'static str] {
    &[
        ".txt", ".md", ".log", ".json", ".xml", ".yaml", ".yml", ".ini", ".conf", ".toml", ".sh",
        ".py", ".js", ".ts", ".sql", ".env",
    ]
}

const OFFICE_EXTS: &[&str] = &[".odt", ".ods", ".odp", ".doc", ".docx", ".xls", ".xlsx"];

mod catalog;
mod launch;

#[cfg(test)]
mod tests;

pub use catalog::build_catalog;
pub use launch::*;
