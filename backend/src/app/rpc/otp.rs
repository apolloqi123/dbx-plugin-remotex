//! Otp RPC handlers.

use super::*;

impl Plugin {
    pub(super) fn handle_otp(
        &self,
        method: &str,
        params: Value,
        _emitter: &PluginEmitter,
    ) -> Result<Value, String> {
        match method {
            "otp/list" => {
                let store = crate::core::otp_store::load_store(&plugin_data_dir());
                Ok(json!({
                    "entries": crate::core::otp_store::list_views(&store),
                    "bindings": crate::core::otp_store::binding_views(&store),
                }))
            }

            "otp/save" => {
                let data_dir = plugin_data_dir();
                let vault = crate::core::otp_store::vault_for(&data_dir);
                let mut store = crate::core::otp_store::load_store(&data_dir);
                let (entry, created) =
                    crate::core::otp_store::save_entry(&mut store, &params, &vault)?;
                crate::core::otp_store::save_store(&data_dir, &store)?;
                Ok(
                    json!({ "entry": crate::core::otp_store::entry_view(&entry), "created": created }),
                )
            }

            "otp/delete" => {
                let data_dir = plugin_data_dir();
                let mut store = crate::core::otp_store::load_store(&data_dir);
                let id = params.get("id").and_then(Value::as_str).unwrap_or_default();
                let deleted = crate::core::otp_store::delete_entry(&mut store, id);
                if deleted {
                    crate::core::otp_store::save_store(&data_dir, &store)?;
                }
                Ok(json!({ "deleted": deleted }))
            }

            "otp/bind" => {
                let data_dir = plugin_data_dir();
                let mut store = crate::core::otp_store::load_store(&data_dir);
                let connection_id = params
                    .get("connectionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let entry_id = params
                    .get("entryId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                crate::core::otp_store::bind(&mut store, connection_id, entry_id)?;
                crate::core::otp_store::save_store(&data_dir, &store)?;
                Ok(json!({ "bound": true }))
            }

            "otp/unbind" => {
                let data_dir = plugin_data_dir();
                let mut store = crate::core::otp_store::load_store(&data_dir);
                let connection_id = params
                    .get("connectionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let removed = crate::core::otp_store::unbind(&mut store, connection_id);
                if removed {
                    crate::core::otp_store::save_store(&data_dir, &store)?;
                }
                Ok(json!({ "removed": removed }))
            }

            "otp/generate" => {
                let data_dir = plugin_data_dir();
                let store = crate::core::otp_store::load_store(&data_dir);
                let id = params
                    .get("entryId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let entry = store
                    .entries
                    .iter()
                    .find(|entry| entry.id == id)
                    .ok_or_else(|| "otp entry not found".to_string())?
                    .clone();
                let vault = crate::core::otp_store::vault_for(&data_dir);
                let secret = crate::core::otp_store::get_decrypted_secret(&vault, &entry)
                    .ok_or_else(|| "otp secret unavailable".to_string())?;
                if entry.otp_type == "hotp" {
                    let counter = entry.counter.unwrap_or(0);
                    let code =
                        crate::core::otp::hotp(entry.algorithm, &secret, counter, entry.digits);
                    let mut store = store;
                    if let Some(entry) = store.entries.iter_mut().find(|e| e.id == id) {
                        entry.counter = Some(counter + 1);
                    }
                    crate::core::otp_store::save_store(&data_dir, &store)?;
                    Ok(
                        json!({ "code": format!("{code:0width$}", width = entry.digits as usize), "hotp": true }),
                    )
                } else {
                    match crate::core::otp_store::take_window_code(
                        &entry.id,
                        &secret,
                        entry.algorithm,
                        entry.digits,
                        entry.period,
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0),
                    ) {
                        Ok(window) => Ok(json!({
                            "code": window.code,
                            "remainingSeconds": window.remaining_secs,
                        })),
                        Err(used) => Ok(json!({
                            "code": null,
                            "reused": true,
                            "remainingSeconds": used.remaining_secs,
                        })),
                    }
                }
            }

            "otp/import-qr" => {
                let image = params
                    .get("imageBase64")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let parsed = crate::core::otp_store::decode_otpauth_qr(image)?;
                Ok(json!({
                    "otpType": parsed.otp_type,
                    "issuer": parsed.issuer,
                    "label": parsed.label,
                    "secretBase32": parsed.secret_base32,
                    "algorithm": parsed.algorithm.as_str(),
                    "digits": parsed.digits,
                    "period": parsed.period,
                    "counter": parsed.counter,
                }))
            }
            _ => Err(format!("Method not found: {method}")),
        }
    }
}
