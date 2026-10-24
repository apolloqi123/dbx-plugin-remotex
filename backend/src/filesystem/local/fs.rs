//! Local filesystem browsing for the in-app folder picker (`local/fs/browse`,
//! `local/fs/drives`). The sandboxed workbench iframe (opaque origin) has no
//! directory-picker API, so the desktop sidecar lists directories on the
//! user's machine — directories only, never file contents, and each listing
//! is capped. Same trust boundary as the other `local/*` methods: the caller
//! is our own plugin UI, and the paths stay on the user's machine.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};

mod dialogs;
mod drives;
mod import;

pub use dialogs::*;
pub use drives::*;
pub use import::*;

fn home_subdir(subdir: &str) -> Option<PathBuf> {
    let base = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    let dir = PathBuf::from(base).join(subdir);
    dir.is_dir().then_some(dir)
}

fn trim_picked(picked: Result<Option<String>, String>) -> Result<Option<String>, String> {
    picked.map(|path| {
        // 只剥对话框输出的 \r\n（osascript/zenity 各带一个换行）；路径本身
        // 合法允许首尾空格，不能 trim。
        path.map(|path| path.trim_matches(['\r', '\n']).to_string())
            .filter(|path| !path.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_lists_dirs_sorted_and_skips_files() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let root = tempfile::tempdir().expect("root");
        std::fs::create_dir_all(root.path().join("beta")).unwrap();
        std::fs::create_dir_all(root.path().join("Alpha")).unwrap();
        std::fs::write(root.path().join("file.txt"), "x").unwrap();
        let result =
            browse_local_dir(Some(root.path().to_str().unwrap()), data_dir.path()).expect("browse");
        let names: Vec<&str> = result["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Alpha", "beta"]);
        assert_eq!(
            result["parent"].as_str().unwrap(),
            root.path().parent().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn browse_rejects_relative_missing_and_file_paths() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        assert!(browse_local_dir(Some("relative/dir"), data_dir.path()).is_err());
        assert!(browse_local_dir(Some("/no/such/path/at/all"), data_dir.path()).is_err());
        let file = data_dir.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(browse_local_dir(Some(file.to_str().unwrap()), data_dir.path()).is_err());
    }

    #[test]
    fn browse_default_falls_back_to_download_dir() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let result = browse_local_dir(None, data_dir.path()).expect("browse default");
        assert!(result["path"].as_str().unwrap().len() > 1);
    }

    #[test]
    fn target_exists_reports_collision_candidate() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("report.pdf"), "x").unwrap();
        let hit = target_exists(dir.path().to_str().unwrap(), "report.pdf").expect("probe");
        assert_eq!(hit["exists"].as_bool(), Some(true));
        let miss = target_exists(dir.path().to_str().unwrap(), "other.pdf").expect("probe");
        assert_eq!(miss["exists"].as_bool(), Some(false));
        assert!(target_exists("relative", "a.txt").is_err());
        assert!(target_exists("", "a.txt").is_err());
    }

    #[test]
    fn browse_truncates_after_sorting_not_before() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let root = tempfile::tempdir().expect("root");
        // >MAX_ENTRIES 个子目录：截断必须发生在排序后，返回的应是字母序前
        // 500 个确定子集（而不是 readdir 序的任意 500 个再排序）。
        for index in 0..(MAX_ENTRIES + 50) {
            std::fs::create_dir_all(root.path().join(format!("dir-{index:04}"))).unwrap();
        }
        let result =
            browse_local_dir(Some(root.path().to_str().unwrap()), data_dir.path()).expect("browse");
        let names = result["entries"].as_array().unwrap();
        assert_eq!(names.len(), MAX_ENTRIES);
        assert_eq!(names[0]["name"].as_str().unwrap(), "dir-0000");
        assert_eq!(
            names[MAX_ENTRIES - 1]["name"].as_str().unwrap(),
            &format!("dir-{:04}", MAX_ENTRIES - 1)
        );
    }

    #[test]
    fn drives_empty_off_windows() {
        if cfg!(windows) {
            assert!(list_local_drives().iter().any(|drive| drive == "C:\\"));
        } else {
            assert!(list_local_drives().is_empty());
        }
    }

    #[test]
    fn quoting_helpers_escape_script_metacharacters() {
        // AppleScript 双引号串内的 `"`/`\` 转义——文件名两者皆合法。
        assert_eq!(applescript_quote("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(applescript_quote("plain"), "plain");
        // PowerShell 单引号串内的 `'` 双写。
        assert_eq!(powershell_quote("it's"), "'it''s'");
        assert_eq!(powershell_quote("*.mxtsessions"), "'*.mxtsessions'");
        // 首尾空格是合法路径/标签成分：quote 不增删。
        assert_eq!(applescript_quote(" /tmp/a "), " /tmp/a ");
        assert_eq!(powershell_quote(" a "), "' a '");
    }

    #[test]
    fn trim_picked_strips_only_dialog_newlines() {
        let picked = Ok(Some(" /tmp/dir with trailing space \r\n".to_string()));
        assert_eq!(
            trim_picked(picked).unwrap(),
            Some(" /tmp/dir with trailing space ".to_string())
        );
        assert_eq!(trim_picked(Ok(Some("\r\n".to_string()))).unwrap(), None);
        assert_eq!(trim_picked(Ok(None)).unwrap(), None);
    }
}
