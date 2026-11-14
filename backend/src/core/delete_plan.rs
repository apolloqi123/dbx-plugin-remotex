//! 批量删除的纯规划逻辑：路径规范化、去重、祖先折叠、分片与 `rm -rf` 拼装。
//!
//! 所有路径都必须已/将被 [`normalize_remote_path`] 归一，拒绝根路径 `/`；
//! 拼装进 shell 命令的每个路径统一经 [`shell_quote`] 单引号转义，并以 `--`
//! 终止选项解析，避免 `-rf` 形态的名字被当成选项。

use std::collections::HashSet;

use crate::core::model::normalize_remote_path;
use crate::core::shell_quote::shell_quote;

/// 单次远端 `rm -rf` 的路径上限：远低于 ARG_MAX，同时把 exec 次数压到最低。
pub(crate) const DELETE_CHUNK_PATHS: usize = 128;

/// normalize + 拒绝 `/` + 完全去重（`recursive` 取 OR）+ 祖先折叠。
///
/// 同时选中祖先与后代时只保留祖先（`/a` 与 `/a/b` → 只删 `/a`；`/a` 与
/// `/ab` 互不影响）。返回顺序为路径字典序，便于分片稳定。
pub(crate) fn normalize_delete_items(
    items: &[(String, bool)],
) -> Result<Vec<(String, bool)>, String> {
    let mut merged: Vec<(String, bool)> = Vec::new();
    for (path, recursive) in items {
        let path = normalize_remote_path(path)?;
        if path == "/" {
            return Err("Refusing to delete the filesystem root".to_string());
        }
        match merged.iter_mut().find(|(candidate, _)| *candidate == path) {
            Some(existing) => existing.1 |= *recursive,
            None => merged.push((path, *recursive)),
        }
    }
    // 字典序仅用于稳定返回顺序；折叠不能只看相邻项——`/a-b` 会夹在 `/a`
    // 与 `/a/b` 之间，必须按「所有已选祖先」判定（祖先集合 + 逐段上溯）。
    merged.sort_by(|left, right| left.0.cmp(&right.0));
    let selected: HashSet<String> = merged.iter().map(|(path, _)| path.clone()).collect();
    merged.retain(|(path, _)| !has_selected_ancestor(path, &selected));
    Ok(merged)
}

/// sudo 车道用：只关心路径（全部按递归删除处理）。
pub(crate) fn normalize_delete_paths(paths: &[String]) -> Result<Vec<String>, String> {
    let items = paths
        .iter()
        .map(|path| (path.clone(), true))
        .collect::<Vec<_>>();
    Ok(normalize_delete_items(&items)?
        .into_iter()
        .map(|(path, _)| path)
        .collect())
}

/// `rm -rf -- 'p1' 'p2' ...`：每个路径独立单引号转义。
pub(crate) fn build_rm_rf_command(paths: &[String]) -> String {
    let mut command = String::from("rm -rf --");
    for path in paths {
        command.push(' ');
        command.push_str(&shell_quote(path));
    }
    command
}

/// 按 [`DELETE_CHUNK_PATHS`] 切片（空输入得到空集合）。
pub(crate) fn chunk_delete_items(items: &[(String, bool)]) -> Vec<Vec<(String, bool)>> {
    items.chunks(DELETE_CHUNK_PATHS).map(<[_]>::to_vec).collect()
}

/// 按 [`DELETE_CHUNK_PATHS`] 切片（空输入得到空集合）。
pub(crate) fn chunk_delete_paths(paths: &[String]) -> Vec<Vec<String>> {
    paths.chunks(DELETE_CHUNK_PATHS).map(<[_]>::to_vec).collect()
}

/// `path` 的任一**按段**祖先是否在已选集合里：逐次裁到最后一个 `/`，
/// 保证 `/ab` 不会被当成 `/a` 的后代（兄弟前缀名互不影响）。
fn has_selected_ancestor(path: &str, selected: &HashSet<String>) -> bool {
    let mut cursor = path;
    while let Some(index) = cursor.rfind('/') {
        // 根路径已在入口拒绝，永远不会出现在集合里。
        if index == 0 {
            return false;
        }
        cursor = &cursor[..index];
        if selected.contains(cursor) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(entries: &[(&str, bool)]) -> Vec<(String, bool)> {
        entries
            .iter()
            .map(|(path, recursive)| ((*path).to_string(), *recursive))
            .collect()
    }

    fn paths(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|path| (*path).to_string()).collect()
    }

    #[test]
    fn collapses_ancestors_but_keeps_sibling_prefixes() {
        let collapsed = normalize_delete_items(&items(&[("/a", true), ("/a/b", true)])).unwrap();
        assert_eq!(collapsed, items(&[("/a", true)]));

        let both = normalize_delete_items(&items(&[("/a", true), ("/ab", true)])).unwrap();
        assert_eq!(both.len(), 2);
    }

    #[test]
    fn collapses_ancestors_across_interleaved_sibling_names() {
        // 回归：`/a-b` 夹在 `/a` 与 `/a/b` 之间，旧实现只比较相邻保留项会漏折叠。
        let collapsed =
            normalize_delete_items(&items(&[("/a", true), ("/a-b", true), ("/a/b", true)])).unwrap();
        assert_eq!(collapsed, items(&[("/a", true), ("/a-b", true)]));

        let mixed = normalize_delete_items(&items(&[
            ("/a", true),
            ("/ab", true),
            ("/a/b", true),
            ("/ab/c", true),
        ]))
        .unwrap();
        assert_eq!(mixed, items(&[("/a", true), ("/ab", true)]));
    }

    #[test]
    fn deduplicates_and_ors_recursive() {
        let merged = normalize_delete_items(&items(&[
            ("/home/a.txt", false),
            ("/home/a.txt", true),
            ("/home//a.txt", false),
        ]))
        .unwrap();
        assert_eq!(merged, items(&[("/home/a.txt", true)]));
    }

    #[test]
    fn rejects_root_empty_and_nul() {
        assert!(normalize_delete_items(&items(&[("/", true)])).is_err());
        assert!(normalize_delete_items(&items(&[("", false)])).is_err());
        assert!(normalize_delete_items(&items(&[("/a\0b", false)])).is_err());
        assert!(normalize_delete_paths(&paths(&["/"])).is_err());
    }

    #[test]
    fn quotes_shell_metacharacters() {
        let command = build_rm_rf_command(&paths(&[
            "/home/a b.txt",
            "/home/it's",
            "--help",
            "/home/中文",
            "/home/line\nbreak",
        ]));
        assert_eq!(
            command,
            "rm -rf -- '/home/a b.txt' '/home/it'\\''s' '--help' '/home/中文' '/home/line\nbreak'"
        );
    }

    #[test]
    fn chunks_at_documented_boundary() {
        assert!(chunk_delete_paths(&[]).is_empty());
        assert!(chunk_delete_items(&[]).is_empty());

        let one = paths(&["/p0"]);
        assert_eq!(chunk_delete_paths(&one).len(), 1);

        let exact: Vec<String> = (0..DELETE_CHUNK_PATHS)
            .map(|index| format!("/p{index}"))
            .collect();
        let exact_items: Vec<(String, bool)> = exact
            .iter()
            .map(|path| (path.clone(), false))
            .collect();
        assert_eq!(chunk_delete_paths(&exact).len(), 1);
        assert_eq!(chunk_delete_items(&exact_items).len(), 1);

        let mut over = exact.clone();
        over.push("/p-over".to_string());
        assert_eq!(chunk_delete_paths(&over).len(), 2);
        assert_eq!(chunk_delete_paths(&over)[1].len(), 1);
    }

    #[test]
    fn sudo_paths_collapse_and_keep_order() {
        let normalized =
            normalize_delete_paths(&paths(&["/var/log/app/old", "/var/log/app", "/tmp/a"])).unwrap();
        assert_eq!(normalized, paths(&["/tmp/a", "/var/log/app"]));
    }
}
