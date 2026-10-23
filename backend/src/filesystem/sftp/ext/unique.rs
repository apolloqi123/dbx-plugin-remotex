//! Extended SFTP operation layer.

use super::*;

/// Validates and bounds a caller-provided file name for the unique-name
/// probe: no path separators, no `.`/`..`, capped length.
pub(crate) fn clean_unique_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("File name is required".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("File name must not contain path separators".to_string());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("File name must not be a relative path component".to_string());
    }
    Ok(trimmed.chars().take(MAX_UNIQUE_NAME_CHARS).collect())
}

/// The n-th collision candidate: `(n)` inserted before the last extension
/// (`report.pdf` -> `report(1).pdf`); names without a usable extension get a
/// suffix instead (`report` -> `report(1)`, `.bashrc` -> `.bashrc(1)`).
pub(crate) fn unique_name_candidate(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(index) if index > 0 => format!("{}({n}){}", &name[..index], &name[index..]),
        _ => format!("{name}({n})"),
    }
}

/// Full probe order for a name: the plain name first, then the bounded
/// `name(1)..name(999)` collision candidates.
pub(crate) fn unique_name_candidates(name: &str) -> Vec<String> {
    let mut candidates = Vec::with_capacity(UNIQUE_NAME_PROBE_LIMIT as usize + 1);
    candidates.push(name.to_string());
    for n in 1..=UNIQUE_NAME_PROBE_LIMIT {
        candidates.push(unique_name_candidate(name, n));
    }
    candidates
}

/// latin-1 裸包探测用的候选序列：目录前缀按 wire 还原为字节，候选名（显示
/// 文本——`(n)` 是 ASCII 增量，编码次序无关）按 [`sftp_name::
/// latin1_encode_display`] 编码回字节。纯函数，供 [`rename_unique`] 与单测。
pub(crate) fn unique_name_raw_candidates(
    dir: &str,
    name: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let clean = clean_unique_name(name)?;
    let raw_dir = sftp_name::unescape_wire(dir);
    Ok(unique_name_candidates(&clean)
        .into_iter()
        .map(|candidate| {
            let raw =
                sftp_name::join_raw_path(&raw_dir, &sftp_name::latin1_encode_display(&candidate));
            (candidate, raw)
        })
        .collect())
}

/// Pure decision core of [`rename_unique`]: the first candidate the `exists`
/// probe reports as free, paired with whether a `(n)` rename was needed.
pub(crate) fn next_unique_name<F>(name: &str, mut exists: F) -> Option<(String, bool)>
where
    F: FnMut(&str) -> bool,
{
    for (index, candidate) in unique_name_candidates(name).into_iter().enumerate() {
        if !exists(&candidate) {
            return Some((candidate, index > 0));
        }
    }
    None
}
