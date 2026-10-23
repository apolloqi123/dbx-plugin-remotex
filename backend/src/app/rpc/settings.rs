//! Settings RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_settings(
        &self,
        method: &str,
        params: Value,
        _emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "ssh/settings/get" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime
                    .block_on(self.ssh.settings_get(session_id, &params))
            }

            "ssh/settings/set" => {
                let session_id = required_string(&params, "sessionId")?;
                self.runtime
                    .block_on(self.ssh.settings_set(session_id, &params))
            }

            "ssh/quickCommands/list" => Ok(self.ssh.quick_commands_list()),

            "ssh/quickCommands/save" => self.ssh.quick_commands_save(&params),

            "ssh/quickCommands/delete" => {
                let id = required_string(&params, "id")?;
                self.ssh.quick_commands_delete(id)
            }

            "ssh/highlightRules/list" => Ok(self.ssh.highlight_rules_list()),

            "ssh/highlightRules/save" => self.ssh.highlight_rules_save(&params),

            "ssh/highlightRules/delete" => {
                let id = required_string(&params, "id")?;
                self.ssh.highlight_rules_delete(id)
            }

            // 插件级 UI 偏好（下载目录、「每次询问」等）：工作台 iframe 是
            // sandbox="allow-scripts"（opaque origin），localStorage 不可用，
            // sidecar 的 preferences.json 是唯一持久存储。固定键白名单。
            "local/preferences/get" => Ok(crate::core::preferences::load_preferences(
                &plugin_data_dir(),
            )),

            "local/preferences/set" => {
                let result =
                    crate::core::preferences::save_preferences(&plugin_data_dir(), &params);
                // Keep the X11 fast-path flag in lockstep with the file.
                let prefs = crate::core::preferences::load_preferences(&plugin_data_dir());
                crate::features::forwarding::x11::set_enabled(
                    prefs.get("x11_forwarding").and_then(Value::as_bool) == Some(true),
                );
                // 自动录制快速标志同样与文件保持同步（对之后 open 的会话生效）。
                crate::features::recording::set_auto_record(
                    prefs.get("auto_record").and_then(Value::as_bool) == Some(true),
                );
                result
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
