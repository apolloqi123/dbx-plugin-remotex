//! Shared parsing helpers for the GPU / NPU collectors.

use super::*;

/// Card-level values shared by every chip of one physical card (parsed once
/// per card row, reused for each following chip row). The NPU index itself
/// comes from the chip row's first column, which npu-smi keeps in sync.
pub(super) struct CardState {
    pub(super) name: String,
    pub(super) health: Option<String>,
    pub(super) power: Option<f64>,
    pub(super) temperature: Option<f64>,
}

/// Raw text between two marker lines. A missing `END` marker runs to the end
/// of the output; a missing `BEGIN` yields nothing. Marker lines are
/// compared trimmed so CRLF output still matches.
pub(super) fn marker_section<'a>(output: &'a str, begin: &str, end: &str) -> &'a str {
    let mut start: Option<usize> = None;
    let mut cursor = 0;
    for line in output.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(begin_offset) = start {
            if trimmed == end {
                return &output[begin_offset..cursor];
            }
        } else if trimmed == begin {
            start = Some(cursor + line.len());
        }
        cursor += line.len();
    }
    match start {
        Some(begin_offset) => &output[begin_offset..],
        None => "",
    }
}

/// Splits one CSV row on commas outside double quotes; a `""` pair inside a
/// quoted field collapses to a literal quote. nvidia-smi quotes the GPU name
/// when it contains a comma (`"NVIDIA GeForce RTX 3090, 24GB"`).
pub(super) fn split_csv_row(row: &str) -> Vec<String> {
    let mut fields: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            fields.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(c);
        }
    }
    fields.push(current.trim().to_string());
    fields
}

/// Trims the outer pipe columns of one npu-smi table row into cells
/// (`| a | b |` becomes `["a", "b"]`).
pub(super) fn pipe_cells(line: &str) -> Vec<String> {
    let columns: Vec<&str> = line.split('|').collect();
    if columns.len() < 3 {
        return Vec::new();
    }
    columns[1..columns.len() - 1]
        .iter()
        .map(|cell| cell.trim().to_string())
        .collect()
}

/// Device-table header first row (`| NPU Name | Health | Power… |`).
pub(super) fn is_card_header(cells: &[String]) -> bool {
    cells.first().is_some_and(|first| {
        let tokens: Vec<&str> = first.split_whitespace().collect();
        tokens.contains(&"NPU") && tokens.contains(&"Name")
    })
}

/// Device-table header second row (`| Chip Device | Bus-Id | Aicore… |`).
pub(super) fn is_chip_header(cells: &[String]) -> bool {
    cells.first().is_some_and(|first| {
        let tokens: Vec<&str> = first.split_whitespace().collect();
        tokens.contains(&"Chip") && tokens.contains(&"Device")
    })
}

/// Whitespace tokens of a table row's detail column (the last one).
pub(super) fn detail_tokens(cells: &[String]) -> Vec<&str> {
    cells
        .get(2)
        .map(|cell| cell.split_whitespace().collect())
        .unwrap_or_default()
}

/// `used / total` pair around the slash token of a detail column.
pub(super) fn slash_pair(tokens: &[&str]) -> Option<(Option<u64>, Option<u64>)> {
    let position = tokens
        .iter()
        .position(|token| *token == "/")
        .filter(|position| *position > 0)?;
    let used = tokens[position - 1].parse::<u64>().ok();
    let total = tokens
        .get(position + 1)
        .and_then(|token| token.parse::<u64>().ok());
    Some((used, total))
}

/// One process-table data row: pid first, memory last, NPU id second to
/// last, everything between is the (multi-token) process name.
pub(super) fn parse_npu_process_row(cells: &[String]) -> Option<(u64, String, u64, Option<u64>)> {
    let flattened = cells.join(" ");
    let tokens: Vec<&str> = flattened.split_whitespace().collect();
    if tokens.len() < 3 {
        return None;
    }
    let pid = tokens[0].parse::<u64>().ok()?;
    let mem = tokens[tokens.len() - 1].parse::<u64>().ok().map(mib_bytes);
    let npu_index = tokens[tokens.len() - 2].parse::<u64>().ok()?;
    let name = tokens[1..tokens.len() - 2].join(" ");
    Some((pid, name, npu_index, mem))
}

/// nvidia-smi marks unsupported counters as `N/A`, `[N/A]`,
/// `[Not Supported]`, `[Insufficient Permissions]` or an empty field; all of
/// them collapse to JSON null instead of failing the row.
fn is_unsupported(field: &str) -> bool {
    let value = field.trim();
    value.is_empty()
        || value.eq_ignore_ascii_case("n/a")
        || (value.starts_with('[') && value.ends_with(']'))
}

pub(super) fn optional_string(field: &str) -> Option<String> {
    let value = field.trim();
    if is_unsupported(value) {
        None
    } else {
        Some(value.to_string())
    }
}

pub(super) fn parse_optional_f64(field: &str) -> Option<f64> {
    if is_unsupported(field) {
        None
    } else {
        field.trim().parse::<f64>().ok()
    }
}

pub(super) fn parse_optional_u64(field: &str) -> Option<u64> {
    if is_unsupported(field) {
        None
    } else {
        field.trim().parse::<u64>().ok()
    }
}

/// MiB column value to a byte count (see [`MIB`]).
pub(super) fn mib_bytes(mib: u64) -> u64 {
    mib.saturating_mul(MIB)
}
