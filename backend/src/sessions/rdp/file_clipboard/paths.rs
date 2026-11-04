//! 文件剪贴板的名字/相对路径校验与临时目录磁盘助手（纯函数为主，单测覆盖）。

use super::{MAX_CLIPBOARD_WIRE_NAME_CHARS, TEMP_PREFIX};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

fn wire_name_len(relative_path: Option<&str>, name: &str) -> usize {
    // The wire field is UTF-16 (520 bytes); count code units so astral
    // characters cannot overflow the engine-side encode that would fail the
    // whole session.
    let units = |text: &str| text.encode_utf16().count();
    match relative_path {
        Some(path) if !path.is_empty() => units(path) + 1 + units(name),
        _ => units(name),
    }
}

fn validate_component(component: &str) -> Result<(), String> {
    if component.is_empty() {
        return Err("clipboard file name is empty".to_string());
    }
    if component == "." || component == ".." {
        return Err("clipboard file path contains a traversal component".to_string());
    }
    if component
        .chars()
        .any(|character| character.is_control() || matches!(character, '/' | '\\' | ':'))
    {
        return Err("clipboard file name contains an invalid character".to_string());
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return Err("clipboard file name ends with a dot or space".to_string());
    }
    if ironrdp::cliprdr::is_windows_device_name(component) {
        return Err("clipboard file name is a reserved Windows device name".to_string());
    }
    Ok(())
}

/// Validates `name` + optional `relativePath`; returns the normalized relative
/// path (backslash separated, `None` at the root) plus the basename.
///
/// Some Linux clipboard bridges (GNOME Remote Desktop through WinPR) put the
/// whole selection-relative path into `cFileName` with `/` separators, so
/// ironrdp decodes it as one flat name. Split it into components here before
/// validation so folders from those servers materialize on download.
pub(crate) fn validate_file_path(
    name: &str,
    relative_path: Option<&str>,
) -> Result<(Option<String>, String), String> {
    let mut components: Vec<&str> = Vec::new();
    if let Some(path) = relative_path.filter(|path| !path.is_empty()) {
        if path.starts_with('/') || path.starts_with('\\') {
            return Err("clipboard file path must be relative".to_string());
        }
        for component in path.split(['/', '\\']) {
            validate_component(component)?;
            components.push(component);
        }
    }

    let mut name_parts = name.split(['/', '\\']);
    let first = name_parts.next().unwrap_or_default();
    let rest: Vec<&str> = name_parts.collect();
    let basename = if rest.is_empty() {
        first
    } else {
        if first.is_empty() {
            return Err("clipboard file path must be relative".to_string());
        }
        validate_component(first)?;
        components.push(first);
        for component in &rest[..rest.len() - 1] {
            validate_component(component)?;
            components.push(component);
        }
        rest[rest.len() - 1]
    };
    validate_component(basename)?;

    let normalized = if components.is_empty() {
        None
    } else {
        Some(components.join("\\"))
    };
    if wire_name_len(normalized.as_deref(), basename) > MAX_CLIPBOARD_WIRE_NAME_CHARS {
        return Err(format!(
            "clipboard file name exceeds the {MAX_CLIPBOARD_WIRE_NAME_CHARS} character limit"
        ));
    }
    Ok((normalized, basename.to_string()))
}

pub(super) fn path_components(relative_path: Option<&str>) -> Vec<&str> {
    match relative_path.filter(|path| !path.is_empty()) {
        Some(path) => path.split('\\').collect(),
        None => Vec::new(),
    }
}

pub(super) fn temp_root() -> PathBuf {
    std::env::temp_dir().join(TEMP_PREFIX)
}

pub(super) fn remove_dir(path: &Path) {
    // 只删除本模块创建的 UUID 目录；失败仅留下系统临时目录垃圾。
    let _ = fs::remove_dir_all(path);
}

/// Deleting a large clipboard temp tree can take a noticeable amount of time;
/// keep that off the CLIPRDR pump thread.
pub(super) fn remove_dir_async(path: PathBuf) {
    std::thread::spawn(move || remove_dir(&path));
}

/// Quarantines the old path before deleting so an immediate retry can recreate
/// the original directory without racing the cleanup thread.
pub(super) fn remove_dir_async_quarantined(path: PathBuf) {
    let failed = path.with_file_name(format!(
        "{}-failed-{}",
        path.file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default(),
        uuid::Uuid::new_v4()
    ));
    match std::fs::rename(&path, &failed) {
        Ok(()) => remove_dir_async(failed),
        Err(_) => remove_dir_async(path),
    }
}

/// Pure path join for metadata validation; no filesystem access or symlink
/// checks (those happen when the user actually starts the download).
pub(super) fn join_relative(root: &Path, relative_path: Option<&str>, name: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for component in path_components(relative_path) {
        path.push(component);
    }
    path.push(name);
    path
}

/// Creates a clipboard temp directory readable only by the current user.
/// On Unix this must be applied at creation, not afterwards: the default
/// 0777&umask (usually 0755) would expose clipboard files to other users.
pub(super) fn create_private_dir(path: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder
        .create(path)
        .map_err(|_| "unable to create the clipboard temp directory".to_string())
}

/// Creates (or truncates) a file that only the current user can read/write.
pub(super) fn create_private_file(path: &Path) -> Result<File, String> {
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(path)
        .map_err(|_| "unable to create the clipboard temp file".to_string())
}

/// Opens an existing clipboard file for writing without changing its length.
pub(super) fn open_private_file(path: &Path) -> Result<File, String> {
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(path)
        .map_err(|_| "unable to open the clipboard temp file".to_string())
}

/// Creates a directory chain under `root`, refusing any pre-existing symlink
/// component (`never follow symlinks`).
pub(super) fn ensure_dir(root: &Path, components: &[&str]) -> Result<PathBuf, String> {
    let mut current = root.to_path_buf();
    for component in components {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err("clipboard temp path is not a regular directory".to_string());
                }
            }
            Err(_) => {
                fs::create_dir(&current)
                    .map_err(|_| "unable to create the clipboard temp directory".to_string())?;
            }
        }
    }
    Ok(current)
}

pub(super) fn local_path(
    root: &Path,
    relative_path: Option<&str>,
    name: &str,
) -> Result<PathBuf, String> {
    let dir = ensure_dir(root, &path_components(relative_path))?;
    let path = dir.join(name);
    reject_symlink(&path)?;
    Ok(path)
}

/// Refuses to read/write through an existing symlink (`never follow symlinks`).
pub(super) fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("clipboard temp path is a symlink".to_string())
        }
        _ => Ok(()),
    }
}
