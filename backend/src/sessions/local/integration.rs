//! Local terminal integration support.

use super::*;

pub(super) const INTEGRATION_ZSH: &str = include_str!("shell_integration/integration.zsh");

pub(super) const INTEGRATION_BASH: &str = include_str!("shell_integration/integration.bash");

pub(super) const INTEGRATION_FISH: &str = include_str!("shell_integration/integration.fish");

pub(super) const INTEGRATION_PWSH: &str = include_str!("shell_integration/integration.ps1");

pub(super) const WRAPPER_ZSHRC: &str = include_str!("shell_integration/wrapper.zshrc");

pub(super) const WRAPPER_ZPROFILE: &str = include_str!("shell_integration/wrapper.zprofile");

pub(super) const WRAPPER_ZLOGIN: &str = include_str!("shell_integration/wrapper.zlogin");

pub(super) const WRAPPER_BASH: &str = include_str!("shell_integration/wrapper.bash");

pub(super) fn integration_dir() -> PathBuf {
    std::env::temp_dir().join("dbx-plugin-sshx-shell-integration")
}

pub(super) fn render_template(template: &str, integration_dir: &str) -> String {
    template.replace("{{INTEGRATION_DIR}}", integration_dir)
}

pub(super) fn escape_posix_single_quoted(text: &str) -> String {
    text.replace('\'', "'\\''")
}

pub(super) fn escape_powershell_single_quoted(text: &str) -> String {
    text.replace('\'', "''")
}

/// Materialize the integration + wrapper scripts for this process. Content is
/// static and secret-free, so a predictable temp location is fine; on any IO
/// failure injection is silently skipped (the terminal still opens raw).
pub(super) fn write_integration_files(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let dir_text = dir.to_string_lossy().into_owned();
    // zsh resolves `$ZDOTDIR/.zshrc` etc. — the wrapper files MUST carry the
    // dotted names; the bash wrapper is free-form (`--rcfile wrapper.bash`).
    let files: [(&str, String); 8] = [
        ("integration.zsh", INTEGRATION_ZSH.to_string()),
        ("integration.bash", INTEGRATION_BASH.to_string()),
        ("integration.fish", INTEGRATION_FISH.to_string()),
        ("integration.ps1", INTEGRATION_PWSH.to_string()),
        (".zshrc", render_template(WRAPPER_ZSHRC, &dir_text)),
        (".zprofile", render_template(WRAPPER_ZPROFILE, &dir_text)),
        (".zlogin", render_template(WRAPPER_ZLOGIN, &dir_text)),
        ("wrapper.bash", render_template(WRAPPER_BASH, &dir_text)),
    ];
    for (name, content) in files {
        std::fs::write(dir.join(name), content)?;
    }
    Ok(())
}

/// Launch args + env additions for injection, or `None` when the shell has no
/// integration (cmd, unknown) or injection is off. Args REPLACE the base
/// login args: bash cannot combine `-l` with `--rcfile`, so the bash wrapper
/// reproduces the login chain itself (see wrapper.bash).
/// Launch args plus `(name, value)` env additions for injection.
pub(super) type InjectionPlan = (Vec<String>, Vec<(String, String)>);

pub(super) fn injection_plan(
    kind: ShellKind,
    dir: &Path,
    user_zdotdir: Option<&str>,
) -> Option<InjectionPlan> {
    let dir_text = dir.to_string_lossy().into_owned();
    match kind {
        ShellKind::Zsh => {
            let mut env = vec![("ZDOTDIR".to_string(), dir_text)];
            if let Some(value) = user_zdotdir
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                env.push(("DBX_USER_ZDOTDIR".to_string(), value.to_string()));
            }
            Some((vec!["-l".to_string()], env))
        }
        ShellKind::Bash => Some((
            vec![
                "--rcfile".to_string(),
                dir.join("wrapper.bash").to_string_lossy().into_owned(),
            ],
            Vec::new(),
        )),
        ShellKind::Fish => {
            let script = dir.join("integration.fish");
            Some((
                vec![
                    "-l".to_string(),
                    "-C".to_string(),
                    format!(
                        "source '{}'",
                        escape_posix_single_quoted(&script.to_string_lossy())
                    ),
                ],
                Vec::new(),
            ))
        }
        ShellKind::PowerShell => {
            let script = dir.join("integration.ps1");
            // -ExecutionPolicy Bypass is process-scoped: it lets the temp-dir
            // script run on machines whose machine/user policy is Restricted
            // (Windows client default) without touching their policy settings.
            // Group-policy-enforced MachinePolicy/UserPolicy still win, in
            // which case the dot-source fails the same way as before and the
            // shell keeps its built-in prompt (fail-safe contract).
            Some((
                vec![
                    "-NoExit".to_string(),
                    "-ExecutionPolicy".to_string(),
                    "Bypass".to_string(),
                    "-Command".to_string(),
                    format!(
                        "& '{}'",
                        escape_powershell_single_quoted(&script.to_string_lossy())
                    ),
                ],
                Vec::new(),
            ))
        }
        ShellKind::Cmd | ShellKind::Other => None,
    }
}

pub(super) struct PreparedIntegration {
    pub(super) args: Vec<String>,
    pub(super) env: Vec<(String, String)>,
    /// Whether integration actually attached (drives the start() response).
    pub(super) injected: bool,
}

pub(super) fn prepare_integration(kind: ShellKind, enabled: bool) -> PreparedIntegration {
    let none = PreparedIntegration {
        args: base_args(kind, current_platform()),
        env: Vec::new(),
        injected: false,
    };
    if !enabled {
        return none;
    }
    let dir = integration_dir();
    if write_integration_files(&dir).is_err() {
        eprintln!(
            "[dbx-plugin-sshx] shell integration files unavailable at {}; continuing without injection",
            dir.display()
        );
        return none;
    }
    let user_zdotdir =
        std::env::var_os("ZDOTDIR").map(|value| value.to_string_lossy().into_owned());
    match injection_plan(kind, &dir, user_zdotdir.as_deref()) {
        Some((args, env)) => PreparedIntegration {
            args,
            env,
            injected: true,
        },
        None => none,
    }
}
