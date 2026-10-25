//! Local terminal env support.

use super::*;

pub(crate) use crate::core::preferences::env::{sanitize_env_pair, USER_ENV_MAX_PAIRS};
#[cfg(test)]
pub(crate) use crate::core::preferences::env::USER_ENV_MAX_VALUE;

/// Pure core of user-env validation over the request map: keeps the valid
/// pairs up to the pair cap. `BTreeMap` iterates key-sorted, so the cap trims
/// deterministically (same input → same survivors).
pub(super) fn sanitize_user_env(env: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for (key, value) in env {
        if pairs.len() >= USER_ENV_MAX_PAIRS {
            break;
        }
        if let Some(pair) = sanitize_env_pair(key, value) {
            pairs.push(pair);
        }
    }
    pairs
}

/// Locale keys that govern character handling (issue #173): when any of them
/// already carries a value — from the sidecar's own environment or the
/// request — the shell already has a locale and injection must stay out.
pub(super) const LOCALE_KEYS: [&str; 3] = ["LANG", "LC_ALL", "LC_CTYPE"];

/// Platform LANG defaults for a shell whose environment names no locale at
/// all. macOS picks `en_US.UTF-8` — present on every macOS install and
/// synthesized on demand for other UTF-8 locales, so it is the universal
/// value (a `zh_CN` one would print dates in Chinese for users who never
/// asked). Linux picks `C.UTF-8` — the one UTF-8 locale glibc always
/// provides (built-in since 2.35, shipped by every mainstream distro and
/// musl long before), unlike `en_US.UTF-8` which minimal installs never
/// generate. Windows returns `None`: console encodings there are code pages
/// driven by `chcp`/console APIs, which `LANG` cannot reach — the UTF-8
/// switch happens in the spawn argv instead
/// ([`apply_windows_utf8_console`]).
pub(super) fn default_utf8_lang(platform: Platform) -> Option<&'static str> {
    match platform {
        Platform::MacOS => Some("en_US.UTF-8"),
        Platform::Linux => Some("C.UTF-8"),
        Platform::Windows => None,
    }
}

/// The LANG value to hand the PTY shell when it would otherwise run in the C
/// locale, or `None` when a locale is already spoken. DBX launches the
/// sidecar as a GUI process, and GUI sessions on macOS carry no `LANG` — the
/// shell inherited that emptiness and fell back to `C`, where CJK input
/// breaks down per byte (line editing, completion, filename display garble).
/// `user_env` is the sanitized request pairs (what will actually be applied);
/// `inherited` reports the sidecar's own environment. The gap is filled only
/// when neither names one of [`LOCALE_KEYS`] with a non-empty value — a
/// user-configured locale always wins, and `None` means the shell keeps what
/// it already has.
pub(super) fn fallback_lang(
    user_env: &[(String, String)],
    platform: Platform,
    inherited: &dyn Fn(&str) -> Option<String>,
) -> Option<&'static str> {
    let user_speaks = user_env
        .iter()
        .any(|(key, _)| LOCALE_KEYS.contains(&key.as_str()));
    let inherited_speaks = LOCALE_KEYS
        .iter()
        .any(|key| inherited(key).is_some_and(|value| !value.is_empty()));
    if user_speaks || inherited_speaks {
        return None;
    }
    default_utf8_lang(platform)
}

/// Switches a Windows ConPTY child's console to the UTF-8 code page by
/// rewriting the spawn argv (issue #173 Windows side). A Chinese/GBK Windows
/// boots the child console at code page 936, where conhost's byte-level
/// console conversions (`ReadConsoleA`/`WriteFile`) and every program that
/// trusts the console code page (`cmd type`, git, node, …) speak GBK while
/// the terminal speaks UTF-8 — CJK output garbles the same way the C-locale
/// shell did on Unix, and this is the same fix in Windows terms.
///
/// `chcp 65001` is the one lever that moves the console at once: it flips
/// the input and output code page, which conhost's byte conversions follow
/// and which the CRT picks up as its console default. PowerShell needs a
/// second preset on top: its ConsoleHost pipes native-command output through
/// its own decode layer keyed to `[Console]::OutputEncoding`, an encoding
/// .NET Framework caches at process start — a later `chcp` moves the console
/// but not that cache, so `cmd /c type` of a UTF-8 file kept garbling under a
/// 65001 console until the .NET layer was pointed at UTF-8 as well (measured:
/// `cmd /c chcp` reported 65001 while `type` still garbled, and presetting
/// both layers made the bytes exact). The alternatives were evaluated and
/// rejected: env injection is a no-op (the console code page is session
/// state, no environment variable reaches it — `LANG` is unreadable by
/// cmd/PowerShell), and either preset alone covers only one of the two
/// layers. The rewrite is argv-only, so the user env merge semantics are
/// untouched, and `chcp`'s own output is suppressed so the startup output
/// window (the ConPTY handshake and the resize gate keyed to it) sees the
/// same traffic as before. WSL manages its own UTF-8 world and arbitrary
/// shells have no safe argv seam, so both stay untouched; the cmd wrapper
/// ends with `title %COMSPEC%` because the `/k` command line would otherwise
/// become the console title for the whole session.
/// The PowerShell-side UTF-8 prelude: the console code page via `chcp`
/// (conhost byte conversions + CRT) plus the .NET decode layer that
/// PowerShell's ConsoleHost caches at process start. See
/// [`apply_windows_utf8_console`] for why one without the other is not enough.
pub(super) const POWERSHELL_UTF8_PRELUDE: &str =
    "chcp 65001 >$null; [Console]::OutputEncoding=[System.Text.Encoding]::UTF8";

pub(super) fn apply_windows_utf8_console(
    platform: Platform,
    kind: ShellKind,
    mut args: Vec<String>,
) -> Vec<String> {
    if platform != Platform::Windows {
        return args;
    }
    match kind {
        ShellKind::Cmd => {
            let mut bootstrapped = vec![
                "/k".to_string(),
                "chcp 65001>nul&title %COMSPEC%".to_string(),
            ];
            bootstrapped.append(&mut args);
            bootstrapped
        }
        ShellKind::PowerShell => {
            // Fold into the integration -Command when injection produced one;
            // a bare shell (integration off or the script write failed) still
            // needs the switch and must stay open, hence -NoExit. The space
            // before `>` is load-bearing: PowerShell lexes `65001>$null` as
            // one literal argument (chcp then fails on `65001>`), cmd's
            // `65001>nul` below has no such tokenizer rule.
            if let Some(index) = args.iter().position(|arg| arg == "-Command") {
                if let Some(payload) = args.get(index + 1) {
                    args[index + 1] = format!("{POWERSHELL_UTF8_PRELUDE}; {payload}");
                    return args;
                }
            }
            args.extend([
                "-NoExit".to_string(),
                "-Command".to_string(),
                POWERSHELL_UTF8_PRELUDE.to_string(),
            ]);
            args
        }
        _ => args,
    }
}
