//! Command sanitizing and ANSI-stripped terminal capture helpers.

/// Decision delivered through an approval challenge's oneshot channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentDecision {
    /// Approved, optionally with the user-edited command text (the approval
    /// dialog shows the command editable, so what was approved is exactly
    /// what gets typed into the terminal).
    Approve {
        command: Option<String>,
    },
    Deny,
}

/// Strips C0 control characters (notably `\x03` interrupt and `\x1b`
/// escape, which could otherwise drive the terminal or the auto-sudo state
/// machine) while keeping `\n` and `\t`. Trims surrounding whitespace and
/// refuses the empty remainder.
pub fn sanitize_command(command: &str) -> Result<String, String> {
    let cleaned: String = command
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return Err("Command is empty after removing control characters".to_string());
    }
    Ok(trimmed.to_string())
}

/// Terminal text injected for an `ssh_exec_sudo` tool call. The tool *means*
/// "run this with sudo", but the terminal path types raw text into the user's
/// shell, so the `sudo` prefix must be part of the injected command or the
/// privilege escalation is silently lost. Idempotent: a command already
/// carrying the prefix (inline from the AI, or edited in by the approval
/// dialog) is returned unchanged so `sudo sudo` can never happen. The prefix
/// lands on the first line only — multi-line commands execute line by line,
/// which is a known limitation of the injection path. An empty command
/// degrades to a bare `sudo ` here; emptiness is rejected upstream by
/// [`sanitize_command`], so this function never has to police it.
pub fn sudo_command_text(command: &str) -> String {
    let first_line = command.lines().next().unwrap_or_default().trim_start();
    if first_line.starts_with("sudo ") || first_line == "sudo" {
        command.to_string()
    } else {
        format!("sudo {command}")
    }
}

/// Removes ANSI escape sequences: CSI, OSC/DCS-style string sequences
/// (BEL or `ESC \` terminated), and other two-byte ESC sequences.
/// Incomplete sequences at the end of the input are swallowed.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                // CSI: parameter bytes, intermediate bytes, then one final
                // byte in 0x40..=0x7e terminates the sequence.
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            // String sequences (OSC, DCS, SOS, PM, APC).
            Some(']' | 'P' | 'X' | '^' | '_') => {
                let mut pending_escape = false;
                for next in chars.by_ref() {
                    if pending_escape {
                        if next == '\\' {
                            break;
                        }
                        // A stray ESC inside the string just continues it.
                        pending_escape = false;
                        continue;
                    }
                    match next {
                        '\x07' => break,
                        '\x1b' => pending_escape = true,
                        _ => {}
                    }
                }
            }
            // Two-byte escapes (ESC 7, ESC (, ...) were consumed above.
            Some(_) => {}
            None => {}
        }
    }
    out
}
