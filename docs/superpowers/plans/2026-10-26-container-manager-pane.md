# 容器管理底部分屏 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把容器管理从右侧栏互斥视图迁为终端列底部独立分屏，提供真流式日志跟随与 SSH/本机 Docker 容器文件浏览、上传下载。

**Architecture:** 布局层用独立的 `dockerPaneOpen`/`dockerSplitRatio` 在终端列内做上下分屏，SFTP 右栏不变；日志由 sidecar 后台任务读取 `docker logs -f` 并通过 `docker/logs/chunk|end` 事件推送；文件浏览用 `docker exec ls/stat`，传输用 `docker cp` 暂存后复用现有 SFTP/本机落盘管线。

**Tech Stack:** Vue 3 `<script setup>` + TypeScript + Vitest；Rust sidecar + russh + tokio；现有 DBX plugin SDK 事件桥。

**Spec:** `docs/superpowers/specs/2026-10-26-container-manager-pane-design.md`

## Global Constraints

- 非测试源文件 ≤600 行，测试文件 ≤1,200 行；不得新增 structure allowlist 条目。
- 前端依赖方向 `app → features → shared`；`features` 不得 import `app`，`shared` 不得 import `features/app`。
- 后端 `core` 保持叶子层；`features/sessions/filesystem/platform` 不得 import `crate::app`。
- 不新增 npm/cargo 依赖；Docker 仍只通过 CLI 调用，不直连 daemon API。
- 容器 id 只接受 12–64 位小写十六进制；tail 钳制 10–2000；单文件上限 512MiB；同时最多 4 条日志流/4 个上传。
- 所有新 UI 文案覆盖 7 个 locale（en、zh-CN、zh-TW、ja、es、it、pt-BR），i18n key 引用守卫必须通过。
- 现有 `docker/list|logs|action`、SFTP、指标、终端、RDP/VNC 行为不变；不改 RPC 既有参数形状。
- 暂存路径只能由服务端生成并按固定前缀清理；不得提供任意路径删除/读取。
- 上传/导入遵守 `ensure_writable`；sudo 日志遵守只读连接限制与 Quick Sudo 开关。
- 每个任务结束前必须跑该任务的聚焦测试并提交；不推送、不合并、不安装插件。

## File Structure

后端新增：

```
backend/src/features/docker/stream_batch.rs     # 8KiB/100ms 批量合并（纯逻辑）
backend/src/features/docker/stream.rs           # 流注册表、SSH/本机启动、事件
backend/src/features/docker/files/mod.rs        # 路径归一、类型
backend/src/features/docker/files/list.rs       # list/stat 命令与解析
backend/src/features/docker/files/transfer.rs   # export/import/cleanup/saveLocal
backend/src/features/docker/files/uploads.rs    # 本机分片上传注册表
backend/src/sessions/ssh/exec/streaming.rs      # plain/sudo 流式 exec
backend/src/sessions/ssh/execution/streaming.rs # SshRuntime::exec_stream
```

后端修改：`exec/mod.rs`、`exec/run.rs`、`execution.rs`、`features/docker/{mod,local,scripts}.rs`、`app/{mod,bootstrap}.rs`、`app/rpc/docker.rs`、`filesystem/local/downloads.rs`。

前端新增：

```
frontend/src/features/docker/components/ContainerManagerPane.vue
frontend/src/features/docker/components/ContainerListPane.vue
frontend/src/features/docker/components/ContainerDetailPane.vue
frontend/src/features/docker/components/ContainerLogsTab.vue
frontend/src/features/docker/components/ContainerFilesTab.vue
frontend/src/features/docker/composables/useDockerLogStream.ts
frontend/src/features/docker/composables/useContainerFiles.ts
frontend/src/features/docker/composables/useContainerFileTransfer.ts
frontend/src/features/docker/lib/dockerLogBuffer.ts
frontend/src/features/docker/lib/dockerLogEvents.ts
frontend/src/features/docker/lib/dockerFilePaths.ts
frontend/src/features/docker/lib/dockerFileTransfer.ts
```

前端修改：`app/components/WorkbenchWorkspace.vue`、`WorkbenchSidePanel.vue`、`WorkbenchToolbar.vue`、`app/workbench/{state/useSftpState.ts,state/useLayoutState.ts,composables/useWorkbenchPanels.ts,composables/useSplitDivider.ts,composables/usePluginEvents.ts}`、`app/workbench/setup/{step1-core,step4-sessions,step7-sftp,step8-shell}.ts`、`features/sftp/composables/transfer/useSftpDownload.ts`、`features/transfers/composables/useTransferOperations.ts`、7 个 i18n base 文件、`app/mock/host/docker.ts`。

---

### Task 1: 后端流式执行与分片批处理

**Files:**
- Create: `backend/src/features/docker/stream_batch.rs`
- Create: `backend/src/sessions/ssh/exec/streaming.rs`
- Create: `backend/src/sessions/ssh/execution/streaming.rs`
- Modify: `backend/src/sessions/ssh/exec/mod.rs`
- Modify: `backend/src/sessions/ssh/exec/run.rs:372-470`
- Modify: `backend/src/sessions/ssh/execution.rs`（声明 `mod streaming;`）
- Test: 各文件内联 `#[cfg(test)]`

**Interfaces:**
- Produces: `ChunkBatcher::{new,push,flush}`（`push(&mut self, bytes: &[u8], now: Instant) -> Option<Vec<u8>>`）、`MAX_BATCH_BYTES=8192`、`BATCH_INTERVAL=100ms`。
- Produces: `exec::streaming::{exec_plain_stream, exec_sudo_stream}`，两者返回 `Result<i32, String>`。
- Produces: `SshRuntime::exec_stream(&self, session_id: &str, command: &str, sudo: bool, cancel: watch::Receiver<bool>, sink: tokio::sync::mpsc::Sender<Vec<u8>>) -> Result<i32, String>`。

- [ ] **Step 1: 写 ChunkBatcher 失败测试**

```rust
// backend/src/features/docker/stream_batch.rs
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn flushes_at_byte_limit() {
        let mut batcher = ChunkBatcher::new();
        let now = Instant::now();
        assert!(batcher.push(&vec![b'a'; MAX_BATCH_BYTES - 1], now).is_none());
        assert_eq!(batcher.push(b"b", now).unwrap().len(), MAX_BATCH_BYTES);
    }

    #[test]
    fn flushes_after_interval() {
        let mut batcher = ChunkBatcher::new();
        let now = Instant::now();
        assert!(batcher.push(b"a", now).is_none());
        assert!(batcher.push(b"b", now + BATCH_INTERVAL - Duration::from_millis(1)).is_none());
        assert_eq!(batcher.push(b"c", now + BATCH_INTERVAL).unwrap(), b"abc");
    }

    #[test]
    fn flush_returns_none_when_empty() {
        assert!(ChunkBatcher::new().flush().is_none());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path backend/Cargo.toml stream_batch -- --nocapture`
Expected: 编译失败（`ChunkBatcher` 未定义）。

- [ ] **Step 3: 实现 ChunkBatcher**

```rust
// backend/src/features/docker/stream_batch.rs
use std::time::{Duration, Instant};

pub const MAX_BATCH_BYTES: usize = 8 * 1024;
pub const BATCH_INTERVAL: Duration = Duration::from_millis(100);

pub struct ChunkBatcher {
    pending: Vec<u8>,
    last_flush: Instant,
}

impl ChunkBatcher {
    pub fn new() -> Self {
        Self { pending: Vec::new(), last_flush: Instant::now() }
    }

    pub fn push(&mut self, bytes: &[u8], now: Instant) -> Option<Vec<u8>> {
        self.pending.extend_from_slice(bytes);
        (self.pending.len() >= MAX_BATCH_BYTES || now.duration_since(self.last_flush) >= BATCH_INTERVAL)
            .then(|| self.take())
    }

    pub fn flush(&mut self) -> Option<Vec<u8>> {
        (!self.pending.is_empty()).then(|| self.take())
    }

    fn take(&mut self) -> Vec<u8> {
        self.last_flush = Instant::now();
        std::mem::take(&mut self.pending)
    }
}

impl Default for ChunkBatcher {
    fn default() -> Self { Self::new() }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path backend/Cargo.toml stream_batch -- --nocapture`
Expected: 3 passed。

- [ ] **Step 5: 给 `run_to_completion` 增加可选 sink**

在 `backend/src/sessions/ssh/exec/run.rs`：
- 参数增加 `sink: Option<&tokio::sync::mpsc::Sender<Vec<u8>>>`，`timeout` 改为 `Option<Duration>`。
- `deadline` 改为 `Option<Instant>`；等待条件用：

```rust
let wait_message = async {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, channel.wait())
            .await
            .map_err(|_| SUDO_WAIT_TIMEOUT_MESSAGE.to_string()),
        None => Ok(channel.wait().await),
    }
};
```

- `ChannelMsg::Data` / `ExtendedData` 分支在既有处理之后追加：

```rust
if let Some(sink) = sink {
    tokio::select! {
        biased;
        _ = wait_cancelled(cancel) => return Err(EXEC_CANCELLED_MESSAGE.to_string()),
        sent = sink.send(data.to_vec()) => {
            sent.map_err(|_| "docker log stream receiver closed".to_string())?;
        }
    }
}
```

- 更新三个既有调用点：`exec_plain_cancellable`、`exec_with_sudo_cancellable` 传 `Some(timeout), None`。
- 保留 `MAX_EXEC_OUTPUT_BYTES` 对缓冲的裁剪；流式路径仍累积 stdout 用于退出码/尾部错误，但 sink 已实时输出。

- [ ] **Step 6: 新增流式 exec 模块并编译**

`backend/src/sessions/ssh/exec/streaming.rs`：

```rust
use super::*;

pub async fn exec_plain_stream(
    handle: &russh::client::Handle<SshClient>,
    command: &str,
    set_env: &[(String, String)],
    cancel: watch::Receiver<bool>,
    sink: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> Result<i32, String> {
    let mut channel = handle.channel_open_session().await
        .map_err(|error| format!("Failed to open exec channel: {error}"))?;
    apply_connection_env(&mut channel, set_env).await?;
    channel.exec(true, command.as_bytes()).await
        .map_err(|error| format!("Failed to start command: {error}"))?;
    let mut cancel = cancel;
    run_to_completion(&mut channel, None, None, &mut cancel, Some(&sink))
        .await
        .map(|outcome| outcome.exit_code)
}

pub async fn exec_sudo_stream(
    handle: &russh::client::Handle<SshClient>,
    auth: &SudoAuth,
    command: &str,
    use_pty: bool,
    set_env: &[(String, String)],
    cancel: watch::Receiver<bool>,
    sink: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> Result<i32, String> {
    let mut cancel = cancel;
    let outcome = exec_with_sudo_inner(
        handle, auth, command, use_pty, set_env, None, &mut cancel, Some(&sink),
    ).await?;
    Ok(outcome.exit_code)
}
```

实现要点：把 `exec_with_sudo_cancellable` 的主体抽成
`async fn exec_with_sudo_inner(handle, auth, command, use_pty, set_env, timeout: Option<Duration>, cancel: &mut watch::Receiver<bool>, sink: Option<&mpsc::Sender<Vec<u8>>>) -> Result<ExecOutcome, String>`；
`exec_with_sudo_cancellable` 传 `Some(timeout), None` 保持原行为；`exec_sudo_stream` 传 `None, Some(&sink)`。

- [ ] **Step 7: 新增 SshRuntime::exec_stream**

`backend/src/sessions/ssh/execution/streaming.rs` 从 `core.rs::exec` 复制连接/会话/`set_env`/sudo 配置解析部分，然后：

```rust
pub async fn exec_stream(
    &self,
    session_id: &str,
    command: &str,
    sudo: bool,
    cancel: tokio::sync::watch::Receiver<bool>,
    sink: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> Result<i32, String> {
    let (handle, orchestration, set_env, use_pty) = self.stream_context(session_id, sudo).await?;
    if sudo {
        crate::sessions::ssh::exec::exec_sudo_stream(
            &handle, &orchestration, command, use_pty, &set_env, cancel, sink,
        ).await
    } else {
        crate::sessions::ssh::exec::exec_plain_stream(
            &handle, command, &set_env, cancel, sink,
        ).await
    }
}
```

`stream_context` 复用 `exec` 的读取逻辑：session 存在、sudo 时拒绝只读、连接存在且 `sudo_enabled`、读取 `orchestration` 与 `set_env`、按 profile 计算 `use_pty`。

- [ ] **Step 8: 编译与现有测试**

Run:
```bash
cargo check --manifest-path backend/Cargo.toml --all-targets
cargo test --manifest-path backend/Cargo.toml sessions::ssh::exec -- --nocapture
```
Expected: 编译无警告，现有 exec 测试全绿。

- [ ] **Step 9: Commit**

```bash
git add backend/src/features/docker/stream_batch.rs backend/src/sessions/ssh/exec/streaming.rs backend/src/sessions/ssh/execution/streaming.rs backend/src/sessions/ssh/exec/mod.rs backend/src/sessions/ssh/exec/run.rs backend/src/sessions/ssh/execution.rs
git commit -m "feat(ssh): add streamable exec for container logs"
```

---

### Task 2: 日志流注册表、命令构造与本机流式进程

**Files:**
- Create: `backend/src/features/docker/stream.rs`
- Modify: `backend/src/features/docker/mod.rs`
- Modify: `backend/src/features/docker/scripts.rs`
- Modify: `backend/src/features/docker/local.rs`
- Modify: `backend/src/app/mod.rs`
- Modify: `backend/src/app/bootstrap.rs`
- Modify: `backend/src/app/rpc/docker.rs`
- Test: 内联模块 + `features/docker/tests.rs`

**Interfaces:**
- Produces: `logs_follow_command(engine: &EngineConfig, container_id: &str, tail: u64) -> String`。
- Produces: `spawn_logs_stream(engine: &EngineConfig, container_id: &str, tail: u64, sink: mpsc::Sender<Vec<u8>>, cancel: watch::Receiver<bool>) -> Result<i32, String>`。
- Produces: `DockerStreams::{new,start_ssh,start_local,stop,stop_all}`；`Plugin.docker_streams: Arc<DockerStreams>`。
- Produces RPC `docker/logs/start|stop` 与事件 `docker/logs/chunk|end`。

- [ ] **Step 1: 写命令与解析测试**

```rust
// backend/src/features/docker/scripts.rs tests
#[test]
fn logs_follow_command_is_fixed_and_clamped() {
    let engine = crate::features::docker::test_engine("docker", None);
    assert_eq!(
        logs_follow_command(&engine, "d4a7c9f1e2b3", 200),
        "docker logs -f --tail 200 d4a7c9f1e2b3"
    );
}

#[test]
fn logs_follow_command_keeps_engine_endpoint() {
    let engine = crate::features::docker::test_engine("podman", Some("unix:///tmp/podman.sock"));
    let command = logs_follow_command(&engine, "d4a7c9f1e2b3", 9999);
    assert!(command.contains("DOCKER_HOST=unix:///tmp/podman.sock"));
    assert!(command.ends_with("podman logs -f --tail 2000 d4a7c9f1e2b3"));
}

#[test]
fn stream_registry_caps_active_streams() {
    let registry = DockerStreams::new();
    for index in 0..MAX_ACTIVE_STREAMS {
        registry.insert_test(format!("s{index}"));
    }
    assert!(registry.start_guard().is_err());
}
```

测试模块内提供：

```rust
fn test_engine(cli: &str, endpoint: Option<&str>) -> EngineConfig {
    EngineConfig {
        cli: cli.to_string(),
        endpoint: endpoint.map(str::to_string),
    }
}
```

`insert_test(id)` 在 `#[cfg(test)]` 下向注册表插入占位 `StreamEntry`；`start_guard()` 是 `start_ssh`/`start_local` 共用的上限检查（活跃数 ≥ `MAX_ACTIVE_STREAMS` 时返回 `Err`）。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --manifest-path backend/Cargo.toml docker::scripts -- --nocapture`
Expected: 编译失败。

- [ ] **Step 3: 实现命令构造与常量**

```rust
// backend/src/features/docker/stream.rs
pub const MAX_ACTIVE_STREAMS: usize = 4;
pub const STREAM_TTL: Duration = Duration::from_secs(30 * 60);

pub fn logs_follow_command(engine: &EngineConfig, container_id: &str, tail: u64) -> String {
    format!(
        "{}{} logs -f --tail {} {}",
        engine.env_prefix(),
        engine.cli,
        clamp_tail(tail),
        container_id
    )
}
```

- [ ] **Step 4: 实现本机流式子进程**

```rust
// backend/src/features/docker/local.rs
pub async fn spawn_logs_stream(
    engine: &EngineConfig,
    container_id: &str,
    tail: u64,
    sink: tokio::sync::mpsc::Sender<Vec<u8>>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<i32, String> {
    validate_container_id(container_id)?;
    let cli = find_local_cli(&engine.cli)
        .ok_or_else(|| format!("{} CLI not found on this machine", engine.cli))?;
    let mut command = tokio::process::Command::new(cli);
    command
        .arg("logs").arg("-f").arg("--tail").arg(clamp_tail(tail).to_string())
        .arg(container_id)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(endpoint) = engine.endpoint.as_deref() {
        command.env(engine.endpoint_env_key(), endpoint);
    }
    let mut child = command.spawn().map_err(|error| format!("Failed to start {} logs: {error}", engine.cli))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let mut stdout_done = false;
    let mut stderr_done = false;
    while !(stdout_done && stderr_done) {
        tokio::select! {
            biased;
            _ = cancel.changed() => { let _ = child.kill().await; return Ok(130); }
            read = read_chunk(&mut stdout), if !stdout_done => match read? {
                None => stdout_done = true,
                Some(bytes) => { let _ = sink.send(bytes).await; }
            },
            read = read_chunk(&mut stderr), if !stderr_done => match read? {
                None => stderr_done = true,
                Some(bytes) => { let _ = sink.send(bytes).await; }
            },
        }
    }
    let status = child.wait().await.map_err(|error| error.to_string())?;
    Ok(status.code().unwrap_or(-1))
}
```

`read_chunk` 是 8KiB 的 `AsyncReadExt` 辅助；`endpoint_env_key`/`endpoint` 按现有 `run_local` 使用方式取值（若无公开方法，则在 `local.rs` 内读取 `engine.endpoint` 并用 `DOCKER_HOST`/`CONTAINER_HOST` 规则）。

- [ ] **Step 5: 实现流注册表与事件转发**

```rust
pub struct DockerStreams {
    inner: Mutex<HashMap<String, StreamEntry>>,
}

struct StreamEntry {
    cancel: tokio::sync::watch::Sender<bool>,
    started: Instant,
}

impl DockerStreams {
    pub fn start_ssh(
        self: &Arc<Self>,
        runtime: Arc<SshRuntime>,
        emitter: PluginEmitter,
        session_id: String,
        command: String,
        sudo: bool,
        tail: u64,
    ) -> Result<String, String>;

    pub fn start_local(
        self: &Arc<Self>,
        emitter: PluginEmitter,
        engine: EngineConfig,
        container_id: String,
        tail: u64,
    ) -> Result<String, String>;

    pub fn stop(&self, stream_id: &str) -> bool;
    pub fn stop_all(&self);
}
```

启动流程（SSH 与本机共用转发任务）：
1. `start_guard()` 检查活跃数 < 4，生成 `Uuid::new_v4().to_string()`。
2. 建 `watch::channel(false)` 与 `mpsc::channel(64)`。
3. spawn runner：调用 `exec_stream`/`spawn_logs_stream`。
4. 再 spawn forwarder：循环接收分片，`ChunkBatcher` 合并，`emitter.event("docker/logs/chunk", json!({"streamId": id, "text": String::from_utf8_lossy(&batch)}))`；runner 结束发送 `docker/logs/end`，payload 含 `exitCode` 或 `error`，权限签名命中时 `needsSudo:true`；最后 `inner.remove(id)`。
5. TTL：forwarder 每 5s 检查 `started.elapsed() > STREAM_TTL`，到期设置 cancel 并结束。
6. `stop`：设置 cancel true 并移除条目；forwarder/runner 自行收口。

`docker/logs/start` RPC 解析 `target`、`engine`、`containerId`、`tail`、`sudo`；SSH 时 `required_string("sessionId")`，`sudo` 时先 `ensure_writable` 并检查连接 `sudo_enabled()`；本机直接启动。`docker/logs/stop` 调 `stop` 返回 `{stopped}`。

- [ ] **Step 6: 测试事件与上限**

使用 `PluginEmitter::for_tests` + `SharedSink`（参考 `sessions/ssh/tests/forwarding.rs`）：
- 启动一个本机假 runner（测试内注入 sink 分片）或直接调用 `ChunkBatcher` 转发函数；
- 断言事件 JSON 的 `method` 为 `docker/logs/chunk`、`streamId` 匹配、文本拼接正确；
- 断言第 5 条流启动报错、`stop` 后注册表为空。

- [ ] **Step 7: 运行测试与编译**

Run:
```bash
cargo test --manifest-path backend/Cargo.toml docker -- --nocapture
cargo check --manifest-path backend/Cargo.toml --all-targets
```
Expected: 全绿、无警告。

- [ ] **Step 8: Commit**

```bash
git add backend/src/features/docker backend/src/app/mod.rs backend/src/app/bootstrap.rs backend/src/app/rpc/docker.rs
git commit -m "feat(docker): stream container logs through sidecar events"
```

---

### Task 3: 容器文件路径、目录列表与 stat

**Files:**
- Create: `backend/src/features/docker/files/mod.rs`
- Create: `backend/src/features/docker/files/list.rs`
- Modify: `backend/src/features/docker/mod.rs`
- Modify: `backend/src/app/rpc/docker.rs`
- Test: 内联 `#[cfg(test)]`

**Interfaces:**
- Produces: `normalize_container_path(&str) -> Result<String, String>`。
- Produces: `parse_ls_output(&str) -> Vec<ContainerEntry>`。
- Produces: `list_command(engine, container_id, path) -> Result<(String, Vec<String>), String>`（SSH 命令 + 本机 argv）。
- Produces RPC `docker/files/list|stat`。

- [ ] **Step 1: 写纯逻辑测试**

```rust
#[test]
fn normalizes_container_paths() {
    assert_eq!(normalize_container_path("/").unwrap(), "/");
    assert_eq!(normalize_container_path("/var//log/../lib/").unwrap(), "/var/lib");
    assert!(normalize_container_path("relative").is_err());
    assert!(normalize_container_path("/tmp/\0x").is_err());
    assert!(normalize_container_path(&format!("/{}", "a".repeat(4097))).is_err());
}

#[test]
fn parses_ls_rows() {
    let output = "bin/\netc/\n.ssh\nserver.log\r\n..\n.\n";
    assert_eq!(
        parse_ls_output(output),
        vec![
            entry("bin", "dir"),
            entry("etc", "dir"),
            entry(".ssh", "file"),
            entry("server.log", "file"),
        ]
    );
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --manifest-path backend/Cargo.toml docker::files -- --nocapture`
Expected: 编译失败。

- [ ] **Step 3: 实现路径与解析**

- `normalize_container_path`：必须 `/` 开头；按 `/` 分段，忽略空段与 `.`，`..` 回退但根不越界；拒绝含 `\0`、`\r`、`\n` 或长度 >4096；输出无尾斜杠（根除外）。
- `parse_ls_output`：逐行 `trim_end_matches('\r')`；跳过空行、`.`、`..`；`ends_with('/')` → `dir`，否则 `file`；名称去尾斜杠；返回保持输入顺序。
- `ContainerEntry { name, path, kind }`，`kind` 序列化为 `"dir"|"file"`。

- [ ] **Step 4: 实现 list/stat 与 RPC**

- `list_command`：SSH 用 `format!("{} exec {} ls -1ap -- {}", engine.env_prefix(), container_id, shell_quote(path))`；本机 argv `["exec", container_id, "ls", "-1ap", "--", path]`。
- RPC `docker/files/list`：目标分支同 docker/list；SSH 调 `ssh.exec`（15s），本机 `run_local`；失败时把 stderr 附在错误里；容器缺 `ls` 时错误文本包含 `ls` 与 `executable file not found` 供前端识别。
- RPC `docker/files/stat`：命令 `stat -c '%F|%s|%Y|%a' -- <path>`；解析为 `{kind,size,mtime,mode}`；`stat` 缺失时返回 `{unavailable:true}` 而非错误。
- 响应 `{path,parent,entries}`；`parent` 用归一后的父路径。

- [ ] **Step 5: 运行测试与编译**

Run:
```bash
cargo test --manifest-path backend/Cargo.toml docker::files -- --nocapture
cargo check --manifest-path backend/Cargo.toml --all-targets
```

- [ ] **Step 6: Commit**

```bash
git add backend/src/features/docker/files backend/src/features/docker/mod.rs backend/src/app/rpc/docker.rs
git commit -m "feat(docker): list and stat container files"
```

---

### Task 4: 容器文件导出、导入、清理与本机保存

**Files:**
- Create: `backend/src/features/docker/files/transfer.rs`
- Modify: `backend/src/features/docker/files/mod.rs`
- Modify: `backend/src/features/docker/local.rs`（本地 `docker cp`/rm 使用现有 run_local 即可）
- Modify: `backend/src/filesystem/local/downloads.rs`
- Modify: `backend/src/app/rpc/docker.rs`
- Test: 内联 + `filesystem/local/downloads/tests.rs`

**Interfaces:**
- Produces: `StagingPath` 校验函数 `validate_staging_path(path: &str, location: StagingLocation) -> Result<(), String>`。
- Produces: `save_local_path(data_dir, name, source: &Path, conflict: Option<&str>) -> Result<Value, String>`。
- Produces RPC `docker/files/export|prepareUpload|import|cleanup|saveLocal`。

- [ ] **Step 1: 写暂存路径与目标语义测试**

```rust
#[test]
fn staging_paths_only_accept_generated_prefix() {
    assert!(validate_staging_path("/tmp/dbx-sshx.1234/report.log", StagingLocation::Remote).is_ok());
    assert!(validate_staging_path("/etc/passwd", StagingLocation::Remote).is_err());
    assert!(validate_staging_path("/tmp/other/report.log", StagingLocation::Remote).is_err());
    assert!(validate_staging_path("/tmp/dbx-sshx.1234/../etc/passwd", StagingLocation::Remote).is_err());
}

#[test]
fn import_destination_keeps_name_for_directory_targets() {
    assert_eq!(resolve_import_dest("/srv/app/", "report.log").unwrap(), "/srv/app/report.log");
    assert_eq!(resolve_import_dest("/srv/app/report.log", "ignored").unwrap(), "/srv/app/report.log");
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --manifest-path backend/Cargo.toml docker::files::transfer -- --nocapture`

- [ ] **Step 3: 实现 export/import/cleanup**

- `export`：先 `stat` 且要求普通文件（目录错误 `目录暂不支持下载`）。
  - SSH：`mktemp -d /tmp/dbx-sshx.XXXXXX`（0700）→ `docker cp <id>:<quoted path> <quoted staging>/<safe name>` → 返回 `{stagingPath,size,name,location:"remote"}`。
  - 本机：`std::env::temp_dir()/dbx-sshx-<uuid>` + `docker cp` argv；返回 `location:"local"`。
  - 安全名：取容器路径 basename，替换 `/\\\0\r\n` 为 `_`；空名用 `download`。
- `prepareUpload`（仅 SSH）：`mktemp -d` + 返回 `<dir>/<safe name>`；文件名由参数 `name` 消毒。
- `import`：校验 staging 前缀；`destPath` 以 `/` 结尾时拼 safe name；SSH `docker cp <staging> <id>:<quoted dest>`；本机 argv；先 `ensure_writable`。写入一条审计（tool=`docker/files/import`、containerId、destPath、size），不含文件内容；复用 `features/security/audit` 的现有记录入口，必要时加一个只接收结构化字段的薄封装。
- `cleanup`：校验前缀后，SSH `rm -f -- <file> && rmdir -- <dir>`（或 `rm -rf -- <dir>` 仅校验通过）；本机 `remove_file` + `remove_dir`；返回 `{removed:true}`。
- `saveLocal`（仅本机目标）：校验 staging 为 local 前缀，调用 `save_local_path`，清理暂存目录。

- [ ] **Step 4: 实现 save_local_path**

在 `filesystem/local/downloads.rs` 增加：

```rust
pub fn save_local_path(
    data_dir: &Path,
    name: &str,
    source: &Path,
    conflict: Option<&str>,
) -> Result<Value, String> {
    let meta = std::fs::metadata(source)
        .map_err(|error| format!("Failed to read '{}': {error}", source.display()))?;
    if !meta.is_file() {
        return Err("Only files can be saved".to_string());
    }
    let dir = downloads_base_dir(|key| std::env::var_os(key), data_dir);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = final_download_path(&dir, &sanitize_file_name(name), matches!(conflict, Some("overwrite")));
    std::fs::copy(source, &path).map_err(|error| format!("Failed to write '{}': {error}", path.display()))?;
    // record_transition 与 save_local_file 相同字段，size 用 meta.len()
    Ok(serde_json::json!({ "localPath": path.to_string_lossy(), "name": path.file_name().unwrap_or_default().to_string_lossy() }))
}
```

测试：写入临时源文件 → `save_local_path` → 目标存在、内容一致、撞名让位、`overwrite` 覆盖。

- [ ] **Step 5: RPC 接线与编译**

`app/rpc/docker.rs` 增加五个分支；上传/导入前 `ensure_writable`；下载/导出允许只读。所有会话/本机/引擎参数复用现有解析函数。

Run:
```bash
cargo test --manifest-path backend/Cargo.toml docker::files filesystem::local::downloads -- --nocapture
cargo check --manifest-path backend/Cargo.toml --all-targets
```

- [ ] **Step 6: Commit**

```bash
git add backend/src/features/docker/files backend/src/features/docker/local.rs backend/src/filesystem/local/downloads.rs backend/src/app/rpc/docker.rs
git commit -m "feat(docker): stage container file transfers"
```

---

### Task 5: 本机分片上传与协议文档

**Files:**
- Create: `backend/src/features/docker/files/uploads.rs`
- Modify: `backend/src/features/docker/files/mod.rs`
- Modify: `backend/src/app/mod.rs`（`docker_uploads` 表）
- Modify: `backend/src/app/bootstrap.rs`
- Modify: `backend/src/app/rpc/docker.rs`
- Modify: `docs/architecture/PROTOCOL.zh-CN.md`
- Test: 内联

**Interfaces:**
- Produces RPC `docker/files/upload/begin|chunk|finish|cancel`。
- Produces: `UploadRegistry::{begin,chunk,finish,cancel}`；常量 `UPLOAD_MAX_BYTES = 512 * 1024 * 1024`、`UPLOAD_CHUNK_BYTES = 256 * 1024`。

- [ ] **Step 1: 写分片测试**

```rust
#[test]
fn rejects_out_of_order_offsets() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = UploadRegistry::new(dir.path().to_path_buf());
    let id = registry.begin("report.log").unwrap();
    assert!(registry.chunk(&id, 1, b"x").is_err());
    assert!(registry.chunk(&id, 0, b"abc").is_ok());
    assert!(registry.chunk(&id, 0, b"dup").is_err());
}

#[test]
fn rejects_oversized_upload() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = UploadRegistry::new(dir.path().to_path_buf());
    let id = registry.begin("big.bin").unwrap();
    assert!(registry.reserve(&id, UPLOAD_MAX_BYTES + 1).is_err());
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --manifest-path backend/Cargo.toml docker::files::uploads -- --nocapture`

- [ ] **Step 3: 实现上传注册表**

- `begin(name)`：创建 `<temp>/dbx-sshx-upload-<uuid>/<safe name>`，记录 `received=0`、`size_cap`，返回 `uploadId`；最多 4 个活跃上传。
- `chunk(id, offset, data)`：offset 必须等于 received；累计超过 512MiB 拒绝；`write_all` 后更新 received，返回 `{received}`。
- `finish(id)`：flush/close，返回 `{stagingPath,size}` 并保留条目供 `import` 后 cleanup；调用方负责 `cleanup`；条目在 30 分钟后过期删除。
- `cancel(id)`：关闭并删除临时目录。
- base64 解码在 RPC 层用现有 `BASE64_STANDARD`，单分片上限 `UPLOAD_CHUNK_BYTES`。

- [ ] **Step 4: RPC 与文档**

- `docker/files/upload/begin|chunk|finish|cancel` 走本机目标（`target:"local"`），不接受 SSH（SSH 用 `prepareUpload` + SFTP）。
- 更新 `PROTOCOL.zh-CN.md` docker 家族：`docker/logs/start|stop`、事件 `docker/logs/chunk|end`、`docker/files/*` 全家、参数/响应/限制/错误语义。
- `mock_host_registry_check.mjs` 的方法清单如包含 docker 家族，补新方法；不包含则跳过。

- [ ] **Step 5: 运行测试与编译**

Run:
```bash
cargo test --manifest-path backend/Cargo.toml docker -- --nocapture
cargo check --manifest-path backend/Cargo.toml --all-targets
node scripts/verify/mock_host_registry_check.mjs
```

- [ ] **Step 6: Commit**

```bash
git add backend/src/features/docker backend/src/app docs/architecture/PROTOCOL.zh-CN.md
git commit -m "feat(docker): add container file uploads and protocol docs"
```

---

### Task 6: 前端布局解耦与底部分屏外壳

**Files:**
- Modify: `frontend/src/app/workbench/state/useSftpState.ts`
- Modify: `frontend/src/app/workbench/state/useLayoutState.ts`
- Modify: `frontend/src/app/workbench/composables/useWorkbenchPanels.ts`
- Modify: `frontend/src/app/workbench/composables/useSplitDivider.ts`
- Modify: `frontend/src/app/components/WorkbenchWorkspace.vue`
- Modify: `frontend/src/app/components/WorkbenchSidePanel.vue`
- Modify: `frontend/src/app/components/WorkbenchToolbar.vue`
- Modify: `frontend/src/app/workbench/setup/{step1-core,step4-sessions,step7-sftp,step8-shell}.ts`
- Create: `frontend/src/features/docker/components/ContainerManagerPane.vue`（仅外壳/关闭/空态）
- Test: `frontend/src/app/workbench.spec.ts`、`frontend/src/app/workbench/panels.spec.ts`（新建）

**Interfaces:**
- Produces: `dockerPaneOpen: Ref<boolean>`、`dockerSplitRatio: Ref<number>`、`dockerPaneBasis: ComputedRef<{flexBasis:string}>`。
- Produces: `toggleDockerPane()`、`closeDockerPane()`；`RightPanelView = "sftp" | "metrics"`。
- `useWorkbenchPanels` options 增加 `dockerPaneOpen: Ref<boolean>` 与 `resetDockerSplitRatio?: () => void`。
- Consumes: `ContainerManagerPane` props `{ t, sessionId?: string, connectionId?: string }`，emit `close`。

- [ ] **Step 1: 写布局独立性测试**

```ts
// frontend/src/app/workbench/panels.spec.ts
// @vitest-environment happy-dom
import { describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { useWorkbenchPanels } from "./composables/useWorkbenchPanels";

function options() {
  return {
    paneOrder: ref("terminal-left"),
    sftpPaneOpen: ref(true),
    rightPanelView: ref("sftp"),
    dockerPaneOpen: ref(false),
    localUiMode: ref(false),
    isRdpConnection: ref(false),
    connected: ref(true),
    metricsOpen: () => false,
    openMetrics: vi.fn(),
    closeMetrics: vi.fn(),
    persistState: vi.fn(),
    scheduleFit: vi.fn(),
    resetDockerSplitRatio: vi.fn(),
  };
}

describe("workbench panels", () => {
  it("toggles the container pane without touching SFTP", () => {
    const input = options();
    const panels = useWorkbenchPanels(input);
    panels.toggleDockerPane();
    expect(input.dockerPaneOpen.value).toBe(true);
    expect(input.sftpPaneOpen.value).toBe(true);
    expect(input.rightPanelView.value).toBe("sftp");
    expect(input.resetDockerSplitRatio).toHaveBeenCalledOnce();
    panels.toggleDockerPane();
    expect(input.dockerPaneOpen.value).toBe(false);
    expect(input.sftpPaneOpen.value).toBe(true);
  });

  it("keeps the container pane when the SFTP panel closes", () => {
    const input = options();
    input.dockerPaneOpen.value = true;
    const panels = useWorkbenchPanels(input);
    panels.closeRightPanel();
    expect(input.dockerPaneOpen.value).toBe(true);
    expect(input.sftpPaneOpen.value).toBe(false);
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm --dir frontend exec vitest run src/app/workbench/panels.spec.ts`
Expected: 类型/参数错误（`dockerPaneOpen` 未定义）。

- [ ] **Step 3: 实现状态与开关**

- `useSftpState`：删除 `dockerPanelOpen` 派生，新增 `const dockerPaneOpen = ref(false);`，`RightPanelView` 去掉 `"docker"`。
- `useWorkbenchPanels`：options 增加 `dockerPaneOpen`；`openRightPanel` 拒绝 docker；新增：
  ```ts
  function toggleDockerPane() {
    if (isRdpConnection.value) return;
    dockerPaneOpen.value = !dockerPaneOpen.value;
    if (dockerPaneOpen.value) resetDockerSplitRatio?.();
    persistState();
    void nextTick(scheduleFit);
  }
  function closeDockerPane() {
    if (!dockerPaneOpen.value) return;
    dockerPaneOpen.value = false;
    persistState();
    void nextTick(scheduleFit);
  }
  ```
  `resetDockerSplitRatio` 作为可选回调注入，由 `useLayoutState` 提供 `resetDockerSplitRatio()`（设为 67）。
- `useLayoutState`：`paneLayout` 不再读 `dockerOpen`；新增 `dockerPaneBasis = computed(() => ({ flexBasis: `${100 - dockerSplitRatio.value}%` }))`；`orderedPaneClass` 去掉 `panes--docker-split`。
- `useSplitDivider`：新增 `startDockerDividerDrag(event)`、`onDockerDividerKeydown(event)`、`nudgeDockerDivider(deltaPx)`，使用 `terminalStackRef.getBoundingClientRect()`，比例 = clamp(40,80, `(pointerY - top)/height*100`)；沿用 pointer capture 与 cleanup。
- `WorkbenchWorkspace`：终端栈内新增：
  ```vue
  <template v-if="dockerPaneOpen && !isRdpConnection">
    <div class="docker-divider" role="separator" aria-orientation="horizontal"
         :aria-valuenow="Math.round(dockerSplitRatio)" aria-valuemin="20" aria-valuemax="60"
         tabindex="0" @pointerdown="startDockerDividerDrag"
         @keydown="onDockerDividerKeydown">
    </div>
    <ContainerManagerPane class="docker-pane-surface" :style="dockerPaneBasis"
      :t="t" :session-id="session?.sessionId" :connection-id="connectionId || undefined"
      @close="closeDockerPane" />
  </template>
  ```
- `WorkbenchSidePanel`：删除 Docker import、`sidePanelDocker` prop 与 docker 模板分支；`RightPanelView` 类型同步。
- `WorkbenchToolbar`：鲸鱼按钮 active 改为 `dockerPaneOpen`，点击 `toggleDockerPane`。
- step 接线：`step1-core` 导出 `dockerPaneOpen`；`step8-shell` 传给工具栏；`step7-sftp` 移除 sidePanelDocker；`step4-sessions` 在 RDP/断开时 `closeDockerPane()`。

- [ ] **Step 4: 新增底部分屏样式**

```css
/* layout.css 或 WorkbenchWorkspace scoped */
.terminal-stack { display: flex; flex-direction: column; min-height: 0; }
.terminal-stack > .terminal-workspace { flex: 1 1 auto; min-height: 0; }
.docker-pane-surface { flex: 0 0 auto; min-height: 120px; }
.docker-divider { height: 5px; flex: 0 0 5px; cursor: row-resize; position: relative; }
.docker-divider::after { content: ""; position: absolute; inset: 2px 0; background: var(--border); }
```

删除 `styles/global/layout.css` 的 `panes--docker-split` 规则；保留 SFTP 分栏规则。

- [ ] **Step 5: 运行布局测试与 typecheck**

Run:
```bash
pnpm --dir frontend exec vitest run src/app/workbench/panels.spec.ts src/app/workbench.spec.ts
pnpm --dir frontend typecheck
```
Expected: 全绿；现有 workbench 测试若引用 `dockerPanelOpen` 同步改名。

- [ ] **Step 6: Commit**

```bash
git add frontend/src/app frontend/src/features/docker/components/ContainerManagerPane.vue frontend/src/app/styles/global/layout.css
git commit -m "refactor(app): split container manager into a bottom pane"
```

---

### Task 7: 容器列表与详情外壳

**Files:**
- Create: `frontend/src/features/docker/components/ContainerListPane.vue`
- Create: `frontend/src/features/docker/components/ContainerDetailPane.vue`
- Modify: `frontend/src/features/docker/components/ContainerManagerPane.vue`
- Delete: `frontend/src/features/docker/components/DockerPanel.vue`
- Delete: `frontend/src/features/docker/components/DockerLogsDialog.vue`
- Modify/Delete: `DockerPanel.spec.ts`（改为 `ContainerListPane.spec.ts`）
- Test: `ContainerListPane.spec.ts`、`ContainerDetailPane.spec.ts`

**Interfaces:**
- `ContainerListPane` props `{t, sessionId?, connectionId?}`，emit `select(container)`、`refresh()`。
- `ContainerManagerPane` 持有 `selected`、`activeTab`，向详情传 `selected`。
- `ContainerDetailPane` props `{t, container, activeTab}`，emit `update:activeTab`、`close`。

- [ ] **Step 1: 写选择与页签测试**

```ts
import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import ContainerListPane from "./ContainerListPane.vue";
import ContainerDetailPane from "./ContainerDetailPane.vue";

it("selects the first running container", async () => {
  const wrapper = mount(ContainerListPane, { props: { t, sessionId: "s", connectionId: "c" } });
  await flushPromises();
  expect(wrapper.emitted("select")?.at(-1)?.[0]).toMatchObject({ name: "web-nginx", state: "running" });
});

it("switches tabs without unmounting the pane", async () => {
  const wrapper = mount(ContainerDetailPane, { props: { t, container, activeTab: "logs" } });
  await wrapper.get('[data-tab="files"]').trigger("click");
  expect(wrapper.emitted("update:activeTab")?.[0]).toEqual(["files"]);
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm --dir frontend exec vitest run src/features/docker/components/ContainerListPane.spec.ts src/features/docker/components/ContainerDetailPane.spec.ts`

- [ ] **Step 3: 实现列表**

- 复用 `useDockerContainers`（session/connection props）。
- 保留刷新、轮询暂停、needsSudo、动作菜单、kill/rm 确认、引擎设置。
- 列表行显示 name/image/state/ports；选中高亮；`watch(containers)` 应用选择规则；emit `select` 仅在选中变化时。
- 沿用现有 scoped 样式，拆分后每文件 ≤600 行。

- [ ] **Step 4: 实现详情外壳**

- 页头：容器名/状态、关闭按钮。
- 页签：日志、文件；`activeTab` 由 manager 持有。
- 日志/文件页先渲染真实空态（“选择容器后加载”/“打开日志以开始跟随”），后续任务填充；不引入临时占位 API。

- [ ] **Step 5: 运行测试与 typecheck**

Run:
```bash
pnpm --dir frontend exec vitest run src/features/docker
pnpm --dir frontend typecheck
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/docker
git commit -m "feat(docker): add container list and detail shell"
```

---

### Task 8: 日志缓冲、事件路由与流 composable

**Files:**
- Create: `frontend/src/features/docker/lib/dockerLogBuffer.ts`
- Create: `frontend/src/features/docker/lib/dockerLogEvents.ts`
- Create: `frontend/src/features/docker/composables/useDockerLogStream.ts`
- Modify: `frontend/src/app/workbench/composables/usePluginEvents.ts`
- Test: `dockerLogBuffer.spec.ts`、`useDockerLogStream.spec.ts`、`dockerLogEvents.spec.ts`（新增）

**Interfaces:**
- Produces: `LogBuffer` 类：`append(text: string): void`、`clear(): void`、`text(): string`、`lineCount`、`truncated`；常量 `MAX_LOG_LINES=10000`、`MAX_LOG_CHARS=2_000_000`。
- Produces: `shouldFollow(scrollTop, clientHeight, scrollHeight): boolean`（距底 ≤4px）。
- Produces: `registerDockerLogStream(streamId, handler)`、`dispatchDockerLogEvent(method, params)`。
- Produces: `useDockerLogStream(options: { sessionId?: () => string | undefined; connectionId?: () => string | undefined; onFollowChange?: (follow: boolean) => void })`，返回 `{ lines, text, follow, status, error, needsSudo, start, stop, restart, retrySudo, clear }`。

- [ ] **Step 1: 写缓冲与事件测试**

```ts
import { describe, expect, it } from "vitest";
import { LogBuffer, MAX_LOG_LINES, shouldFollow } from "./dockerLogBuffer";

it("joins partial lines across chunks", () => {
  const buffer = new LogBuffer();
  buffer.append("hel");
  buffer.append("lo\nworld");
  expect(buffer.text()).toBe("hello\nworld");
});

it("trims from the head at the line cap", () => {
  const buffer = new LogBuffer();
  buffer.append(Array.from({ length: MAX_LOG_LINES + 2 }, (_, i) => `line-${i}`).join("\n"));
  expect(buffer.lineCount).toBe(MAX_LOG_LINES);
  expect(buffer.text().startsWith("line-2\n")).toBe(true);
  expect(buffer.truncated).toBe(true);
});

it("detects follow from scroll distance", () => {
  expect(shouldFollow(900, 100, 1000)).toBe(true);
  expect(shouldFollow(850, 100, 1000)).toBe(false);
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm --dir frontend exec vitest run src/features/docker/lib/dockerLogBuffer.spec.ts`

- [ ] **Step 3: 实现缓冲与事件表**

- `LogBuffer` 内部保存 `lines: string[]`、`partial = ""`、`chars`；append 按 `\n` 拆分，末段留 partial；裁剪头部并维护 `truncated`；`MAX_LOG_CHARS` 同样从头部裁剪行。
- `dockerLogEvents.ts`：`Map<string, Handler>`；`register` 返回注销函数；`dispatch` 忽略未注册 streamId；handler 收到 `{method, params}`。
- `usePluginEvents` 顶层增加：
  ```ts
  if (event.method === "docker/logs/chunk" || event.method === "docker/logs/end") {
    dispatchDockerLogEvent(event.method, event.params);
    return;
  }
  ```

- [ ] **Step 4: 实现 useDockerLogStream**

- `start(containerId, {tail, sudo})`：先 `stop()`；`rpc("docker/logs/start", { sessionId/target, containerId, tail, sudo, ...engine })`；注册 handler；状态 `starting→streaming`。
- `chunk` handler：`buffer.append(params.text)`；触发 `text` 更新。
- `end` handler：状态 `ended`/`error`；`needsSudo` 标记；注销 handler；不自动重试。
- `stop()`：注销 + `rpc("docker/logs/stop", {streamId})`，幂等；清 buffer 可选。
- `restart()`：保留 containerId/tail/sudo，重新 start。
- `retrySudo()`：sudo=true 后 restart。
- `onScopeDispose(stop)`。
- `follow` 逻辑放 UI 层（Task 9），composable 只暴露 buffer/status。

- [ ] **Step 5: 运行测试**

Run:
```bash
pnpm --dir frontend exec vitest run src/features/docker/lib/dockerLogBuffer.spec.ts src/features/docker/composables/useDockerLogStream.spec.ts
pnpm --dir frontend typecheck
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/docker/lib/dockerLogBuffer.ts frontend/src/features/docker/lib/dockerLogEvents.ts frontend/src/features/docker/composables/useDockerLogStream.ts frontend/src/features/docker/lib/dockerLogBuffer.spec.ts frontend/src/features/docker/composables/useDockerLogStream.spec.ts frontend/src/app/workbench/composables/usePluginEvents.ts
git commit -m "feat(docker): wire streaming log events into the workbench"
```

---

### Task 9: 日志页 UI 与跟随交互

**Files:**
- Create: `frontend/src/features/docker/components/ContainerLogsTab.vue`
- Modify: `ContainerDetailPane.vue`、`ContainerManagerPane.vue`
- Test: `ContainerLogsTab.spec.ts`

**Interfaces:**
- Props `{t, container, stream}`（`stream` 为 `useDockerLogStream` 返回值）。
- 内部状态：`follow`、`tail`、`scroller` ref。

- [ ] **Step 1: 写跟随测试**

```ts
import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import ContainerLogsTab from "./ContainerLogsTab.vue";

const stream = {
  text: ref("line-1\nline-2"),
  status: ref("streaming"),
  error: ref(""),
  needsSudo: ref(false),
  follow: ref(true),
  start: vi.fn(),
  stop: vi.fn(),
  restart: vi.fn(),
  retrySudo: vi.fn(),
  clear: vi.fn(),
};

it("starts following and jumps to the bottom", async () => {
  const wrapper = mount(ContainerLogsTab, { props: { t, container, stream } });
  Object.defineProperty(wrapper.get(".docker-log-scroll").element, "scrollHeight", { value: 1000 });
  Object.defineProperty(wrapper.get(".docker-log-scroll").element, "clientHeight", { value: 100 });
  await wrapper.get(".docker-log-scroll").trigger("scroll");
  expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("true");
});

it("pauses follow when scrolled up and resumes on button", async () => {
  const wrapper = mount(ContainerLogsTab, { props: { t, container, stream } });
  const scroller = wrapper.get(".docker-log-scroll");
  Object.defineProperty(scroller.element, "scrollHeight", { value: 1000 });
  Object.defineProperty(scroller.element, "clientHeight", { value: 100 });
  scroller.element.scrollTop = 500;
  await scroller.trigger("scroll");
  expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("false");
  expect(wrapper.text()).toContain("已暂停跟随");
  await wrapper.get('[data-action="follow"]').trigger("click");
  expect(scroller.element.scrollTop).toBe(1000);
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm --dir frontend exec vitest run src/features/docker/components/ContainerLogsTab.spec.ts`

- [ ] **Step 3: 实现 UI**

- 顶部工具条：tail Select（复用 `TAIL_OPTIONS`）、跟随按钮（`data-action="follow"`、`aria-pressed`）、清空、重试/结束提示。
- `<pre class="docker-log-scroll" @scroll="onScroll">{{ stream.text.value }}</pre>`；`watch(text)` + `follow` 时 `nextTick(() => scroller.scrollTop = scrollHeight)`。
- `onScroll`：`follow = shouldFollow(...)`；滚回底部自动恢复。
- 按钮：`follow=true; nextTick(scrollToBottom)`；跟随时再点仅 `follow=false`。
- 结束/needsSudo 横幅按 spec；tail 变更调用 `stream.restart({tail})` 并清空。
- 页签切换到文件时不停止流；manager 在容器切换/关闭时调用 `stream.stop()`。

- [ ] **Step 4: 运行测试与 typecheck**

Run:
```bash
pnpm --dir frontend exec vitest run src/features/docker/components/ContainerLogsTab.spec.ts src/features/docker
pnpm --dir frontend typecheck
```

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/docker
git commit -m "feat(docker): add live-follow log tab"
```

---

### Task 10: 文件路径、目录浏览与文件页骨架

**Files:**
- Create: `frontend/src/features/docker/lib/dockerFilePaths.ts`
- Create: `frontend/src/features/docker/composables/useContainerFiles.ts`
- Create: `frontend/src/features/docker/components/ContainerFilesTab.vue`
- Modify: `ContainerDetailPane.vue`、`ContainerManagerPane.vue`
- Test: `dockerFilePaths.spec.ts`、`ContainerFilesTab.spec.ts`

**Interfaces:**
- Produces: `normalizeContainerPath(path)`, `parentPath(path)`, `joinContainerPath(dir,name)`, `sortEntries(entries)`。
- Produces: `useContainerFiles(options)` 返回 `{ path, entries, selected, loading, error, enter, parent, refresh, select }`。
- RPC list/stat 参数与 Task 3 一致。

- [ ] **Step 1: 写路径与浏览测试**

```ts
import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";
import { normalizeContainerPath, parentPath, joinContainerPath } from "./dockerFilePaths";
import ContainerFilesTab from "./ContainerFilesTab.vue";

vi.mock("../../../../shared/lib/rpc", () => ({ rpc: vi.fn() }));

it("normalizes and joins container paths", () => {
  expect(normalizeContainerPath("/var//log/../lib/")).toBe("/var/lib");
  expect(parentPath("/var/lib")).toBe("/var");
  expect(parentPath("/")).toBe("/");
  expect(joinContainerPath("/srv/app", "report.log")).toBe("/srv/app/report.log");
});

it("enters a directory and loads its entries", async () => {
  const wrapper = mount(ContainerFilesTab, { props: { t, container, sessionId: "s" } });
  await flushPromises();
  expect(wrapper.findAll(".docker-file-row").map((row) => row.text())).toContain("etc");
  await wrapper.get('[data-name="etc"]').trigger("dblclick");
  expect(rpc).toHaveBeenCalledWith("docker/files/list", expect.objectContaining({ path: "/etc" }));
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm --dir frontend exec vitest run src/features/docker/lib/dockerFilePaths.spec.ts`

- [ ] **Step 3: 实现路径与 composable**

- 路径函数与后端同规则；列表响应做防御性归一（非数组/非法条目丢弃）。
- `useContainerFiles`：`watch(container/sessionId)` 时重置到 `/` 并加载；`enter(dir)`、`parent()`、`refresh()`；错误保留供 UI 展示。
- stat 懒加载：`select(entry)` 时调用 `docker/files/stat`，结果存入 `selectedMeta`；`unavailable` 不报错。

- [ ] **Step 4: 实现文件页浏览 UI**

- 面包屑（祖先可点击）、上一级、刷新、上传按钮、条目列表（图标/名称/类型）。上传/下载按钮在本任务渲染为禁用态并带“传输功能未启用”文案，Task 11/12 接管点击事件。
- 目录双击进入；文件单击选中、双击触发下载（Task 11/12 接管；本任务保持禁用态）。
- 空态/错误态；容器缺 `ls` 时显示“容器内无 ls，无法浏览”。
- 隐藏文件直接显示。

- [ ] **Step 5: 运行测试与 typecheck**

Run:
```bash
pnpm --dir frontend exec vitest run src/features/docker/lib/dockerFilePaths.spec.ts src/features/docker/components/ContainerFilesTab.spec.ts
pnpm --dir frontend typecheck
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/docker
git commit -m "feat(docker): browse container files in the bottom pane"
```

---

### Task 11: SSH 容器文件下载/上传

**Files:**
- Modify: `frontend/src/features/sftp/composables/transfer/useSftpDownload.ts`
- Modify: `frontend/src/features/transfers/composables/useTransferOperations.ts`
- Modify: `frontend/src/app/workbench/setup/step7-sftp.ts`
- Create: `frontend/src/features/docker/lib/dockerFileTransfer.ts`
- Create: `frontend/src/features/docker/composables/useContainerFileTransfer.ts`
- Modify: `ContainerFilesTab.vue`
- Test: `dockerFileTransfer.spec.ts`、`useSftpDownload.spec.ts`（补充）

**Interfaces:**
- Produces: `downloadRemoteFile(input: { remotePath, name, size, forceSudo?: boolean }): Promise<void>`（从 `useSftpDownload` 返回）。
- Produces: `uploadSource(..., options?: { duplicatePreCheckedAbsent?: boolean; batchId?: string; displayName?: string })`。
- Produces: `useContainerFileTransfer({ t, sessionId, containerId, showError, showNotice, downloadRemoteFile, uploadSource, confirmDialog })`。

- [ ] **Step 1: 写下载/上传规划测试**

```ts
// 测试文件顶部：vi.mock("../../../../shared/lib/rpc", () => ({ rpc: vi.fn() }));
// 用 vi.fn() 注入 downloadRemoteFile/uploadSource/confirmDialog 到 composable。
it("exports then downloads then cleans up", async () => {
  rpc.mockResolvedValueOnce({ stagingPath: "/tmp/dbx-sshx.1/report.log", size: 3, name: "report.log", location: "remote" });
  await transfer.download({ name: "report.log", path: "/var/report.log" });
  expect(rpc).toHaveBeenNthCalledWith(1, "docker/files/export", expect.objectContaining({ path: "/var/report.log" }));
  expect(downloadRemoteFile).toHaveBeenCalledWith(expect.objectContaining({ remotePath: "/tmp/dbx-sshx.1/report.log", name: "report.log" }));
  expect(rpc).toHaveBeenLastCalledWith("docker/files/cleanup", { stagingPath: "/tmp/dbx-sshx.1/report.log" });
});

it("cleans up when the download fails", async () => {
  downloadRemoteFile.mockRejectedValueOnce(new Error("boom"));
  await expect(transfer.download(file)).rejects.toThrow("boom");
  expect(rpc).toHaveBeenCalledWith("docker/files/cleanup", expect.anything());
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm --dir frontend exec vitest run src/features/docker/composables/useContainerFileTransfer.spec.ts`

- [ ] **Step 3: 抽取通用下载函数**

在 `useSftpDownload.ts`：
- 把 `downloadEntry(entry, forceSudo?)` 主体抽成 `downloadRemoteFile({ remotePath, name, size, forceSudo })`；`downloadEntry` 只做 entry→参数的适配。
- `downloadRemoteFile` 使用 `remotePath` 替代 `pathFromUri(entry.uri)`，其余保存目录/冲突/主机保存逻辑不变。
- 返回值加入 `downloadRemoteFile`；`step7-sftp` 解构并导出。
- 现有 `useSftpDownload.spec.ts` 全绿；新增一个用例直接调用 `downloadRemoteFile` 证明不依赖树条目。

- [ ] **Step 4: 增加上传显示名**

`uploadSource` options 增加 `displayName?: string`；`transferTasks` 建卡与进度事件保持后端文件名，但任务卡 `fileName` 用 `options.displayName ?? uploadName`。仅展示影响，不变远端路径。

- [ ] **Step 5: 实现 SSH 传输 composable**

- `download(file)`：`resolveMode` → `docker/files/export` → `downloadRemoteFile({remotePath: stagingPath, name, size})` → `finally cleanup`。
- `upload(file: File)`：`docker/files/prepareUpload({name})` → `uploadSource(name, size, readChunk, undefined, stagingDir, { duplicatePreCheckedAbsent: true, displayName: name })`，其中 `targetDir` 为 stagingPath 的父目录、`name` 为 staging 文件名 → `docker/files/import({stagingPath, destPath})` → `finally cleanup`。
- `readChunk` 用 `file.slice(...).arrayBuffer()`。
- 覆盖确认：上传前 `docker/files/stat(destPath)`；存在则 `confirmDialog`。
- 只读连接：`canWrite` 为 false 时上传入口禁用。

- [ ] **Step 6: 接入文件页**

- 下载按钮/双击文件调用 `transfer.download`；目录禁用。
- 上传按钮触发隐藏 `<input type=file>`，选择后调用 `transfer.upload`。
- 传输中显示行内 busy；SSH 进度由现有传输面板展示。

- [ ] **Step 7: 运行测试与 typecheck**

Run:
```bash
pnpm --dir frontend exec vitest run src/features/docker src/features/sftp/composables/transfer
pnpm --dir frontend typecheck
```

- [ ] **Step 8: Commit**

```bash
git add frontend/src/features/sftp/composables/transfer/useSftpDownload.ts frontend/src/features/transfers/composables/useTransferOperations.ts frontend/src/app/workbench/setup/step7-sftp.ts frontend/src/features/docker
git commit -m "feat(docker): transfer container files over SSH"
```

---

### Task 12: 本机容器文件下载/上传

**Files:**
- Modify: `frontend/src/features/docker/composables/useContainerFileTransfer.ts`
- Modify: `ContainerFilesTab.vue`
- Test: `useContainerFileTransfer.spec.ts`

- [ ] **Step 1: 写本机分支测试**

```ts
it("saves a local staging file to Downloads", async () => {
  rpc.mockResolvedValueOnce({ stagingPath: "/tmp/dbx-sshx.1/report.log", size: 3, name: "report.log", location: "local" });
  rpc.mockResolvedValueOnce({ localPath: "/Users/me/Downloads/report.log" });
  await transfer.download(file);
  expect(rpc).toHaveBeenCalledWith("docker/files/saveLocal", { target: "local", stagingPath: "/tmp/dbx-sshx.1/report.log", name: "report.log" });
  expect(rpc).toHaveBeenCalledWith("docker/files/cleanup", { target: "local", stagingPath: "/tmp/dbx-sshx.1/report.log" });
});

it("uploads chunks in order and imports", async () => {
  rpc.mockResolvedValueOnce({ uploadId: "u1", maxBytes: 512 * 1024 * 1024 });
  rpc.mockResolvedValueOnce({ received: 3 });
  rpc.mockResolvedValueOnce({ stagingPath: "/tmp/dbx-sshx-upload-u1/report.log", size: 3 });
  await transfer.upload(new File(["abc"], "report.log", { type: "text/plain" }));
  expect(rpc).toHaveBeenCalledWith("docker/files/upload/chunk", expect.objectContaining({ target: "local", uploadId: "u1", offset: 0 }));
  expect(rpc).toHaveBeenCalledWith("docker/files/import", expect.objectContaining({ target: "local", stagingPath: "/tmp/dbx-sshx-upload-u1/report.log" }));
});
```

- [ ] **Step 2: 运行确认失败**

Run: `pnpm --dir frontend exec vitest run src/features/docker/composables/useContainerFileTransfer.spec.ts`

- [ ] **Step 3: 实现本机下载分支**

- `export` 返回 `location:"local"` 时调用 `docker/files/saveLocal`（带 `conflict`），成功后 notice + `local/reveal` 动作，`finally cleanup`。
- 下载进度：本机保存为单次 RPC，显示不限期 busy；失败保留错误。

- [ ] **Step 4: 实现本机分片上传**

- `upload/begin` → 循环 256KiB 分片读取 File → base64 → `upload/chunk`（校验 `received` 递增）→ `upload/finish` → `import` → cleanup。所有本机上传 RPC 都传 `target:"local"`，与测试断言一致。
- 进度：`uploaded/size` 百分比；取消按钮调用 `docker/files/upload/cancel` + cleanup；组件卸载取消。
- 512MiB 前端预检；超限直接错误，不建上传。

- [ ] **Step 5: 运行测试与 typecheck**

Run:
```bash
pnpm --dir frontend exec vitest run src/features/docker/composables/useContainerFileTransfer.spec.ts src/features/docker
pnpm --dir frontend typecheck
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/docker
git commit -m "feat(docker): transfer container files on the local daemon"
```

---

### Task 13: Mock 宿主、i18n、文档与浏览器夹具

**Files:**
- Modify: `frontend/src/app/mock/host/docker.ts`
- Modify: `frontend/src/app/mockDbxHost.spec.ts`（补方法形状）
- Modify: `frontend/src/shared/lib/i18n/messages/base/{en,zh-CN,zh-TW,ja,es,it,pt-BR}.ts`
- Modify: `docs/architecture/PROTOCOL.zh-CN.md`（若 Task 5 未完成）
- Modify: `docs/architecture/FEATURE_PARITY.zh-CN.md`
- Create: `frontend/src/features/docker/README.md`
- Create: `scripts/smoke/smoke_docker_files.py`
- Test: `mock_host_registry_check.mjs`、i18n key guard、`mockDbxHost.spec.ts`

- [ ] **Step 1: 添加 mock 日志流与文件方法**

`mock/host/docker.ts`：
- `docker/logs/start`：校验 id/tail，返回 `{streamId:"mock-stream"}`；用 `setInterval` 每 700ms `emitHostEvent({method:"docker/logs/chunk", params:{streamId, text: ...}})`；10s 后发 end。
- `docker/logs/stop`：清 interval，返回 `{stopped:true}`。
- `docker/files/list`：按路径返回固定树（`/`、`/etc`、`/var`）。
- `docker/files/stat|export|prepareUpload|import|cleanup|saveLocal|upload/*`：返回稳定夹具路径；`cleanup` 幂等。
- 用 fixture 参数 `?docker=` 扩展 `off|sudo|files`，供浏览器走查。

- [ ] **Step 2: 补测试**

- `mockDbxHost.spec.ts`：断言 start 返回 streamId、chunk 事件经 onEvent 到达、stop 后不再有事件；`docker/files/list` 形状。
- `mock_host_registry_check.mjs` 全绿。

- [ ] **Step 3: 补 i18n 7 语言**

按 spec §8 的 key 列表补全；key 必须字面量出现在代码中。运行现有 i18n 引用守卫。

- [ ] **Step 4: 更新协议与能力文档**

- `PROTOCOL.zh-CN.md`：新增 docker logs 流式与 files 家族章节，声明限制（512MiB、目录不可下载、`ls` 依赖、30 分钟 TTL）。
- `FEATURE_PARITY.zh-CN.md`：更新 Docker 行为条目（底部分屏、日志跟随、文件浏览/传输），标注 SSH/本机覆盖与已知限制。
- 新增 `features/docker/README.md`：说明组件/composable 划分与“暂停跟随”语义。
- 新增 `scripts/smoke/smoke_docker_files.py`：本机 Docker 可用时创建临时容器，跑 list → logs follow 2 秒 → export/saveLocal → prepareUpload/import → cleanup；`docker images` 无可用镜像时打印 SKIP 并 exit 0。

- [ ] **Step 5: 运行守卫**

Run:
```bash
pnpm --dir frontend exec vitest run src/shared/lib/i18n src/app/mockDbxHost.spec.ts
node scripts/verify/mock_host_registry_check.mjs
pnpm --dir frontend exec vitest run src/shared/lib/structure.spec.ts
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src/app/mock frontend/src/app/mockDbxHost.spec.ts frontend/src/shared/lib/i18n docs frontend/src/features/docker/README.md
git commit -m "test(docker): cover container manager mocks and copy"
```

---

### Task 14: 全量验证与 beta.32 打包

**Files:** 版本与 CHANGELOG（实现完成后由控制器执行）

- [ ] **Step 1: 全量门禁**

Run:
```bash
scripts/test.sh --skip-host
```
Expected: repo contract、后端测试、前端 typecheck/test、dead-code、MCP/本地 smoke 全绿；live 容器相关 SKIP 需记录原因。

- [ ] **Step 2: 浏览器夹具走查**

Run Vite fixture（`pnpm --dir frontend exec vite --port 5299`），打开 `http://localhost:5299/visual.html`：
- 打开 SFTP → 点鲸鱼 → 断言 SFTP 仍在、容器页在终端下方。
- 拖动分隔条 → 终端 fit；关闭 → 终端恢复。
- 日志：默认跟随、上滚暂停、点按钮跳底。
- 文件：进入目录、返回、上传/下载 mock 流程、清理提示。

- [ ] **Step 3: 本机 Docker 冒烟（有 Docker 时）**

Run:
```bash
python3 scripts/smoke/smoke_docker_local.py
python3 scripts/smoke/smoke_docker_files.py   # 本任务新增；无镜像/容器时 SKIP
```
记录 list/logs/上传/下载/清理结果；无 Docker 环境明确报告 SKIP。

- [ ] **Step 4: 版本与 CHANGELOG**

- `manifest.json`、`backend/Cargo.toml`、`backend/Cargo.lock` 升到 `0.7.9-beta.32`。
- `CHANGELOG.md` 增加 `[0.7.9-beta.32] — 2026-10-26`，描述底部分屏、日志跟随、文件浏览/传输与限制。
- 运行 `python3 scripts/verify/validate_repo.py`。

- [ ] **Step 5: 打包与哈希**

Run:
```bash
scripts/build.sh
scripts/release/package_and_verify.sh --no-build
shasum -a 256 dist/io.dbx.sshx-0.7.9-beta.32-darwin-arm64.dbxp
```
记录产物路径、大小、SHA-256；安装/实机验收交给用户。

- [ ] **Step 6: Commit**

```bash
git add manifest.json backend/Cargo.toml backend/Cargo.lock CHANGELOG.md docs/superpowers
git commit -m "chore: release beta.32"
```

---

## Self-Review Notes

- 覆盖 spec 的布局、日志、文件、i18n、测试与限制；`docker/logs` 快照 RPC 保留。
- 所有新接口在前后任务中名称一致：`dockerPaneOpen`、`dockerSplitRatio`、`dockerPaneBasis`、`downloadRemoteFile`、`uploadSource(displayName)`、`DockerStreams`、`ChunkBatcher`。
- 不含 TBD/TODO；每个任务有失败测试、实现、通过测试与提交步骤。
- 计划按 Task 1–14 顺序执行；Task 6 是前端可见的第一个可验收里程碑，Task 9/12 分别完成日志与文件能力。
