# 代码层级重构设计：组合根收口与大文件拆分

- 日期：2026-10-25
- 状态：已实施（2026-10-26）
- 范围：`frontend/`、`backend/` 的目录层级与文件拆分
- 约束：行为、RPC、i18n key、CSS 级联结果、依赖版本均不变

## 1. 背景

当前仓库不是“整体混乱”，而是三个集中问题叠加：

| 问题 | 现状事实 | 后果 |
| --- | --- | --- |
| `App.vue` 职责爆炸 | 8,620 行（script 6,988 + template 1,626 + 外部 scoped CSS） | 终端运行时、连接生命周期、多协议会话、SFTP 状态、装饰引擎、键盘/焦点仲裁、持久化全部挤在一个文件 |
| `app/` 成为杂物间 | `app/composables` 29 个模块，其中约 20 个是 `sftp/terminal/serial/metrics/quick-commands` 的领域逻辑 | 领域代码不在领域目录；`app` 与 `features` 的边界失去语义 |
| 底座层反向依赖 | `frontend/src/shared` 有 12 处 import `features`；后端 `core` 与 `filesystem` 成环，`core → features/sessions` 共 22 处 | 分层失效，底层改动会反向牵动上层 |

后端生产代码整体健康：97,537 行中最大文件为 718 行。真正超限的是 2 个生产文件和 4 个测试文件。

## 2. 目标与非目标

### 2.1 目标

1. `App.vue` 降为纯组合根：只做状态装配与模块接线，模板只保留外壳布局。
2. 领域逻辑回到 `features/<domain>`；跨域编排留在 `app/workbench/`。
3. `shared` 只依赖标准库、Vue、通用工具，不再依赖 `features`/`app`。
4. 后端 `core` 成为真正的叶子层。
5. 单文件规模受自动守卫约束，后续不再回涨。
6. 大域（terminal、sftp）内部按子域分目录，降低单目录认知负担。

### 2.2 非目标

- 不改变运行时行为、RPC 方法名与参数形状、i18n key、CSS 级联顺序。
- 不升级/降级依赖，不做无关格式化或重命名。
- 不改 `backend/src/filesystem/sftp/tree.rs`、`automation/task_config.rs`、`platform/job_object.rs` 等 unsafe 模块内部。
- 不重构根目录 `shared/frontend`、`shared/sdk`：这是四插件共用宿主 SDK，保持相对路径引用约定。
- 不在本任务中修复功能缺陷；发现的问题单独记录，不夹带。

## 3. 验收硬指标

| 指标 | 上限 |
| --- | --- |
| `App.vue` | 600 行 |
| 前端非测试源文件 | 600 行 |
| 前端 spec 文件 | 1,200 行 |
| 后端生产 `.rs` 文件 | 600 行 |
| 后端测试 `.rs` 文件 | 1,200 行 |
| 依赖方向 | 见 §4.2，零违规 |

行数按物理行计算，测试模块包含在内；超限必须在对应波次内拆完，不留 allowlist。

## 4. 目标结构

### 4.1 目录形态

```text
frontend/src/
  app/                      # 组合根
    App.vue                 # ≤600 行，只做装配与外壳
    main.ts
    workbench/              # 跨域工作台编排（会话路由/偏好/弹窗/启动）
      state/                # 按域创建的本地状态工厂（非全局单例）
      composables/
      lib/
    components/             # 外壳 UI：工具栏、终端舞台、侧栏、对话框宿主
    lib/                    # 工作台级纯函数
    mock/                   # 宿主模拟与夹具
    styles/                 # 全局 CSS（保持现有 import 顺序）
  features/<domain>/
    components/             # 领域 UI
    composables/            # 有状态行为（Vue 响应式/生命周期）
    lib/                    # 纯函数与类型（无 Vue 运行时）
  shared/                   # 通用底座：不引 features、不引 app
shared/frontend/            # 四插件共用宿主 SDK：不动
```

### 4.2 依赖方向

```text
app        -> features, shared
features   -> shared
shared     -> (仅第三方/标准库)
backend: app -> features, sessions, filesystem, core, platform
         features/sessions/filesystem -> core, platform
         core -> platform（仅此一层内部依赖）
```

禁止：

- `features/**` 出现指向 `app/**` 的 import。
- `shared/**` 出现指向 `features/**` 或 `app/**` 的 import。
- `backend/src/core/**` 出现 `crate::app::`、`crate::features::`、`crate::filesystem::`、`crate::sessions::`。

### 4.3 大域子目录

`features/terminal`（77 文件）与 `features/sftp`（52 文件）按子域分组，每个文件只归属一个组：

```text
features/terminal/lib/
  emulator/       终端解析与输入模型（anchor/input/inputQueue/lineModel/osc/prompt/backpressure/webkitInput/directoryTracking）
  appearance/     外观与渲染（appearance/font/scheme/schemeCatalog/background/webgl/layout/gutter/zoom）
  decorations/    actionLinks/keywordHighlight/ghostSuggest/quickSelect
  panels/         historyPanel/commandHistory/commandMarkers/shellTabTitle
  transfer/       batchSend/trzsz/zmodem
  behavior/       hotkeys/interaction/dangerousCommands/interactivePromptGuard/enterCapture/outputText/reconnect/split
features/terminal/composables/
  session/        transportWatch、新增 instance/writer/reconnect
  io/             新增 input/clipboard
  ui/             现有多数 UI 型 composable 与新增 decorations 引擎
features/sftp/lib/
  entries/        entries/filters/name/errors/paths
  tree/           fileTree/virtualWindow/dropEntries/fileRowKeydown
  transfer/       folderUpload/archive/overwrite/sanitizedDownload/folderDownload/downloadPrefs/hostFileDrop/hostDragMapping
  editor/         editorRules/editorTheme/textSniff/jsonPreview/watchEdits
  actions/        bookmarks/contextMenu/details/rename
features/sftp/composables/
  listing/        navigation/listLayout/treeWorkspace/pathHistory
  tree/           treeVirtualScroll/treeDetailsCard/treeFileDrop
  transfer/       uploadChain/uploadDuplicates/folderUpload/hostFileDrag/download/archive
  editor/         editorTabs/openWith/splitPane/filePreview/externalEdits
  actions/        chmod/delete/rename/createDirectory/attributes/bookmarks/selectionClipboard/symlink/folderActions/rowSelection/dialogs/dropUpload
```

分组迁移只移动文件并批量修正 import；同一提交内不做逻辑修改，便于 review 和二分。

## 5. 前端拆分设计

### 5.1 App.vue 的定位

`App.vue` 只做装配与外壳，不持有领域实现。共享状态由 `app/workbench/state/` 下的工厂按域创建（纯本地工厂，不是全局单例或 Pinia），由 `useWorkbenchContext()` 组合后交给各行为模块：

```ts
const ctx = useWorkbenchContext()
const { openSession } = useSshConnection(ctx, deps)
```

所有行为模块沿用现有约定：

```ts
const x = useX(ctxOrRefs, { callbacks })
```

返回扁平对象，`App.vue` 解构后模板绑定名保持不变。这样既避免 600 行内再造一个 God file，也保持状态可测试、可独立创建。

### 5.2 脚本模块清单

行号为当前文件近似区间，实施时以符号边界为准。

| 模块 | 目标位置 | 来源 | 职责 |
| --- | --- | --- | --- |
| `useTerminalInstance` | terminal/composables/session | 2,269–2,511；3,043–3,200 | xterm/addons 创建、挂载、尺寸、webgl、销毁 |
| `useTerminalWriter` | terminal/composables/session | 1,622–1,737；3,304–3,632 | 输出 gate、写节流、帧排水、二进制分派 |
| `useTerminalInput` | terminal/composables/io | 1,688–1,737 | 输入队列与发送 |
| `useTerminalKeys` | terminal/composables/io | 2,511–2,894 | 按键路由、overlay 仲裁、历史/ghost/补全 |
| `useTerminalTransport` | terminal/composables/session | 3,632–3,666 | 传输探测与重连调度接线 |
| `useTerminalHighlight` | terminal/composables/ui | 4,624–4,776 | 关键词高亮装饰 |
| `useTerminalActionLinks` | terminal/composables/ui | 4,791–4,989 | 动作链接扫描与激活 |
| `useTerminalGutter` | terminal/composables/ui | 4,989–5,160 | 行号/时间戳 gutter |
| `useCommandMarker` | terminal/composables/session | 1,335–1,348；3,200–3,304 | OSC 633 命令标记状态机 |
| `useTerminalEnterCapture` | terminal/composables/io | 2,894–3,043 | Enter 行捕获与回显确认 |
| `useTerminalClipboard` | terminal/composables/io | 5,254–5,311；5,759–5,902 | 复制/粘贴/危险确认/OSC52 |
| `useSshConnection` | connections/composables | 3,666–4,057；4,500–4,542 | SSH 打开/附着/关闭/重连/host reopen |
| `useLocalShellSession` | terminal/composables/session | 4,057–4,146；4,384–4,500 | 本地 shell 生命周期与重启 |
| `useTelnetWorkbench` | telnet/composables | 4,146–4,204；4,212–4,224 | Telnet 会话生命周期 |
| `useSerialWorkbench` | serial/composables | 4,294–4,334 | 串口会话生命周期 |
| `useVncWorkbench` | remote-desktop/composables | 4,208–4,238 | VNC 会话接线（复用 useVncSession） |
| `useRdpWorkbench` | remote-desktop/composables | 3,700–3,717；4,255–4,289 | RDP 会话接线（复用 useRdpSession） |
| `useSftpWorkspace` | sftp/composables/listing | 874–1,114；5,188–5,254 | 列表/导航/选择/对话框状态 |
| `useSftpDropUpload` | sftp/composables/transfer | 5,657–5,759 | 本机文件拖放、目标目录解析、上传提示 |
| `useSftpPreview` | sftp/composables/editor | 5,401–5,530 | 预览面板与保存链 |
| `useWorkbenchState` | app/workbench/composables | 2,085–2,269 | 布局恢复、写回、节流持久化 |
| `useSplitDivider` | app/workbench/composables | 5,311–5,401 | 分隔条拖拽/键盘微调 |
| `useWorkbenchModals` | app/workbench/composables | 6,275–6,535 | 模态栈、焦点、文档级按键 |
| `useWorkbenchBootstrap` | app/workbench/composables | 6,593–6,989 | initialize、事件订阅、宿主拖拽、重挂、MCP 镜像、卸载 |

拆分后 `App.vue` 仅保留：`useWorkbench()` 单入口、外壳布局与生命周期转发，目标 ≤600 行；共享状态由 `app/workbench/state/` 的按域工厂持有。

### 5.3 模板组件

| 组件 | 职责 | 目标 |
| --- | --- | --- |
| `WorkbenchToolbar.vue` | 顶栏外壳、身份区、主操作位 | ≤300 |
| `SessionMenu.vue` | 新会话/本地 shell/连接列表菜单 | ≤300 |
| `QuickCommandsMenu.vue` | 快速命令分组/搜索/展开 | ≤250 |
| `RdpToolbarControls.vue` | 缩放/分辨率/粘贴/按键输入 | ≤200 |
| `AppearanceMenu.vue` | 外观/主题/更多设置入口 | ≤200 |
| `WorkbenchStage.vue` | 终端舞台与浮层宿主 | ≤350 |
| `TerminalStatusOverlays.vue` | 重连/录制/本地退出/Telnet/串口/VNC/RDP 状态层 | ≤350 |
| `TransferOverlays.vue` | zmodem/trzsz/串口上传/回放 | ≤350 |
| `WorkbenchSidePanel.vue` | 右侧 SFTP/Docker/Metrics 三视图外壳 | ≤300 |
| `WorkbenchDialogs.vue` | 未抽走的 OTP/导入/预览/录制/审计/拖放上传/目录选择 | ≤350 |

DOM 迁移时同步迁移对应 scoped CSS。若规则依赖 `.workbench` 祖先，保留在外壳样式或改为 `:deep()`，并以浏览器截图核对。

### 5.4 `app/composables` 归位

| 文件 | 去向 |
| --- | --- |
| `useSftpTreeWorkspace`、`useSftpBookmarks`、`useSftpRowSelection`、`useSftpAttributes`、`useSftpChmod`、`useSftpFolderActions`、`sftpRemotePaths` | `features/sftp/` |
| `useEditorOpenWith`、`useEditorSplitPane` | `features/sftp/composables/editor/` |
| `useTerminalMenus`、`useTerminalSearch`、`useTerminalTooltip`、`usePromptHints`、`useWallpaper`、`useAiAssistant`、`useShellTabs` | `features/terminal/composables/ui/` |
| `useSerialUpload` | `features/serial/composables/` |
| `useDockMetrics` | `features/metrics/composables/` |
| `useOnlineSearch` | `features/terminal/composables/ui/` |
| `useQuickCommandPanel` | `features/quick-commands/composables/` |
| `useNotices` | `shared/composables/`（去掉 feature 引用）或 `app/workbench/composables/` |
| `usePluginEvents`、`useWorkbenchPrefs`、`useWorkbenchViewPrefs`、`useWorkbenchPanels`、`useWorkbenchToolbar`、`useWorkbenchSpawnTabs` | `app/workbench/composables/` |

### 5.5 `shared` 越界修复

| 文件 | 处理 |
| --- | --- |
| `shared/lib/preferencesMirror.ts` | 迁 `app/workbench/lib/`（它组合 feature 偏好消毒器） |
| `shared/lib/appearanceSync.ts` | 迁 `app/workbench/lib/` |
| `shared/composables/useAppearanceCrossSurfaceSync.ts` | 迁 `app/workbench/composables/` |
| `shared/lib/pluginContext.ts` | 把 `normalizeConnectionText` 下沉为 shared 纯函数或内联，消除 feature import |
| 相关 spec | 随实现迁移并保留断言 |

### 5.6 其余超限前端文件

- `features/settings/components/SettingsDialog.vue`（929）：拆保存链/导航状态 composable + 各设置分区组件，≤600。
- `features/completion/lib/completion/fig/figSourceCore.ts`（737）：拆语料装载、查询执行、类型定义三部分，引擎逻辑不变。
- `features/sftp/components/SftpTreePane.spec.ts`（1,323）：按“行交互”与“详情卡/拖放”拆成两个 spec，共享夹具下沉。

## 6. 后端拆分设计

### 6.1 解除 `core` 反向依赖

| 现状 | 处理 |
| --- | --- |
| `core/exec` 依赖 `sessions::ssh::SshClient` | 整体迁到 `sessions/ssh/exec/`（auth/metrics/quote/run/terminal），更新 39 处引用 |
| `core/preferences` 引 `features::transfer::compress` | `CompressPolicy` 与阈值常量下沉 `core/preferences/transfer.rs`，feature 侧 re-export |
| `core/preferences` 引 `features::terminal::startup_commands` | sanitizer 下沉 `core/preferences/startup_commands.rs` |
| `core/preferences` 引 `sessions::local` 环境变量清洗 | 常量与清洗函数下沉 `core/preferences/env.rs` |
| `core/model/connection` 引 `features::automation::triggers` | `TriggersConfig` 类型下沉 `core/model/connection/triggers.rs`，automation 引用 core |
| `core/preferences` 引 `filesystem::sftp::name::NameEncoding` | 类型下沉 `core/model/nameEncoding.rs` |
| `core/keys` 引 `sessions::ssh::private_key_decode_failure` | 错误文案构造下沉 `core/keys/decode_error.rs` |
| `filesystem` 引 `core::exec::shell_quote` | `shell_quote` 迁 `core/shell_quote.rs`（纯函数） |

完成后 `core` 不再出现 `crate::app/features/filesystem/sessions`。

### 6.2 生产文件拆分

| 文件 | 现在 | 目标 |
| --- | --- | --- |
| `sessions/ssh/connection/open.rs` | 718 | `connection/open/{mod,pty,register,reader,error}.rs` |
| `sessions/local/runtime.rs` | 671 | `local/runtime/{mod,lifecycle,io,control}.rs` |

### 6.3 测试文件拆分

| 文件 | 现在 | 目标 |
| --- | --- | --- |
| `sessions/ssh/tests.rs` | 5,535 | `sessions/ssh/tests/{mod,connection,transfer,tree,terminal,forwarding,security}.rs`，按现有用例前缀归类 |
| `features/mcp/tests.rs` | 3,772 | `features/mcp/tests/{mod,tools,safety,connections,bridge}.rs` |
| `features/import/tests.rs` | 1,692 | `features/import/tests/{mod,ssh_config,stream,apply}.rs` |
| `core/model/manifest_contract_tests.rs` | 1,397 | `core/model/manifest_contract/{mod,transport,permissions,localization}.rs` |

拆分只移动测试函数与共享 helper，不修改断言。

## 7. 结构守卫

### 7.1 前端 `frontend/src/shared/lib/structure.spec.ts`

- 用 `import.meta.glob("../../**/*.{ts,vue}", { query: "?raw", eager: true })` 扫描源码。
- 行数：非 `*.spec.ts` ≤600；`*.spec.ts` ≤1200。
- 方向：`shared/**` 不得出现含 `/features/` 或 `/app/` 的 import 说明符；`features/**` 不得出现含 `/app/` 的说明符。
- 文件位置：`features/<domain>/{components,composables,lib}/**` 之外不得出现 `.ts/.vue`（子域目录合法）。
- 迁移期用 `ALLOWLIST: Record<string, number>` 记录“当前行数 + 原因”，每波只减不增；最后一波清空。

### 7.2 后端 `backend/src/structure_tests.rs`

- 递归扫描 `backend/src/**/*.rs`。
- 测试文件判定：`tests.rs`、`*_tests.rs`、位于 `tests/` 目录下；上限 1,200，其余 600。
- `core/**` 禁止出现 `crate::app::`、`crate::features::`、`crate::filesystem::`、`crate::sessions::`。
- 与前端一样以 `#[cfg(test)]` 编入 `cargo test`。

### 7.3 既有源码扫描守卫同步

`uuidCallSites.spec.ts`、`i18nKeyReferences.spec.ts`、`hostKeyChallengeProtocol.spec.ts`、`workbench.spec.ts` 的结构守卫改为扫描“App.vue + 新模块”联合源；断言意图不变，只更新位置。禁止通过删除守卫来让测试变绿。

## 8. 执行波次

| 波次 | 内容 | 关键门禁 |
| --- | --- | --- |
| W0 | 实测基线；加入结构守卫（带 allowlist） | 现有全量测试数不变 |
| W1 | 前端目录归位：`app/composables` → 各域；`shared` 越界上移；terminal/sftp 子域分组 | typecheck + 前端全量测试 |
| W2 | App.vue 低风险抽取：装饰引擎、剪贴板、分隔条、持久化、bootstrap 辅助 | 定向测试 + typecheck |
| W3 | App.vue 会话抽取：SSH/本地/Telnet/串口/VNC/RDP | 定向测试 + 浏览器冒烟 |
| W4 | App.vue 终端内核抽取：instance/writer/input/keys/output | 特征测试 + 浏览器冒烟 |
| W5 | App.vue 模板组件化 + CSS 迁移 | 截图对比 + 控制台无报错 |
| W6 | SettingsDialog、figSourceCore、SftpTreePane.spec 拆分 | 前端全量测试 |
| W7 | 后端：core 下行、exec 迁移、生产/测试文件拆分 | `cargo test` + dead-code |
| W8 | 清空 allowlist，跑 `scripts/test.sh --skip-host`，浏览器全流程，升版打包 | 全量门禁 + 产物哈希 |

- W1–W6 串行修改 `App.vue`（单写者）；W7 与 W2–W6 无共享文件，可并行执行。
- 每波独立提交；不做 squash；不 push、不自行合并。
- 每波结束独立 review diff，重点看行为漂移、CSS 作用域、import 方向。

## 9. 验证矩阵

| 层级 | 手段 |
| --- | --- |
| 纯函数 | 现有 Vitest / Rust 单测；新增模块保持同一断言迁移 |
| Vue 行为 | typecheck + 组件/组合式函数测试 + 宿主 mock |
| UI | `frontend/visual.html`（:5299）真实浏览器：主终端、新标签、分屏、`#` AI 搜索、SFTP 选中/展开/右键、拖放上传、设置、Docker、窄视口 |
| 后端 | `cargo test`、dead-code 零警告、MCP stdio smoke |
| 产物 | `scripts/build.sh` 生成 `.dbxp`，报告大小与 SHA-256，由用户安装验证 |

基线测试数量以 W0 实测为准；后续不得低于基线。

## 10. 风险与回滚

| 风险 | 缓解 |
| --- | --- |
| App.vue 抽取导致行为漂移 | 低风险到高风险分波；每波只搬一类；复杂状态机先补特征测试；浏览器冒烟 |
| CSS scoped 失效 | 迁移规则时同步迁移选择器；关键页面截图对比；必要时保留外壳级规则 |
| 源码扫描守卫误报/漏报 | 联合源扫描；保留“扫描集非空/关键文件存在”断言；禁止删除守卫 |
| 目录迁移 import 遗漏 | 纯移动提交 + 全量 typecheck/test；用 codemod 后人工抽查 |
| 后端分层下移改变语义 | 只移动类型/纯函数，不改实现；单独提交标注供人工复核 |
| 重构期间又出现新需求 | 冻结行为；新需求另开分支，避免混入 |

回滚单位是波次：每波一个独立提交，必要时 `git revert` 单波而不影响其他波次。

## 11. 完成定义

1. `App.vue` ≤600 行，且只含 `useWorkbench()` 单入口、外壳模板与生命周期转发。
2. 前端/后端无文件超过 §3 限额，结构守卫无 allowlist。
3. 依赖方向规则全绿。
4. `scripts/test.sh --skip-host` 全绿：后端测试、前端 typecheck/test/build、dead-code、MCP/本地 smoke。
5. 浏览器冒烟清单全部通过，控制台无新增错误。
6. 生成新版本 `.dbxp`，报告路径、大小、SHA-256；由用户安装做最终验收。

## 实施结果

- 行数预算与结构守卫已落地：前端 `frontend/src/shared/lib/structure.spec.ts`、后端 `backend/src/structure_tests.rs`；两侧 allowlist 均为空。
- `frontend/src/app/App.vue` 现为 12 行组合根，工作台外壳在 `frontend/src/app/components/WorkbenchShell.vue`。
- 后端 `core` 保持叶子层（无 `crate::app/features/filesystem/sessions` 引用）。
- 已知且接受的唯一例外：`backend/src/features/**` 与 `backend/src/sessions/**` 仍存在围绕 `sessions::ssh::exec`、sudo/transfer helper 的兄弟 import，待后续提取；后端方向表并未完全强制，守卫当前只覆盖 `core` 叶子约束。
