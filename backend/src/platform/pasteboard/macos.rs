//! macOS file clipboard via NSPasteboard (native Finder copy/paste).

use objc2::rc::autoreleasepool;
use objc2::ClassType;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL};
use objc2_foundation::{NSArray, NSURL};
use std::path::PathBuf;
use std::process::Command;

fn pasteboard() -> objc2::rc::Retained<NSPasteboard> {
    NSPasteboard::generalPasteboard()
}

#[cfg(test)]
fn named_pasteboard(name: &str) -> objc2::rc::Retained<NSPasteboard> {
    use objc2_foundation::NSString;
    NSPasteboard::pasteboardWithName(&NSString::from_str(name))
}

fn file_urls_of(board: &NSPasteboard) -> Vec<PathBuf> {
    let classes = NSArray::from_slice(&[NSURL::class()]);
    // SAFETY: the class array contains NSURL and options is nil, which is
    // exactly what AppKit documents for reading file URLs.
    let Some(objects) = (unsafe { board.readObjectsForClasses_options(&classes, None) }) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for object in objects.iter() {
        let Some(url) = object.downcast_ref::<NSURL>() else {
            continue;
        };
        let Some(path) = url.path() else {
            continue;
        };
        let path = path.to_string();
        if path.starts_with('/') {
            paths.push(PathBuf::from(path));
        }
    }
    paths
}

fn declares_file_urls(board: &NSPasteboard) -> bool {
    // SAFETY: AppKit exports this pasteboard type static for the process lifetime.
    let target = unsafe { NSPasteboardTypeFileURL }.to_string();
    board
        .types()
        .map(|types| types.iter().any(|kind| kind.to_string() == target))
        .unwrap_or(false)
}

pub(crate) fn supported() -> bool {
    true
}

/// Cheap change counter: poll this instead of reading the pasteboard.
pub(crate) fn change_count() -> i64 {
    autoreleasepool(|_| pasteboard().changeCount() as i64)
}

/// True when the general pasteboard currently advertises file URLs.
pub(crate) fn has_file_urls() -> bool {
    // Unit tests must not depend on (or read) the developer's clipboard.
    if cfg!(test) {
        return false;
    }
    autoreleasepool(|_| declares_file_urls(&pasteboard()))
}

/// Absolute local paths of the Finder selection on the general pasteboard.
/// Only called after `change_count` changed; never reads file contents.
pub(crate) fn file_urls() -> Vec<PathBuf> {
    // Unit tests must not read the developer's clipboard.
    if cfg!(test) {
        return Vec::new();
    }
    autoreleasepool(|_| {
        let board = pasteboard();
        if !declares_file_urls(&board) {
            return Vec::new();
        }
        file_urls_of(&board)
    })
}

/// Replace the macOS clipboard with file URLs (Finder can Cmd+V them).
/// Paths are passed as JXA arguments, so quotes/newlines in names need no
/// string escaping. Only plugin-downloaded files are ever passed here.
fn write_to(name: Option<&str>, paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut command = Command::new("/usr/bin/osascript");
    command
        .arg("-l")
        .arg("JavaScript")
        .arg("-e")
        .arg(
            r#"ObjC.import('AppKit');
function run(argv) {
  const name = argv[0];
  const paths = argv.slice(1);
  const board = name ? $.NSPasteboard.pasteboardWithName($(name)) : $.NSPasteboard.generalPasteboard;
  board.clearContents;
  const urls = paths.map((path) => $.NSURL.fileURLWithPath($(path)));
  return board.writeObjects($(urls)) ? 'ok' : 'failed';
}"#,
        )
        .arg("--")
        .arg(name.unwrap_or(""));
    for path in paths {
        command.arg(path);
    }
    let output = command
        .output()
        .map_err(|error| format!("Unable to run osascript: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Writing the macOS clipboard failed: {}",
            stderr.trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim() == "ok" {
        Ok(())
    } else {
        Err(format!(
            "Writing the macOS clipboard was rejected: {}",
            stdout.trim()
        ))
    }
}

pub(crate) fn write_file_urls(paths: &[PathBuf]) -> Result<(), String> {
    // Unit tests must never replace the developer's clipboard.
    if cfg!(test) {
        return Ok(());
    }
    write_to(None, paths)
}

#[cfg(test)]
pub(crate) fn test_write_named(name: &str, paths: &[PathBuf]) -> Result<(), String> {
    write_to(Some(name), paths)
}

#[cfg(test)]
pub(crate) fn test_read_named(name: &str) -> Vec<PathBuf> {
    autoreleasepool(|_| file_urls_of(&named_pasteboard(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_pasteboard_round_trips_file_urls() {
        let dir = std::env::temp_dir().join("dbx-sshx-pasteboard-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let a = dir.join("a.txt");
        let b = dir.join("b b.txt");
        std::fs::write(&a, b"a").expect("write a");
        std::fs::write(&b, b"bb").expect("write b");

        let name = format!("io.dbx.sshx.test.{}", std::process::id());
        test_write_named(&name, &[a.clone(), b.clone()]).expect("write named pasteboard");
        let mut read = test_read_named(&name);
        read.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(read, expected);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
