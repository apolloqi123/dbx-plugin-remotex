# features/terminal — 子域分组

终端域按「内核适配 / 外观 / 装饰 / 面板 / 传输 / 行为」六类纯逻辑，以及
「会话 / IO / UI」三类编排拆分。每个文件只属于一个子域，spec 与被测模块同目录。

## 子域职责

| 子域 | 职责 | 代表模块 |
| --- | --- | --- |
| `lib/emulator/` | xterm 内核适配层：按键/输入编码与队列、行模型与列宽、模式查询（DECRQM）、OSC 应答、提示符识别、写回压与限流、WebKit 输入补丁、点击定位光标、cwd 目录跟踪、终端应答矩阵锚点 | `terminalInput` `terminalInputQueue` `terminalLineModel` `terminalModeQueries` `terminalOsc` `terminalPromptHints` `terminalBackpressure` `terminalWriteThrottle` `terminalClickCursor` `terminalDirectoryTracking` `terminalProtocolMatrix.spec` |
| `lib/appearance/` | 外观与偏好模型：配色方案与内置目录、背景策略、字体、WebGL 渲染器、布局与 gutter、缩放，以及「终端」行为偏好（渲染/键盘/鼠标/剪贴板/声音）的默认值、归一化与持久化 | `terminalAppearance` `terminalScheme` `terminalSchemeCatalog` `terminalBackground` `terminalFont` `terminalWebgl` `terminalLayout` `terminalGutter` `terminalZoom` `terminalBehavior` |
| `lib/decorations/` | 终端表面装饰：链接识别与 addon、关键字高亮、ghost 建议、快速选择 | `actionLinksAddon` `actionLinksMatcher` `keywordHighlight` `terminalGhostSuggest` `quickSelect` |
| `lib/panels/` | 面板与标签页支撑：历史面板模型、命令历史环形存储、命令标记、shell 标签标题 | `historyPanel` `commandHistory` `terminalCommandMarkers` `shellTabTitle` |
| `lib/transfer/` | 文件传输协议：批量发送、trzsz、zmodem 的检测与状态机 | `batchSend` `terminalTrzsz` `terminalZmodem` |
| `lib/behavior/` | 行为与策略：快捷键表、交互策略（选中复制/右键）、危险命令、交互式提示守卫、回车行采集、输出文本处理、重连、分屏模型、提示词语料 | `terminalHotkeys` `terminalInteraction` `dangerousCommands` `interactivePromptGuard` `terminalEnterCapture` `terminalOutputText` `terminalReconnect` `terminalSplit` `promptCorpus.spec` |
| `composables/session/` | 会话级编排：与传输/连接生命周期绑定的 composable，以及本地 shell 偏好 | `useTerminalTransportWatch` `useLocalShellPreferences` |
| `composables/io/` | IO 编排：终端输入队列与发送、按键路由与 overlay 仲裁、复制/粘贴/危险确认/OSC52 | `useTerminalInput` `useTerminalKeys` `useTerminalClipboard` |
| `composables/ui/` | 组件状态与交互：搜索、菜单、tooltip、提示、分屏、字体缩放、外观、历史面板、批量发送与传输对话框、AI 助手等 | `useTerminalSearch` `useTerminalMenus` `useSplitTerminal` `useTerminalAppearance` `useCommandHistory` `useZmodem` `useTrzsz` … |

`components/` 不参与子域分组：它是 UI 组合层，可同时消费 `lib` 与 `composables` 的公开导出。

## 边界规则

1. **跨子域只经 `lib` 的具名导出**：引用 `lib/<subdomain>/<module>` 导出的具名符号，
   不要依赖其未导出的内部结构；不得跨子域引用 `composables/`（`session`/`io`/`ui`
   互不 import），也不得跨子域引用组件。
2. 同子域内的文件可自由相互引用（含相对路径与 `index.ts`）。
3. 依赖方向：`components` → `composables` → `lib` → `shared`。任何反向引用都是违规。
4. spec 与被测模块同目录，文件名 `<module>.spec.ts`；纯逻辑单测不得挂到 `components/`。

## 已知例外

`composables/ui/useOnlineSearch.ts` 目前仍从 `components/TerminalContextMenu.vue` 导入纯函数
（`buildSearchUrl` 等），属于规则 3 的既存例外；这些 helper 需先抽到 `lib/`，规则才能完全落地。
