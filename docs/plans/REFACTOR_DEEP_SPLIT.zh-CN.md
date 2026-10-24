# 深度重构：大文件拆分与可维护性治理

状态：执行中（2026-10-24）
分支：`refactor/deep-split`

## 进度

- P0 完成：基线提交 `9db22a9`；后端 1286、前端 209 文件 / 2279 测试通过，typecheck 干净。
- P1 完成：12 个后端大模块目录化 + 6 处内联测试迁移；后端 1286 测试全绿。
- P2 完成：SettingsDialog 2,680→924、SftpTreePane 1,129→424、DockerPanel 1,075→550、OtpPanel 822→360、useUploadChain 890→405、useRecording 750→410、SplitTerminalPane 800→648；全局 CSS 拆为 19 个有序文件；i18n 按 locale 拆分。前端 typecheck + 2279 测试全绿。
- P3 完成（安全边界）：App.vue 11,356→9,981（script 9,136→7,761）；新增 `useNotices`、`useTerminalTooltip`、`useTerminalSearch`、`useDockMetrics`、`usePromptHints`、`useWallpaper`、`useEditorSplitPane`、`useEditorOpenWith`、`useSftpBookmarks`、`useSftpRowSelection`、`useSftpAttributes`、`useSftpTreeWorkspace`、`useWorkbenchPanels`、`useOnlineSearch`、`useWorkbenchPrefs`。剩余为终端状态机/连接/弹窗仲裁等高风险核心，暂不再硬拆。
- 独立 review 结论：后端与前端均无 P0/P1 行为漂移；已修复 4 个 P2：上传批次决策槽改为实例级、历史条数草稿交回对话框持有、compress 引号测试恢复真实断言、去掉冗余 shell 引号副本与无意义 glob。
- 阶段提交：`10d0748 refactor: split oversized backend modules and frontend files`（后端 1286、前端 2279 通过）。
- P4 完成：App.vue 9,981→9,550；抽出 `DockMetricsStrip`、`ConnectionInfoPopover`、`TransferPopover`、`BatchBar`、`WorkbenchSftpDialogs`、`WorkbenchSessionDialogs` 六个模板组件（1,107 行），对应 scoped CSS 随 DOM 迁移。浏览器冒烟修复两处抽取遗漏：`FolderOpen` 图标导入、`upload-watched-once` 事件名与 Vue `.once` 归一化冲突（改为 `upload-watched-single`）。右侧 SFTP 面板与终端状态机保留在 App，避免高耦合区风险。

## 目标

在行为、公开接口和测试语义不变的前提下，把超大文件拆成职责单一的模块/组件，消除重复工具函数，使每个文件可以独立阅读、测试和修改。

硬约束：

- 不改变运行时行为、RPC 协议、工具 schema、i18n key、CSS 级联结果。
- 不升级依赖，不做无关格式化，不重排无关代码。
- 不触碰 unsafe 模块（`filesystem/sftp/tree.rs`、`automation/task_config.rs`、`platform/job_object.rs`）与 `sessions/ssh/connection/open.rs` 内部。
- 认证、sudo、命令执行语义只做机械搬移，改完单独标注供人工复核。
- 每阶段通过各自的测试门禁后才进入下一阶段；需要改行为时停下报告。

## 阶段

### P0 基线

- 提交 AI 搜索/审批 schema 改动，建 `refactor/deep-split` 分支。
- 记录 `cargo test`、`pnpm typecheck`、`pnpm test` 基线数量。

### P1 后端大模块目录化

| 原文件 | 目标 |
| --- | --- |
| `sessions/ssh/tree_download.rs` | `tree_download/{mod,start,prep,chunk,read}.rs` |
| `features/security/audit.rs` | `audit/{mod,codec,ledger,tests}.rs` |
| `filesystem/local/editors.rs` | `editors/{mod,catalog/{mod,macos,windows,posix},launch,tests}.rs` |
| `core/exec/auth.rs` | 生产逻辑保留 + `tests.rs` |
| `sessions/ssh/execution.rs` | `execution/{mod,core,terminal,approvals,keepalive}.rs` |
| `features/metrics/gpu.rs` | `gpu/{mod,types,parse_gpu,parse_npu,parse_shared,tests}.rs` |
| `features/agent/terminal.rs` | `terminal/{mod,policy,capture,recorder,tests}.rs` |
| `features/mcp/sftp.rs` | `sftp/{mod,upload,download,paths}.rs` |
| `features/security/sudo/fs.rs` | `fs/{mod,parse,io,mutate,tests}.rs` |
| `features/recording.rs` | `recording/{mod,recorder,catalog,search,tests}.rs` |
| `features/transfer/compress.rs` | `compress/{mod,policy,remote,local,tests}.rs` |
| `core/keys.rs` | `keys/{mod,discovery,known_hosts,tests}.rs` |

沿用仓库既有惯例：`foo.rs` 声明子模块，子文件首行 `use super::*;`，父模块按需 `pub(super) use` 回导；测试用 sibling `tests.rs`。

### P1b 内联测试与去重

- ≥200 行内联测试迁到 sibling：`exec/auth.rs`、`security/alerts.rs`、`core/keys.rs`、`security/audit.rs`、`transfer/compress.rs`、`agent/terminal.rs`、`startup_commands.rs`、`metrics/gpu.rs`、`host_key.rs`、`recording.rs`、`local/downloads.rs`、`local/editors.rs`、`terminal/highlight_rules.rs`、`otp.rs`。
- `unix_now_secs`（12 份）收口到 `core/`；`shell_quote`（2 份）、`string_field`/`optional_string`（各 2 份）去重。
- 重复的连接参数解析（`mcp/connections.rs` vs `core/model/connection.rs`）只在测试保护下合并，风险高则暂缓并记录。

### P2 前端次级热点

| 原文件 | 目标 |
| --- | --- |
| `settings/components/SettingsDialog.vue` | `components/settings/*Pane.vue` + `composables/useConnectionSettings.ts` + `useSudoProfiles.ts` |
| `sftp/components/SftpTreePane.vue` | `SftpTreeRow.vue` + `useTreeVirtualScroll` / `useTreeDetailsCard` / `useTreeFileDrop` |
| `docker/components/DockerPanel.vue` | `lib/dockerPanelModel.ts` + `useDockerContainers.ts` + 4 个子组件 |
| `sftp/composables/useUploadChain.ts` | `useUploadDuplicates` / `useFolderUpload` / `useHostFileDrag`，返回对象不变 |
| `security/components/OtpPanel.vue` | `useOtpCodes.ts` + `OtpEditorDialog.vue` + `OtpBindSection.vue` |
| `recording/composables/useRecording.ts` | `useReplayPlayer.ts` + `lib/recordingExport.ts` |
| `terminal/components/SplitTerminalPane.vue` | `SplitTerminalHeader.vue` + `useSplitSearch.ts` |
| `app/style.css` | 按特性拆 `app/styles/*.css`，import 顺序不变；传送门/全局浮层保持全局 |
| `i18n/.../supplemental/terminalBehavior.ts` | 按 locale 拆 6 个文件，合并方式不变 |

### P3 App.vue 逻辑抽取（串行）

顺序：通知 → 工具提示 → 终端搜索 → AI 修复 → dock 指标 → 提示行 → 壁纸 → 偏好持久化 → SFTP 工作区 → 终端浮层 → 弹窗栈。

方法：`composables/useX({ ...getters, callbacks })` 返回扁平对象，App.vue 解构后模板绑定不变。目标 App.vue < 5,000 行。

### P4 App.vue 模板组件化

对话框群、终端浮层（zmodem/trzsz/serial/录制/批量栏）、传输弹层、SFTP 面板、连接信息、dock 指标条抽到 `app/components/`；DOM 迁移时同步迁移 scoped CSS，reka 传送门样式保持全局。目标 App.vue < 3,500 行。

### P5 验收

- `cargo test`、`pnpm typecheck`、`pnpm test` 全绿且数量不低于基线。
- `scripts/test.sh --skip-host`。
- 浏览器冒烟：主终端、新标签页、分屏、`#` AI 搜索、SFTP 选中/展开/右键菜单、设置、Docker。
- 升版本、更新 CHANGELOG、生成 `.dbxp` 并报告路径/大小/SHA-256。

## 执行与审查

- P1/P2 用并行子 agent 处理互不相交的文件组；每个 agent 只做机械迁移并运行定向测试。
- 每批结束后主 agent 跑全量测试 + typecheck，并独立复审 diff。
- P3/P4 单写者串行，避免同一文件并发修改。
- 每阶段一个提交，不 push。
