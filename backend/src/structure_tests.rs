//! Structural guards: file budgets and layer direction.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_SOURCE_LINES: usize = 600;
const MAX_TEST_LINES: usize = 1_200;

const LINE_ALLOWLIST: &[(&str, usize)] = &[];
const UPPER_LAYERS: &[&str] = &["app", "features", "filesystem", "sessions"];

fn count_lines(text: &str) -> usize {
    text.lines().count()
}

fn is_test_file(rel: &str) -> bool {
    rel.ends_with("tests.rs")
        || rel.ends_with("_tests.rs")
        || rel == "test.rs"
        || rel.ends_with("/test.rs")
        || rel.ends_with("_test.rs")
        || rel == "test_support.rs"
        || rel.ends_with("/test_support.rs")
        || rel.contains("/tests/")
        // `core/model/manifest_contract` is declared under `#[cfg(test)]`.
        || rel.contains("/manifest_contract/")
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source dir") {
        let path = entry.expect("read entry").path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext.to_string_lossy() == "rs")
        {
            out.push(path);
        }
    }
}

/// Returns every `crate::<layer>` / `super::<layer>` hit. The match is
/// token-based, so whitespace variants (`crate :: sessions`) and grouped
/// imports (`crate::{features::x}`) are caught; comments and string literals
/// are stripped first.
fn layer_refs(text: &str, banned: &[&str]) -> Vec<String> {
    let cleaned = strip_comments_and_strings(text);
    let tokens = tokenize(&cleaned);
    let mut hits = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if token != "crate" && token != "super" {
            continue;
        }
        let mut cursor = index + 1;
        while cursor < tokens.len() && cursor <= index + 64 {
            match tokens[cursor].as_str() {
                ":" | "{" | "}" | "," => cursor += 1,
                ";" | "(" | ")" | "=" => break,
                segment => {
                    if banned.contains(&segment) {
                        hits.push(format!("{token}::{segment}"));
                    }
                    cursor += 1;
                }
            }
        }
    }
    hits.sort();
    hits.dedup();
    hits
}

/// `#[path = "../sessions/x.rs"]` escapes the module tree without a path token.
fn path_attribute_escapes(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("#[path") && line.contains("\"..")
    })
}

fn strip_comments_and_strings(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '/' if chars.get(index + 1) == Some(&'/') => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
            }
            '/' if chars.get(index + 1) == Some(&'*') => {
                index += 2;
                while index + 1 < chars.len() && !(chars[index] == '*' && chars[index + 1] == '/') {
                    index += 1;
                }
                index = (index + 2).min(chars.len());
            }
            '"' => {
                index += 1;
                while index < chars.len() {
                    if chars[index] == '\\' {
                        index += 2;
                        continue;
                    }
                    if chars[index] == '"' {
                        index += 1;
                        break;
                    }
                    index += 1;
                }
            }
            '\'' => {
                // Skip char literals but keep lifetimes (`'a`).
                if chars.get(index + 2) == Some(&'\'') {
                    index += 3;
                } else if chars.get(index + 1) == Some(&'\\') && chars.get(index + 3) == Some(&'\'')
                {
                    index += 4;
                } else {
                    out.push('\'');
                    index += 1;
                }
            }
            character => {
                out.push(character);
                index += 1;
            }
        }
    }
    out
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() || character == '_' {
            current.push(character);
            continue;
        }
        if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
        if matches!(character, ':' | '{' | '}' | ',' | ';' | '(' | ')' | '=') {
            tokens.push(character.to_string());
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn collect_dir(manifest: &Path, rel: &str) -> Vec<(String, usize)> {
    let root = manifest.join(rel);
    let mut files = Vec::new();
    if root.is_dir() {
        collect_rs(&root, &mut files);
    }
    files
        .into_iter()
        .map(|path| {
            let rel = path
                .strip_prefix(manifest)
                .expect("relative path")
                .to_string_lossy()
                .replace('\\', "/");
            let lines = count_lines(&fs::read_to_string(&path).expect("read source"));
            (rel, lines)
        })
        .collect()
}

#[test]
fn classifies_test_files() {
    assert!(is_test_file("sessions/ssh/tests.rs"));
    assert!(is_test_file("core/model/manifest_contract_tests.rs"));
    assert!(is_test_file("features/mcp/tests/tools.rs"));
    assert!(is_test_file("sessions/ssh/connection/test.rs"));
    assert!(is_test_file("filesystem/sftp/raw/test_support.rs"));
    assert!(is_test_file("test.rs"));
    assert!(is_test_file("core/model/manifest_contract/fields.rs"));
    assert!(!is_test_file("features/latest.rs"));
    assert!(!is_test_file("sessions/ssh/mod.rs"));
    assert_eq!(count_lines("a\nb\n"), 2);
}

#[test]
fn sources_stay_within_line_budget() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = collect_dir(manifest, "src");
    files.extend(collect_dir(manifest, "examples"));
    let present: BTreeSet<&str> = files.iter().map(|(rel, _)| rel.as_str()).collect();
    let mut violations = Vec::new();
    for (rel, lines) in &files {
        let budget = if is_test_file(rel) {
            MAX_TEST_LINES
        } else {
            MAX_SOURCE_LINES
        };
        let allowed = LINE_ALLOWLIST
            .iter()
            .find(|(name, _)| name == rel)
            .map(|(_, limit)| *limit);
        let limit = allowed.unwrap_or(budget);
        if *lines > limit {
            violations.push(format!("{rel}: {lines} > {limit}"));
        }
        if let Some(limit) = allowed {
            if limit > budget && *lines <= budget {
                violations.push(format!(
                    "{rel}: allowlist entry is stale ({lines} <= {budget})"
                ));
            }
        }
    }
    for (name, _) in LINE_ALLOWLIST {
        if !present.contains(name) {
            violations.push(format!("{name}: allowlist entry no longer exists"));
        }
    }
    assert!(
        violations.is_empty(),
        "oversized backend sources:\n{}",
        violations.join("\n")
    );
}

#[test]
fn core_does_not_depend_on_upper_layers() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for (rel, _) in collect_dir(manifest, "src/core") {
        let path = manifest.join(&rel);
        let text = fs::read_to_string(&path).expect("read core source");
        for hit in layer_refs(&text, UPPER_LAYERS) {
            violations.push(format!("{rel}: {hit}"));
        }
        if path_attribute_escapes(&text) {
            violations.push(format!("{rel}: #[path] escapes the core module tree"));
        }
    }
    assert!(
        violations.is_empty(),
        "core layer must stay a leaf:\n{}",
        violations.join("\n")
    );
}

#[test]
fn non_app_layers_do_not_import_app() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for dir in ["features", "sessions", "filesystem", "platform"] {
        for (rel, _) in collect_dir(manifest, &format!("src/{dir}")) {
            let text = fs::read_to_string(manifest.join(&rel)).expect("read layer source");
            for hit in layer_refs(&text, &["app"]) {
                violations.push(format!("{rel}: {hit}"));
            }
            if path_attribute_escapes(&text) {
                violations.push(format!("{rel}: #[path] escapes the module tree"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "only the app layer may depend on crate::app:\n{}",
        violations.join("\n")
    );
}

#[test]
fn detects_banned_layer_refs() {
    for text in [
        "use crate::app::paths::plugin_data_dir;",
        "use crate::{features::transfer::x};",
        "use crate :: sessions :: ssh :: x;",
        "use super::features::x;",
        "use super::super::filesystem::x;",
        "fn run() { crate::sessions::ssh::open(); }",
        "use crate::core::{model::x, app::y};",
    ] {
        assert!(!layer_refs(text, UPPER_LAYERS).is_empty(), "{text}");
    }
    for text in [
        "use crate::core::model::x;",
        "use crate::platform::paths::plugin_data_dir;",
        "// crate::sessions::ssh is intentionally not imported here",
        "const NAME: &str = \"crate::features\";",
        "use crate::core_features::x;",
    ] {
        assert!(layer_refs(text, UPPER_LAYERS).is_empty(), "{text}");
    }
    assert!(path_attribute_escapes(
        "#[path = \"../sessions/runtime.rs\"] mod runtime;"
    ));
    assert!(!path_attribute_escapes("#[path = \"child.rs\"] mod child;"));
}
