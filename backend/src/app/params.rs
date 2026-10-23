//! Shared request-parameter parsing helpers.

use super::*;

pub(super) fn required_string<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Missing {key}"))
}

pub(super) fn required_u32(params: &Value, key: &str) -> Result<u32, String> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("Invalid {key}"))
}

pub(super) fn bounded_bytes(params: &Value, key: &str, default: usize) -> usize {
    params
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(default)
        .clamp(1, 1024 * 1024)
}

/// Host API 1.1 passes `operationId` to correlate connection lifecycle calls;
/// on Host API 1.0 it is absent, so a locally generated id is used instead.
/// The id only needs to stay stable between a challenge prompt and its resolve.
pub(super) fn operation_id(params: &Value) -> String {
    params
        .get("operationId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

pub(super) fn optional_u64(params: &Value, key: &str, default: u64) -> u64 {
    params.get(key).and_then(Value::as_u64).unwrap_or(default)
}

pub(super) fn content_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "txt" | "md" | "log" | "json" | "yaml" | "yml" | "toml" | "rs" | "ts" | "js" => {
            "text/plain"
        }
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => "application/octet-stream",
    }
}

pub(super) fn parse<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Invalid request parameters: {error}"))
}
