//! Ssh Config session importer.

use super::*;

/// One top-level section of an OpenSSH config: the global preamble (before
/// the first `Host`/`Match` line), a `Host` pattern block or a `Match`
/// conditional block.
#[derive(Debug, Default)]
pub(super) struct SshConfigBlock {
    /// `Host` patterns; `None` for the global preamble and `Match` blocks.
    pub(super) host_patterns: Option<Vec<String>>,
    /// `Match` criteria as (lowercased keyword, value) pairs; `None` outside
    /// `Match` blocks. No-argument criteria (`all`/`final`/`canonical`) get an
    /// empty value.
    pub(super) match_criteria: Option<Vec<(String, String)>>,
    pub(super) directives: Vec<(String, String)>,
}

/// `Match` criteria that take no argument. They are recognized for parsing
/// but never supported for import (anything beyond `host`/`user` degrades to
/// a skipped-with-reason note).
pub(super) const SSH_MATCH_NO_ARG_CRITERIA: [&str; 3] = ["all", "final", "canonical"];

/// Parses an OpenSSH config file into imported sessions, one per concrete
/// (non-wildcard) `Host` pattern. Wildcard-only patterns carry defaults for
/// matching entries instead. Resolution follows OpenSSH first-value-wins
/// semantics across every applicable block (global preamble, `Host` blocks
/// whose pattern list matches the candidate, and `Match` blocks restricted to
/// `host`/`user` criteria).
///
/// Trust model (same as the tssh Expect importer): the pipeline only ever
/// receives the uploaded file's bytes, so `Include` is never followed and
/// `ProxyCommand` / `ProxyJump` are never executed — both are surfaced as
/// annotations on the affected rows. `IdentityFile` maps the path only; the
/// key file is never read, so no key material can enter the preview model.
pub fn parse_ssh_config(text: &str) -> Result<Vec<ImportedSession>, String> {
    let blocks = ssh_config_blocks(text)?;
    let candidates = ssh_config_candidates(&blocks)?;
    let mut degraded: Vec<String> = Vec::new();
    let mut work: u64 = 0;
    let mut sessions = Vec::new();
    for candidate in &candidates {
        if let Some(session) = resolve_ssh_candidate(candidate, &blocks, &mut degraded, &mut work)?
        {
            push_preview_session(&mut sessions, session)?;
        }
    }
    // Unsupported `Match` criteria apply to no row we can identify, so the
    // reason is reported once per parse on every imported row (bounded by
    // MAX_SSH_CONFIG_NOTES distinct reasons) — skipped, but not silently.
    if !degraded.is_empty() {
        let note = degraded.join("; ");
        for session in &mut sessions {
            session.description = if session.description.is_empty() {
                note.clone()
            } else {
                format!("{}; {}", session.description, note)
            };
        }
    }
    Ok(sessions)
}

/// Splits one config line into (lowercased keyword, argument). The keyword is
/// separated from the argument by whitespace or `=`; a double-quoted argument
/// keeps inner spaces. `None` for comments, blank lines and malformed lines
/// (an unterminated quote degrades to a skipped line, never a panic).
pub(super) fn split_ssh_config_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let split_at = trimmed.find(|byte: char| byte.is_whitespace() || byte == '=')?;
    let keyword = trimmed.get(..split_at)?.to_ascii_lowercase();
    if keyword.is_empty() {
        return None;
    }
    let mut rest = trimmed.get(split_at..)?.trim_start();
    if let Some(after_eq) = rest.strip_prefix('=') {
        rest = after_eq.trim_start();
    }
    if let Some(unquoted) = rest.strip_prefix('"') {
        let close = unquoted.find('"')?;
        return Some((keyword, unquoted.get(..close)?.to_string()));
    }
    Some((keyword, rest.to_string()))
}

/// Tokenizes the config into top-level blocks. Directives before the first
/// `Host`/`Match` line land in the implicit global preamble; a `Host` line
/// with no pattern degrades to an empty block that matches nothing.
/// Host/Match 行的参数与普通指令同帽：敌意超长模式串截断到上限（按 char
/// 边界）。块结构必须保留——跳过整块会让后续指令错落到前一个块。
pub(super) fn truncate_ssh_config_arg(argument: &str) -> &str {
    if argument.len() <= MAX_SSH_CONFIG_ARG_BYTES {
        return argument;
    }
    let mut end = MAX_SSH_CONFIG_ARG_BYTES;
    while !argument.is_char_boundary(end) {
        end -= 1;
    }
    &argument[..end]
}

pub(super) fn ssh_config_blocks(text: &str) -> Result<Vec<SshConfigBlock>, String> {
    let mut blocks: Vec<SshConfigBlock> = vec![SshConfigBlock::default()];
    let mut lines = 0usize;
    for line in text.lines() {
        lines += 1;
        if lines > MAX_SSH_CONFIG_LINES {
            return Err(format!(
                "OpenSSH config exceeds the {MAX_SSH_CONFIG_LINES} line limit"
            ));
        }
        let Some((keyword, argument)) = split_ssh_config_line(line) else {
            continue;
        };
        match keyword.as_str() {
            "host" => blocks.push(SshConfigBlock {
                host_patterns: Some(
                    truncate_ssh_config_arg(&argument)
                        .split_whitespace()
                        .map(str::to_string)
                        .collect(),
                ),
                match_criteria: None,
                directives: Vec::new(),
            }),
            "match" => blocks.push(SshConfigBlock {
                host_patterns: None,
                match_criteria: Some(parse_ssh_match_criteria(truncate_ssh_config_arg(&argument))),
                directives: Vec::new(),
            }),
            _ => {
                if argument.len() > MAX_SSH_CONFIG_ARG_BYTES {
                    // Hostile giant value: degrade the directive, keep parsing.
                    continue;
                }
                if let Some(block) = blocks.last_mut() {
                    block.directives.push((keyword, argument));
                }
            }
        }
    }
    Ok(blocks)
}

/// Parses `Match` criteria into (keyword, value) pairs. No-argument criteria
/// get an empty value; a trailing keyword without its argument yields an empty
/// value too (degrade, never panic).
pub(super) fn parse_ssh_match_criteria(argument: &str) -> Vec<(String, String)> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    let mut criteria = Vec::new();
    let mut index = 0usize;
    while index < tokens.len() {
        let keyword = tokens[index].to_ascii_lowercase();
        index += 1;
        let value = if SSH_MATCH_NO_ARG_CRITERIA.contains(&keyword.as_str()) {
            String::new()
        } else {
            let value = tokens.get(index).copied().unwrap_or_default().to_string();
            index += 1;
            value
        };
        criteria.push((keyword, value));
    }
    criteria
}

/// Only `host`/`user` criteria are evaluated at import time.
pub(super) fn ssh_match_supported(criteria: &[(String, String)]) -> bool {
    criteria
        .iter()
        .all(|(keyword, _)| matches!(keyword.as_str(), "host" | "user"))
}

/// Compact reason for an unsupported `Match` block, e.g.
/// `Match exec/localuser: settings not imported`.
pub(super) fn ssh_match_unsupported_reason(criteria: &[(String, String)]) -> String {
    let keywords = criteria
        .iter()
        .map(|(keyword, _)| keyword.as_str())
        .collect::<Vec<_>>()
        .join("/");
    format!("Match {keywords}: settings not imported")
}

/// OpenSSH single-pattern glob: `*` any sequence, `?` any single byte,
/// ASCII-case-insensitive (multibyte sequences compare byte-exact). Iterative
/// two-pointer scan so hostile star-heavy patterns stay O(pattern × text)
/// instead of exploding exponentially.
pub(super) fn ssh_glob_match(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_pi, mut star_ti) = (usize::MAX, 0usize);
    while ti < text.len() {
        if pi < pattern.len()
            && (pattern[pi] == b'?' || pattern[pi].eq_ignore_ascii_case(&text[ti]))
        {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == b'*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }
    pi == pattern.len()
}

/// OpenSSH pattern list: comma-separated patterns; a `!`-prefixed pattern
/// vetoes the whole list, otherwise any positive match passes. Charges the
/// O(pattern × candidate) cost to `work`; `None` means the budget is
/// exhausted and the caller must fail closed.
pub(super) fn ssh_pattern_list_matches(
    list: &str,
    candidate: &str,
    work: &mut u64,
) -> Option<bool> {
    let mut positive = false;
    for pattern in list.split(',') {
        let (negated, body) = match pattern.trim().strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, pattern.trim()),
        };
        *work = work.saturating_add((body.len().saturating_mul(candidate.len().max(1))) as u64);
        if *work > MAX_SSH_MATCH_WORK {
            return None;
        }
        if ssh_glob_match(body, candidate) {
            if negated {
                return Some(false);
            }
            positive = true;
        }
    }
    Some(positive)
}

/// A `Host` pattern becomes a connection candidate only when it is concrete:
/// wildcard (`*`/`?`) and negated patterns never name a single machine.
pub(super) fn ssh_concrete_pattern(pattern: &str) -> bool {
    !pattern.is_empty() && !pattern.starts_with('!') && !pattern.contains(['*', '?'])
}

/// Collects the deduped concrete patterns across all `Host` blocks.
pub(super) fn ssh_config_candidates(blocks: &[SshConfigBlock]) -> Result<Vec<String>, String> {
    let mut candidates: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for block in blocks {
        let Some(patterns) = &block.host_patterns else {
            continue;
        };
        for pattern in patterns {
            // OpenSSH treats `Host web1,web2` as a pattern list: split on the
            // comma so each name becomes its own entry. Without the split the
            // whole token passes as one "concrete" name and imports a
            // connection that can never resolve.
            for part in pattern.split(',') {
                if !ssh_concrete_pattern(part) || !seen.insert(part.to_string()) {
                    continue;
                }
                if candidates.len() >= MAX_PREVIEW_SESSIONS {
                    return Err(format!(
                        "Import exceeds the {MAX_PREVIEW_SESSIONS} session limit"
                    ));
                }
                candidates.push(part.to_string());
            }
        }
    }
    Ok(candidates)
}

/// Evaluates the supported `Match` criteria against the candidate's state
/// resolved so far. `Match host` compares against the resolved `Hostname`
/// (falling back to the candidate alias), `Match user` against the resolved
/// `User`; a user that is not set only matches a `*` pattern. `None` = the
/// pattern budget is exhausted.
pub(super) fn ssh_match_criteria_hit(
    criteria: &[(String, String)],
    candidate: &str,
    hostname: Option<&str>,
    username: Option<&str>,
    work: &mut u64,
) -> Option<bool> {
    for (keyword, value) in criteria {
        let hit = match keyword.as_str() {
            "host" => ssh_pattern_list_matches(value, hostname.unwrap_or(candidate), work)?,
            "user" => match username {
                Some(user) => ssh_pattern_list_matches(value, user, work)?,
                None => value == "*",
            },
            // Unreachable: support is checked before evaluation.
            _ => true,
        };
        if !hit {
            return Some(false);
        }
    }
    Some(true)
}

/// IdentityFile path mapping: `~`/`$(HomeDir)` prefixes and the `%d` token
/// (home) expand against the local home directory; every other token stays
/// verbatim. Only the PATH is mapped — the key file is never read, so no key
/// material can enter the preview model.
pub(super) fn expand_ssh_identity_path(path: &str) -> String {
    let expanded = match home_dir() {
        Some(home) => path.replacen("%d", &home, 1),
        None => path.to_string(),
    };
    expand_home_prefix(&expanded)
}

/// Resolves one concrete candidate through every applicable block with
/// OpenSSH first-value-wins semantics and builds the imported session.
pub(super) fn resolve_ssh_candidate(
    candidate: &str,
    blocks: &[SshConfigBlock],
    degraded: &mut Vec<String>,
    work: &mut u64,
) -> Result<Option<ImportedSession>, String> {
    let mut hostname: Option<String> = None;
    let mut username: Option<String> = None;
    let mut port: Option<u16> = None;
    let mut port_seen = false;
    let mut identity: Option<String> = None;
    let mut annotations: Vec<String> = Vec::new();
    let mut annotated = std::collections::HashSet::new();
    for block in blocks {
        let applies = if let Some(patterns) = &block.host_patterns {
            let mut hit = false;
            for list in patterns {
                match ssh_pattern_list_matches(list, candidate, work) {
                    Some(matched) if matched => {
                        hit = true;
                        break;
                    }
                    Some(_) => {}
                    None => return Err(SSH_CONFIG_MATCH_WORK_EXCEEDED.to_string()),
                }
            }
            hit
        } else if let Some(criteria) = &block.match_criteria {
            if !ssh_match_supported(criteria) {
                let reason = ssh_match_unsupported_reason(criteria);
                if degraded.len() < MAX_SSH_CONFIG_NOTES && !degraded.contains(&reason) {
                    degraded.push(reason);
                }
                false
            } else {
                match ssh_match_criteria_hit(
                    criteria,
                    candidate,
                    hostname.as_deref(),
                    username.as_deref(),
                    work,
                ) {
                    Some(hit) => hit,
                    None => return Err(SSH_CONFIG_MATCH_WORK_EXCEEDED.to_string()),
                }
            }
        } else {
            true // global preamble applies everywhere
        };
        if !applies {
            continue;
        }
        for (keyword, value) in &block.directives {
            match keyword.as_str() {
                "hostname" if hostname.is_none() => {
                    // %h expands to the alias the block matched on.
                    let expanded = value.replacen("%h", candidate, 1);
                    let trimmed = expanded.trim();
                    if !trimmed.is_empty() {
                        hostname = Some(trimmed.to_string());
                    }
                }
                "user" if username.is_none() => {
                    let trimmed = value.trim();
                    if !trimmed.is_empty() {
                        username = Some(trimmed.to_string());
                    }
                }
                "port" if !port_seen => {
                    port_seen = true;
                    port = value.trim().parse::<u16>().ok().filter(|port| *port > 0);
                }
                "identityfile" if identity.is_none() => {
                    identity = Some(expand_ssh_identity_path(value.trim()));
                }
                "proxyjump" if annotated.insert("proxyjump") => {
                    annotations.push(format!("ProxyJump {} (needs manual mapping)", value.trim()));
                }
                "proxycommand" if annotated.insert("proxycommand") => {
                    annotations.push(format!(
                        "ProxyCommand {} (not executed, needs manual mapping)",
                        value.trim()
                    ));
                }
                "include" if annotated.insert("include") => {
                    annotations.push(format!("Include {} (not followed)", value.trim()));
                }
                "userknownhostsfile" if annotated.insert("userknownhostsfile") => {
                    annotations.push(format!("UserKnownHostsFile {} (not carried)", value.trim()));
                }
                _ => {}
            }
        }
    }
    let mut session = ImportedSession::new(
        candidate.to_string(),
        hostname.unwrap_or_else(|| candidate.to_string()),
        port.unwrap_or(22),
        username
            .filter(|user| !user.is_empty())
            .unwrap_or_else(|| "root".to_string()),
    );
    session.description = annotations.join("; ");
    if let Some(path) = identity {
        session.auth = ImportedAuth::PrivateKey {
            path: Some(path),
            has_secret: false,
        };
        session.secret_note = SECRET_NOTE_KEY_PATH_ONLY.to_string();
    }
    Ok(Some(session))
}
