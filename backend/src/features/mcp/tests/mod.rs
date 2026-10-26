use super::*;

mod tools;
mod safety;
mod connections;
mod bridge;

fn state() -> McpState {
    // Unique data dir per call: settings persistence (limits AND the
    // §1.3 permission keys) must not leak across parallel tests.
    McpState::new(std::env::temp_dir().join(format!("dbx-mcp-test-{}", uuid::Uuid::new_v4())))
}

async fn multi_exec_error(state: &McpState, arguments: Value) -> String {
    tool_error(state, "ssh_multi_exec", arguments).await
}

async fn tool_error(state: &McpState, name: &str, arguments: Value) -> String {
    String::from(
        state
            .dispatch(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": name, "arguments": arguments },
            }))
            .await
            .unwrap()["error"]["message"]
            .as_str()
            .unwrap_or_default(),
    )
}

/// Builds a saved-connection payload with the given declared sudo source;
/// returns the connection plus its (runtime-composed) login password.
/// All credential values in tests are assembled at runtime — never real
/// credentials, never literals in source.
fn source_connection(source: &str, profile_ref: &str) -> (StoredConnection, String) {
    let login = format!("login-{}", uuid::Uuid::new_v4());
    let stored = StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "conn-src",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "password": login,
            "external_config": {
                "sudo_source": source,
                "sudo_profile": profile_ref,
            },
        }
    }))
    .unwrap();
    (stored, login)
}

#[cfg(windows)]
const ABS_TEST_ROOT: &str = "C:\\tmp\\transfers";

#[cfg(not(windows))]
const ABS_TEST_ROOT: &str = "/tmp/transfers";

fn stored_connection(id: &str, host: &str, port: u16, extra: Value) -> StoredConnection {
    let mut connection = json!({
        "connection": {
            "id": id,
            "host": host,
            "port": port,
            "username": "deploy",
            "password": format!("pw-{}", uuid::Uuid::new_v4()),
        }
    });
    if let (Some(target), Some(source)) = (connection.get_mut("connection"), extra.as_object()) {
        for (key, value) in source {
            target[key.clone()] = value.clone();
        }
    }
    StoredConnection::from_lifecycle_params(&connection).unwrap()
}
