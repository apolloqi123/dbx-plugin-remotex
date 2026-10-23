//! Plugin data-directory resolution and writability checks.

use super::*;
use crate::core::plugin_ids::PLUGIN_ID;

/// Pure resolver for the plugin data directory so the fallback order is unit
/// testable without mutating process environment state (`lookup` abstracts
/// `std::env::var_os`). An env var counts as set only when present and
/// non-blank after trimming. Fallback order, first available wins:
///
/// 1. `DBX_PLUGIN_DATA_DIR` — host-injected explicit override (future
///    integration point).
/// 2. `DBX_DATA_DIR` → `<DBX_DATA_DIR>/plugin-data/io.dbx.sshx` (portable/web
///    host mode; `plugin-data/` avoids the installer-managed registration
///    tree).
/// 3. Platform standard user data dir: macOS `$HOME/Library/Application
///    Support`, other unix `${XDG_DATA_HOME:-$HOME/.local/share}`, Windows
///    `%APPDATA%`.
/// 4. `std::env::temp_dir()` — last resort so this function never fails.
pub(super) fn resolve_plugin_data_dir(lookup: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    let env = |key: &str| lookup(key).filter(|value| !value.to_string_lossy().trim().is_empty());
    if let Some(dir) = env("DBX_PLUGIN_DATA_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dbx_data_dir) = env("DBX_DATA_DIR") {
        return PathBuf::from(dbx_data_dir)
            .join("plugin-data")
            .join(PLUGIN_ID);
    }
    let platform_base = if cfg!(windows) {
        env("APPDATA").map(PathBuf::from)
    } else {
        env("HOME").map(|home| {
            let home = PathBuf::from(home);
            if cfg!(target_os = "macos") {
                home.join("Library").join("Application Support")
            } else {
                env("XDG_DATA_HOME")
                    .map_or_else(|| home.join(".local").join("share"), PathBuf::from)
            }
        })
    };
    platform_base
        .map(|base| base.join("dbx-plugin-data").join(PLUGIN_ID))
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("dbx-plugin-data")
                .join(PLUGIN_ID)
        })
}

/// RDP is experimental and must be explicitly enabled in persisted preferences;
/// keep this backend gate independent of UI visibility so direct RPC cannot
/// bypass the release posture.
pub(super) fn rdp_start_gate(data_dir: &std::path::Path) -> Result<(), String> {
    if crate::core::preferences::rdp_experimental_enabled(data_dir) {
        Ok(())
    } else {
        Err("rdp/start is disabled until experimental RDP is explicitly enabled".to_string())
    }
}

pub(crate) fn plugin_data_dir() -> PathBuf {
    // Closure (not the generic `var_os` fn item) so the HRTB bound unifies.
    let requested = resolve_plugin_data_dir(|key| std::env::var_os(key));
    // Docker deployments may provide a read-only or not-yet-mounted DBX data
    // directory. Keep startup viable by falling back to the OS temp directory;
    // the warning makes the loss of persistence explicit to the host logs.
    match prepare_plugin_data_dir(&requested) {
        Ok(data_dir) => data_dir,
        Err(request_error) => {
            let fallback = std::env::temp_dir()
                .join("dbx-plugin-data")
                .join(PLUGIN_ID);
            match prepare_plugin_data_dir(&fallback) {
                Ok(data_dir) => {
                    eprintln!(
                        "[dbx-plugin-sshx] data directory {} is unavailable: {request_error}; using temporary directory {}",
                        requested.display(),
                        data_dir.display()
                    );
                    data_dir
                }
                Err(fallback_error) => {
                    eprintln!(
                        "[dbx-plugin-sshx] failed to prepare data directories {} ({request_error}) and {} ({fallback_error})",
                        requested.display(),
                        fallback.display()
                    );
                    requested
                }
            }
        }
    }
}

fn prepare_plugin_data_dir(data_dir: &std::path::Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|error| format!("{}: {error}", data_dir.display()))?;
    let probe = data_dir.join(format!(".dbx-plugin-write-test-{}", uuid::Uuid::new_v4()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|error| format!("{} is not writable: {error}", data_dir.display()))?;
    std::fs::remove_file(&probe)
        .map_err(|error| format!("failed to remove write probe {}: {error}", probe.display()))?;
    // The env vars are the plugin's only path inputs; resolve symlinks and `..`
    // once at the boundary so every store path below it is canonical.
    std::fs::canonicalize(data_dir).map_err(|error| format!("{}: {error}", data_dir.display()))
}
