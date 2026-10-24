use super::*;

mod macos;
mod posix;
mod windows;

/// One `push` sink handed to a platform table; see [`build_catalog`].
pub(super) type PushEntry<'a> =
    &'a mut dyn FnMut(&'static str, &'static str, LaunchSpec, bool, &'static [&'static str]);

/// Builds the well-known editor catalog for `platform`. Pure over the
/// injected probe/lister, so every branch is unit-testable off-platform.
pub fn build_catalog(
    platform: &str,
    env: &EnvPaths,
    probe: &dyn Fn(&Path) -> bool,
    lister: &dyn Fn(&Path) -> Vec<PathBuf>,
) -> Vec<CatalogEntry> {
    let mut entries: Vec<CatalogEntry> = Vec::new();
    let mut push = |id: &'static str,
                    name: &'static str,
                    launch: LaunchSpec,
                    available: bool,
                    exts: &'static [&'static str]| {
        entries.push(CatalogEntry {
            id,
            name,
            launch,
            available,
            suggested_extensions: exts,
        });
    };

    match platform {
        "macos" => macos::extend(&mut push, env, probe),
        "windows" => windows::extend(&mut push, env, probe, lister),
        _ => posix::extend(&mut push, env, probe),
    }
    entries
}
