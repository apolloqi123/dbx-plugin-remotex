//! Stable plugin and contribution ids declared in `manifest.json`.
//!
//! These strings are wire/storage identity: the DBX host matches them, the
//! manifest declares them, and `PLUGIN_ID` also names the plugin data
//! directory and keychain service. Keep every shared id here so a rename is
//! one edit plus the manifest, and let [`tests::ids_match_manifest_identity`]
//! catch drift.

/// Plugin id (`manifest.json` `id`): sidecar metadata, data directory,
/// keychain service, `ssh/status`, MCP `serverInfo`, tunnel menu ownership.
pub const PLUGIN_ID: &str = "io.dbx.remotex";

/// Workbench contribution opened by the tunnel context menu.
pub const TUNNELS_WORKBENCH_ID: &str = "io.dbx.remotex.tunnels";

/// The unified RemoteX connection-provider contribution (identity check only).
#[cfg(test)]
pub const CONNECTION_PROVIDER: &str = "io.dbx.remotex.connection";

/// Legacy standalone local-shell provider id. Connections saved before the
/// provider collapse still carry this lifecycle stamp; new local targets are
/// `protocol=local` inside [`CONNECTION_PROVIDER`].
pub const LEGACY_LOCAL_CONNECTION_PROVIDER: &str = "io.dbx.remotex.local";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;

    fn manifest() -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../manifest.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read manifest at {}: {error}", path.display()));
        serde_json::from_str(&raw).expect("manifest.json must be valid JSON")
    }

    #[test]
    fn ids_match_manifest_identity() {
        let manifest = manifest();
        assert_eq!(manifest["id"].as_str(), Some(PLUGIN_ID));
        let contributions = manifest["contributions"]
            .as_array()
            .expect("manifest contributions array");
        for (kind, id) in [
            ("workbench", TUNNELS_WORKBENCH_ID),
            ("connection-provider", CONNECTION_PROVIDER),
        ] {
            assert!(
                contributions
                    .iter()
                    .any(|entry| entry["type"] == kind && entry["id"] == id),
                "manifest is missing {kind} contribution '{id}'"
            );
        }
    }
}
