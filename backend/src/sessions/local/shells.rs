//! Local terminal shells support.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOS,
    Linux,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
    PowerShell,
    Cmd,
    Other,
}

pub(super) struct ShellSpec {
    pub(super) program: String,
    pub(super) kind: ShellKind,
}

pub(super) fn current_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::MacOS
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        Platform::Linux
    }
}

/// Basename without directory separators and a trailing `.exe`.
pub(super) fn shell_basename(program: &str) -> String {
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

pub(super) fn shell_kind_from_program(program: &str) -> ShellKind {
    match shell_basename(program).as_str() {
        "zsh" => ShellKind::Zsh,
        "bash" => ShellKind::Bash,
        "fish" => ShellKind::Fish,
        "pwsh" | "powershell" => ShellKind::PowerShell,
        "cmd" => ShellKind::Cmd,
        _ => ShellKind::Other,
    }
}

/// Shells that would terminate immediately (service accounts); treated as
/// "unset" so detection falls through to the platform default.
pub(super) fn is_login_unusable(program: &str) -> bool {
    matches!(shell_basename(program).as_str(), "nologin" | "false")
}

pub(super) fn base_args(kind: ShellKind, platform: Platform) -> Vec<String> {
    if platform == Platform::Windows || kind == ShellKind::Cmd {
        Vec::new()
    } else {
        // macOS GUI apps ship with a skeletal PATH; Linux daemons too. Every
        // Unix shell therefore starts as a login shell to pick up
        // /etc/profile & friends (Ghostty's macOS login-shell rule).
        vec!["-l".to_string()]
    }
}

/// Resolve which program to launch. `requested` wins; on Unix the user's
/// login shell (macOS Directory Services first, `$SHELL` second) and on
/// Windows PowerShell — Warp's default-shell matrix.
pub(super) fn pick_shell(
    requested: Option<&str>,
    shell_env: Option<&str>,
    ds_shell: Option<&str>,
    platform: Platform,
) -> ShellSpec {
    let (program, kind) = if let Some(value) = requested {
        (value.to_string(), shell_kind_from_program(value))
    } else {
        match platform {
            Platform::Windows => ("powershell.exe".to_string(), ShellKind::PowerShell),
            _ => {
                let detected = [ds_shell, shell_env]
                    .into_iter()
                    .flatten()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .find(|value| !is_login_unusable(value));
                match detected {
                    Some(value) => (value.to_string(), shell_kind_from_program(value)),
                    None => match platform {
                        Platform::MacOS => ("/bin/zsh".to_string(), ShellKind::Zsh),
                        _ => ("/bin/bash".to_string(), ShellKind::Bash),
                    },
                }
            }
        }
    };
    ShellSpec { program, kind }
}

/// One row of `local/shells/list`: a launchable shell with display metadata.
#[derive(Debug, Clone)]
pub struct ShellEntry {
    /// Program path/name to spawn (also the stored preference value).
    pub program: String,
    /// Basename for display ("zsh", "PowerShell", …).
    pub name: String,
    /// The platform default this runtime would pick with no explicit choice.
    pub is_default: bool,
    /// The user's login shell (dscl / $SHELL / USERPROFILE-adjacent).
    pub is_user_shell: bool,
    /// Whether shell integration injection applies to this shell kind
    /// (ksh/csh/cmd-style shells open bare, the toggle is inert for them).
    pub injectable: bool,
}

/// Pure core of shell discovery: merge candidate program paths in priority
/// order (user login shell first, then /etc/shells, then platform defaults),
/// deduplicating case-insensitively on the basename and dropping login-unusable
/// entries (`nologin`/`false` are real files in `/etc/shells` but would exit
/// immediately). `exists` is injected so
/// tests can simulate the filesystem.
pub(super) fn merge_shell_candidates(
    user_shell: Option<&str>,
    etc_shells: &[String],
    fallbacks: &[&str],
    exists: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut ordered: Vec<String> = Vec::new();
    let mut push = |program: &str| {
        let program = program.trim();
        if program.is_empty() || !exists(program) || is_login_unusable(program) {
            return;
        }
        let key = shell_basename(program);
        if seen.insert(key) {
            ordered.push(program.to_string());
        }
    };
    if let Some(value) = user_shell.map(str::trim).filter(|v| !v.is_empty()) {
        push(value);
    }
    for line in etc_shells {
        let candidate = line.trim();
        if candidate.is_empty() || candidate.starts_with('#') {
            continue;
        }
        push(candidate);
    }
    for candidate in fallbacks {
        push(candidate);
    }
    ordered
}

/// `/etc/shells` content (Unix). Empty on Windows or when unreadable.
pub(super) fn etc_shells() -> Vec<String> {
    if cfg!(windows) {
        return Vec::new();
    }
    std::fs::read_to_string("/etc/shells")
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Windows candidate lookup over PATH-style directories (pure in `dirs`).
pub(super) fn find_in_dirs(program: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}

/// Windows shell candidates, best first: PowerShell 7 if installed, the
/// always-present Windows PowerShell, cmd, and WSL for Unix tooling.
pub(super) fn windows_shell_candidates() -> Vec<String> {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    let mut dirs = dirs;
    if let Some(windir) = std::env::var_os("SystemRoot").map(PathBuf::from) {
        let system32 = windir.join("System32");
        dirs.push(system32.clone());
        dirs.push(system32.join("WindowsPowerShell").join("v1.0"));
    }
    ["pwsh.exe", "powershell.exe", "cmd.exe", "wsl.exe"]
        .iter()
        .filter_map(|name| find_in_dirs(name, &dirs))
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

/// Discover launchable shells for `local/shells/list`: the user's login shell
/// first, then `/etc/shells` (Unix), then platform defaults — each row marked
/// with the default/user-shell roles the picker UI renders.
pub fn discover_shells(platform: Platform) -> Vec<ShellEntry> {
    let ds_shell = directory_services_shell();
    let shell_env = std::env::var_os("SHELL").map(|value| value.to_string_lossy().into_owned());
    let user_shell = ds_shell.as_deref().or(shell_env.as_deref());
    let fallbacks: Vec<String> = match platform {
        Platform::Windows => windows_shell_candidates(),
        _ => {
            let default = match platform {
                Platform::MacOS => "/bin/zsh",
                _ => "/bin/bash",
            };
            vec![default.to_string()]
        }
    };
    let exists = |program: &str| Path::new(program).is_file();
    let ordered = merge_shell_candidates(
        user_shell,
        &etc_shells(),
        &fallbacks.iter().map(String::as_str).collect::<Vec<_>>(),
        &exists,
    );
    let default_program = pick_shell(None, None, None, platform).program;
    ordered
        .into_iter()
        .map(|program| {
            let name = shell_basename(&program);
            let kind = shell_kind_from_program(&program);
            ShellEntry {
                name: shell_display_name(&name),
                is_default: name == shell_basename(&default_program),
                is_user_shell: user_shell
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| shell_basename(value) == name)
                    .unwrap_or(false),
                injectable: injection_plan(kind, Path::new("/dev/null"), None).is_some(),
                program,
            }
        })
        .collect()
}

/// Friendlier picker label: basename without the .exe already applied.
pub(super) fn shell_display_name(basename: &str) -> String {
    match basename {
        "powershell" => "Windows PowerShell".to_string(),
        "pwsh" => "PowerShell".to_string(),
        "wsl" => "WSL".to_string(),
        other => {
            let mut name = other.to_string();
            if let Some(first) = name.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            name
        }
    }
}

/// Copy for the launch-option group, keyed by the host UI locale (the host
/// sends it as the `locale` param on `local/terminal/launch-options`). The
/// table mirrors the workbench locale set (frontend/src/shared/lib/i18n/); unknown
/// or missing locales fall back to English. Returns (default entry label,
/// group label, description prefix) — the caller appends the resolved default
/// shell program. Discovered shells carry the `group` label so the host
/// renders them as one collapsible "RemoteX" section; the default
/// entry stays top-level.
pub(super) fn launch_option_copy(locale: &str) -> (&'static str, &'static str, &'static str) {
    match locale {
        "zh-CN" | "zh" => ("RemoteX", "RemoteX", "默认 Shell："),
        "zh-TW" => ("RemoteX", "RemoteX", "預設 Shell："),
        "ja" => ("RemoteX", "RemoteX", "デフォルトシェル: "),
        "es" => ("RemoteX", "RemoteX", "Shell predeterminado: "),
        "it" => ("RemoteX", "RemoteX", "Shell predefinita: "),
        "pt-BR" | "pt" => ("RemoteX", "RemoteX", "Shell padrão: "),
        _ => ("RemoteX", "RemoteX", "Default shell: "),
    }
}

/// The shell a plain "local terminal" opens with: the configured `localShell`
/// preference when set, otherwise the same auto-detect chain as
/// `local/terminal/start` (Directory Services → `$SHELL` → platform default).
/// Surfaced in the dock picker description; the default itself is set from the
/// workbench settings dialog (also the shell picker's `localShell` key).
pub fn resolve_default_shell(data_dir: &Path) -> String {
    resolve_configured_shell(data_dir).unwrap_or_else(|| {
        let shell_env = std::env::var("SHELL").ok();
        pick_shell(
            None,
            shell_env.as_deref(),
            directory_services_shell().as_deref(),
            current_platform(),
        )
        .program
    })
}

/// The explicitly configured `localShell` preference, or `None` when unset —
/// the dock picker collapses its discovery list to the single configured
/// shell exactly when this returns `Some`.
pub fn resolve_configured_shell(data_dir: &Path) -> Option<String> {
    crate::core::preferences::load_preferences(data_dir)
        .get("localShell")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// macOS only: the user's real login shell from Directory Services, which is
/// authoritative when `$SHELL` is missing inside a GUI-launched process.
pub(super) fn directory_services_shell() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let user = std::env::var_os("USER")?;
    let output = std::process::Command::new("dscl")
        .args([
            ".",
            "-read",
            &format!("/Users/{}", user.to_string_lossy()),
            "UserShell",
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("UserShell:").map(str::trim_start))
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

pub(super) fn home_dir() -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}
