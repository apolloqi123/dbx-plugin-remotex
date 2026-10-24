use super::*;

/// Real directory lister for the versioned probes (only direct subdirs).
pub fn list_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = read
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    // Latest version first: version names sort lexicographically the way
    // installers bump them (11.x.y); a stable order also keeps tests sane.
    dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    dirs
}

/// `local/editors/list` payload: platform plus the full catalog with
/// per-entry availability. Unavailable entries are still listed so saved
/// associations stay resolvable after the editor is installed later.
pub fn editors_list_payload() -> Value {
    let platform = local_downloads::platform_name();
    let env = detect_env_paths();
    let entries = build_catalog(platform, &env, &|path| path.exists(), &list_subdirs);
    let editors: Vec<Value> = entries
        .iter()
        .map(|entry| {
            json!({
                "id": entry.id,
                "name": entry.name,
                "available": entry.available,
                "launch": entry.launch.to_json(),
                "suggestedExtensions": entry.suggested_extensions,
            })
        })
        .collect();
    json!({ "platform": platform, "editors": editors })
}

/// Splits a user-provided command line into argv tokens. POSIX-shlex style
/// with one Windows-friendly deviation: outside quotes a backslash is literal
/// unless it escapes a quote or another backslash (so `C:\Tools\app.exe`
/// survives unquoted and `"C:\Program Files\..."` keeps its separators).
/// Unterminated quotes are an error, not silently swallowed.
pub fn tokenize_command(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            '"' | '\'' => {
                has_token = true;
                let quote = ch;
                loop {
                    match chars.next() {
                        None => return Err("Command has an unterminated quote".to_string()),
                        Some(c) if c == quote => break,
                        Some('\\') if quote == '"' => match chars.peek() {
                            Some('\\') | Some('"') => {
                                current.push(chars.next().expect("peeked"));
                            }
                            _ => current.push('\\'),
                        },
                        Some(c) => current.push(c),
                    }
                }
            }
            '\\' => match chars.peek() {
                Some('\\') => {
                    chars.next();
                    current.push('\\');
                    has_token = true;
                }
                Some('"') | Some('\'') => {
                    current.push(chars.next().expect("peeked"));
                    has_token = true;
                }
                _ => {
                    current.push('\\');
                    has_token = true;
                }
            },
            _ => {
                current.push(ch);
                has_token = true;
            }
        }
    }
    if has_token {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Replaces every `{file}` placeholder with `path`; when the template has no
/// placeholder the path is appended, so `code` alone still opens the file.
pub fn substitute_file(args: &[String], path: &Path) -> Vec<String> {
    let text = path.to_string_lossy().into_owned();
    let substituted: Vec<String> = args
        .iter()
        .map(|arg| arg.replace("{file}", &text))
        .collect();
    if args.iter().any(|arg| arg.contains("{file}")) {
        substituted
    } else {
        substituted
            .into_iter()
            .chain(std::iter::once(text))
            .collect()
    }
}

/// Builds the launch target for a `local/open-with` request: a catalog
/// `editorId`, or a user-defined `custom` command string.
pub fn launch_spec_from_params(params: &Value) -> Result<LaunchSpec, String> {
    if let Some(id) = params.get("editorId").and_then(Value::as_str) {
        let platform = local_downloads::platform_name();
        let env = detect_env_paths();
        let entry = build_catalog(platform, &env, &|path| path.exists(), &list_subdirs)
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| format!("Unknown editor id: {id}"))?;
        if !entry.available {
            return Err(format!(
                "Editor '{}' is not installed on this machine",
                entry.name
            ));
        }
        return Ok(entry.launch);
    }
    if let Some(custom) = params.get("custom") {
        let command = custom
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Custom editor requires a non-empty command".to_string())?;
        let mut tokens = tokenize_command(command)?;
        let exe = tokens.remove(0);
        return Ok(LaunchSpec::Exe { exe, args: tokens });
    }
    Err("editorId or custom is required".to_string())
}

/// Same allowlist as `local/open` (a path recorded by a completed download),
/// then hands off to the injected launcher. `open_with_validated` plugs in
/// the real spawner; tests inject a recorder.
pub fn open_with_validated_with(
    history: &[Value],
    path: &Path,
    spec: &LaunchSpec,
    launcher: &dyn Fn(&Path, &LaunchSpec) -> Result<(), String>,
) -> Result<(), String> {
    if !local_downloads::is_recorded_download(history, path) {
        return Err("Path was not saved by a completed download of this plugin".to_string());
    }
    launcher(path, spec)
}

/// Production launcher: `open -a <app> <file>` on macOS, otherwise direct
/// argv spawn with `{file}` substitution. Never a shell.
pub fn launch_editor(path: &Path, spec: &LaunchSpec) -> Result<(), String> {
    if !path.is_file() {
        return Err("Downloaded file no longer exists".to_string());
    }
    match spec {
        LaunchSpec::OpenApp { app_id } => {
            if !cfg!(target_os = "macos") {
                return Err("openApp launch is only supported on macOS".to_string());
            }
            std::process::Command::new("open")
                .arg("-a")
                .arg(app_id)
                .arg(path)
                .spawn()
                .map(|_| ())
                .map_err(|error| format!("Failed to launch '{app_id}': {error}"))
        }
        LaunchSpec::Exe { exe, args } => {
            if exe.is_empty() {
                return Err("Editor executable path is empty".to_string());
            }
            std::process::Command::new(exe)
                .args(substitute_file(args, path))
                .spawn()
                .map(|_| ())
                .map_err(|error| format!("Failed to launch editor: {error}"))
        }
    }
}

/// `local/open-with`: allowlist check + real launch.
pub fn open_with_validated(
    history: &[Value],
    path: &Path,
    spec: &LaunchSpec,
) -> Result<(), String> {
    open_with_validated_with(history, path, spec, &launch_editor)
}
