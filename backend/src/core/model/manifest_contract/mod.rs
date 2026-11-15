use super::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

mod fields;
mod form;
mod localization;
mod triggers;

fn manifest() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../manifest.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read manifest at {}: {error}", path.display()));
    serde_json::from_str(&raw).expect("manifest.json must be valid JSON")
}

fn connection_fields() -> Vec<Value> {
    manifest()["contributions"]
        .as_array()
        .expect("contributions array")
        .iter()
        .find(|entry| entry["type"] == "connection-provider")
        .expect("connection-provider contribution")["fields"]
        .as_array()
        .expect("fields array")
        .clone()
}

fn field(key: &str) -> Value {
    connection_fields()
        .into_iter()
        .find(|entry| entry["key"].as_str() == Some(key))
        .unwrap_or_else(|| panic!("manifest field '{key}' missing"))
}

fn condition_one_of(entry: &Value, condition: &str) -> Option<Vec<String>> {
    entry[condition]["one_of"].as_array().map(|values| {
        values
            .iter()
            .map(|value| value.as_str().unwrap_or_default().to_string())
            .collect()
    })
}

fn condition_field<'a>(entry: &'a Value, condition: &str) -> Option<&'a str> {
    entry[condition]["field"].as_str()
}

/// Dialog value of a field: the stored/typed value, else the manifest default.
fn form_value(values: &BTreeMap<String, Value>, entry: &Value) -> Option<Value> {
    let key = entry["key"].as_str().unwrap_or_default();
    match values.get(key) {
        Some(value) => Some(value.clone()),
        None => entry
            .get("default")
            .filter(|value| !value.is_null())
            .cloned(),
    }
}

fn form_condition_matches(
    condition: &Value,
    values: &BTreeMap<String, Value>,
    fields: &[Value],
) -> bool {
    let Some(key) = condition["field"].as_str() else {
        return false;
    };
    let Some(target) = fields
        .iter()
        .find(|entry| entry["key"].as_str() == Some(key))
    else {
        return false;
    };
    let Some(value) = form_value(values, target) else {
        return false;
    };
    let text = match value {
        Value::Null => return false,
        Value::String(text) => text,
        other => other.to_string(),
    };
    if text.trim().is_empty() {
        return false;
    }
    condition["one_of"].as_array().is_some_and(|one_of| {
        one_of
            .iter()
            .any(|option| option.as_str() == Some(text.as_str()))
    })
}

fn form_field_visible(entry: &Value, values: &BTreeMap<String, Value>, fields: &[Value]) -> bool {
    let mut seen = vec![entry["key"].as_str().unwrap_or_default().to_string()];
    let mut current = entry;
    loop {
        let Some(condition) = current.get("visible_when").filter(|value| !value.is_null()) else {
            return true;
        };
        if !form_condition_matches(condition, values, fields) {
            return false;
        }
        let Some(target_key) = condition["field"].as_str() else {
            return true;
        };
        if seen.iter().any(|key| key == target_key) {
            return true;
        }
        seen.push(target_key.to_string());
        let Some(target) = fields
            .iter()
            .find(|candidate| candidate["key"].as_str() == Some(target_key))
        else {
            return true;
        };
        current = target;
    }
}

fn form_field_required(entry: &Value, values: &BTreeMap<String, Value>, fields: &[Value]) -> bool {
    if entry["required"].as_bool().unwrap_or(false) {
        return true;
    }
    match entry.get("required_when").filter(|value| !value.is_null()) {
        None => false,
        Some(condition) => form_condition_matches(condition, values, fields),
    }
}

fn form_value_present(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    }
}

/// The field the dialog would block "save" on, if any.
fn form_blocking_field(fields: &[Value], values: &BTreeMap<String, Value>) -> Option<String> {
    fields
        .iter()
        .find(|entry| {
            form_field_visible(entry, values, fields)
                && form_field_required(entry, values, fields)
                && !form_value_present(form_value(values, entry).as_ref())
        })
        .map(|entry| entry["key"].as_str().unwrap_or_default().to_string())
}

/// The lifecycle `connection` object a save would persist, built the way the
/// host builds it (config → `external_config`, secret → `connection_secrets`
/// with empty values removed, identity/password bindings hoisted).
fn connection_payload(fields: &[Value], values: &BTreeMap<String, Value>) -> Value {
    let mut external = serde_json::Map::new();
    let mut secrets = serde_json::Map::new();
    let mut name = String::from("SSH server");
    let mut host = String::new();
    let mut port: u64 = 0;
    let mut username = String::new();
    let mut password = String::new();
    let text = |value: Option<Value>| -> String {
        match value {
            Some(Value::String(text)) => text,
            Some(other) => other.to_string(),
            None => String::new(),
        }
    };
    for entry in fields {
        let key = entry["key"].as_str().unwrap_or_default().to_string();
        let value = form_value(values, entry);
        match entry["binding"].as_str().unwrap_or_default() {
            "config" => match value {
                None | Some(Value::Null) => {
                    external.remove(&key);
                }
                Some(value) => {
                    external.insert(key.clone(), value);
                }
            },
            "secret" => match value {
                None | Some(Value::Null) => {
                    secrets.remove(&key);
                }
                Some(Value::String(value)) if value.is_empty() => {
                    secrets.remove(&key);
                }
                Some(value) => {
                    secrets.insert(key.clone(), Value::String(text(Some(value))));
                }
            },
            "name" => name = text(value),
            "host" => host = text(value),
            "port" => {
                port = value
                    .as_ref()
                    .and_then(Value::as_u64)
                    .map(|port| port.min(u64::from(u16::MAX)))
                    .unwrap_or(0);
            }
            "username" => username = text(value),
            "password" => password = text(value),
            _ => {}
        }
    }
    serde_json::json!({
        "id": "form-matrix",
        "name": name,
        "host": host,
        "port": port,
        "username": username,
        "password": password,
        "external_config": external,
        "connection_secrets": secrets,
    })
}
