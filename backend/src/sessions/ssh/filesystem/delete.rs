//! `sftp/delete` / `sftp/deleteMany` for [`SshRuntime`].
//!
//! 普通会话（含 files-only 传输会话）优先走一次远端 `rm -rf -- ...`：
//! 批量删除不再逐项 2 个 SFTP 往返，`rm -rf` 也不跟随符号链接。exec 不可用
//! （无 shell / 非零退出）时回退原有 SFTP 逐项删除，功能与错误语义不变；
//! latin-1 连接全程走裸包客户端字节保真，绝不进入 shell 快路径（exec 命令串
//! 是 UTF-8 String）。

use super::*;

use crate::core::delete_plan::{build_rm_rf_command, chunk_delete_items, normalize_delete_items};

/// 整批/递归删除的 exec 预算（`exec` 内部上限同为 300 秒）。
const DELETE_TIMEOUT_SECS: u64 = 300;

impl SshRuntime {
    pub async fn sftp_delete(
        &self,
        session_id: &str,
        path: &str,
        recursive: bool,
        encoding: NameEncoding,
    ) -> Result<(), String> {
        self.ensure_writable(session_id).await?;
        let path = normalize_remote_path(path)?;
        if path == "/" {
            return Err("Refusing to delete the filesystem root".to_string());
        }
        // 递归快路径：非 latin-1 且 exec 可用时一次 `rm -rf` 删完整树（同一
        // 根因在单条删除里同样存在）。失败（无 shell / 非零退出）回退下面的
        // SFTP 逻辑，保持旧语义与错误信息；`-f` 让不存在的路径幂等成功。
        if recursive && encoding != NameEncoding::Latin1 {
            let command = build_rm_rf_command(std::slice::from_ref(&path));
            if let Ok(outcome) = self
                .filesystem_exec(session_id, &command, DELETE_TIMEOUT_SECS)
                .await
            {
                if outcome.exit_code == 0 {
                    return Ok(());
                }
            }
        }
        self.sftp_delete_via_sftp(session_id, &path, recursive, encoding)
            .await
    }

    /// 纯 SFTP 删除：latin-1 裸包字节保真 + 高层 LSTAT/REMOVE/RMDIR/树删。
    /// `sftp_delete` 的回退路径与 `sftp_delete_many` 的逐项归因共用；调用方
    /// 已完成 `ensure_writable` 与 `normalize_remote_path`（path 已归一）。
    /// 刻意不走 [`Self::filesystem_exec`]：批量场景下列级 exec 已确认失败，
    /// 逐项重试只会把同一个超时/无 shell 失败再等 N 次。
    async fn sftp_delete_via_sftp(
        &self,
        session_id: &str,
        path: &str,
        recursive: bool,
        encoding: NameEncoding,
    ) -> Result<(), String> {
        if encoding == NameEncoding::Latin1 {
            // latin-1：wire 路径整条还原为原始字节后走裸包删除（LSTAT 判型
            // → REMOVE/RMDIR/递归树删，symlink 绝不跟随）。回退策略与
            // mkdir/rename 相同：仅客户端建立失败时回退高层路径。
            match self.raw_sftp_client(session_id).await {
                Ok(mut client) => {
                    let raw_path = sftp_name::unescape_wire(path);
                    return raw_delete_path(&mut client, &raw_path, recursive).await;
                }
                Err(error) => {
                    eprintln!(
                        "[dbx-plugin-sshx] raw byte delete unavailable, falling back: {error}"
                    );
                }
            }
        }
        let sftp = self.sftp(session_id).await?;
        let metadata = sftp
            .lock()
            .await
            .symlink_metadata(path.to_string())
            .await
            .map_err(sftp_error)?;
        if metadata.is_symlink() {
            sftp.lock()
                .await
                .remove_file(path.to_string())
                .await
                .map_err(sftp_error)
        } else if metadata.is_dir() {
            if !recursive {
                return sftp
                    .lock()
                    .await
                    .remove_dir(path.to_string())
                    .await
                    .map_err(sftp_error);
            }
            delete_directory_tree(&sftp, path.to_string()).await
        } else {
            sftp.lock()
                .await
                .remove_file(path.to_string())
                .await
                .map_err(sftp_error)
        }
    }

    /// 批量删除：一次 RPC 完成整批（`{deleted, failed}` 逐项归因）。
    ///
    /// 路径先归一/去重/祖先折叠，再按 128 项分片，每片一次
    /// `rm -rf -- ...`。某片 exec 建立失败或非零退出时逐项回退：先确认路径
    /// 是否还在（批命令可能已部分删除，已不存在视为成功），仍在则走纯 SFTP
    /// 删除归因（不再重试已失败的 exec 快路径）。单项失败不中断其余项。
    /// latin-1 只建立一次裸包客户端逐项删除，不做 shell 快路径。
    pub(crate) async fn sftp_delete_many(
        &self,
        session_id: &str,
        items: &[(String, bool)],
        encoding: NameEncoding,
    ) -> Result<Value, String> {
        self.ensure_writable(session_id).await?;
        let items = normalize_delete_items(items)?;
        let mut deleted: Vec<String> = Vec::new();
        let mut failed: Vec<Value> = Vec::new();
        if encoding == NameEncoding::Latin1 {
            match self.raw_sftp_client(session_id).await {
                Ok(mut client) => {
                    for (path, recursive) in items {
                        let raw_path = sftp_name::unescape_wire(&path);
                        match raw_delete_path(&mut client, &raw_path, recursive).await {
                            Ok(()) => deleted.push(path),
                            Err(error) => failed.push(json!({ "path": path, "error": error })),
                        }
                    }
                }
                Err(error) => {
                    eprintln!(
                        "[dbx-plugin-sshx] raw byte batch delete unavailable, falling back: {error}"
                    );
                    for (path, recursive) in items {
                        match self
                            .sftp_delete_via_sftp(session_id, &path, recursive, encoding)
                            .await
                        {
                            Ok(()) => deleted.push(path),
                            Err(error) => failed.push(json!({ "path": path, "error": error })),
                        }
                    }
                }
            }
            return Ok(json!({ "deleted": deleted, "failed": failed }));
        }
        for chunk in chunk_delete_items(&items) {
            let paths = chunk
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            let outcome = self
                .filesystem_exec(
                    session_id,
                    &build_rm_rf_command(&paths),
                    DELETE_TIMEOUT_SECS,
                )
                .await;
            let batch_ok = matches!(&outcome, Ok(found) if found.exit_code == 0);
            if batch_ok {
                deleted.extend(paths);
                continue;
            }
            // 诊断（不改控制流）：慢删报告里要能区分「无 shell」与「rm 真的
            // 慢/失败」；每条失败分片只打一行。
            match &outcome {
                Ok(found) => {
                    let detail: String = found
                        .output
                        .trim()
                        .replace('\n', " ")
                        .chars()
                        .take(200)
                        .collect();
                    eprintln!(
                        "[dbx-plugin-sshx] batch delete rm -rf exited {} for {} item(s): {}",
                        found.exit_code,
                        chunk.len(),
                        detail
                    );
                }
                Err(error) => eprintln!(
                    "[dbx-plugin-sshx] batch delete exec failed for {} item(s): {error}; falling back to SFTP",
                    chunk.len()
                ),
            }
            for (path, recursive) in chunk {
                match sftp_ext::exists(self, session_id, &path, encoding, false).await {
                    // 批命令可能已部分删除：不存在即视为成功。
                    Ok(false) => deleted.push(path),
                    Ok(true) => match self
                        .sftp_delete_via_sftp(session_id, &path, recursive, encoding)
                        .await
                    {
                        Ok(()) => deleted.push(path),
                        Err(error) => failed.push(json!({ "path": path, "error": error })),
                    },
                    Err(error) => failed.push(json!({ "path": path, "error": error })),
                }
            }
        }
        Ok(json!({ "deleted": deleted, "failed": failed }))
    }
}
