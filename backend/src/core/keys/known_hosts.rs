use std::path::Path;

use serde_json::{json, Value};

use crate::core::host_key::HostKeyVerifier;

/// Lists the plugin's known_hosts store as `{entries: [...]}` with fields
/// `{host, port, keyType, fingerprint, marker}`. `marker` is the OpenSSH
/// marker (`@cert-authority` / `@revoked`) or `null` for plain entries. The
/// store keeps no timestamps, so there is no `learnedAt` to report. The
/// system `~/.ssh/known_hosts` stays read-only and is not included, matching
/// HostKeyVerifier::list_known_hosts.
pub fn list_known_hosts(data_dir: &Path) -> Result<Value, String> {
    let verifier = HostKeyVerifier::new(data_dir.join("known_hosts"));
    let mut entries: Vec<Value> = verifier
        .list_known_hosts()?
        .into_iter()
        .map(|entry| {
            let (host, port) = split_host_field(&entry.host_field);
            json!({
                "host": host,
                "port": port,
                "keyType": entry.key_type,
                "fingerprint": entry.fingerprint,
                "marker": entry.marker,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        let host = a["host"].as_str().cmp(&b["host"].as_str());
        host.then(a["port"].as_u64().cmp(&b["port"].as_u64()))
            .then(a["keyType"].as_str().cmp(&b["keyType"].as_str()))
    });
    Ok(json!({ "entries": entries }))
}

/// Removes entries matching `host:port` from the plugin's known_hosts store
/// and returns how many lines were removed.
pub fn remove_known_host(data_dir: &Path, host: &str, port: u16) -> Result<usize, String> {
    let verifier = HostKeyVerifier::new(data_dir.join("known_hosts"));
    verifier.remove_known_host(host, port)
}

/// Splits a known_hosts host field into `(host, port)`. `[host]:2222` carries
/// an explicit port; a bare `host` means the default port 22.
pub(super) fn split_host_field(host_field: &str) -> (String, u16) {
    if let Some(rest) = host_field.strip_prefix('[') {
        if let Some((host, port)) = rest.split_once("]:") {
            if let Ok(port) = port.parse::<u16>() {
                return (host.to_string(), port);
            }
        }
    }
    (host_field.to_string(), 22)
}
