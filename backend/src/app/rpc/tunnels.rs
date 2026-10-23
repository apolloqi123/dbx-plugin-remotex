//! Tunnels RPC handlers.

use super::*;
use crate::core::plugin_ids::PLUGIN_ID;

impl Plugin {
    pub(super) fn handle_tunnels(
        &self,
        method: &str,
        params: Value,
        emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "contextMenu/resolve/manage-tunnels" => {
                if params.get("ownerPluginId").and_then(Value::as_str) != Some(PLUGIN_ID) {
                    return Ok(json!({"items": []}));
                }
                let connection_id = params
                    .pointer("/connection/id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or("connection.id is required")?;
                let profiles = crate::features::forwarding::tunnel::load_profiles(
                    &plugin_data_dir(),
                    connection_id,
                );
                let rows = self
                    .ssh
                    .forward_list(&json!({"connectionId": connection_id}));
                let rows = rows["forwards"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let locale = params.get("locale").and_then(Value::as_str).unwrap_or("en");
                Ok(crate::features::forwarding::tunnel::menu(
                    &profiles, rows, locale,
                ))
            }

            "contextMenu/manage-tunnels" => {
                let connection_id = params
                    .pointer("/connection/id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or("connection.id is required")?;
                let item_id = required_string(&params, "itemId")?;
                let profiles = crate::features::forwarding::tunnel::load_profiles(
                    &plugin_data_dir(),
                    connection_id,
                );
                let listed = self
                    .ssh
                    .forward_list(&json!({"connectionId": connection_id}));
                let rows = listed["forwards"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                if item_id == "stop-all" {
                    let ids: Vec<String> = rows
                        .iter()
                        .filter_map(|row| row["id"].as_str().map(str::to_owned))
                        .collect();
                    let mut stopped = 0;
                    for id in ids {
                        self.runtime.block_on(self.ssh.forward_stop(&id))?;
                        stopped += 1;
                    }
                    return Ok(json!({"message": format!("Stopped {stopped} tunnels")}));
                }
                let selected: Vec<_> = if item_id == "start-all" {
                    profiles.iter().collect()
                } else if let Some(id) = item_id.strip_prefix("toggle/") {
                    vec![profiles
                        .iter()
                        .find(|profile| profile.id == id)
                        .ok_or("Saved tunnel was not found")?]
                } else {
                    return Err("Unknown tunnel menu action".to_string());
                };
                let mut started = 0;
                let mut stopped = 0;
                let mut errors = Vec::new();
                for profile in selected {
                    if let Some(active) =
                        crate::features::forwarding::tunnel::active_row(profile, rows)
                    {
                        if item_id != "start-all" {
                            let id = active["id"].as_str().ok_or("Active tunnel has no id")?;
                            self.runtime.block_on(self.ssh.forward_stop(id))?;
                            stopped += 1;
                        }
                        continue;
                    }
                    let mut spec = profile.spec()?;
                    spec["connectionId"] = json!(connection_id);
                    match self
                        .runtime
                        .block_on(self.ssh.forward_start(&spec, emitter.clone()))
                    {
                        Ok(_) => started += 1,
                        Err(error) => errors.push(format!("{}: {error}", profile.route())),
                    }
                }
                if !errors.is_empty() {
                    return Err(errors.join("\n"));
                }
                Ok(json!({"message": format!("Started {started}, stopped {stopped} tunnels")}))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
