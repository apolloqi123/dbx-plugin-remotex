//! Metrics support.

use super::*;

/// Human-readable mapping description (used in traces and diagnostics):
/// `127.0.0.1:8080 -> db.internal:5432`. Pure so tests pin the shape.
pub(crate) fn describe(
    kind: ForwardKind,
    listen_host: &str,
    listen_port: u16,
    target_host: &str,
    target_port: u16,
) -> String {
    if kind == ForwardKind::Dynamic {
        return format!("SOCKS5 {listen_host}:{listen_port}");
    }
    let arrow = match kind {
        ForwardKind::Local => "->",
        // Remote direction reads right-to-left from the server's view; the
        // arrow keeps the user's mental model (client dials the target).
        ForwardKind::Remote => "<-",
        ForwardKind::Dynamic => unreachable!(),
    };
    format!("{listen_host}:{listen_port} {arrow} {target_host}:{target_port}")
}

/// Validates and normalizes one mapping request. Hosts accept IPv4, IPv6
/// (bracketed or bare) and plain hostnames; the listen host defaults like
/// ssh(1) — empty is the loopback interface, never the wildcard — and `*`
/// (bind-everywhere) is only meaningful server-side, so a local mapping
/// rejects it instead of failing later at bind time. Listen port 0 asks the
/// OS (local) or the server (remote) to pick a port; the actually bound
/// value is reported through `boundPort`. Pure so tests exercise every
/// rejection without touching the network.
pub(crate) fn parse_spec(
    params: &Value,
) -> Result<(ForwardKind, String, u16, String, u16), String> {
    let kind = ForwardKind::parse(params.get("kind").unwrap_or(&Value::Null))?;
    let listen_host = normalize_host(
        params
            .get("listenHost")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    if kind != ForwardKind::Remote && listen_host == "*" {
        return Err(
            "listenHost \"*\" is a server-side wildcard; local mappings bind a concrete host (use 0.0.0.0 for all interfaces)"
                .to_string(),
        );
    }
    let listen_port = parse_port(params.get("listenPort"), "listenPort")?;
    if kind == ForwardKind::Dynamic {
        if params
            .get("targetHost")
            .is_some_and(|host| host.as_str().is_some_and(|host| !host.trim().is_empty()))
            || params
                .get("targetPort")
                .is_some_and(|port| port.as_u64().is_some_and(|port| port != 0))
        {
            return Err("Dynamic forwarding selects its destination per SOCKS5 request; targetHost and targetPort must be omitted".to_string());
        }
        return Ok((kind, listen_host, listen_port, String::new(), 0));
    }
    // Unlike the listen host, a missing target has no sensible default —
    // reject before normalize_host silently substitutes the loopback.
    let raw_target = params
        .get("targetHost")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if raw_target.is_empty() {
        return Err("targetHost is required".to_string());
    }
    let target_host = normalize_host(raw_target)?;
    let target_port = parse_port(params.get("targetPort"), "targetPort")?;
    Ok((kind, listen_host, listen_port, target_host, target_port))
}

/// Host syntax gate: IPv4 / IPv6 (URL-style `[...]` brackets stripped) /
/// hostname labels / the `*` listen wildcard. Empty stays the caller's
/// contract (loopback default applied here). Everything that embeds a port
/// or scheme (`host:8080`, `http://…`, `user@host`) is rejected so a typo
/// never silently becomes a hostname lookup of garbage.
pub(crate) fn normalize_host(raw: &str) -> Result<String, String> {
    let invalid = |reason: &str| format!("Invalid host {raw:?}: {reason}");
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok("127.0.0.1".to_string());
    }
    if trimmed.len() > 253 || trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid("must be a hostname or IP without whitespace"));
    }
    // `[::1]` / `[fe80::1]`: brackets are a URL convention — strip them and
    // reject `[::1]:8080` (the port belongs in its own field).
    let host = if trimmed.starts_with('[') {
        let Some(end) = trimmed.find(']') else {
            return Err(invalid("unclosed bracket"));
        };
        if end != trimmed.len() - 1 {
            return Err(invalid(
                "bracketed IPv6 must not be followed by more text (put the port in its own field)",
            ));
        }
        &trimmed[1..end]
    } else {
        trimmed
    };
    let lower = host.to_lowercase();
    if lower == "*" {
        return Ok(lower);
    }
    // Bare IPs (v4 and v6, including scoped zones like fe80::1%en0) win over
    // hostname rules; std's parser is the authority.
    if lower.parse::<std::net::IpAddr>().is_ok() {
        return Ok(lower);
    }
    if lower.contains([':', '/', '@']) {
        return Err(invalid("expected an IPv4/IPv6 address or a hostname"));
    }
    // All-numeric dotted strings that failed the IPv4 parse above are typos
    // like `999.1.1.1`, not hostnames — reject instead of a DNS surprise.
    if !lower.is_empty() && lower.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(invalid("looks like an IPv4 address but is not a valid one"));
    }
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    if !lower.split('.').all(label_ok) {
        return Err(invalid(
            "hostname labels accept letters, digits and inner hyphens only",
        ));
    }
    Ok(lower)
}

/// True when two listen endpoints of the same direction collide on the same
/// machine (local) or server (remote): equal ports and equal hosts, or
/// either side a wildcard (`*` / `0.0.0.0` / `::` / empty). Port 0 lets the
/// OS or the server pick, so it never pre-conflicts. Pure; both sides are
/// expected pre-normalized.
pub(crate) fn listen_endpoints_conflict(
    a_host: &str,
    a_port: u16,
    b_host: &str,
    b_port: u16,
) -> bool {
    if a_port == 0 || a_port != b_port {
        return false;
    }
    let wildcard = |host: &str| matches!(host, "*" | "0.0.0.0" | "::" | "");
    wildcard(a_host) || wildcard(b_host) || a_host == b_host
}

/// 1..=65535 for explicit ports; 0 is only meaningful for a remote listen
/// port (server-picked), which `parse_spec` special-cases before calling.
pub(crate) fn parse_port(value: Option<&Value>, field: &str) -> Result<u16, String> {
    let raw = value
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{field} must be an integer"))?;
    u16::try_from(raw).map_err(|_| format!("{field} must be within 0..=65535, got {raw}"))
}

/// Registers the `(listen_host, bound_port) -> target` row the
/// forwarded-tcpip handler matches against. The server echoes the address as
/// it was requested (OpenSSH lowercases nothing), so lookups normalize and
/// fall back to a port-only match when the bound form differs (e.g. the
/// client asked for 127.0.0.1 and the daemon reports ::1 or 0.0.0.0).
pub(crate) fn register_remote_target(
    table: &RemoteForwardTable,
    listen_host: &str,
    bound_port: u16,
    target_host: &str,
    target_port: u16,
    entry: Arc<ForwardEntry>,
) {
    let mut table = table.lock().unwrap_or_else(|poison| poison.into_inner());
    let host = listen_host.to_lowercase();
    table.insert(
        (host.clone(), u32::from(bound_port)),
        RelayTarget {
            host: target_host.to_string(),
            port: target_port,
            entry: entry.clone(),
        },
    );
    if host == "localhost" {
        table.insert(
            ("127.0.0.1".to_string(), u32::from(bound_port)),
            RelayTarget {
                host: target_host.to_string(),
                port: target_port,
                entry,
            },
        );
    }
}

/// Handler-side lookup: exact normalized key first, then a same-port scan —
/// the daemon may report the bound form differently from what the client
/// asked for (0.0.0.0 / :: vs 127.0.0.1), so wildcard binds win the fallback
/// and otherwise the single same-port mapping is used.
pub(crate) fn lookup_remote_target(
    table: &RemoteForwardTable,
    connected_address: &str,
    connected_port: u32,
) -> Option<RelayTarget> {
    let table = table.lock().unwrap_or_else(|poison| poison.into_inner());
    let host = connected_address.trim().to_lowercase();
    if let Some(target) = table.get(&(host, connected_port)) {
        return Some(target.clone());
    }
    let is_wildcard = |key_host: &str| matches!(key_host, "" | "*" | "0.0.0.0" | "::");
    let mut fallback: Option<&RelayTarget> = None;
    for ((key_host, key_port), target) in table.iter() {
        if *key_port != connected_port {
            continue;
        }
        if is_wildcard(key_host) {
            return Some(target.clone());
        }
        fallback = fallback.or(Some(target));
    }
    fallback.cloned()
}

/// Client-machine interface addresses for the forward dialog's listen-host
/// picker (`ssh/forward/interfaces`): one row per distinct bindable IP with
/// its interface name and loopback flag. Ordered loopback → IPv4 → IPv6 →
/// name so the picker reads stably; deduped because one IP can appear on
/// several interfaces. Takes normalized `(interface, ip, is_loopback)` rows
/// so tests pin the ordering without constructing host-specific structs.
/// An empty probe result degrades to an empty list — the picker hides and
/// manual input keeps working.
pub(crate) fn interface_rows(probe: Vec<(String, std::net::IpAddr, bool)>) -> Value {
    let mut rows = probe;
    // Loopback first, then IPv4, then IPv6; !is_loopback so true sorts first.
    rows.sort_by(|a, b| {
        (!a.2, a.1.is_ipv6(), &a.0, a.1.to_string()).cmp(&(
            !b.2,
            b.1.is_ipv6(),
            &b.0,
            b.1.to_string(),
        ))
    });
    rows.dedup_by(|a, b| a.1 == b.1);
    let interfaces: Vec<Value> = rows
        .into_iter()
        .map(|(name, ip, is_loopback)| {
            json!({
                "name": name,
                "addr": ip.to_string(),
                "isLoopback": is_loopback,
            })
        })
        .collect();
    json!({ "interfaces": interfaces })
}

/// Live probe wrapper for the RPC arm; the pure sorting lives in
/// [`interface_rows`].
pub(crate) fn local_interface_rows() -> Value {
    let probe: Vec<_> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|iface| {
            let ip = iface.ip();
            (iface.name, ip, ip.is_loopback())
        })
        .collect();
    interface_rows(probe)
}
