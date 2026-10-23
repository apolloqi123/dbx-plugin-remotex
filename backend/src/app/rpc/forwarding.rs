//! Forwarding RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_forwarding(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "ssh/forward/list" => Ok(self.ssh.forward_list(&params)),

            "ssh/forward/interfaces" => {
                Ok(crate::features::forwarding::forward::local_interface_rows())
            }

            "ssh/forward/start" => self
                .runtime
                .block_on(self.ssh.forward_start(&params, emitter.clone())),

            "ssh/forward/stop" => {
                let id = required_string(&params, "id")?;
                self.runtime.block_on(self.ssh.forward_stop(id))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
