//! Local source planning and remote-name collision helpers.

use crate::core::model::MAX_TRANSFER_SIZE;
use crate::filesystem::sftp::name::NameEncoding;
use crate::sessions::ssh::SshRuntime;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(super) const MAX_COPY_ENTRIES: usize = 20_000;
pub(super) const MAX_COPY_DEPTH: usize = 64;
pub(super) const ENCODING: NameEncoding = NameEncoding::Auto;

pub(super) struct FileItem {
    pub(super) relative: String,
    pub(super) path: PathBuf,
    pub(super) size: u64,
}

pub(super) struct SourcePlan {
    pub(super) name: String,
    pub(super) directory: bool,
    pub(super) dirs: Vec<String>,
    pub(super) files: Vec<FileItem>,
    pub(super) total: u64,
    pub(super) skipped: usize,
}

pub(super) fn join_remote(dir: &str, relative: &str) -> String {
    let relative = relative.trim_start_matches('/');
    if dir == "/" {
        format!("/{relative}")
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), relative)
    }
}

fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index + 1..]),
        _ => (name, ""),
    }
}

pub(super) fn unique_name(existing: &mut HashSet<String>, name: &str) -> String {
    if existing.insert(name.to_string()) {
        return name.to_string();
    }
    let (stem, extension) = split_name(name);
    for counter in 1..=999 {
        let candidate = if extension.is_empty() {
            format!("{stem} ({counter})")
        } else {
            format!("{stem} ({counter}).{extension}")
        };
        if existing.insert(candidate.clone()) {
            return candidate;
        }
    }
    format!("{stem} ({})", Uuid::new_v4())
}

pub(super) async fn remote_names(
    ssh: &SshRuntime,
    session_id: &str,
    dir: &str,
) -> Result<HashSet<String>, String> {
    let entries = ssh
        .sftp_list_path(session_id, dir, false, ENCODING)
        .await?;
    Ok(entries.into_iter().map(|entry| entry.name).collect())
}

pub(super) fn read_filled_sync(file: &mut fs::File, buffer: &mut [u8]) -> Result<usize, String> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Unable to read the local file: {error}")),
        }
    }
    Ok(filled)
}

pub(super) fn plan_source(root: &Path) -> Result<SourcePlan, String> {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("Unable to read the name of '{}'", root.display()))?
        .to_string();
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("Unable to read '{}': {error}", root.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "'{}' is a symbolic link; copy the target instead",
            root.display()
        ));
    }
    if metadata.is_file() {
        if metadata.len() > MAX_TRANSFER_SIZE {
            return Err(format!(
                "'{}' exceeds the per-file transfer limit",
                root.display()
            ));
        }
        return Ok(SourcePlan {
            name: name.clone(),
            directory: false,
            dirs: Vec::new(),
            files: vec![FileItem {
                relative: name,
                path: root.to_path_buf(),
                size: metadata.len(),
            }],
            total: metadata.len(),
            skipped: 0,
        });
    }
    if !metadata.is_dir() {
        return Err(format!(
            "'{}' is neither a regular file nor a directory",
            root.display()
        ));
    }
    let mut plan = SourcePlan {
        name,
        directory: true,
        dirs: Vec::new(),
        files: Vec::new(),
        total: 0,
        skipped: 0,
    };
    let mut stack = vec![(root.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, prefix, depth)) = stack.pop() {
        if plan.files.len() + plan.dirs.len() >= MAX_COPY_ENTRIES {
            return Err(format!(
                "'{}' contains more than {MAX_COPY_ENTRIES} entries",
                root.display()
            ));
        }
        let entries = fs::read_dir(&dir)
            .map_err(|error| format!("Unable to list '{}': {error}", dir.display()))?;
        for entry in entries {
            let Ok(entry) = entry else {
                plan.skipped += 1;
                continue;
            };
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                plan.skipped += 1;
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let path = entry.path();
            let Ok(link_metadata) = fs::symlink_metadata(&path) else {
                plan.skipped += 1;
                continue;
            };
            if link_metadata.file_type().is_symlink() {
                plan.skipped += 1;
                continue;
            }
            if link_metadata.is_dir() {
                if depth >= MAX_COPY_DEPTH {
                    plan.skipped += 1;
                    continue;
                }
                plan.dirs.push(relative.clone());
                stack.push((path, relative, depth + 1));
            } else if link_metadata.is_file() {
                if link_metadata.len() > MAX_TRANSFER_SIZE {
                    return Err(format!(
                        "'{}' exceeds the per-file transfer limit",
                        path.display()
                    ));
                }
                plan.total = plan.total.saturating_add(link_metadata.len());
                plan.files.push(FileItem {
                    relative,
                    path,
                    size: link_metadata.len(),
                });
            } else {
                plan.skipped += 1;
            }
            if plan.files.len() + plan.dirs.len() >= MAX_COPY_ENTRIES {
                return Err(format!(
                    "'{}' contains more than {MAX_COPY_ENTRIES} entries",
                    root.display()
                ));
            }
        }
    }
    plan.dirs.sort_by_key(|dir| dir.matches('/').count());
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_join_handles_root_and_nested_paths() {
        assert_eq!(join_remote("/", "a/b.txt"), "/a/b.txt");
        assert_eq!(join_remote("/home/user/", "a"), "/home/user/a");
        assert_eq!(join_remote("/home/user", "dir/a"), "/home/user/dir/a");
    }

    #[test]
    fn unique_name_renames_collisions_and_keeps_extensions() {
        let mut existing = HashSet::from(["a.txt".to_string()]);
        assert_eq!(unique_name(&mut existing, "a.txt"), "a (1).txt");
        assert_eq!(unique_name(&mut existing, "a.txt"), "a (2).txt");
        assert_eq!(unique_name(&mut existing, "dir"), "dir");
        assert_eq!(unique_name(&mut existing, "dir"), "dir (1)");
    }

    #[test]
    fn plan_source_walks_directories_and_skips_symlinks() {
        let root = tempfile::tempdir().expect("root");
        fs::create_dir_all(root.path().join("sub/deep")).unwrap();
        fs::write(root.path().join("top.txt"), b"top").unwrap();
        fs::write(root.path().join("sub/deep/a.bin"), b"1234").unwrap();
        let plan = plan_source(root.path()).expect("plan");
        assert!(plan.directory);
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.total, 7);
        assert_eq!(plan.dirs, vec!["sub", "sub/deep"]);
        assert!(plan.files.iter().any(|file| file.relative == "sub/deep/a.bin"));
    }

    #[cfg(unix)]
    #[test]
    fn plan_source_rejects_a_symlink_root() {
        let root = tempfile::tempdir().expect("root");
        let target = root.path().join("target.txt");
        fs::write(&target, b"x").unwrap();
        let link = root.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(plan_source(&link).is_err());
    }

    #[test]
    fn plan_source_reports_a_single_file() {
        let root = tempfile::tempdir().expect("root");
        let file = root.path().join("only.txt");
        fs::write(&file, b"abc").unwrap();
        let plan = plan_source(&file).expect("plan");
        assert!(!plan.directory);
        assert_eq!(plan.name, "only.txt");
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.total, 3);
    }
}
