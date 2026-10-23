//! MCP safety layer.

use super::*;

/// Classification of one shell command under the safety gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRisk {
    /// Provably read-only; allowed on read-only connections.
    ReadOnly,
    /// Matches a catastrophic pattern; carries a human-readable reason.
    Destructive(&'static str),
    /// Not provably read-only and not a known pattern; refused on
    /// read-only connections, allowed (unconfirmed) elsewhere.
    Unknown,
}

/// Classifies a full command line (which may chain several commands).
pub fn assess_command(command: &str) -> CommandRisk {
    assess_command_depth(command, 0)
}

/// Escalates to Destructive when the inner command of a wrapper (`sudo …`,
/// `sh -c '…'`) classifies as destructive at the next recursion depth.
/// Escalation only: anything else keeps the caller's own verdict.
pub(crate) fn destructive_after_wrap(inner: &str, depth: u8) -> Option<&'static str> {
    let next = depth + 1;
    if next >= MAX_ASSESS_DEPTH {
        return None;
    }
    match assess_command_depth(inner, next) {
        CommandRisk::Destructive(reason) => Some(reason),
        _ => None,
    }
}

pub(crate) fn assess_command_depth(command: &str, depth: u8) -> CommandRisk {
    let neutralized = neutralize_fd_dups(command);
    // Reliability round 5 (adversarial sweep): a catastrophic payload
    // hidden inside `$(...)` / backticks must still trip the destructive
    // gate even though the surrounding command looks benign —
    // `echo $(rm -rf /)` used to classify as a plain `Unknown` and run
    // unconfirmed on writable connections. The sweep only ever escalates
    // Unknown → Destructive; it can never whitelist anything.
    if depth < MAX_ASSESS_DEPTH {
        if let Some(reason) = hidden_destructive_subcommand(&neutralized, depth) {
            return CommandRisk::Destructive(reason);
        }
    }
    let mut overall = CommandRisk::ReadOnly;
    for segment in split_segments(&neutralized) {
        match assess_segment(segment, depth) {
            CommandRisk::Destructive(reason) => return CommandRisk::Destructive(reason),
            CommandRisk::Unknown => overall = CommandRisk::Unknown,
            CommandRisk::ReadOnly => {}
        }
    }
    overall
}

/// Assesses the texts of every `$( ... )` / `` ` ... ` `` span: when any
/// span's content classifies as destructive, the whole command is
/// destructive.
pub(crate) fn hidden_destructive_subcommand(text: &str, depth: u8) -> Option<&'static str> {
    for sub in substitution_spans(text) {
        for segment in split_segments(&sub) {
            if let CommandRisk::Destructive(reason) = assess_command_depth(segment, depth + 1) {
                return Some(reason);
            }
        }
    }
    None
}

/// Collects the texts of `$( … )` spans (nesting-aware) and `` ` … ` ``
/// spans. Unterminated spans keep their remainder (fail closed: an
/// unterminated `` `rm -rf /`` still trips the sweep).
pub(crate) fn substitution_spans(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'$' if bytes.get(index + 1) == Some(&b'(') => {
                let mut parens = 1usize;
                let mut cursor = index + 2;
                while cursor < bytes.len() && parens > 0 {
                    match bytes[cursor] {
                        b'(' => parens += 1,
                        b')' => parens -= 1,
                        _ => {}
                    }
                    cursor += 1;
                }
                let end = if parens == 0 { cursor - 1 } else { cursor };
                spans.push(text[index + 2..end].to_string());
                index = cursor;
            }
            b'`' => {
                let start = index + 1;
                match text[start..].find('`') {
                    Some(offset) => {
                        spans.push(text[start..start + offset].to_string());
                        index = start + offset + 1;
                    }
                    None => {
                        spans.push(text[start..].to_string());
                        index = bytes.len();
                    }
                }
            }
            _ => index += 1,
        }
    }
    spans
}

/// True when any top-level command segment runs under `sudo` (after the same
/// env-prefix / wrapper unwrapping the classifier applies). Agent terminal
/// mode treats these as privilege escalation regardless of the inner verb,
/// so teaching-mode approval covers inline `sudo …`, not just `ssh_exec_sudo`.
pub fn runs_under_sudo(command: &str) -> bool {
    let neutralized = neutralize_fd_dups(command);
    split_segments(&neutralized)
        .iter()
        .any(|segment| effective_tokens(segment).first().map(String::as_str) == Some("sudo"))
}

/// Normalizes terminal input for injection (IMPL_PLAN §1.2): `\r\n` and
/// `\n` fold to `\r` (the Enter an interactive shell expects), NUL bytes
/// are stripped, and the result is capped at 8 KiB on a char boundary.
pub fn normalize_terminal_input(input: &str) -> String {
    let mut normalized = String::with_capacity(input.len().min(MAX_TERMINAL_INPUT_BYTES + 4));
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                // Fold CRLF to a single CR.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                normalized.push('\r');
            }
            '\n' => normalized.push('\r'),
            '\0' => {}
            other => normalized.push(other),
        }
        if normalized.len() >= MAX_TERMINAL_INPUT_BYTES {
            break;
        }
    }
    if normalized.len() > MAX_TERMINAL_INPUT_BYTES {
        let mut end = MAX_TERMINAL_INPUT_BYTES;
        while !normalized.is_char_boundary(end) {
            end -= 1;
        }
        normalized.truncate(end);
    }
    normalized
}

/// True when every character of the input is a control character: pure
/// control sequences (Enter, Ctrl+C, escape runs) carry no shell-visible
/// text, so they are the only input a read-only connection accepts
/// (IMPL_PLAN §1.2 ①).
pub fn is_control_only_input(input: &str) -> bool {
    input.chars().all(char::is_control)
}

/// Assesses terminal input by its worst `\r`-delimited line: each line is
/// classified like a shell command and the highest risk wins. Destructive
/// short-circuits; an empty (pure-control) input is read-only.
pub fn assess_terminal_input(input: &str) -> CommandRisk {
    let mut overall = CommandRisk::ReadOnly;
    for line in input.split('\r') {
        if line.trim().is_empty() {
            continue;
        }
        match assess_command(line) {
            CommandRisk::Destructive(reason) => return CommandRisk::Destructive(reason),
            CommandRisk::Unknown => overall = CommandRisk::Unknown,
            CommandRisk::ReadOnly => {}
        }
    }
    overall
}

/// Replaces fd duplications (`2>&1`, `1>&2`, ...) with a neutral token so
/// the `&` splitter does not shred them; they duplicate fds, not files.
pub(crate) fn neutralize_fd_dups(command: &str) -> String {
    command
        .replace("2>&1", " FDDUP ")
        .replace("1>&2", " FDDUP ")
        .replace("2>&2", " FDDUP ")
        .replace("1>&1", " FDDUP ")
        .replace(">&1", " FDDUP ")
        .replace(">&2", " FDDUP ")
}

/// Splits a command line into pipeline/chain segments. Naive on purpose:
/// quote contents are not parsed, and a split inside quotes can only make
/// the assessment more conservative.
pub(crate) fn split_segments(command: &str) -> Vec<&str> {
    command
        .split([';', '|', '\n', '&'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// Assesses one chain segment (no `;`/`&&`/`|` left inside).
pub(crate) fn assess_segment(segment: &str, depth: u8) -> CommandRisk {
    if segment.contains(":(){") {
        return CommandRisk::Destructive("fork bomb");
    }
    if segment.contains("$(") || segment.contains('`') {
        return CommandRisk::Unknown;
    }
    let tokens = effective_tokens(segment);
    let Some((verb, args)) = tokens.split_first() else {
        return CommandRisk::Unknown;
    };
    let verb = verb.as_str();

    if let Some(reason) = destructive_pattern(verb, args) {
        return CommandRisk::Destructive(reason);
    }
    if has_redirect(args) {
        // Redirection writes a file; only raw-device / critical targets are
        // destructive (checked inside destructive_pattern), everything else
        // is merely a write.
        return CommandRisk::Unknown;
    }
    if verb == "sudo" {
        // Privileged commands never count as whitelisted reads. Their
        // destructive shape still matters: unwrap the sudo prefix (flags
        // included) so `sudo rm -rf /` / `sudo -u root mkfs.ext4 …` trip
        // the catastrophic gate instead of slipping through as a plain
        // Unknown. Everything not destructive stays Unknown as before.
        if let Some(inner) = unwrap_sudo_prefix(&tokens) {
            let inner_text = inner.join(" ");
            let unwrapped = effective_tokens(&inner_text);
            if let Some((inner_verb, inner_args)) = unwrapped.split_first() {
                if let Some(reason) = destructive_pattern(inner_verb, inner_args) {
                    return CommandRisk::Destructive(reason);
                }
            }
            // `sudo sh -c 'rm -rf /'`: the inner command may itself be a
            // wrapper hiding the payload — recurse (destructive-only).
            if let Some(reason) = destructive_after_wrap(&inner_text, depth) {
                return CommandRisk::Destructive(reason);
            }
        }
        return CommandRisk::Unknown;
    }
    if matches!(verb, "sh" | "bash" | "dash" | "zsh" | "ksh") {
        // A `-c` script hides its payload in one string: assess the script
        // text so `sh -c 'rm -rf /'` trips the catastrophic gate. The
        // wrapper itself never whitelists anything — anything not
        // destructive stays Unknown (unchanged from before).
        if let Some(script) = shell_c_script(args) {
            if let Some(reason) = destructive_after_wrap(&script, depth) {
                return CommandRisk::Destructive(reason);
            }
        }
        return CommandRisk::Unknown;
    }
    let sensitive = touches_sensitive_path(args);
    if verb == "dmesg" {
        // `-c`/`-C` print-and-clear or clear the kernel ring buffer;
        // `-n`/`--console-level` changes console logging.
        let mutating = args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "-C" | "-c" | "-n" | "--clear" | "--read-clear" | "--console-level"
            ) || arg.starts_with("--console-level=")
        });
        return if mutating || sensitive {
            CommandRisk::Unknown
        } else {
            CommandRisk::ReadOnly
        };
    }
    if verb == "history" {
        // Only display forms stay read-only; -c/-d/-w/... rewrite history.
        let mutating = args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "-c" | "-d" | "-a" | "-r" | "-w" | "-p" | "-s" | "--clear"
            )
        });
        return if mutating || sensitive {
            CommandRisk::Unknown
        } else {
            CommandRisk::ReadOnly
        };
    }
    if verb == "sort" {
        // `sort -o FILE` writes its output to an arbitrary path.
        let writes = args.iter().any(|arg| {
            arg == "-o"
                || arg == "--output"
                || arg.starts_with("--output=")
                || (arg.starts_with("-o") && arg.len() > 2)
        });
        return if writes || sensitive {
            CommandRisk::Unknown
        } else {
            CommandRisk::ReadOnly
        };
    }
    if READ_ONLY_VERBS.contains(&verb) {
        return if sensitive {
            CommandRisk::Unknown
        } else {
            CommandRisk::ReadOnly
        };
    }
    if verb == "find" {
        // `-exec`/`-ok` run arbitrary programs; any `-f…` option (`-fprint`,
        // `-fprintf`, `-fls`, …) writes the match list to a file.
        if args
            .iter()
            .any(|arg| arg.starts_with("-exec") || arg.starts_with("-f") || arg == "-ok")
        {
            return CommandRisk::Unknown;
        }
        return if sensitive {
            CommandRisk::Unknown
        } else {
            CommandRisk::ReadOnly
        };
    }
    if verb == "git" {
        return git_segment_risk(args, sensitive);
    }
    if verb == "journalctl" {
        if args
            .iter()
            .any(|arg| arg.starts_with("--vacuum") || arg.starts_with("--rotate"))
        {
            return CommandRisk::Unknown;
        }
        return CommandRisk::ReadOnly;
    }
    if verb == "crontab" {
        // `crontab -r` removes the crontab and `-e` opens an editor; only
        // the listing forms are read-only.
        let listing = !args.is_empty() && args.iter().all(|arg| arg == "-l" || arg == "--list");
        return if listing {
            CommandRisk::ReadOnly
        } else {
            CommandRisk::Unknown
        };
    }
    if verb == "service" {
        return if args.iter().any(|arg| arg == "status") {
            CommandRisk::ReadOnly
        } else {
            CommandRisk::Unknown
        };
    }
    if let Some((_, allowed)) = SUBCOMMAND_VERBS.iter().find(|(owner, _)| *owner == verb) {
        let Some(position) = args.iter().position(|arg| !arg.starts_with('-')) else {
            return CommandRisk::Unknown;
        };
        let sub = args[position].as_str();
        if !allowed.contains(&sub) {
            return CommandRisk::Unknown;
        }
        // `ip` takes a second-level command (`ip route flush all`): mutating
        // object commands stay out of the read-only class even though the
        // object itself (`route`, `link`, …) inspects by default.
        let mutating = verb == "ip"
            && args[position + 1..]
                .iter()
                .any(|arg| MUTATING_OBJECT_COMMANDS.contains(&arg.as_str()));
        return if mutating || sensitive {
            CommandRisk::Unknown
        } else {
            CommandRisk::ReadOnly
        };
    }
    CommandRisk::Unknown
}
