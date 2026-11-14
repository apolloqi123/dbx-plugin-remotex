# SFTP 多选批量删除性能专项方案（2026-11-14）

> 现象：SFTP 面板多选文件/文件夹后批量删除，对话框逐项显示「正在处理 N / 总数」，
> 但每项都要等待明显时间；选中的文件夹越多/越大，耗时越不可接受。
>
> 本方案先定位根因，再把批量删除收敛为「一次 RPC + 一次远端 `rm -rf`」，并保留
> 失败逐项归因与 latin-1 字节保真回退。

## 1. 根因（证据）

### 1.1 前端逐项串行 RPC

`frontend/src/features/sftp/composables/actions/useSftpDelete.ts:84-123`
`confirmBatchDelete` 对每个选中条目 `await` 一次：

- 普通模式：`sftp/delete { sessionId, path, recursive }`
- sudo 模式：`sudo/remove` 或 `sudo/removeAll`

即 N 个条目 = N 次宿主桥 invoke + N 次后端会话查找/权限检查，完全没有批处理。

### 1.2 每条 `sftp/delete` 至少 2 个 SFTP 往返

`backend/src/sessions/ssh/filesystem.rs:420-473`：

1. `ensure_writable` + `session()` 查找；
2. `sftp(session_id)` 取共享 `AsyncMutex<SftpSession>`；
3. `symlink_metadata(path)`（1 个网络往返）；
4. `remove_file` / `remove_dir`（再 1 个网络往返）。

### 1.3 文件夹递归删除逐条目往返

`backend/src/sessions/ssh/tree_support.rs:21-60` `delete_directory_tree`：
每个目录一次 `read_dir`，每个文件一次 `remove_file`，每个目录一次 `remove_dir`，
全部串行。即耗时 ≈ 树内条目数 × RTT。

### 1.4 结论

批量删除总耗时 ≈

```
N 条目 × (宿主桥开销 + 2×RTT)  +  树内条目数 × RTT
```

在 50–200ms RTT 的远程链路上，多选 15 项、其中若干目录时，秒级到分钟级都正常。
用户截图「3 / 15」正是逐项推进、每项都在等网络往返的表现。

## 2. 修复目标

1. 批量删除不再逐项走 SFTP：非 latin-1 且 exec 可用时，整批用一条远端
   `rm -rf -- <shell-quoted paths...>` 完成，整批约 1 个 exec 通道往返
   （大选择按 128 路径分片，避免 argv 过长）。
2. 失败仍能逐项归因：返回 `{ deleted, failed }`，前端只关闭确实删掉的编辑器页签。
3. latin-1 连接保持字节保真：不走 shell（exec 命令串是 UTF-8 String），
   改为一次建立裸包客户端后逐路径 `raw_delete_path`，避免每条都重开通道。
4. sudo 通道同样批量化：新增 `sudo/removeMany`，一次 `rm -rf` 完成（Quick Sudo
   认证只发生一次）。
5. 单条「删除文件夹」也复用同一快路径，避免同一个根因在单条操作里继续存在。

非目标：不改动选择模型、不改对话框视觉、不引入新依赖、不做进度事件流
（批量完成后进度直接落到总数；删除通常已在一秒内完成）。

## 3. 新增 RPC 契约

### 3.1 `sftp/deleteMany`

参数：

```json
{
  "sessionId": "session-1",
  "items": [
    { "path": "/home/demo/a.txt", "recursive": false },
    { "path": "/home/demo/logs", "recursive": true }
  ]
}
```

返回：

```json
{
  "deleted": ["/home/demo/a.txt", "/home/demo/logs"],
  "failed": [{ "path": "/home/demo/locked", "error": "Permission denied" }]
}
```

语义：

- 所有路径先 `normalize_remote_path`；空路径 / 含 NUL / `/` 在任何删除发生前
  整体拒绝（fail-closed）。
- 完全重复路径去重（`recursive` 取 OR）；若同时选中祖先与后代，只保留祖先
  （`/a` 与 `/a/b` → 只删 `/a`；`/a` 与 `/ab` 互不影响）。
- 只读连接由 `ensure_writable` 拒绝。
- 符号链接不跟随（`rm -rf` 只删链接本身；SFTP 回退同样按 `symlink_metadata`
  判型）。
- 非 latin-1：按 128 项分片，每片执行一次 `rm -rf -- '<path>' ...`。全部成功则
  整片计入 `deleted`；某片非零退出或 exec 建立失败时，对该片逐项回退
  `sftp_delete` 归因——回退前先用 `sftp/exists` 同款判定确认路径是否还在，
  已不存在视为删除成功（批命令可能已部分删除）。
- latin-1：只建立一次 `raw_sftp_client`，逐项 `raw_delete_path`
  （`unescape_wire` 还原整条 wire 路径），不做 shell 快路径。
- 批内单项失败不中断其余项（与旧「首错即停」不同；这样一次 RPC 才能跑完整批，
  且失败项会明确返回）。

### 3.2 `sudo/removeMany`

参数：

```json
{ "sessionId": "session-1", "paths": ["/var/log/app", "/tmp/a.txt"] }
```

返回形状同 `sftp/deleteMany`。

语义：

- `normalize_remote_path` + 拒绝 `/`（整体预检）。
- 去重/祖先折叠后按 128 项分片，每片一次
  `sudo_exec(runtime, session_id, "rm -rf -- ...", TIMEOUT_WRITE_SECS)`。
- 某片失败时逐项 `sudo fs::remove_all` 归因（`rm -rf -f` 对已不存在路径仍成功，
  无需存在性预检）。
- 单项失败不中断其余项。

## 4. 后端实现

### 4.1 纯函数模块 `backend/src/core/delete_plan.rs`（新增）

在 `backend/src/core/mod.rs` 增加 `pub(crate) mod delete_plan;`。

建议接口（命名可按仓库风格微调，但行为必须一致）：

```rust
pub(crate) const DELETE_CHUNK_PATHS: usize = 128;

/// normalize + 拒绝 "/" + 去重 + 祖先折叠；返回规范化后的 (path, recursive)。
pub(crate) fn normalize_delete_items(items: &[(String, bool)]) -> Result<Vec<(String, bool)>, String>;

/// sudo 用：只关心路径。
pub(crate) fn normalize_delete_paths(paths: &[String]) -> Result<Vec<String>, String>;

/// `rm -rf -- 'p1' 'p2'`（使用 crate::core::shell_quote::shell_quote）。
pub(crate) fn build_rm_rf_command(paths: &[String]) -> String;

/// 按 DELETE_CHUNK_PATHS 切片。
pub(crate) fn chunk_delete_items(items: &[(String, bool)]) -> Vec<Vec<(String, bool)>>;
pub(crate) fn chunk_delete_paths(paths: &[String]) -> Vec<Vec<String>>;
```

单测覆盖：

- `/a` + `/a/b` → 只保留 `/a`；`/a` + `/ab` → 两者都保留；
- 完全重复路径去重，`recursive` OR；
- 拒绝 `/`、空串、含 `\0`；
- 引号转义：空格、单引号、`--help` 形式、中文、换行；
- 分片边界 0 / 1 / 128 / 129。

### 4.2 exec 辅助：同时支持普通会话与 files-only 会话

`files-only` 会话（远程桌面文件传输会话）不在宿主连接注册表里，
`SshRuntime::exec` 会失败；现有 `files_session_exec` 已处理该分支
（`backend/src/sessions/ssh/execution/core.rs:71-82`）。

在 `backend/src/sessions/ssh/execution/core.rs` 增加：

```rust
/// 文件操作族的普通命令通道：普通会话走 exec()（保留 env/timeout/登记），
/// files-only 会话直接复用已认证 transport 的 handle。
pub(crate) async fn filesystem_exec(
    &self,
    session_id: &str,
    command: &str,
    timeout_secs: u64,
) -> Result<crate::sessions::ssh::exec::ExecOutcome, String>;
```

实现要点：

- `self.session(session_id).await?` 取会话；
- `session.files_only == true` → `exec_plain(&session.handle, command, Duration::from_secs(timeout_secs.clamp(5,300)), &[])`；
- 否则 → `self.exec(session_id, None, command, false, Some(timeout_secs))`，把返回的
  `Value` 映射为 `ExecOutcome { output, exit_code }`（`exitCode` 缺省 -1）。

### 4.3 `sftp/deleteMany` 编排：`backend/src/sessions/ssh/filesystem/delete.rs`（新增方法）

作为 `SshRuntime` 方法，与 `sftp_delete` 同文件族。`filesystem.rs` 受
600 行结构预算限制，实现落在 `filesystem/delete.rs`（由 `filesystem.rs`
里的 `mod delete;` 声明）；**不要**放到 `filesystem/sftp/ext/`：
latin-1 回退需要的 `raw_delete_path` 是 `tree_support.rs` 里的
`pub(super)`，只在 `sessions::ssh` 子树可见。

```rust
pub(crate) async fn sftp_delete_many(
    &self,
    session_id: &str,
    items: &[(String, bool)],
    encoding: NameEncoding,
) -> Result<Value, String>
```

步骤：

1. `self.ensure_writable(session_id).await?`；
2. `normalize_delete_items(items)?`；
3. 若 `encoding == Latin1`：`self.raw_sftp_client(session_id).await` 成功则
   复用一个客户端逐项 `raw_delete_path(&mut client, &unescape_wire(&path), recursive)`；
   建立失败则逐项回退 `self.sftp_delete(..., encoding)`（保持现有回退口径）。
4. 否则按片：
   - `self.filesystem_exec(session_id, &build_rm_rf_command(&paths), DELETE_TIMEOUT_SECS).await`；
   - `exit_code == 0` → 全片计入 `deleted`；
   - `Err(_)` 或非零退出 → 逐项归因：
     - 先 `sftp_ext::exists(self, session_id, &path, encoding, false).await`；
     - `Ok(false)` → 计入 `deleted`（批命令可能已删）；
     - `Ok(true)` → `self.sftp_delete(session_id, &path, recursive, encoding).await`，
       成功计入 `deleted`，失败计入 `failed`；
     - `Err(e)` → 计入 `failed`。
5. 返回 `json!({ "deleted": deleted, "failed": failed })`。

超时常量：`const DELETE_TIMEOUT_SECS: u64 = 300;`（exec 内部上限也是 300）。

### 4.4 单条递归删除快路径（同一根因）

`backend/src/sessions/ssh/filesystem.rs::sftp_delete`：

- 在 `encoding != Latin1 && recursive` 时，先尝试
  `self.filesystem_exec(session_id, &build_rm_rf_command(std::slice::from_ref(&path)), DELETE_TIMEOUT_SECS)`；
- `exit_code == 0` 直接返回 `Ok(())`；
- exec 建立失败或非零退出 → 落到现有 SFTP 逻辑（`symlink_metadata` + 递归树删），
  保持旧语义与错误信息。
- 非递归路径保持不变（仍用 SFTP LSTAT + REMOVE/RMDIR）。

注意：`rm -rf` 对不存在的路径返回成功（`-f` 语义），单条删除因此从
「不存在报错」变为「幂等成功」；这是可接受且更友好的行为，需在测试/文档说明。

### 4.5 `sudo/removeMany`：`backend/src/features/security/sudo/fs/mutate.rs`

增加：

```rust
pub async fn remove_many(runtime: &SshRuntime, session_id: &str, paths: &[String]) -> Result<Value, String>
```

- `normalize_delete_paths`；
- 分片 `sudo_exec(..., build_rm_rf_command(&chunk), TIMEOUT_WRITE_SECS)`；
- 成功片全计入 `deleted`；失败片逐项 `remove_all` 归因；
- 返回 `{ deleted, failed }`。

### 4.6 RPC 接线

- `backend/src/app/rpc/sftp.rs` 的 `handle_sftp` 增加
  `"sftp/deleteMany"` 分支：解析 `sessionId` 与 `items`（数组，每项 `{path, recursive}`），
  `resolve_sftp_encoding(session_id)`，`self.runtime.block_on(self.ssh.sftp_delete_many(...))`。
- `backend/src/app/rpc/sudo.rs` 的 `handle_sudo` 增加
  `"sudo/removeMany"`：解析 `sessionId` 与 `paths: string[]`，
  `block_on(sudo::fs::remove_many(...))`。
- 参数校验失败必须返回明确错误（缺 `items` / 非字符串等），与现有 `required_string` 风格一致。

## 5. 前端实现

### 5.1 `frontend/src/features/sftp/composables/actions/useSftpDelete.ts`

`confirmBatchDelete` 改为单次 RPC：

```ts
type BatchDeleteResult = { deleted: string[]; failed: Array<{ path: string; error: string }> };
const DELETE_BATCH_TIMEOUT_MS = 10 * 60 * 1000;

const items = targets.map((entry) => ({
  path: pathFromUri(entry.uri),
  recursive: entry.kind === "directory",
}));
const result = sudoMode.value
  ? await window.dbxPlugin.invoke<BatchDeleteResult>(
      "sudo/removeMany",
      { sessionId, paths: items.map((item) => item.path) },
      { timeoutMs: DELETE_BATCH_TIMEOUT_MS },
    )
  : await window.dbxPlugin.invoke<BatchDeleteResult>(
      "sftp/deleteMany",
      { sessionId, items },
      { timeoutMs: DELETE_BATCH_TIMEOUT_MS },
    );

batchProgress.value = {
  total: targets.length,
  done: result.deleted.length,
  failed: result.failed.length,
  current: "",
};

if (result.failed.length > 0) {
  showError(result.failed[0].error);
} else {
  batchDeleteOpen.value = false;
  clearRowSelection();
  showNotice(t("deleted"));
}
onDeleted?.(result.deleted);
```

保留：

- `batchDeleteSubmitting` 守卫与 finally 复位；
- `batchProgress` 在开始时 `createBatchProgress(targets.length)`；
- `refreshAfterMutation()` 无论成败都执行；
- 失败时对话框保持打开，`onDeleted` 只含成功路径。

### 5.2 测试 `useSftpDelete.spec.ts`

更新 mock：`invoke` 对 `sftp/deleteMany` / `sudo/removeMany` 返回
`{ deleted, failed }`；按 `failPath` 构造 `failed` 项。

用例：

1. 批量全成功：只调用一次 `sftp/deleteMany`，参数 `items` 含
   `{path, recursive}`，`onDeleted` 收到返回的 `deleted`，对话框关闭、清选择、
   `showNotice("deleted")`。
2. 批量部分失败：`failed` 非空 → `showError` 第一项错误，对话框保持打开，
   `onDeleted` 只含成功路径，`onMutated` 仍执行。
3. sudo 模式：只调用一次 `sudo/removeMany`，参数为 `paths` 数组。
4. 单条删除用例保持原样（`sftp/delete` 不变）。

### 5.3 mock 宿主 `frontend/src/app/mock/host/sftpFiles.ts`

新增 `sftp/deleteMany` 与 `sudo/removeMany` 分支：

- 从 `mockFileSystem` 逐路径移除节点（把现有 `sftp/delete` 的删除逻辑抽成小函数复用）；
- 成功路径进 `deleted`，不存在/异常路径进 `failed`；
- 返回 `{ deleted, failed }`。

## 6. 文档

`docs/architecture/PROTOCOL.zh-CN.md`：

- 在方法清单行（`sftp/createDirectory`、`sftp/rename`、`sftp/delete` … 那一行）
  加入 `sftp/deleteMany` 与 `sudo/removeMany`；
- 在 `sudo/removeAll` 附近增加 `sudo/removeMany` 小节；
- 在扩展文件操作 `sftp/delete` 附近增加 `sftp/deleteMany` 小节；
- 明确：批量语义、`{deleted, failed}` 返回、`rm -rf` 快路径、符号链接不跟随、
  latin-1 回退、单项失败不中断。

## 7. 验收

必须全部通过：

```bash
cargo test --manifest-path backend/Cargo.toml -- --test-threads=1
pnpm --dir frontend typecheck
pnpm --dir frontend test
node scripts/verify/mock_host_registry_check.mjs
```

建议再跑：

```bash
pnpm --dir frontend build
```

代码变更落盘后：

```bash
codegraph sync && codegraph status
```

人工/代码审查要点：

- `rm -rf` 命令串全部经 `shell_quote`，且带 `--`；
- 根路径 `/` 在批量与单条路径上都被拒绝；
- 符号链接不被跟随；
- latin-1 不进入 shell 快路径；
- files-only 会话走 `filesystem_exec` 而不是失败的 `exec`；
- 前端失败分支不误关编辑器页签（`onDeleted` 只含成功路径）。

## 8. 风险与取舍

1. **exec 不可用**：极少数 `ForceCommand internal-sftp` 服务器没有 shell。
   快路径失败后自动回退 SFTP 逐项删除，功能不丢，只是恢复旧速度。
2. **部分失败**：批内失败不再「首错即停」，其余项继续删除；返回逐项结果，
   前端报第一项错误并保留对话框。
3. **单条删除不存在路径**：`rm -rf -f` 幂等成功；与旧「报 no such file」不同，
   视为可接受改进。
4. **argv 长度**：128 路径/片，单条命令最多约几十 KB，远低于 ARG_MAX。
