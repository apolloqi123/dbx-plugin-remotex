//! macOS 剪贴板文件列表 → CLIPRDR FileGroupDescriptorW（原生文件复制粘贴）。
//!
//! Finder 复制文件/文件夹后，worker 轮询 `NSPasteboard`（仅 `changeCount`
//! 变化时读取，且只读文件 URL），这里把绝对路径展开为远端可接受的描述符：
//! - 目录：`DIRECTORY` 描述符，子项带相对路径（Windows 资源管理器据此建目录）；
//! - 文件：`ARCHIVE` 描述符 + 真实大小；正文不在复制时读取，只有远端粘贴、
//!   服务端发 `FileContentsRequest` 时才按需打开原文件。
//!
//! 文件系统安全边界与上传路径一致：限制 2000 项 / 单文件 512 MiB / 总量
//! 512 MiB；跳过符号链接与非常规文件；名字按 Windows 合法字符清洗（macOS
//! 允许 `:`、`*`、`?` 等字符），保留设备名加前缀；不记录路径或内容。

use super::upload::{StagedFile, StagedFiles};
use super::{
    FileClipboardStage, MAX_CLIPBOARD_FILES, MAX_CLIPBOARD_FILE_BYTES, MAX_CLIPBOARD_TOTAL_BYTES,
    MAX_CLIPBOARD_WIRE_NAME_CHARS,
};
use ironrdp::cliprdr::pdu::{ClipboardFileAttributes, FileDescriptor};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_LOCAL_DEPTH: usize = 64;
const MAX_NAME_UNITS: usize = 255;

fn sanitize_component(raw: &str) -> Option<String> {
    let mut out = String::new();
    for character in raw.chars() {
        if character.is_control()
            || matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
        {
            out.push('_');
        } else {
            out.push(character);
        }
    }
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() || out == "." || out == ".." {
        return None;
    }
    if out.encode_utf16().count() > MAX_NAME_UNITS {
        let mut truncated = String::new();
        for character in out.chars() {
            if truncated.encode_utf16().count() + character.len_utf16() > MAX_NAME_UNITS {
                break;
            }
            truncated.push(character);
        }
        out = truncated;
    }
    if ironrdp::cliprdr::is_windows_device_name(&out) {
        out.insert(0, '_');
    }
    Some(out)
}

fn wire_len(relative: &str, name: &str) -> usize {
    let units = |text: &str| text.encode_utf16().count();
    if relative.is_empty() {
        units(name)
    } else {
        units(relative) + 1 + units(name)
    }
}

fn unique_name(used: &mut HashSet<String>, relative: &str, name: String) -> Option<String> {
    let key = |candidate: &str| {
        if relative.is_empty() {
            candidate.to_lowercase()
        } else {
            format!("{}\\{}", relative.to_lowercase(), candidate.to_lowercase())
        }
    };
    if wire_len(relative, &name) > MAX_CLIPBOARD_WIRE_NAME_CHARS {
        return None;
    }
    if used.insert(key(&name)) {
        return Some(name);
    }
    let (stem, extension) = match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name.as_str(), ""),
    };
    for attempt in 2..1000 {
        let candidate = format!("{stem} ({attempt}){extension}");
        if wire_len(relative, &candidate) > MAX_CLIPBOARD_WIRE_NAME_CHARS {
            return None;
        }
        if used.insert(key(&candidate)) {
            return Some(candidate);
        }
    }
    None
}

fn descriptor(name: &str, relative: &str, size: u64, directory: bool) -> FileDescriptor {
    let mut descriptor = FileDescriptor::new(name.to_string());
    descriptor = if directory {
        descriptor.with_attributes(ClipboardFileAttributes::DIRECTORY)
    } else {
        descriptor
            .with_attributes(ClipboardFileAttributes::ARCHIVE)
            .with_file_size(size)
    };
    if !relative.is_empty() {
        descriptor = descriptor.with_relative_path(relative.to_string());
    }
    descriptor
}

struct Collector {
    descriptors: Vec<FileDescriptor>,
    files: Vec<StagedFile>,
    used: HashSet<String>,
    total: u64,
    skipped: usize,
}

impl Collector {
    fn new() -> Self {
        Self {
            descriptors: Vec::new(),
            files: Vec::new(),
            used: HashSet::new(),
            total: 0,
            skipped: 0,
        }
    }

    fn push(
        &mut self,
        path: &Path,
        name: &str,
        relative: &str,
        size: u64,
        directory: bool,
    ) -> Result<Option<String>, String> {
        if self.files.len() >= MAX_CLIPBOARD_FILES {
            return Err(format!(
                "clipboard file list exceeds the {MAX_CLIPBOARD_FILES} file limit"
            ));
        }
        let Some(name) = unique_name(&mut self.used, relative, name.to_string()) else {
            self.skipped += 1;
            return Ok(None);
        };
        self.descriptors
            .push(descriptor(&name, relative, size, directory));
        self.files.push(StagedFile {
            path: path.to_path_buf(),
            size,
            directory,
        });
        Ok(Some(name))
    }

    fn walk(&mut self, dir: &Path, relative: &str, depth: usize) -> Result<(), String> {
        if depth > MAX_LOCAL_DEPTH {
            self.skipped += 1;
            return Ok(());
        }
        let Ok(read) = fs::read_dir(dir) else {
            self.skipped += 1;
            return Ok(());
        };
        let mut entries: Vec<_> = read.filter_map(|entry| entry.ok()).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                self.skipped += 1;
                continue;
            };
            if metadata.file_type().is_symlink() {
                self.skipped += 1;
                continue;
            }
            let Some(name) = sanitize_component(&entry.file_name().to_string_lossy()) else {
                self.skipped += 1;
                continue;
            };
            if metadata.is_dir() {
                // A sanitized/deduplicated directory name must match the
                // relative paths its children are advertised under.
                let Some(final_name) = self.push(&path, &name, relative, 0, true)? else {
                    continue;
                };
                let child_relative = if relative.is_empty() {
                    final_name
                } else {
                    format!("{relative}\\{final_name}")
                };
                self.walk(&path, &child_relative, depth + 1)?;
            } else if metadata.is_file() {
                let size = metadata.len();
                if size > MAX_CLIPBOARD_FILE_BYTES {
                    return Err(format!(
                        "clipboard file exceeds the {} MiB limit",
                        MAX_CLIPBOARD_FILE_BYTES / 1024 / 1024
                    ));
                }
                self.total = self
                    .total
                    .checked_add(size)
                    .ok_or_else(|| "clipboard file list size overflow".to_string())?;
                if self.total > MAX_CLIPBOARD_TOTAL_BYTES {
                    return Err(format!(
                        "clipboard file list exceeds the {} MiB total limit",
                        MAX_CLIPBOARD_TOTAL_BYTES / 1024 / 1024
                    ));
                }
                self.push(&path, &name, relative, size, false)?;
            } else {
                self.skipped += 1;
            }
        }
        Ok(())
    }
}

impl FileClipboardStage {
    /// Stages Finder file/folder URLs as the outgoing CLIPRDR file list.
    /// Replaces any previously advertised clipboard content (last copy wins).
    pub(crate) fn stage_local_files(
        &self,
        roots: &[PathBuf],
    ) -> Result<(Vec<FileDescriptor>, u64), String> {
        let mut collector = Collector::new();
        for root in roots {
            let Ok(metadata) = fs::symlink_metadata(root) else {
                collector.skipped += 1;
                continue;
            };
            if metadata.file_type().is_symlink() {
                collector.skipped += 1;
                continue;
            }
            let Some(name) = root
                .file_name()
                .and_then(|name| sanitize_component(&name.to_string_lossy()))
            else {
                collector.skipped += 1;
                continue;
            };
            if metadata.is_dir() {
                if let Some(final_name) = collector.push(root, &name, "", 0, true)? {
                    collector.walk(root, &final_name, 0)?;
                }
            } else if metadata.is_file() {
                let size = metadata.len();
                if size > MAX_CLIPBOARD_FILE_BYTES {
                    return Err(format!(
                        "clipboard file exceeds the {} MiB limit",
                        MAX_CLIPBOARD_FILE_BYTES / 1024 / 1024
                    ));
                }
                collector.total = collector
                    .total
                    .checked_add(size)
                    .ok_or_else(|| "clipboard file list size overflow".to_string())?;
                if collector.total > MAX_CLIPBOARD_TOTAL_BYTES {
                    return Err(format!(
                        "clipboard file list exceeds the {} MiB total limit",
                        MAX_CLIPBOARD_TOTAL_BYTES / 1024 / 1024
                    ));
                }
                collector.push(root, &name, "", size, false)?;
            } else {
                collector.skipped += 1;
            }
        }
        if collector.descriptors.is_empty() {
            return Err("clipboard contains no transferable files".to_string());
        }
        let total = collector.total;
        let mut inner = self.lock()?;
        let previous = inner.outgoing.replace(StagedFiles::pasteboard(
            collector.files,
            collector.descriptors.clone(),
        ));
        if let Some(previous) = previous {
            previous.cleanup();
        }
        inner.last_error = None;
        Ok((collector.descriptors, total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dbx-sshx-local-clipboard-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn sanitizes_windows_invalid_names_and_device_names() {
        assert_eq!(
            sanitize_component("a:b*c?.txt").as_deref(),
            Some("a_b_c_.txt")
        );
        assert_eq!(sanitize_component("CON").as_deref(), Some("_CON"));
        assert_eq!(sanitize_component("name. ").as_deref(), Some("name"));
        assert_eq!(sanitize_component(".."), None);
    }

    #[test]
    fn expands_files_and_directories_with_directory_descriptors() {
        let dir = temp_dir("tree");
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).expect("sub dir");
        fs::write(dir.join("a.txt"), b"a").expect("a");
        fs::write(sub.join("b.txt"), b"bb").expect("b");
        let stage = FileClipboardStage::new("s-local".to_string());
        let (descriptors, total) = stage
            .stage_local_files(std::slice::from_ref(&dir))
            .expect("stage local files");
        assert_eq!(total, 3);
        // Directory itself, its file, the nested directory and nested file.
        assert_eq!(descriptors.len(), 4);
        assert!(descriptors.iter().any(|descriptor| {
            descriptor.name == "sub"
                && descriptor.attributes.is_some_and(|attributes| {
                    attributes.contains(ClipboardFileAttributes::DIRECTORY)
                })
        }));
        let root_name = dir.file_name().unwrap().to_string_lossy().to_string();
        assert!(descriptors
            .iter()
            .any(|descriptor| descriptor.name == "b.txt"
                && descriptor.relative_path.as_deref()
                    == Some(format!("{root_name}\\sub").as_str())));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitized_directory_names_match_their_children_relative_paths() {
        let dir = temp_dir("collide");
        fs::create_dir_all(dir.join("a?b")).expect("first dir");
        fs::create_dir_all(dir.join("a*b")).expect("second dir");
        fs::write(dir.join("a?b").join("x.txt"), b"x").expect("x");
        fs::write(dir.join("a*b").join("y.txt"), b"y").expect("y");
        let stage = FileClipboardStage::new("s-local".to_string());
        let (descriptors, _) = stage
            .stage_local_files(std::slice::from_ref(&dir))
            .expect("stage");
        let root_name = dir.file_name().unwrap().to_string_lossy().to_string();
        let mut child_dirs: Vec<(String, String)> = descriptors
            .iter()
            .filter(|descriptor| {
                descriptor.attributes.is_some_and(|attributes| {
                    attributes.contains(ClipboardFileAttributes::DIRECTORY)
                })
            })
            .filter_map(|descriptor| {
                descriptor
                    .relative_path
                    .as_ref()
                    .map(|relative| (descriptor.name.clone(), relative.clone()))
            })
            .collect();
        child_dirs.sort();
        assert_eq!(
            child_dirs,
            vec![
                ("a_b".to_string(), root_name.clone()),
                ("a_b (2)".to_string(), root_name.clone()),
            ]
        );
        let mut file_parents: Vec<String> = descriptors
            .iter()
            .filter(|descriptor| descriptor.name.ends_with(".txt"))
            .map(|descriptor| {
                descriptor
                    .relative_path
                    .as_ref()
                    .expect("file relative path")
                    .rsplit('\\')
                    .next()
                    .expect("parent")
                    .to_string()
            })
            .collect();
        file_parents.sort();
        assert_eq!(file_parents, vec!["a_b".to_string(), "a_b (2)".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_symlinks() {
        #[cfg(unix)]
        {
            let dir = temp_dir("symlink");
            let target = dir.join("target.txt");
            fs::write(&target, b"x").expect("target");
            std::os::unix::fs::symlink(&target, dir.join("link.txt")).expect("symlink");
            let stage = FileClipboardStage::new("s-local".to_string());
            let (descriptors, _) = stage
                .stage_local_files(std::slice::from_ref(&dir))
                .expect("stage");
            assert!(descriptors
                .iter()
                .all(|descriptor| descriptor.name != "link.txt"));
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
