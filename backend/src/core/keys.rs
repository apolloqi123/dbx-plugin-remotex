//! Local SSH key discovery and known-hosts management, mirroring tiny-rdm.

mod discovery;
mod known_hosts;

#[cfg(test)]
mod tests;

pub use discovery::*;
pub use known_hosts::*;

// Registration for backend/src/main.rs (add `mod keys;` next to the other
// module declarations, then these arms inside `Plugin::handle_request`):
//
// ```text
// "keys/discover" => {
//     let keys = crate::core::keys::discover()?;
//     Ok(json!({ "keys": keys }))
// }
// "ssh/knownHosts/list" => {
//     Ok(crate::core::keys::list_known_hosts(&plugin_data_dir())?)
// }
// "ssh/knownHosts/remove" => {
//     let host = required_string(&params, "host")?;
//     let port = params
//         .get("port")
//         .and_then(Value::as_u64)
//         .and_then(|value| u16::try_from(value).ok())
//         .filter(|value| *value > 0)
//         .unwrap_or(22);
//     let removed = crate::core::keys::remove_known_host(&plugin_data_dir(), host, port)?;
//     Ok(json!({ "success": true, "removed": removed }))
// }
// ```
