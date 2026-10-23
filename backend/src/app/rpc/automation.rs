//! Automation RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_automation(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "triggers/validate" => {
                let raw = params.get("triggers");
                let format = trigger_input_format(raw);
                let secrets = params.get("secretSlots").and_then(Value::as_object);
                let parsed = crate::features::automation::triggers::parse_triggers(raw, &|key| {
                    secrets
                        .and_then(|values| values.get(key))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })?;
                Ok(json!({
                    "valid": true,
                    "enabled": parsed.is_some(),
                    "format": format,
                    "stages": parsed.as_ref().map(|config| config.stages.len()).unwrap_or(0),
                }))
            }

            // Scheduler Task Provider（io.dbx.sshx.tasks）：冻结契约只允许这
            // 五个固定方法（ADR §6.2）；禁止 ssh/runScheduledCommand 之类的
            // provider-specific RPC。凭据只经既有 connection/connect 生命周
            // 期进入连接注册表，task config 只携带 connectionId。
            "task/validate" => {
                crate::features::automation::task_provider::TaskProvider::validate_request(&params)
            }

            "task/execute" => {
                self.runtime
                    .block_on(self.tasks.execute(&self.ssh, &params, emitter.clone()))
            }

            "task/start" => {
                let tasks = self.tasks.clone();
                let ssh = self.ssh.clone();
                self.runtime
                    .block_on(tasks.start_resident(ssh, params, emitter.clone()))
            }

            "task/stop" => self.runtime.block_on(self.tasks.stop(&params)),

            "task/status" => self.tasks.status(&params),
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
