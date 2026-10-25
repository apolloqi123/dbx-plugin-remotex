//! Metrics support.

use super::*;

/// Extracts the raw text of a `--marker--` section: everything after the
/// marker line until the next `--…--` marker or end of output.
pub(crate) fn section_of<'a>(output: &'a str, marker: &str) -> &'a str {
    let mut start: Option<usize> = None;
    let mut cursor = 0;
    for line in output.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.len() > 4 && trimmed.starts_with("--") && trimmed.ends_with("--") {
            if let Some(begin) = start {
                return &output[begin..cursor];
            }
            if trimmed == marker {
                start = Some(cursor + line.len());
            }
        }
        cursor += line.len();
    }
    match start {
        Some(begin) => &output[begin..],
        None => "",
    }
}

/// Parses the collector output into the metrics JSON object: the base
/// CPU/memory/disk document plus the `network`, `processes`, `topMemory` and
/// per-disk `inodeUsePercent` extensions. Pure so it can be unit-tested
/// without a server.
pub fn parse_metrics_output(output: &str) -> serde_json::Value {
    let mut root = crate::sessions::ssh::exec::parse_metrics_output(output);
    if let Some(object) = root.as_object_mut() {
        let network = serde_json::to_value(parse_network_samples(output)).unwrap_or_default();
        object.insert("network".to_string(), network);
        let processes = serde_json::to_value(parse_processes(output)).unwrap_or_default();
        object.insert("processes".to_string(), processes);
        let top_memory = serde_json::to_value(parse_process_lines(section_of(output, "--psm--")))
            .unwrap_or_default();
        object.insert("topMemory".to_string(), top_memory);
        // Distribution identification (IMPL_PLAN §1.5): `osId` / `osPretty`
        // are omitted entirely when the host has no readable os-release —
        // old callers and the frontend treat absence as "no badge".
        let (os_id, os_pretty) = parse_os_release(section_of(output, "--os--"));
        if let Some(id) = os_id {
            object.insert("osId".to_string(), serde_json::json!(id));
        }
        if let Some(pretty) = os_pretty {
            object.insert("osPretty".to_string(), serde_json::json!(pretty));
        }
    }
    merge_inode_usage(&mut root, section_of(output, "--dfi--"));
    root
}

/// Strips one optional layer of single or double quotes from an os-release
/// value (shell-style quoting, per the os-release spec).
pub(crate) fn unquote_os_release_value(value: &str) -> &str {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

/// Parses `ID=` and `PRETTY_NAME=` from an os-release style text into
/// `(osId, osPretty)`. `ID_LIKE` never matches the `ID=` prefix; CRLF and
/// quoted values are tolerated; blank or missing values yield `None` so the
/// metrics payload omits the fields instead of writing empty strings.
pub fn parse_os_release(text: &str) -> (Option<String>, Option<String>) {
    let mut os_id: Option<String> = None;
    let mut os_pretty: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("ID=") {
            if os_id.is_none() {
                let parsed = unquote_os_release_value(value);
                if !parsed.is_empty() {
                    os_id = Some(parsed.to_string());
                }
            }
        } else if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            if os_pretty.is_none() {
                let parsed = unquote_os_release_value(value);
                if !parsed.is_empty() {
                    os_pretty = Some(parsed.to_string());
                }
            }
        }
    }
    (os_id, os_pretty)
}

/// Merges per-mount inode usage into the base `disks` array as an additive
/// `inodeUsePercent` field; mounts without inode data stay untouched.
pub(crate) fn merge_inode_usage(root: &mut serde_json::Value, inode_text: &str) {
    let usage = parse_inode_usage(inode_text);
    if usage.is_empty() {
        return;
    }
    let Some(disks) = root
        .get_mut("disks")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for disk in disks.iter_mut() {
        let Some(mount) = disk.get("mount").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some((_, percent)) = usage.iter().find(|(entry_mount, _)| entry_mount == mount) else {
            continue;
        };
        if let Some(object) = disk.as_object_mut() {
            object.insert("inodeUsePercent".to_string(), serde_json::json!(percent));
        }
    }
}

/// Parses the `df -iP` inode section into `(mount, usePercent)` rows.
/// Dialects differ in where the percent column sits (GNU prints `IUse%`
/// fourth, busybox third), so the last `%`-suffixed token before the mount
/// column wins; header rows have none and are skipped.
pub fn parse_inode_usage(text: &str) -> Vec<(String, f64)> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // filesystem <counts...> use% mount — at least 5 columns.
        if fields.len() < 5 {
            continue;
        }
        let Some(percent) = fields
            .iter()
            .rev()
            .skip(1)
            .find_map(|field| parse_percent(field))
        else {
            continue;
        };
        rows.push((fields[fields.len() - 1].to_string(), percent));
    }
    rows
}

pub(crate) fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect()
    }
}
