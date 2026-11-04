//! Remote desktop file transfer RPCs.
//!
//! The transfer window uses a dedicated SFTP-only SSH session rather than the
//! remote desktop protocol itself: RFB has no file clipboard and CLIPRDR only
//! carries files the remote user copied. See `features/remote_files`.

use super::*;
use crate::core::model::StoredConnection;
use crate::core::non_ssh_credentials::NonSshCredentials;
use crate::features::remote_files::{clipboard, copy, credentials, local, remote_clipboard};
use serde_json::{json, Value};
use uuid::Uuid;

/// Builds the synthetic SSH connection used by the transfer window. It is
/// never stored in the host connection registry and never appears in
/// `ssh/sessions/list`.
///
/// When the form has no SSH password, the RDP/VNC connection's already stored
/// password (same host, same username) is tried first: the user clicked the
/// transfer button and expects the companion channel to open without a second
/// login. A missing entry is reported distinctly so the UI can show the form
/// without a red error.
fn transfer_connection(
    params: &Value,
    non_ssh_creds: &NonSshCredentials,
    use_desktop_credentials: bool,
) -> Result<StoredConnection, String> {
    let host = params
        .get("host")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("Missing host")?
        .to_string();
    let port = params
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(22);
    let mut username = params
        .get("username")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let authentication = match params.get("authentication").and_then(Value::as_str) {
        Some("private-key") => "private-key",
        Some("private-key-password") => "private-key-password",
        Some("agent") => "agent",
        Some("none") => "none",
        _ => "password",
    };
    let text = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let mut password = text("password");
    if use_desktop_credentials && authentication == "password" && password.is_empty() {
        let connection_id = params
            .get("connectionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        if let Some(connection_id) = connection_id {
            if let Some((stored_user, stored_password)) = non_ssh_creds.get(connection_id) {
                if username.is_empty() {
                    username = stored_user;
                    password = stored_password;
                } else if username == stored_user {
                    password = stored_password;
                }
            }
        }
    }
    if username.is_empty() {
        return Err("Missing username".to_string());
    }
    if authentication == "password" && password.is_empty() {
        return Err(
            "No stored SSH credentials for this connection; enter an SSH password.".to_string(),
        );
    }
    let payload = json!({
        "connection": {
            "id": format!("filetransfer-{}", Uuid::new_v4()),
            "name": "Remote desktop files",
            "host": host.clone(),
            "port": port,
            "username": username,
            "password": password,
            "external_config": {
                "protocol": "ssh",
                "authentication": authentication,
                "private_key_path": text("privateKeyPath"),
                "agent_socket": text("agentSocket"),
            },
            "connection_secrets": {
                "private_key": text("privateKey"),
                "private_key_passphrase": text("privateKeyPassphrase"),
            }
        },
        "runtime": { "host": host, "port": port }
    });
    StoredConnection::from_lifecycle_params(&payload)
}

impl Plugin {
    pub(super) fn handle_remote_files(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "filetransfer/connect" => {
                let mut params = params;
                // A password saved by an earlier successful transfer wins over
                // the desktop form's credentials.
                if params.get("authentication").and_then(Value::as_str).unwrap_or("password")
                    == "password"
                    && params.get("password").and_then(Value::as_str).unwrap_or("").is_empty()
                {
                    if let (Some(host), Some(username)) = (
                        params.get("host").and_then(Value::as_str),
                        params.get("username").and_then(Value::as_str),
                    ) {
                        let port = params
                            .get("port")
                            .and_then(Value::as_u64)
                            .and_then(|value| u16::try_from(value).ok())
                            .unwrap_or(22);
                        if let Some(saved) =
                            credentials::load(&self.ssh.data_dir, host, port, username)
                        {
                            params["password"] = json!(saved);
                        }
                    }
                }
                let use_desktop_credentials = params
                    .get("useDesktopCredentials")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let connection =
                    transfer_connection(&params, &self.non_ssh_creds, use_desktop_credentials)?;
                let workbench_id = params
                    .get("workbenchId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("remote-desktop-files");
                let ssh = self.ssh.clone();
                // A retried/abandoned connect must not leave a second hidden
                // transport for the same workbench.
                if let Some(previous) = self.remote_files.take_session_for_workbench(workbench_id) {
                    self.remote_files.cancel_session(&previous);
                    let _ = self.runtime.block_on(ssh.close_session(&previous));
                }
                let session_id = self
                    .runtime
                    .block_on(ssh.open_files_session(&connection, workbench_id, emitter))?;
                let home = match self.runtime.block_on(ssh.sftp_home(&session_id)) {
                    Ok(home) => home,
                    Err(error) => {
                        let _ = self.runtime.block_on(ssh.close_session(&session_id));
                        return Err(format!(
                            "SSH connected but SFTP is unavailable: {error}"
                        ));
                    }
                };
                if connection.authentication.method_name() == "password"
                    && !connection.password.is_empty()
                {
                    let _ = credentials::save(
                        &self.ssh.data_dir,
                        &connection.host,
                        connection.port,
                        &connection.username,
                        &connection.password,
                    );
                }
                self.remote_files.track_session(workbench_id, &session_id);
                Ok(json!({
                    "sessionId": session_id,
                    "home": home,
                    "host": connection.host,
                    "username": connection.username,
                }))
            }
            "filetransfer/ping" => {
                let session_id = required_string(&params, "sessionId")?;
                let tracked = self.remote_files.touch_session(session_id);
                let alive =
                    tracked && self.runtime.block_on(self.ssh.files_session_alive(session_id));
                if !alive {
                    self.remote_files.cancel_session(session_id);
                    self.remote_files.untrack_session(session_id);
                    let _ = self.runtime.block_on(self.ssh.close_session(session_id));
                }
                Ok(json!({ "alive": alive }))
            }
            "filetransfer/disconnect" => {
                let session_id = required_string(&params, "sessionId")?;
                self.remote_files.cancel_session(session_id);
                self.remote_files.untrack_session(session_id);
                // Idempotent: the panel also disconnects on unmount and after
                // a transport drop, where the session may already be gone.
                let _ = self.runtime.block_on(self.ssh.close_session(session_id));
                Ok(json!({ "success": true }))
            }
            "filetransfer/local/list" => local::list(&params),
            "filetransfer/credentials/forget" => {
                let host = required_string(&params, "host")?;
                let username = required_string(&params, "username")?;
                let port = params
                    .get("port")
                    .and_then(Value::as_u64)
                    .and_then(|value| u16::try_from(value).ok())
                    .unwrap_or(22);
                credentials::forget(&self.ssh.data_dir, &host, port, &username)?;
                Ok(json!({ "success": true }))
            }
            "filetransfer/clipboard/files" => Ok(clipboard::files()),
            "filetransfer/clipboard/write" => clipboard::write(&params),
            // VNC has no file clipboard: inspect the remote X11 selection
            // through the transfer session's transport (fixed read-only probe).
            "filetransfer/clipboard/remote" => {
                let session_id = required_string(&params, "sessionId")?;
                if !self.remote_files.touch_session(&session_id) {
                    return Ok(json!({ "supported": false, "paths": [] }));
                }
                self.runtime
                    .block_on(remote_clipboard::read(&self.ssh, &session_id))
            }
            "filetransfer/clipboard/fetch" => {
                let handle = self.runtime.handle().clone();
                copy::start_clipboard_download(&self.remote_files, &handle, &params, emitter)
            }
            "filetransfer/upload/start" => {
                let handle = self.runtime.handle().clone();
                copy::start_upload(&self.remote_files, &handle, &params, emitter)
            }
            "filetransfer/download/start" => {
                let handle = self.runtime.handle().clone();
                copy::start_download(&self.remote_files, &handle, &params, emitter)
            }
            "filetransfer/cancel" => copy::cancel(&self.remote_files, &params),
            _ => Err(format!("Method not found: {method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Value {
        json!({
            "host": "example.test",
            "port": 22,
            "username": "dev",
            "authentication": "password",
            "password": "secret",
        })
    }

    #[test]
    fn transfer_connection_requires_host_and_user() {
        let creds = NonSshCredentials::new();
        assert!(transfer_connection(&json!({ "username": "dev" }), &creds, false).is_err());
        assert!(transfer_connection(&json!({ "host": "h", "username": " " }), &creds, false).is_err());
    }

    #[test]
    fn transfer_connection_accepts_password_and_key_auth() {
        let creds = NonSshCredentials::new();
        let password = transfer_connection(&base(), &creds, false).expect("password connection");
        assert_eq!(password.host, "example.test");
        assert_eq!(password.port, 22);
        assert_eq!(password.username, "dev");
        assert_eq!(password.authentication.method_name(), "password");

        let key = transfer_connection(
            &json!({
                "host": "example.test",
                "username": "dev",
                "authentication": "private-key",
                "privateKeyPath": "/home/dev/.ssh/id_ed25519",
            }),
            &creds,
            false,
        )
        .expect("key connection");
        assert_eq!(key.authentication.method_name(), "private-key");
        assert_eq!(key.private_key_path, "/home/dev/.ssh/id_ed25519");
    }

    #[test]
    fn transfer_connection_reuses_stored_desktop_credentials() {
        let creds = NonSshCredentials::new();
        creds.store("rdp-1", "LSXD", "windows-password", "rdp");
        // No form password: the same host/user retries with the stored
        // RDP/VNC password instead of asking again.
        let connection = transfer_connection(
            &json!({
                "connectionId": "rdp-1",
                "host": "192.168.6.83",
                "username": "LSXD",
                "authentication": "password",
            }),
            &creds,
            true,
        )
        .expect("stored credentials");
        assert_eq!(connection.password, "windows-password");
        // A different username must not receive the stored password.
        assert!(transfer_connection(
            &json!({
                "connectionId": "rdp-1",
                "host": "192.168.6.83",
                "username": "someone-else",
                "authentication": "password",
            }),
            &creds,
            true,
        )
        .is_err());
    }

    #[test]
    fn transfer_connection_rejects_missing_password() {
        let creds = NonSshCredentials::new();
        let mut params = base();
        params["password"] = json!("");
        assert!(transfer_connection(&params, &creds, false).is_err());
    }

    #[test]
    fn transfer_connection_defaults_to_port_22() {
        let creds = NonSshCredentials::new();
        let mut params = base();
        params.as_object_mut().unwrap().remove("port");
        let connection = transfer_connection(&params, &creds, false).expect("connection");
        assert_eq!(connection.port, 22);
    }
}
