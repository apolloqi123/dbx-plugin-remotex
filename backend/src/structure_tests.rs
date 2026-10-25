//! Structural guards: file budgets and layer direction.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_SOURCE_LINES: usize = 600;
const MAX_TEST_LINES: usize = 1_200;

const LINE_ALLOWLIST: &[(&str, usize)] = &[
    ("sessions/ssh/tests.rs", 5535),
    ("features/mcp/tests.rs", 3772),
    ("features/import/tests.rs", 1692),
    ("core/model/manifest_contract_tests.rs", 1397),
];

fn count_lines(text: &str) -> usize {
    text.lines().count()
}

fn is_test_file(rel: &str) -> bool {
    rel.ends_with("tests.rs") || rel.ends_with("_tests.rs") || rel.contains("/tests/")
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

#[test]
fn classifies_test_files() {
    assert!(is_test_file("sessions/ssh/tests.rs"));
    assert!(is_test_file("core/model/manifest_contract_tests.rs"));
    assert!(is_test_file("features/mcp/tests/tools.rs"));
    assert!(!is_test_file("sessions/ssh/mod.rs"));
    assert_eq!(count_lines("a\nb\n"), 2);
}

#[test]
fn sources_stay_within_line_budget() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    let mut violations = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .expect("relative path")
            .to_string_lossy()
            .replace('\\', "/");
        let lines = count_lines(&fs::read_to_string(&path).expect("read source"));
        let budget = if is_test_file(&rel) {
            MAX_TEST_LINES
        } else {
            MAX_SOURCE_LINES
        };
        let allowed = LINE_ALLOWLIST
            .iter()
            .find(|(name, _)| *name == rel.as_str())
            .map(|(_, limit)| *limit)
            .unwrap_or(budget);
        if lines > allowed {
            violations.push(format!("{rel}: {lines} > {allowed}"));
        }
        if allowed != budget && lines <= budget {
            violations.push(format!(
                "{rel}: allowlist entry is stale ({lines} <= {budget})"
            ));
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    let banned = [
        "crate::app::",
        "crate::features::",
        "crate::filesystem::",
        "crate::sessions::",
    ];
    let mut violations = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).expect("read core source");
        for needle in banned {
            if text.contains(needle) {
                violations.push(format!("{}: {needle}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "core layer must stay a leaf:\n{}",
        violations.join("\n")
    );
}
