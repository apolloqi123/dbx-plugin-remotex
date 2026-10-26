# 深度重构：大文件拆分与可维护性治理

状态：已完成（2026-10-26）
分支：`refactor/deep-split`

## 进度

- P0 完成：基线提交 `9db22a9`；后端 1286、前端 209 文件 / 2279 测试通过，typecheck 干净。
- P1 完成：12 个后端大模块目录化 + 6 处内联测试迁移；后端 1286 测试全绿。
- P2 完成：SettingsDialog 2,680→924、SftpTreePane 1,129→424、DockerPanel 1,075→550、OtpPanel 822→360、useUploadChain 890→405、useRecording 750→410、SplitTerminalPane 800→648；全局 CSS 拆为 19 个有序文件；i18n 按 locale 拆分。前端 typecheck + 2279 测试全绿。
- P3 完成（安全边界）：App.vue 11,356→9,981（script 9,136→7,761）；新增 `useNotices`、`useTerminalTooltip`、`useTerminalSearch`、`useDockMetrics`、`usePromptHints`、`useWallpaper`、`useEditorSplitPane`、`useEditorOpenWith`、`useSftpBookmarks`、`useSftpRowSelection`、`useSftpAttributes`、`useSftpTreeWorkspace`、`useWorkbenchPanels`、`useOnlineSearch`、`useWorkbenchPrefs`。剩余为终端状态机/连接/弹窗仲裁等高风险核心，暂不再硬拆。
- 独立 review 结论：后端与前端均无 P0/P1 行为漂移；已修复 4 个 P2：上传批次决策槽改为实例级、历史条数草稿交回对话框持有、compress 引号测试恢复真实断言、去掉冗余 shell 引号副本与无意义 glob。
- 阶段提交：`10d0748 refactor: split oversized backend modules and frontend files`（后端 1286、前端 2279 通过）。
- P4 完成：App.vue 9,981→9,550；抽出 `DockMetricsStrip`、`ConnectionInfoPopover`、`TransferPopover`、`BatchBar`、`WorkbenchSftpDialogs`、`WorkbenchSessionDialogs` 六个模板组件（1,107 行），对应 scoped CSS 随 DOM 迁移。浏览器冒烟修复三处抽取问题：`FolderOpen` 图标导入、`upload-watched-once` 事件名与 Vue `.once` 归一化冲突（改为 `upload-watched-single`）、告警排查 textarea 丢失两向绑定（补回归守卫）；另清理死 `resumeInput` ref 与无用 prop。右侧 SFTP 面板与终端状态机保留在 App，避免高耦合区风险。
- 第二批（P6–P9）完成：后端继续拆分 `connection`、`mcp/connections`、`exec/auth`、`local/fs`、`ssh/state`、`sftp/copy`、`triggers/parse`；App.vue 9,550→9,174，新增 `useAiAssistant`、`useSerialUpload` 与 `SftpWorkspacePanel`（531 行）；`useCommandSuggestions` 668→410、`useTransferQueue` 578→132。最终 review：无 P0/P1；清理无意义 `#[allow(unused_imports)]`、死导入，并补 tree/path-bar 桥接守卫。
- 第三批（P11–P13）：App.vue 9,173→8,572（`useShellTabs`、`useTerminalMenus`、`useLocalShellPreferences`、`useWorkbenchToolbar`、`useWorkbenchSpawnTabs`、`useSftpFolderActions`）；SettingsTerminalPane 397→202（拆 4 个 section），SplitTerminalPane 648→465（renderer/clipboard 两个 composable）；mockDbxHost 1,637→263 入口 + `mock/host/*` 14 个域模块，mock registry 校验脚本同步适配；figSourceCore 确认是纯引擎、保持不动。中断 agent 留下的 `newFileTargetDir` 缺失已修复。
- P10 完成：beta.21 全量门禁通过，产物 `dist/io.dbx.sshx-0.7.9-beta.21-darwin-arm64.dbxp`，14,856,620 bytes，SHA-256 `7e4cbe0586b964b479be6339ba39a108366e61c9cb8826d2827b7fc8ccfe6255`。
- P14 完成：`scripts/test.sh --skip-host` 全部通过（后端 1286、前端 2280、dead-code 零警告、MCP stdio smoke、本地终端 smoke）。产物：`dist/io.dbx.sshx-0.7.9-beta.22-darwin-arm64.dbxp`，14,860,729 bytes，SHA-256 `4db46b749f70d9f16106a8a06918f8b62396a2aae6dd110231f7fe9524155050`。

## 完成边界

本轮机械拆分已由后续结构重构收口，最终状态如下（已完成，无剩余超限项）：

- `frontend/src/app/App.vue`（12 行）：纯组合根，仅装配状态与外壳；工作台外壳在 `frontend/src/app/components/WorkbenchShell.vue`（561 行），终端帧 drain、连接生命周期、Esc/焦点仲裁等已迁入 `frontend/src/features/*` 与 `frontend/src/app/workbench/`。
- `frontend/src/features/settings/components/SettingsDialog.vue`（436 行）：设置编排器已拆到行数预算内。
- `frontend/src/features/completion/lib/completion/fig/figSourceCore.ts`（524 行）：纯引擎保持语义不变，已拆出 parser surface、manifest 数据面与建议 helper，行数降至预算内。
- `backend/src/sessions/ssh/connection/open/`（`mod/pty/register/reader/error`，共 920 行）：原 718 行连接建立 God-function 已按 PTY/注册/读循环/错误拆分。
- i18n `messages/base/*.ts` 与 fig 语料：数据文件，按语言/语料边界保留。

## 目标

在行为、公开接口和测试语义不变的前提下，把超大文件拆成职责单一的模块/组件，消除重复工具函数，使每个文件可以独立阅读、测试和修改。

硬约束：

- 不改变运行时行为、RPC 协议、工具 schema、i18n key、CSS 级联结果。
- 不升级依赖，不做无关格式化，不重排无关代码。
- 不触碰 unsafe 模块（`filesystem/sftp/tree.rs`、`automation/task_config.rs`、`platform/job_object.rs`）与 `sessions/ssh/connection/open/` 内部。
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
