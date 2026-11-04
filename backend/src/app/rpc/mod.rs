//! RPC dispatch: route by method namespace into the domain handlers.

use super::actions::*;
use super::params::*;
use crate::platform::paths::*;
use super::*;

pub(super) mod agent;
pub(super) mod automation;
pub(super) mod connections;
pub(super) mod docker;
pub(super) mod docker_files;
pub(super) mod docker_stats;
pub(super) mod exec;
pub(super) mod filesystem;
pub(super) mod forwarding;
pub(super) mod import;
pub(super) mod local;
pub(super) mod mcp;
pub(super) mod metrics;
pub(super) mod otp;
pub(super) mod recording;
pub(super) mod remote_desktop;
pub(super) mod remote_files;
pub(super) mod security;
pub(super) mod serial;
pub(super) mod sessions;
pub(super) mod settings;
pub(super) mod sftp;
pub(super) mod sudo;
pub(super) mod telnet;
pub(super) mod terminal;
pub(super) mod transfers;
pub(super) mod tunnels;

impl Plugin {
    pub(super) fn handle_request(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        let mut segments = method.split('/');
        let namespace = segments.next().unwrap_or_default();
        let group = segments.next().unwrap_or_default();
        match namespace {
            "contextMenu" => self.handle_tunnels(method, params, emitter),
            "otp" => self.handle_otp(method, params, emitter),
            "task" | "triggers" => self.handle_automation(method, params, emitter),
            "connection" | "keys" => self.handle_connections(method, params, emitter),
            "completion" => self.handle_exec(method, params, emitter),
            "workbench" => self.handle_sessions(method, params, emitter),
            "ssh" => match group {
                "session" | "sessions" | "status" => self.handle_sessions(method, params, emitter),
                "forward" => self.handle_forwarding(method, params, emitter),
                "exec" => self.handle_exec(method, params, emitter),
                "terminal" | "batchBar" => self.handle_terminal(method, params, emitter),
                "metrics" | "processes" => self.handle_metrics(method, params, emitter),
                "recording" => self.handle_recording(method, params, emitter),
                "agent" => self.handle_agent(method, params, emitter),
                "alert" | "audit" => self.handle_security(method, params, emitter),
                "settings" | "quickCommands" | "highlightRules" => {
                    self.handle_settings(method, params, emitter)
                }
                "host-key" | "knownHosts" => self.handle_connections(method, params, emitter),
                _ => Err(format!("Method not found: {method}")),
            },
            "local" => match group {
                "metrics" => self.handle_metrics(method, params, emitter),
                "preferences" => self.handle_settings(method, params, emitter),
                "terminal" | "session" | "recording" | "shells" | "capabilities" | "saveFile"
                | "wallpaper" | "fs" | "reveal" | "open" | "open-with" | "editors" | "pointer" => {
                    self.handle_local(method, params, emitter)
                }
                _ => Err(format!("Method not found: {method}")),
            },
            "telnet" => self.handle_telnet(method, params, emitter),
            "serial" => self.handle_serial(method, params, emitter),
            "vnc" | "rdp" => self.handle_remote_desktop(method, params, emitter),
            "filetransfer" => self.handle_remote_files(method, params, emitter),
            "sftp" => match group {
                "upload" | "download" | "transfer" => {
                    self.handle_transfers(method, params, emitter)
                }
                _ => self.handle_sftp(method, params, emitter),
            },
            "sudo" => match group {
                "upload" | "download" => self.handle_transfers(method, params, emitter),
                _ => self.handle_sudo(method, params, emitter),
            },
            "watch" | "filesystem" => self.handle_filesystem(method, params, emitter),
            "docker" => self.handle_docker(method, params, emitter),
            "mcp" => self.handle_mcp(method, params, emitter),
            "import" => self.handle_import(method, params, emitter),
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
