//! MCP exec helpers.

use super::*;

mod bg;
mod command;
mod multi;
mod profile;
mod terminal;

#[allow(unused_imports)]
pub(crate) use bg::*;
#[allow(unused_imports)]
pub(crate) use command::*;
#[allow(unused_imports)]
pub(crate) use multi::*;
#[allow(unused_imports)]
pub(crate) use profile::*;
#[allow(unused_imports)]
pub(crate) use terminal::*;

impl McpState {
    pub(crate) async fn run_tool(
        &self,
        name: &str,
        arguments: &Value,
        emitter: Option<&PluginEmitter>,
    ) -> Result<Value, String> {
        match name {
            "ssh_close" => self.ssh_close(arguments).await,
            // Local↔remote transfers validate their local side before any
            // connection I/O; validation refusals must keep the pooled
            // connection (they are not transport failures), so these tools
            // own their drop-on-transport-error semantics instead of the
            // blanket cleanup below.
            "sftp_upload" => self.sftp_upload_tool(arguments).await,
            "sftp_download" => self.sftp_download_tool(arguments).await,
            "ssh_list_known_hosts" => {
                let verifier = HostKeyVerifier::new(self.runtime.known_hosts_path());
                let entries: Vec<Value> = verifier
                    .list_known_hosts()?
                    .into_iter()
                    .map(|entry| {
                        json!({
                            "hostField": entry.host_field,
                            "keyType": entry.key_type,
                            "fingerprint": entry.fingerprint,
                            "comment": entry.comment,
                        })
                    })
                    .collect();
                Ok(json!({ "knownHosts": entries }))
            }
            "ssh_list_connections" => self.ssh_list_connections_tool().await,
            "ssh_remove_known_host" => {
                let host = required_str(arguments, "host")?;
                let port = arg_port(arguments)?;
                let verifier = HostKeyVerifier::new(self.runtime.known_hosts_path());
                let removed = verifier.remove_known_host(host, port)?;
                Ok(json!({ "host": host, "port": port, "removed": removed }))
            }
            _ => {
                let mut result = match name {
                    "ssh_exec" | "ssh_exec_sudo" => {
                        self.ssh_exec_tool(name, arguments, emitter).await
                    }
                    "ssh_run_bg" => self.ssh_run_bg_tool(arguments).await,
                    "ssh_terminal_input" => self.ssh_terminal_input_tool(arguments).await,
                    "ssh_multi_exec" => self.ssh_multi_exec_tool(arguments).await,
                    "ssh_task_status" => self.ssh_task_status_tool(arguments).await,
                    other => self.sftp_tool(other, arguments).await,
                };
                if result.is_err() {
                    // Drop the cached connection on failure so the next call
                    // reconnects with fresh credentials instead of reusing a
                    // broken transport.
                    self.drop_connection(arguments).await;
                    // Flapping-network recovery: when the pooled transport
                    // died BEFORE the remote command could start (channel
                    // open/exec refused on a dead connection), retry once on
                    // the fresh connection. Pre-exec failures cannot
                    // double-execute the command; post-exec transport deaths
                    // stay terminal because the command may already have run.
                    if matches!(name, "ssh_exec" | "ssh_exec_sudo" | "ssh_run_bg") {
                        let retryable = result
                            .as_ref()
                            .err()
                            .map(|error| exec::is_pre_exec_transport_error(error))
                            .unwrap_or(false);
                        if retryable {
                            result = match name {
                                "ssh_exec" | "ssh_exec_sudo" => {
                                    self.ssh_exec_tool(name, arguments, emitter).await
                                }
                                _ => self.ssh_run_bg_tool(arguments).await,
                            };
                        }
                    }
                }
                result
            }
        }
    }
}
