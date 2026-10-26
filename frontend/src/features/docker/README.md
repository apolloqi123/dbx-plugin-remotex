# features/docker — 容器管理底部分屏

容器管理从右侧栏互斥视图迁为终端列底部独立分屏后的前端模块（设计见
`docs/superpowers/specs/2026-10-26-container-manager-pane-design.md`）。
RPC/事件契约以 `docs/architecture/PROTOCOL.zh-CN.md` 的「Docker 管理面板」节为准；
本文只描述前端内部划分与运行时语义。

## 文件划分

组件（`components/`）只做渲染与事件转发，状态与副作用在 composable/lib：

| 文件 | 职责 | 边界 |
| --- | --- | --- |
| `ContainerManagerPane.vue` | 页面外壳：页头（标题/本机徽标/引擎设置/刷新/关闭）、左右分栏、页签、内部弹层与 Esc 仲裁 | 持有选中容器、页签状态；把 RPC 调用委托给子组件/composable |
| `ContainerListPane.vue` | 容器列表：10s 轮询、3 连败暂停、动作菜单、kill/rm 确认、引擎设置 | 轮询与 needsSudo 逻辑在 `dockerPanelModel` / `dockerActions` / `useDockerContainers` |
| `ContainerDetailPane.vue` | 选中容器详情与页签容器 | 只渲染，不订阅全局事件 |
| `ContainerLogsTab.vue` | 日志视图：滚动容器、跟随按钮、tail 选择、清空、结束态/sudo 重试 | 行缓冲与 follow 判定在 `lib/dockerLogBuffer`，流生命周期在 `useDockerLogStream` |
| `ContainerFilesTab.vue` | 文件浏览：面包屑/上一级/刷新、条目详情、下载/上传、覆盖确认、进度/取消 | 路径运算在 `lib/dockerFilePaths`，浏览在 `useContainerFiles`，传输编排在 `useContainerFileTransfer` |
| `DockerRowMenu.vue` / `DockerConfirmDialog.vue` / `DockerEnginePopover.vue` / `DockerWhaleLogo.vue` | 复用件（历史容器面板） | 无全局状态 |

Composable/lib：

- `composables/useDockerContainers.ts`：列表轮询、失败退避、动作执行。
- `composables/useDockerLogStream.ts`：`start/stop/restart/tail/sudoRetry`、状态机、按 `streamId` 过滤事件。
- `composables/useContainerFiles.ts`：目录浏览、选中详情、错误映射与上传/下载入口。
- `lib/dockerLogBuffer.ts`：纯函数——跨 chunk 拼行、行数/字符裁剪、follow 判定（无 Vue 依赖，可单测）。
- `lib/dockerLogEvents.ts`：`docker/logs/chunk|end` 的处理器表。
- `lib/dockerFilePaths.ts`：路径归一、父路径、面包屑、目录优先排序。
- `lib/dockerFileTransfer.ts`：传输纯逻辑与跨域桥——目标参数、512 MiB 上限、暂存路径拆分、base64 分片编码，以及 `setDockerTransferBridge`/`getDockerTransferBridge`（setup 期注入 SFTP 下载/上传队列，避免 App 级状态进 props）。
- `composables/useContainerFileTransfer.ts`：SSH（SFTP 暂存）与本机（分片上传/保存）两条传输链路，负责覆盖确认、暂存清理与提示。

事件订阅只有一处：`app/workbench/composables/usePluginEvents.ts` 把
`docker/logs/*` 事件分派到 `dockerLogEvents`。组件内**不得**新增第二个
`onEvent` 订阅（多订阅会导致同一 stream 的行被追加两次）。

## 日志流生命周期

```
挂载日志页 / 选中变化
  → start({containerId, tail, sudo?})           // 返回 streamId
  → chunk 事件：按 streamId 过滤 → 拼行 → 入缓冲 → follow ? 滚底 : 只入缓冲
  → end 事件：保留已收日志，记录 exitCode/error/needsSudo
stop 触发点：切容器 / 关闭容器页 / 组件卸载 / 换 tail / 显式清空流
```

- 打开日志页才 `start`；切到文件页**不**停止流（返回时历史仍在）。切容器、
  关闭页面、卸载组件才停止。
- `stop` 必须幂等且先摘处理器再发 RPC：先摘监听避免 stop 在途时把迟到的
  chunk 追加进已清空的视图。sidecar 在主动 stop 时补发一次
  `docker/logs/end {streamId, error:"cancelled"}`（已结束的流不再补发），
  前端据此把状态机收口到「已停止」。
- 换 tail：停止当前流 → 清空缓冲 → 以新 tail 重启（事件可能迟到，靠
  `streamId` 过滤丢弃旧流分片）。
- `end` 后不自动重连。`needsSudo:true` 时展示「以 sudo 重试」（只读连接/本机
  目标不提供）；其余原因展示退出码/错误并提供「重新开始」。
- 清空按钮只清视图，不断流。关闭页面先停流 → 取消上传 → 停轮询 → 清理暂存。

## 「暂停跟随」语义

- 新日志到达时：`follow=true` 则滚到底部。
- 用户向上滚动超过 **24px**：`follow=false`，显示「已暂停跟随」；流继续接收，
  新行进入缓冲区，**不**强制滚动；滚动位置保持。
- 用户滚回距底部 **≤4px**：自动恢复 `follow=true` 并回到贴底。
- 点「实时跟随」：`follow=true` 并立即滚到底；按钮在跟随时为激活态；跟随时
  再点一次只关闭自动滚动，**不**停止流。
- 缓冲上限 **10,000 行或 2,000,000 字符**，超限从头部裁剪并显示「已截断」。

## 文件浏览与传输

- 浏览：`docker/files/list`（容器内 `ls -1ap`），默认 `/`；目录优先、名称升序；
  条目选中时按需 `docker/files/stat`（缺 `stat` 时显示「—」）。缺 `ls` 时显示
  「容器内无 ls，无法浏览」并禁用上传/下载。
- 下载（SSH）：`export`（远端暂存）→ 作为普通 SFTP 下载任务入现有传输队列
  （显示名覆盖为容器内文件名）→ 完成/失败/取消后 `cleanup`；清理失败只提示。
- 下载（本机）：`export`（`location:"local"`）→ `saveLocal`（沿用下载目录、
  撞名让位/覆盖、传输历史）→ 成功后清理暂存。
- 上传（SSH）：`prepareUpload` → 现有 SFTP 上传队列写暂存 → `import`
  （`destPath` 以 `/` 结尾视为目录并保留文件名）→ `cleanup`。
- 上传（本机）：`upload/begin` → 256 KiB 分片 `upload/chunk`（`offset` 必须等于
  已收字节）→ `upload/finish` → `import` → `cleanup`。
- 目标已存在时先弹覆盖确认，取消则不上传；目录下载按钮禁用并提示。

## 限制

- 单文件 512 MiB（上传/下载开始前前端拦截）。
- 目录不可下载；不支持文件夹上传、容器内编辑/删除/重命名/权限修改、内联预览。
- 浏览依赖容器内 `ls`，详情依赖 `stat`；distroless 类容器不可浏览。
- 文件名含换行/控制字符显示不完整；符号链接按 Docker CLI 默认行为处理。
- 日志仅内存保存（关闭页面即丢失），单条流最长 30 分钟；sidecar 侧注册表上限
  4 条流、4 个并发上传，暂存/上传 30 分钟 TTL 兜底 webview 崩溃遗留。
- 日志流不落盘、不进会话回放/录制。

## 夹具与冒烟

- 浏览器夹具：`?docker=files`（补充 `/opt/app` 多级树与零字节文件）、
  `?docker=off`（CLI 缺失）、`?docker=sudo`（daemon 拒绝：文件页报权限错误，
  日志流不推 chunk、直接以 `needsSudo:true` 结束）。日志流在 mock 中每 700ms
  推一行、10s 自然结束；`stop` 会补发 `error:"cancelled"` 的 end。流事件
  方法名/字段与 sidecar 一致（`docker/logs/chunk {streamId,text}`）。
- 本机 Docker 冒烟：`python3 scripts/smoke/smoke_docker_files.py`。
  仅当 docker CLI、可连通的 daemon 或可用的本机镜像缺失时打印 `SKIP` 并退出 0；
  未找到 sidecar 二进制时**不算 SKIP**——自动回落到 docker CLI 面执行同一套
  契约（`exec ls/stat`、`logs -f`、`cp` 往返）并打印 `PASS`。脚本只创建临时
  容器与临时目录，退出时删除，不改动既有容器/镜像。
