//! Agent RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_agent(
        &self,
        method: &str,
        params: Value,
        _emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "ssh/agent/resolve" => {
                let challenge_id = required_string(&params, "challengeId")?;
                let decision = required_string(&params, "decision")?;
                let command = params.get("command").and_then(Value::as_str);
                let remember = params
                    .get("remember")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.ssh
                    .resolve_agent_challenge(challenge_id, decision, command, remember)?;
                Ok(json!({ "success": true }))
            }

            "ssh/agent/mode/get" => {
                let connection_id = required_string(&params, "connectionId")?;
                self.runtime
                    .block_on(self.ssh.agent_mode_get(connection_id))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
