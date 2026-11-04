//! Host system file clipboard for the native RDP file clipboard (CLIPRDR).
//!
//! The DBX host bridge only exposes text/image clipboard APIs (checked against
//! the 0.6.62 host), so the sidecar reads and writes the OS file clipboard
//! itself:
//! - **macOS**: `NSPasteboard` file URLs (Finder copy/paste).
//! - **Windows**: `CF_HDROP` (Explorer copy/paste), plus `Preferred DropEffect`.
//! - **Linux**: the standard clipboard helpers through `text/uri-list` /
//!   `x-special/gnome-copied-files` (`xclip` on X11, `wl-clipboard` on Wayland).
//!
//! Contract shared by all backends:
//! - `change_count()` is cheap and must never block the RDP engine loop.
//! - `file_urls()` is called only after a change and returns absolute paths;
//!   file contents are never read here (the server requests them on paste).
//! - `write_file_urls()` only receives plugin-downloaded files and must be a
//!   no-op in unit tests so a test run never replaces the developer's clipboard.
//! - Paths are never logged.

mod uri_list;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::{change_count, file_urls, has_file_urls, supported, write_file_urls};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{change_count, file_urls, has_file_urls, supported, write_file_urls};

// Compiled on every host so its pure helpers stay tested, but only selected on
// Linux (the RDP worker never calls it elsewhere).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::{change_count, file_urls, has_file_urls, supported, write_file_urls};

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod unsupported {
    use std::path::PathBuf;

    pub(crate) fn supported() -> bool {
        false
    }

    pub(crate) fn change_count() -> i64 {
        0
    }

    pub(crate) fn has_file_urls() -> bool {
        false
    }

    pub(crate) fn file_urls() -> Vec<PathBuf> {
        Vec::new()
    }

    pub(crate) fn write_file_urls(_paths: &[PathBuf]) -> Result<(), String> {
        Err("The native file clipboard is not implemented on this platform".to_string())
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub(crate) use unsupported::{change_count, file_urls, has_file_urls, supported, write_file_urls};
