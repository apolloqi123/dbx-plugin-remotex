//! Import source specs and the picker entry points that stage an import.

use super::*;

/// One "Import connections" source type. `kind` matches the
/// `connection_import` sniff vocabulary so the parsed file kind and the
/// user-chosen type share one vocabulary; `extensions`/`macos_types` drive the
/// file-picker filter per platform (custom extensions have no macOS UTI, so
/// they stay unfiltered there and rely on the explicit type step).
pub struct ImportSourceSpec {
    pub kind: &'static str,
    pub label: &'static str,
    pub extensions: &'static [&'static str],
    pub macos_types: &'static [&'static str],
    /// HOME-relative directory the file picker starts in (`.ssh` for the
    /// OpenSSH config); `None` starts at the home directory itself.
    pub default_subdir: Option<&'static str>,
}

/// Same sources (and order) as the workbench import wizard's step 1.
pub const IMPORT_SOURCES: &[ImportSourceSpec] = &[
    ImportSourceSpec {
        kind: "moba",
        label: "MobaXterm (.mxtsessions)",
        extensions: &["mxtsessions"],
        macos_types: &[],
        default_subdir: None,
    },
    ImportSourceSpec {
        kind: "xshell",
        label: "Xshell (.xts)",
        extensions: &["xts"],
        macos_types: &[],
        default_subdir: None,
    },
    ImportSourceSpec {
        kind: "windterm",
        label: "WindTerm (.sessions)",
        extensions: &["sessions"],
        macos_types: &[],
        default_subdir: None,
    },
    ImportSourceSpec {
        kind: "securecrt",
        label: "SecureCRT (.xml)",
        extensions: &["xml"],
        macos_types: &["public.xml"],
        default_subdir: None,
    },
    ImportSourceSpec {
        kind: "finalshell",
        label: "FinalShell (.zip)",
        extensions: &["zip"],
        macos_types: &["public.zip-archive"],
        default_subdir: None,
    },
    ImportSourceSpec {
        kind: "electerm",
        label: "Electerm (.json)",
        extensions: &["json"],
        macos_types: &["public.json"],
        default_subdir: None,
    },
    ImportSourceSpec {
        kind: "termius",
        label: "Termius (.json)",
        extensions: &["json"],
        macos_types: &["public.json"],
        default_subdir: None,
    },
    ImportSourceSpec {
        kind: "sshconfig",
        label: "OpenSSH config (~/.ssh)",
        extensions: &[],
        macos_types: &[],
        default_subdir: Some(".ssh"),
    },
];

/// Looks up a source spec by its `kind` (the `source` parameter the smoke
/// tests inject to skip the native dialog).
pub fn import_source_by_kind(kind: &str) -> Option<&'static ImportSourceSpec> {
    IMPORT_SOURCES.iter().find(|source| source.kind == kind)
}

/// Native "choose the source type" dialog for the connection form's
/// "Import connections" action. Returns the index into [`IMPORT_SOURCES`];
/// `None` when the user cancels.
pub fn pick_import_source() -> Result<Option<usize>, String> {
    let _flight = DialogFlightGuard::acquire()?;
    let labels: Vec<&str> = IMPORT_SOURCES.iter().map(|s| s.label).collect();
    pick_source_dialog(&labels).map(|picked| {
        picked.and_then(|label| labels.iter().position(|candidate| *candidate == label))
    })
}

/// Opens the per-source filtered file picker after the user picked an import
/// source; `None` when the user cancels.
pub fn pick_import_file(source: &ImportSourceSpec) -> Result<Option<String>, String> {
    let _flight = DialogFlightGuard::acquire()?;
    let default_dir = source.default_subdir.and_then(home_subdir);
    trim_picked(pick_file_dialog(
        default_dir.as_deref(),
        Some(&FilePickFilter::from_source(source)),
    ))
}

/// Pre-download conflict probe for the "ask me" policy: reports whether
/// `<dir>/<sanitized name>` already exists, plus the exact candidate path so
/// the prompt can show it verbatim.
pub fn target_exists(dir: &str, name: &str) -> Result<Value, String> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Err("Directory is required".to_string());
    }
    let dir_path = PathBuf::from(dir);
    if !dir_path.is_absolute() {
        return Err("Directory must be an absolute path".to_string());
    }
    let candidate = dir_path.join(crate::filesystem::local::downloads::sanitize_file_name(
        name,
    ));
    Ok(json!({
        "exists": candidate.exists(),
        "path": candidate.to_string_lossy(),
    }))
}
