//! Sudo RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_sudo(
        &self,
        method: &str,
        params: Value,
        _emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "sudo/stat" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                self.runtime
                    .block_on(crate::features::security::sudo::fs::stat(
                        &self.ssh, session_id, path,
                    ))
            }

            "sudo/exists" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                let exists = self
                    .runtime
                    .block_on(crate::features::security::sudo::fs::exists(
                        &self.ssh, session_id, path,
                    ))?;
                Ok(json!({ "exists": exists }))
            }

            "sudo/touch" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                self.runtime
                    .block_on(crate::features::security::sudo::fs::touch(
                        &self.ssh, session_id, path,
                    ))?;
                Ok(json!({ "success": true }))
            }

            "sudo/listDir" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                self.runtime
                    .block_on(crate::features::security::sudo::fs::list_dir(
                        &self.ssh, session_id, path,
                    ))
            }

            "sudo/readFile" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                let offset = optional_u64(&params, "offset", 0);
                let length = optional_u64(&params, "length", 0);
                self.runtime
                    .block_on(crate::features::security::sudo::fs::read_file(
                        &self.ssh, session_id, path, offset, length,
                    ))
            }

            "sudo/writeFile" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                let data_base64 = required_string(&params, "dataBase64")?;
                self.runtime
                    .block_on(crate::features::security::sudo::fs::write_file(
                        &self.ssh,
                        session_id,
                        path,
                        data_base64,
                    ))?;
                Ok(json!({ "success": true }))
            }

            "sudo/mkdir" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                self.runtime
                    .block_on(crate::features::security::sudo::fs::mkdir(
                        &self.ssh, session_id, path,
                    ))?;
                Ok(json!({ "success": true }))
            }

            "sudo/remove" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                self.runtime
                    .block_on(crate::features::security::sudo::fs::remove(
                        &self.ssh, session_id, path,
                    ))?;
                Ok(json!({ "success": true }))
            }

            "sudo/removeAll" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                self.runtime
                    .block_on(crate::features::security::sudo::fs::remove_all(
                        &self.ssh, session_id, path,
                    ))?;
                Ok(json!({ "success": true }))
            }

            "sudo/chmod" => {
                let session_id = required_string(&params, "sessionId")?;
                let path = required_string(&params, "path")?;
                let mode = required_string(&params, "mode")?;
                self.runtime
                    .block_on(crate::features::security::sudo::fs::chmod(
                        &self.ssh, session_id, path, mode,
                    ))?;
                Ok(json!({ "success": true }))
            }

            "sudo/rename" => {
                let session_id = required_string(&params, "sessionId")?;
                let source = required_string(&params, "sourcePath")?;
                let target = required_string(&params, "targetPath")?;
                self.runtime
                    .block_on(crate::features::security::sudo::fs::rename(
                        &self.ssh, session_id, source, target,
                    ))?;
                Ok(json!({ "success": true }))
            }

            "sudo/profiles/list" => Ok(self.ssh.profiles_list()),

            "sudo/profiles/options" => Ok(self.ssh.profiles_options()),

            "sudo/profiles/reveal" => {
                let id = required_string(&params, "id")?;
                self.ssh.profiles_reveal(id)
            }

            "sudo/profiles/save" => self.runtime.block_on(self.ssh.profiles_save(&params)),

            "sudo/profiles/delete" => {
                let id = required_string(&params, "id")?;
                self.runtime.block_on(self.ssh.profiles_delete(id))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
