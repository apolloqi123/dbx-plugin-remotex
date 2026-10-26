# 容器管理底部分屏设计

状态：待评审（2026-10-26）
分支：`refactor/structure-v2`（后续实现分支可另开）
范围：容器管理从右侧栏互斥视图迁为终端列底部独立分屏；日志真流式跟随；容器内文件浏览与上传下载。

## 1. 背景与目标

### 1.1 现状

- 容器面板 `DockerPanel` 目前作为右侧栏第三视图，与 SFTP/指标互斥；打开容器必须牺牲 SFTP。
- 布局层已有 `dockerSplitRatio` 与 `panes--docker-split` 的“终端在上、右栏整体变底栏”历史实现，但它仍复用右栏容器，因此 SFTP 与容器无法并存。
- 日志是 `docker logs --tail N` 的一次性快照，弹窗展示，刷新需手动点击。
- 没有容器文件浏览能力。

### 1.2 目标

1. 容器管理成为终端列底部的独立分屏，默认占 33%，可拖拽/键盘调整；右侧 SFTP 保持原视图、原宽度、原开合状态。
2. 日志默认实时跟随；向上滚动暂停自动滚动但继续接收；点“实时跟随”恢复并跳到最新。
3. 支持浏览容器内文件、下载到本机、上传到容器；SSH 与本机 Docker 两类目标都支持。
4. 不改变 SFTP、指标、终端、RDP/VNC 的既有行为；不改现有 docker/list、docker/action、docker/logs 的协议语义。

### 1.3 非目标

- 不做目录递归下载、文件夹上传、容器内文件编辑/删除/重命名/改权限。
- 不做内联文本/图片预览（下载后用本机应用打开）。
- 不新增独立 DBX 工作台贡献点，不新开宿主窗口/tab。
- 不引入 Docker 客户端 crate，不直接访问 Docker daemon API。
- 不持久化日志；容器页开合状态不跨刷新恢复。

## 2. 布局与状态

### 2.1 视图解耦

- `rightPanelView` 收敛为 `"sftp" | "metrics"`；删除 Docker 互斥视图。
- 新增独立布尔状态 `dockerPaneOpen`（替代由 `rightPanelView` 派生的 `dockerPanelOpen`），初始 `false`。
- 工具栏鲸鱼按钮只切换 `dockerPaneOpen`；不调用 `openRightPanel`，不改 `sftpPaneOpen`。
- 关闭右侧栏（`closeRightPanel`）不影响 `dockerPaneOpen`；关闭容器页也不影响 SFTP。

### 2.2 终端列内部布局

`WorkbenchWorkspace` 的 `.terminal-stack` 改为纵向三段（编辑器有页签时才占第一段）：

```
EditorPane（可选）
editor-divider（可选）
terminal-workspace（flex: 1 1 auto，min-height: 0）
docker-divider（dockerPaneOpen 时）
ContainerManagerPane（dockerPaneOpen 时，flex-basis: 100 - dockerSplitRatio）
```

- `dockerSplitRatio` 语义为“终端列中终端+编辑器占比”，默认 67（容器 33%），范围 40–80（容器 20%–60%）；每次打开容器页重置为 67。
- 拖动 `docker-divider` 或上下方向键微调（±16px，Shift ×3）更新 `dockerSplitRatio`，调用 `scheduleFit()`。
- 容器页关闭时保留比例值但不渲染；下次打开重置为默认。
- 容器页只在 `dockerPaneOpen && !isRdpConnection` 时渲染；切到 RDP 连接自动关闭。
- 本地 UI 模式允许打开（本机 daemon）；SFTP 在本地模式仍按既有规则不可用。
- 刷新/重挂载后容器页默认关闭，不启动列表轮询或日志流。

### 2.3 涉及状态与组件

- `useSftpState`：删除 `dockerPanelOpen` 派生，新增 `dockerPaneOpen`。
- `useWorkbenchPanels`：`openRightPanel` 仅接受 sftp/metrics；新增 `toggleDockerPane`、`closeDockerPane`。
- `useLayoutState`：`paneLayout` 不再受 Docker 影响；保留 `dockerSplitRatio`，并新增 `dockerPaneBasis`（`{ flexBasis: `${100 - dockerSplitRatio}%` }`）供容器页内联样式使用。
- `useSplitDivider`：新增容器页把手的指针/键盘处理，沿用既有 pointer capture 与取消清理模式。
- `useWorkbenchModals` 的 Esc 链：容器页最上层为内部弹层（确认/引擎设置）时不关闭页面；无弹层时 Esc 关闭容器页。
- `WorkbenchSidePanel`：删除 Docker 视图与 `sidePanelDocker` 传递。
- `workbenchState` 持久化字段不新增；`dockerPaneOpen` 不落盘。

## 3. 容器页 UI

新组件树（均在 `frontend/src/features/docker/`）：

```
components/ContainerManagerPane.vue      页面外壳：选择、页头、页签、弹层
components/ContainerListPane.vue         容器列表/轮询/动作/引擎设置
components/ContainerDetailPane.vue       选中容器详情与页签
components/ContainerLogsTab.vue          实时日志视图
components/ContainerFilesTab.vue         文件浏览与传输
```

复用：`DockerRowMenu`、`DockerConfirmDialog`、`DockerEnginePopover`、`useDockerContainers`、`dockerActions`、`dockerPanelModel`。

### 3.1 外壳

- 页头：标题、本机/远端徽标、引擎设置齿轮、刷新、关闭。
- 主体左右分栏：左侧列表（宽 38%，最小 240px，最大 420px），右侧详情（flex）。
- 选中规则：列表首次加载后选中第一个 `running` 容器，否则第一个；轮询后选中项消失则回退同一规则；切换选中停止旧日志流并按需启动新流。
- 列表保留现有 10s 轮询、3 连败暂停、重试、动作菜单、kill/rm 确认、needsSudo 提示。
- 容器页关闭/组件卸载时停止轮询与日志流，并释放上传临时文件。

### 3.2 日志页

- 打开日志页才启动流；切到文件页不停止流（避免返回时丢历史），切换容器或关闭页面时才停止。
- 默认“跟随”：新日志到达后滚到底部。
- 用户向上滚动超过 24px：`follow=false`，显示“已暂停跟随”标记；流继续接收并进入缓冲区。
- 用户滚回距底部 ≤4px：自动恢复 `follow=true`。
- 点击“实时跟随”按钮：`follow=true` 并滚到底部；按钮在跟随时为激活态，再点一次仅关闭自动滚动（不停止流）。
- 缓冲区：最多 10,000 行或 2,000,000 字符，超限从头部裁剪并显示“已截断”提示。
- tail 选择沿用 100/200/500/1000/2000；变更即停止当前流、清空缓冲区、以新 tail 重启。
- 清空按钮只清视图，不断流。
- 流结束：保留已有日志，显示结束原因/退出码与“重新开始”按钮；若结束原因是权限不足，显示“以 sudo 重试”（只读连接禁用）。
- 滚动容器使用固定高度 `flex: 1; min-height: 0; overflow: auto`，禁止整页滚动。

## 4. 日志真流式协议

### 4.1 RPC 与事件

| 名称 | 方向 | 载荷 |
| --- | --- | --- |
| `docker/logs/start` | 请求 | `{sessionId}` 或 `{target:"local"}`；`containerId`；`tail`；`sudo?`；可选 `cli/socket/host` |
| | 响应 | `{streamId}` |
| `docker/logs/stop` | 请求 | `{streamId}` |
| | 响应 | `{stopped: boolean}` |
| `docker/logs/chunk` | 事件 | `{streamId, text}`，UTF-8 有损解码，可能切在半行 |
| `docker/logs/end` | 事件 | `{streamId, exitCode?: number, error?: string, needsSudo?: boolean}` |

- `containerId` 复用 `validate_container_id`（12–64 位小写十六进制）。
- `tail` 钳制 10–2000，默认 200。
- `sudo` 仅允许非只读连接且连接启用 Quick Sudo；否则返回明确错误。
- 每个 sidecar 实例最多 4 条活跃流；超过返回错误。单条流最长 30 分钟，到期自动结束并发送 end 事件。
- 流不落盘；事件不进入会话回放或录制。
- 现有快照 RPC `docker/logs` 保留原语义，供兼容调用与离线冒烟使用；新 UI 只用流式接口。

### 4.2 sidecar 实现

- 新增 `features/docker/stream.rs`：流注册表（`streamId -> cancel watch + task`）、启动/停止、上限与 TTL、事件发送。
- SSH：
  - 在 `sessions/ssh/exec` 增加流式执行。复用现有 channel 打开、env、sudo 提示编排与取消路径；`run_to_completion` 增加可选 chunk sink，现有调用传 `None`，行为不变。
  - chunk 经有界通道（64 个分片）交给 sidecar 转发任务；发送与取消用 `select!` 竞争，避免宿主卡住时无法取消。
  - 命令文本：`{engine.env_prefix()}{cli} logs -f --tail {tail} {containerId}`；containerId 已校验，tail 已钳制。
  - 取消时走现有 `abort_exec_channel` 路径关闭 channel；不裸 abort，避免远端进程残留。
- 本机：
  - `features/docker/local.rs` 增加流式子进程：`tokio::process::Command`，`kill_on_drop(true)`，stdout/stderr 管道，读取循环与取消 `select!`。
  - 复用 `find_local_cli`、端点 env、Windows argv 直执。
- 转发任务按约 100ms 或 8KiB 合并 chunk，再调用 `emitter.emit("docker/logs/chunk", ...)`；结束时发送一次 `docker/logs/end` 并移除注册表条目。
- 权限不足：
  - 远端 stderr/退出码命中现有 daemon 权限签名时，end 事件携带 `needsSudo:true`；不自动二次启动，UI 提供显式 sudo 重试。
  - 本地模式无 Quick Sudo 管线，end 事件给出 docker group/Docker Desktop 指引。
- 会话断开、容器停止、channel 关闭均以 end 事件收口；前端保留已有日志。

### 4.3 前端流管理

- 新增 `useDockerLogStream`：start/stop/restart/tail/sudoRetry、状态、行缓冲、部分行拼接、follow 状态。
- 事件经 `usePluginEvents` 的单一订阅入口分派到 `features/docker/lib/dockerLogEvents.ts` 的处理器表；不在组件内新增第二个 `onEvent` 订阅。
- `stop` 幂等，先摘处理器再调用 RPC；组件卸载、容器切换、页面关闭均调用。
- 纯逻辑（分片拼行、裁剪、follow 判定）放在 `lib/dockerLogBuffer.ts`，无 Vue 依赖，便于单测。

## 5. 文件浏览与传输

### 5.1 浏览

- 文件页默认路径 `/`；面包屑可点击任意祖先；“上一级”“刷新”。
- `docker/files/list`：
  - 请求：会话/本机目标、`containerId`、`path`、引擎参数。
  - 命令：SSH `docker exec <id> ls -1ap -- <shell-quoted path>`；本机 argv `exec <id> ls -1ap -- <path>`。
  - 解析：按行；去 `\r`；丢弃空行、`.`、`..`；尾部 `/` 判定目录；其余一律按文件展示。
  - 响应：`{path, parent, entries:[{name, path, kind:"dir"|"file"}]}`；目录优先、名称升序。
- `docker/files/stat`：选中条目时按需获取 `{kind,size,mtime,mode}`；命令 `docker exec <id> stat -c '%F|%s|%Y|%a' -- <path>`。`stat` 缺失时返回 `unavailable`，UI 显示“—”，不影响下载。
- 路径规则：必须是绝对路径；拒绝 NUL/控制字符；折叠 `.`/`..`（容器内语义）；容量上限 4096 字符。
- 容器缺少 `ls` 时返回明确错误（含 stderr），文件页显示“容器内无 ls，无法浏览”，并禁用上传/下载入口。
- 已知限制：文件名含换行会显示不完整；v1 不处理；符号链接按 Docker CLI 默认行为拷贝，失败时展示错误。
- v1 目录可进入但不可下载；下载按钮对目录禁用并提示。

### 5.2 RPC

| 名称 | 用途 | 关键载荷 |
| --- | --- | --- |
| `docker/files/list` | 列目录 | `path` → `{path,parent,entries}` |
| `docker/files/stat` | 条目详情 | `path` → `{kind,size,mtime,mode,unavailable?}` |
| `docker/files/export` | 容器文件 → 暂存 | `path` → `{stagingPath,size,name,location:"remote"|"local"}` |
| `docker/files/prepareUpload` | SSH 上传暂存路径 | `name` → `{stagingPath,name}` |
| `docker/files/import` | 暂存 → 容器 | `stagingPath,destPath` → `{path}` |
| `docker/files/cleanup` | 删除暂存 | `stagingPath` → `{removed}` |
| `docker/files/saveLocal` | 本机暂存 → 下载目录 | `stagingPath,name,conflict?` → `{localPath}` |
| `docker/files/upload/begin` | 本机上传开始 | `name` → `{uploadId,maxBytes}` |
| `docker/files/upload/chunk` | 本机上传分片 | `uploadId,offset,dataBase64` → `{received}` |
| `docker/files/upload/finish` | 本机上传完成 | `uploadId` → `{stagingPath,size}` |
| `docker/files/upload/cancel` | 本机上传取消 | `uploadId` → `{canceled}` |

所有传输 RPC 与列表共用会话/本机目标与引擎参数解析；上传/导入走 `ensure_writable`，只读连接拒绝。下载/浏览允许只读连接。

### 5.3 SSH 下载链路

1. `docker/files/export` 先 `stat` 确认目标是普通文件（目录直接拒绝并提示“目录暂不支持下载”），在远端创建临时目录（`mktemp -d /tmp/dbx-sshx.XXXXXX`，权限 0700），执行 `docker cp <id>:<path> <stagingDir>/<name>`，返回暂存路径与大小。
2. 前端把该暂存路径作为一次普通 SFTP 下载任务加入现有传输队列，显示名覆盖为容器内文件名；进度/取消/撞名沿用现有 UI。
3. 任务完成、失败或取消后调用 `docker/files/cleanup` 删除暂存目录；清理失败只提示，不改变传输结果。

### 5.4 本机下载链路

1. `docker/files/export` 先 `stat` 确认目标是普通文件（目录拒绝），在本机临时目录执行 `docker cp`，返回 `location:"local"` 与暂存路径。
2. 前端调用 `docker/files/saveLocal`：新增 `save_local_path` 复用现有下载目录、撞名让位/覆盖、传输历史与 `local/reveal` 语义，把暂存文件移动/复制到 Downloads。
3. 成功后清理暂存文件；`saveLocal` 只接受本功能生成的暂存前缀。

### 5.5 SSH 上传链路

1. 前端调用 `docker/files/prepareUpload` 获取远端暂存路径（临时目录 + 消毒后的文件名）。
2. 用现有 SFTP 上传队列把用户选择的单个文件写到该暂存路径；进度/取消沿用现有 UI。
3. 上传成功后调用 `docker/files/import`：远端执行 `docker cp <stagingPath> <id>:<destPath>`。`destPath` 以 `/` 结尾时视为目录并保留暂存文件名，否则视为完整目标文件路径。
4. 成功/失败/取消后清理暂存目录。

### 5.6 本机上传链路

1. `docker/files/upload/begin` 创建临时文件并返回 `uploadId`；上限 512MiB。
2. 前端按 256KiB 分片读取 File，base64 后调用 `upload/chunk`；必须按顺序、offset 必须等于已接收字节数，否则拒绝。
3. `upload/finish` 关闭文件并返回暂存路径；随后调用 `docker/files/import`。
4. 取消或失败删除临时文件；上传页显示字节进度与取消按钮。
5. 每个 sidecar 最多 4 个并发上传；未完成上传 30 分钟过期并删除临时文件。

### 5.7 目标与覆盖

- 上传目标为当前目录 + 原文件名；若目标已存在（`stat` 命中），先弹确认覆盖，取消则不上传。
- 下载撞名沿用现有传输队列/本机保存的“让位”或“覆盖”语义；本机保存把 `conflict` 透传给 `save_local_path`。
- 暂存文件名由服务端消毒，不使用用户路径拼接宿主机路径。
- 所有容器文件传输的单文件上限为 512MiB；超过时前端在上传/下载开始前拒绝并提示。
- 本机保存无额外内存上限（文件移动，不整体读入内存）。

### 5.8 安全与清理

- `cleanup` 只接受本功能生成的暂存路径：远端必须以 `/tmp/dbx-sshx.` 开头且包含本次 UUID 目录；本机必须规范化后位于系统临时目录且带同前缀。拒绝任意路径删除。
- 删除命令/调用统一使用 `rm -f -- <shell-quoted>`（远端）或 `std::fs::remove_file`（本机）；目录用 `remove_dir_all` 仅限校验通过的暂存目录。
- 上传/导入写审计：记录 `tool`、containerId、目标路径、字节数，不记录文件内容。
- 容器内路径只影响容器，不构成宿主机路径穿越；仍拒绝 NUL/控制字符，避免命令/协议破坏。
- 下载/导出不写审计（只读操作），但本机保存沿用传输历史。

## 6. 错误处理与生命周期

- 列表/动作沿用现有错误文案与 needsSudo 逻辑。
- 日志流：start 失败回显错误；end 保留日志；权限不足给 sudo 重试；会话断开提示重连。
- 文件页：容器消失、路径不存在、`ls`/`stat` 缺失、权限不足分别给出可操作错误；不显示空目录伪装成功。
- 传输：暂存创建失败、`docker cp` 失败、SFTP 上传/下载失败、清理失败均可见；清理失败不覆盖主错误。
- 组件卸载顺序：停止日志流 → 取消上传 → 停止轮询 → 清理暂存。
- 后端流注册表与上传注册表在任务结束时自清理；30 分钟 TTL 防止 webview 崩溃遗留。

## 7. 代码落点

前端：

```
features/docker/components/ContainerManagerPane.vue
features/docker/components/ContainerListPane.vue
features/docker/components/ContainerDetailPane.vue
features/docker/components/ContainerLogsTab.vue
features/docker/components/ContainerFilesTab.vue
features/docker/composables/useDockerLogStream.ts
features/docker/composables/useContainerFiles.ts
features/docker/lib/dockerLogBuffer.ts
features/docker/lib/dockerLogEvents.ts
features/docker/lib/dockerFilePaths.ts
features/docker/lib/dockerFileTransfer.ts
app/components/WorkbenchWorkspace.vue
app/workbench/composables/useWorkbenchPanels.ts
app/workbench/composables/useSplitDivider.ts
app/workbench/state/useLayoutState.ts
app/workbench/state/useSftpState.ts
app/components/WorkbenchSidePanel.vue
app/components/WorkbenchToolbar.vue
app/workbench/setup/step1-core.ts / step4-sessions.ts / step7-sftp.ts / step8-shell.ts
app/workbench/composables/usePluginEvents.ts
```

删除：`DockerLogsDialog.vue`、`DockerPanel.vue`（由新组件替代）；`DockerPanel.spec.ts` 迁移为新的列表/详情测试。

后端：

```
features/docker/stream.rs
features/docker/files/mod.rs
features/docker/files/list.rs
features/docker/files/transfer.rs
features/docker/files/uploads.rs
features/docker/local.rs（补流式进程）
features/docker/scripts.rs（补 logs follow 命令构造）
sessions/ssh/exec/streaming.rs（新）
sessions/ssh/exec/run.rs（可选 sink）
sessions/ssh/execution/streaming.rs（新）
app/rpc/docker.rs
app/mod.rs（注册流/上传表）
filesystem/local/downloads.rs（save_local_path）
```

所有新文件受 600 行预算约束，测试文件受 1,200 行约束；不允许新增 allowlist。

## 8. i18n

新增 key 至少包括：

- `docker.managerTitle`、`docker.logsTab`、`docker.filesTab`
- `docker.followLive`、`docker.followPaused`、`docker.followResume`
- `docker.logsCleared`、`docker.streamEnded`、`docker.streamRetry`、`docker.sudoRetry`
- `docker.filesParent`、`docker.filesRefresh`、`docker.filesEmpty`、`docker.filesDownload`、`docker.filesUpload`
- `docker.filesUploading`、`docker.filesDownloading`、`docker.filesCancel`
- `docker.filesOverwriteTitle`、`docker.filesOverwriteConfirm`
- `docker.filesTooLarge`、`docker.filesUnsupported`、`docker.filesToolMissing`
- `docker.filesTransferFailed`、`docker.filesCleanupFailed`、`docker.filesDirNotDownloadable`

覆盖全部 7 个 locale；复用现有 `docker.logsTail`、`docker.logsEmpty`、`docker.close` 等 key。i18n key 引用守卫必须通过。

## 9. 测试与验收

### 9.1 后端单测

- `ls` 输出解析：隐藏文件、目录尾斜杠、空格、CRLF、`.`/`..`、空输出、异常行。
- 路径归一：绝对路径、折叠 `.`/`..`、拒绝 NUL/控制字符/超长。
- 暂存路径校验：合法/非法前缀、UUID、任意路径删除拒绝。
- 日志分片合并/裁剪边界、tail 钳制、流上限与 TTL（纯逻辑）。
- 切分上传：offset 校验、超过 512MiB 拒绝、取消清理。
- 现有 docker/list、docker/logs、docker/action 回归测试全绿。

### 9.2 前端单测

- `dockerLogBuffer`：跨 chunk 拼行、裁剪、follow 判定。
- `useDockerLogStream`：start/chunk/end/stop、sudo 重试、容器切换、卸载清理。
- `dockerFilePaths`：归一、父路径、面包屑、排序。
- `ContainerManagerPane`：选择规则、页签切换、日志跟随按钮、错误态。
- `ContainerFilesTab`：目录进入、目录下载禁用、覆盖确认、进度/取消。
- 布局：`dockerPaneOpen` 切换不影响 `sftpPaneOpen`/`rightPanelView`；SFTP 保持原状；比例重置与钳制。

### 9.3 集成与冒烟

- mock host 增加 `docker/logs/start|stop`、`docker/files/*` 与事件推送；`mock_host_registry_check.mjs`、`connection-forms/verify.mjs` 全绿。
- 浏览器 fixture：打开 SFTP → 打开容器页 → 验证 SFTP 仍在、底部分屏可拖拽、日志自动跟随/暂停/恢复、文件页进出目录。
- 本机 Docker 冒烟（有 Docker 时）：启动临时容器 → list → logs follow → 上传/下载/清理；无 Docker 环境明确 SKIP。
- SSH 容器联调（人工/测试机）：真流式、sudo 重试、SFTP 暂存下载/上传、清理。
- 全量门禁：`pnpm --dir frontend typecheck`、`pnpm --dir frontend test`、`cargo test --locked`、结构守卫、dead-code、`scripts/test.sh --skip-host`、打包与打包后 MCP 冒烟。

### 9.4 验收标准

1. SFTP 打开时点击容器管理，SFTP 页面、宽度、视图不变；容器页出现在终端下方。
2. 容器页默认 33%，可拖拽与键盘调整，终端重新 fit；关闭后终端恢复。
3. 日志 <1 秒内可见新增行；默认跟随；上滚暂停且流不断；点“实时跟随”跳到最新。
4. tail 切换重启并清空；容器停止/权限不足/会话断开的提示可操作。
5. 文件页可浏览、进入目录、查看条目详情；目录下载禁用；文件可下载到本机、上传到容器，进度/取消/覆盖确认可用，暂存必清理。
6. SSH 与本机 Docker 两种目标均通过；只读连接拒绝上传/导入与 sudo 日志。
7. 无新增 allowlist；所有文件在行数预算内；i18n 7 语言齐全；无 dead-code 警告。
8. RPC/事件写入 `PROTOCOL.zh-CN.md` 与 `FEATURE_PARITY.zh-CN.md`；README/终端 UX 契约同步更新。

## 10. 已知限制

- 目录不可下载；不支持文件夹上传；无内联预览。
- 浏览依赖容器内 `ls`（详情依赖 `stat`）；distroless 类容器无法浏览。
- 文件名含换行/控制字符显示不完整。
- 符号链接按 Docker CLI 默认行为处理，链接到目录时下载会失败并提示。
- SSH 文件浏览/传输首版面向 POSIX 远端；Windows OpenSSH 的 cmd 引号差异不做保证（日志流不受影响）。
- 日志仅内存保存，关闭页面即丢失；单流最长 30 分钟。
- 关闭页面时若 sidecar 未收到 stop（webview 崩溃），靠 TTL 与流上限兜底。
