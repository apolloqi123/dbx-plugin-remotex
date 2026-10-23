//! MCP safety layer.

use super::*;

/// Narrowing rules for git subcommands that are read-only only in their
/// listing shapes.
pub(crate) fn git_segment_risk(args: &[String], sensitive: bool) -> CommandRisk {
    let Some(position) = args.iter().position(|arg| !arg.starts_with('-')) else {
        return CommandRisk::Unknown;
    };
    let sub = args[position].as_str();
    if !GIT_READ_SUBCOMMANDS.contains(&sub) {
        return CommandRisk::Unknown;
    }
    let rest = &args[position + 1..];
    let read_only = match sub {
        // `git branch [-a]` / `git tag [-l 'v*']` list; `git branch work`,
        // `git branch -D x`, `git tag v1`, `git tag -d v1` mutate.
        "branch" | "tag" => {
            rest.iter().any(|arg| arg == "-l" || arg == "--list")
                || rest.iter().all(|arg| arg.starts_with('-'))
        }
        // `git remote [-v]` lists; add/rename/remove/set-url/prune mutate.
        "remote" => !rest
            .first()
            .map(|arg| {
                matches!(
                    arg.as_str(),
                    "add"
                        | "rename"
                        | "remove"
                        | "rm"
                        | "set-url"
                        | "set-head"
                        | "prune"
                        | "update"
                )
            })
            .unwrap_or(false),
        // `git reflog [show]` reads; delete/expire rewrite history.
        "reflog" => rest.first().map(|arg| arg == "show").unwrap_or(true),
        _ => true,
    };
    if !read_only || sensitive {
        return CommandRisk::Unknown;
    }
    CommandRisk::ReadOnly
}

/// Tokenizes a segment, strips leading `FOO=bar` assignments, and unwraps
/// prefix wrappers (`timeout 10 df`, `nohup ./job`, `env X=1 cmd`, ...) so
/// the inner command is what gets assessed.
pub(crate) fn effective_tokens(segment: &str) -> Vec<String> {
    let mut tokens: Vec<String> = segment
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|c: char| c == '"' || c == '\'')
                .to_string()
        })
        .collect();
    // Assignments and wrappers are unwrapped iteratively with a depth cap so
    // pathological input cannot loop forever.
    for _ in 0..16 {
        strip_leading_assignments(&mut tokens);
        let Some(verb) = tokens.first().cloned() else {
            break;
        };
        if !WRAPPER_VERBS.contains(&verb.as_str()) {
            break;
        }
        tokens.remove(0);
        strip_wrapper_flags(&mut tokens);
        if verb == "timeout" {
            // `timeout` takes a duration argument before the command.
            if tokens
                .first()
                .map(|token| !token.starts_with('-'))
                .unwrap_or(false)
            {
                tokens.remove(0);
            }
        }
    }
    strip_leading_assignments(&mut tokens);
    tokens
}

pub(crate) fn strip_leading_assignments(tokens: &mut Vec<String>) {
    while tokens
        .first()
        .map(|token| {
            let Some(eq) = token.find('=') else {
                return false;
            };
            eq > 0
                && token[..eq]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                && token[..eq]
                    .chars()
                    .next()
                    .is_some_and(|c| !c.is_ascii_digit())
        })
        .unwrap_or(false)
    {
        tokens.remove(0);
    }
}

/// Drops leading wrapper flags; flags in [`WRAPPER_VALUE_FLAGS`] also drop
/// the token after them (their value).
pub(crate) fn strip_wrapper_flags(tokens: &mut Vec<String>) {
    while let Some(first) = tokens.first() {
        if !first.starts_with('-') {
            return;
        }
        let takes_value = WRAPPER_VALUE_FLAGS.contains(&first.as_str());
        tokens.remove(0);
        if takes_value
            && tokens
                .first()
                .map(|token| !token.starts_with('-'))
                .unwrap_or(false)
        {
            tokens.remove(0);
        }
    }
}

/// Strips one leading `sudo` (and its flags) from a token list, returning
/// the inner command tokens. Like real sudo, the option section ends at the
/// first non-flag token — everything from there on is the inner command
/// (`sudo rm -rf /` must keep `-rf` with the command, not eat it as a sudo
/// flag). `None` when nothing remains.
pub(crate) fn sudo_inner_tokens(tokens: &[String]) -> Option<Vec<String>> {
    let mut index = 1; // tokens[0] is "sudo"
    while index < tokens.len() {
        let token = &tokens[index];
        if !(token.starts_with('-') && token.len() > 1) {
            break; // inner command starts here
        }
        index += 1;
        if SUDO_VALUE_FLAGS.contains(&token.as_str())
            && tokens
                .get(index)
                .map(|next| !next.starts_with('-'))
                .unwrap_or(false)
        {
            index += 1; // the flag's value
        }
    }
    let inner = &tokens[index.min(tokens.len())..];
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_vec())
    }
}

/// Unwraps repeated sudo prefixes (`sudo sudo …`) with a depth cap so the
/// destructive-pattern check sees the real inner command.
pub(crate) fn unwrap_sudo_prefix(tokens: &[String]) -> Option<Vec<String>> {
    let mut current: Vec<String> = tokens.to_vec();
    for _ in 0..4 {
        if current.first().map(String::as_str) != Some("sudo") {
            break;
        }
        current = sudo_inner_tokens(&current)?;
    }
    if current.is_empty() {
        None
    } else {
        Some(current)
    }
}

/// The `-c` script text of a shell segment (`sh -c '…'`, `bash -lc '…'`):
/// the tokens after the `-c`-bearing flag joined back into a command line.
/// `None` when the segment is not a `-c` invocation.
pub(crate) fn shell_c_script(args: &[String]) -> Option<String> {
    let position = args.iter().position(|arg| {
        arg == "-c" || (arg.starts_with('-') && !arg.starts_with("--") && arg.ends_with('c'))
    })?;
    let script = &args[position + 1..];
    if script.is_empty() {
        return None;
    }
    Some(script.join(" "))
}

/// Recognized catastrophic patterns for one (verb, args) pair. Raw-device
/// and critical-file redirection is checked here (not in `has_redirect`)
/// because it must stay `Destructive` rather than merely `Unknown`.
pub(crate) fn destructive_pattern(verb: &str, args: &[String]) -> Option<&'static str> {
    if DESTRUCTIVE_VERBS.contains(&verb) {
        return Some("formats or wipes a disk / powers the machine off");
    }
    if matches!(verb, "init" | "telinit") && args.iter().any(|arg| arg == "0" || arg == "6") {
        return Some("init 0/6 shuts down or reboots the machine");
    }
    if verb == "dd" {
        if let Some(output) = args.iter().find(|arg| arg.starts_with("of=")) {
            if is_raw_device(&output["of=".len()..]) {
                return Some("dd writes to a raw device");
            }
        }
    }
    if verb == "rm" {
        let recursive = args.iter().any(|arg| {
            (arg.starts_with('-')
                && !arg.starts_with("--")
                && arg.len() > 1
                && arg[1..].chars().any(|c| c == 'r' || c == 'R'))
                || arg == "--recursive"
        });
        for target in args.iter().filter(|arg| !arg.starts_with('-')) {
            if is_critical_file(target) {
                return Some("rm targets a critical system file");
            }
            if recursive && destructive_delete_target(target) {
                return Some("recursive rm targets a system root");
            }
        }
    }
    if matches!(verb, "chmod" | "chown") {
        let recursive = args
            .iter()
            .any(|arg| arg.starts_with("-R") || arg == "--recursive");
        for target in args
            .iter()
            .filter(|arg| !arg.starts_with('-') && !arg.contains('='))
        {
            if is_critical_file(target) {
                return Some("chmod/chown targets a critical system file");
            }
            if recursive && destructive_delete_target(target) {
                return Some("recursive chmod/chown targets a system root");
            }
        }
    }
    if verb == "truncate"
        && args
            .iter()
            .any(|arg| !arg.starts_with('-') && is_critical_file(arg))
    {
        return Some("truncate targets a critical system file");
    }
    if verb == "find" && args.iter().any(|arg| arg == "-delete") {
        return Some("find -delete removes files");
    }
    if verb == "docker" && args.iter().any(|arg| arg == "prune") {
        return Some("docker prune deletes images/containers/volumes");
    }
    if verb == "kill" {
        // `kill -9 -1` / `kill -1` signal every process the user owns.
        if args.iter().any(|arg| arg == "-1" || arg == "-9") {
            let pid_like = args.iter().filter(|arg| !arg.starts_with('-')).count();
            if pid_like == 0 {
                return Some("kill signals every process (pid -1)");
            }
        }
    }
    if SQL_VERBS.contains(&verb) {
        let joined = args.join(" ").to_ascii_lowercase();
        if joined.contains("drop database") || joined.contains("drop table") {
            return Some("SQL DROP statement");
        }
    }
    for (index, token) in args.iter().enumerate() {
        if is_redirect_token(token) {
            if let Some(target) = args.get(index + 1) {
                if is_raw_device(target) {
                    return Some("redirection writes to a raw device");
                }
                if is_critical_file(target) {
                    return Some("redirection overwrites a critical system file");
                }
            }
        }
    }
    None
}

/// True when the token opens an output redirection.
pub(crate) fn is_redirect_token(token: &str) -> bool {
    token.starts_with('>')
        || token.starts_with("&>")
        || token.starts_with("2>")
        || token.starts_with("1>")
}

/// True when the token list contains an output redirection.
pub(crate) fn has_redirect(args: &[String]) -> bool {
    args.iter().any(|token| is_redirect_token(token))
}

/// `/dev/sda`, `/dev/nvme0n1`, ... — block devices a shell must never touch
/// directly. `/dev/null` and `/dev/zero` deliberately do not match.
pub(crate) fn is_raw_device(path: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "/dev/sd",
        "/dev/nvme",
        "/dev/vd",
        "/dev/mmcblk",
        "/dev/disk/",
    ];
    PREFIXES.iter().any(|prefix| path.starts_with(prefix))
}

pub(crate) fn is_critical_file(path: &str) -> bool {
    CRITICAL_FILES
        .iter()
        .any(|critical| path == *critical || critical.ends_with('/') && path.starts_with(*critical))
}

/// Recursive-delete target rule: the filesystem root and everything up to
/// two components deep, except under `/tmp` / `/var/tmp` where cleanup is a
/// routine operation.
pub(crate) fn destructive_delete_target(target: &str) -> bool {
    let target = target.trim_end_matches('/');
    if target.is_empty() {
        return true; // "/" (or "//") itself
    }
    if matches!(
        target,
        "*" | "." | ".." | "./*" | "~" | "$HOME" | "${HOME}" | "~/*"
    ) {
        return true;
    }
    let Some(path) = target.strip_prefix('/') else {
        return false;
    };
    let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    match components.first() {
        None => true, // "/" itself
        Some(&"tmp") | Some(&"var") => {
            // `/tmp`, `/var/tmp` are routine cleanup targets.
            components.len() <= 1 || components.len() == 2 && components[1] == "tmp"
        }
        // Wildcards only escalate at shallow depth: `/*` and `/etc/*` wipe a
        // root, but `/root/.cache/*` is routine cleanup.
        _ => components.len() <= 2,
    }
}

/// True when any argument token references a credential / private-key path.
pub fn touches_sensitive_path(args: &[String]) -> bool {
    args.iter().any(|arg| is_sensitive_path(arg))
}

/// True when one command token references a credential / private-key path.
///
/// Such a command is still "read-only" in the write sense, but the realistic
/// read-only-connection threat is an LLM driven by injected instructions to
/// exfiltrate SSH keys, sudo or database credentials. The token therefore
/// downgrades the segment to [`CommandRisk::Unknown`]: refused on read-only
/// connections by the whitelist gate, allowed on normal connections where
/// the operator already grants full access. Defense in depth, not an
/// exhaustive secret scan — unrecognized verbs are refused anyway.
pub fn is_sensitive_path(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if lower.is_empty() || lower == "/" {
        return false;
    }
    // The `.pub` half of a keypair is public by design.
    if lower.ends_with(".pub") {
        return false;
    }
    // Match on a collapsed form so `//etc//shadow`, `/etc/./shadow` and
    // friends cannot dodge the component / prefix rules (reliability
    // round 5 adversarial pass).
    let normalized = normalized_path(&lower);
    let file_name = std::path::Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .trim_end_matches('*');
    // `id_*` also matches glob patterns like `find / -name 'id_*'`; host
    // keys are `/etc/ssh/ssh_host_<type>_key`.
    if file_name.starts_with("id_")
        || (file_name.starts_with("ssh_host_") && file_name.ends_with("_key"))
        || SENSITIVE_BASENAMES.contains(&file_name)
        || file_name == ".env"
        || file_name.starts_with(".env.")
    {
        return true;
    }
    // System credential files keep their secrets under relative paths too
    // (`cat shadow` from a `/etc` cwd is the same exfiltration).
    if matches!(file_name, "shadow" | "gshadow" | "sudoers") {
        return true;
    }
    // Extension check runs on the whole token so globs (`*.pem`) match too.
    if SENSITIVE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
        return true;
    }
    if normalized
        .split('/')
        .any(|component| SENSITIVE_COMPONENTS.contains(&component))
    {
        return true;
    }
    SENSITIVE_SYSTEM_PREFIXES
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
}

/// Collapses a path for matching: drops empty components (double slashes)
/// and `.` self-references, preserving absolute vs relative shape. `..`
/// components are kept — resolving them needs a base directory, and
/// keeping them is the conservative choice.
pub(crate) fn normalized_path(path: &str) -> String {
    // Windows 的 canonicalize 产出 `\` 分隔（含 \\?\ 前缀），组件匹配必须
    // 同时认两种分隔符，否则敏感路径黑名单在 Windows 上静默失效。
    let absolute = path.starts_with('/') || path.starts_with('\\');
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|component| !component.is_empty() && *component != ".")
        .collect();
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}
