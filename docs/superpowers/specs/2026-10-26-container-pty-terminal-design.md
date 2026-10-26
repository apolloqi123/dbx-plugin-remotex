# 容器管理默认分栏与容器 PTY 终端设计

**日期：** 2026-10-26
**状态：** 已批准（用户确认设计后进入实现）
**关联：** `2026-10-26-container-manager-pane-design.md`

## 目标

1. 打开容器管理分屏时同时显示「容器列表 + 日志」左右分栏，而不是先只显示列表。
2. 容器终端页升级为真正的 PTY 会话，输入与显示体验对齐宿主服务器终端（交互式 shell、颜色、方向键/历史、Ctrl+C、全屏程序、resize、复制粘贴）。

## 非目标

- 不复制宿主终端的 AI、gutter、命令高亮、命令面板、会话录制、目录跟随等专属装饰。
- 不做容器终端内部的页签/分屏。
- 容器终端不跨「切换容器 / 收起详情 / 关闭分屏 / 组件卸载」保活。
- 不改动「在终端打开」按钮（仍向宿主终端输入行回填 `docker exec -it …`）。

## 一、默认分栏与选中语义

### 打开容器管理

- 分屏内部初始：列表占 40%，详情占余量；分隔条可拖拽或方向键调整（25%–70%，既有实现）。
- 列表首轮非空数据落地后，自动选中第一个 `running` 容器（否则第一个），详情以 `Logs` 页签打开；日志流按既有 `visible` 语义立即开始。
- 列表为空或加载中：只显示列表，不渲染详情列。

### 用户动作

| 动作 | 结果 |
| --- | --- |
| 行点击（详情已打开） | 切换详情目标容器，保持当前页签 |
| 行点击（详情已收起） | 以 `Logs` 页签重新打开详情 |
| 行菜单 Logs / Files | 打开（或切换）详情并定位页签 |
| 详情页头 X | 只收起详情，列表恢复整宽 |
| 轮询/刷新自动选中（含状态变化） | 只更新已打开详情的快照；详情收起后**不**自动重开 |
| 列表清空 | 收起详情 |
| 连接切换 | 清空选中、收起详情、复位“首轮自动打开”状态 |

实现要点：列表的 `select` 事件恢复 `explicit` 参数（用户显式点击为 true）；面板用 `autoOpened` 标记保证自动打开只发生一次。

## 二、容器 PTY 终端

### 2.1 交互形态

- `ContainerDetailPane` 三个页签保持不变：Logs / Files / Terminal（仅图标 + title/aria-label）。
- 三个页签仍用 `v-show` 常驻挂载；终端组件只在「页签可见 + 容器 running + 非只读」时建立 PTY。
- 切走页签时 PTY 保持存活，切回重新 fit 并聚焦；关详情、切容器、关分屏、组件卸载时关闭 PTY。
- 关闭后重新切回会建立新 shell（不保留 cwd/env）。

### 2.2 前端技术形态

- xterm.js + `FitAddon` + `Unicode11Addon` + `WebLinksAddon`；scrollback 5,000。
- 主题与字体沿用工作台：`WorkbenchWorkspace` 已有 `splitTerminalTheme` / `splitTerminalFont`，经 `ContainerManagerPane` → `ContainerDetailPane` → `ContainerTerminalTab` 下传（props 可选，缺省用内置浅色/等宽回退）。
- 键盘输入 → `onData` → 输入队列 → 二进制通道；窗口尺寸变化 → `FitAddon` → resize RPC（同尺寸去重）。
- 复制粘贴使用 xterm 原生选区与 webview 默认行为，不额外拦截。
- 终端区域填充详情面板剩余高度；ResizeObserver 同时响应底部分屏拖拽。

### 2.3 协议

请求（host → sidecar）：

```text
docker/terminal/open
  { terminalId?: string,      // 客户端预生成的 UUID；合法则沿用，避免 open 返回前的输出丢失
    containerId: string,      // 过容器 id 校验
    cols: number, rows: number,
    sessionId?: string,       // SSH 面
    target?: "local",         // 本机面（缺省 SSH）
    cli?, socket?, host? }    // 引擎参数（既有 parse_engine）
  → { terminalId: string }

docker/terminal/resize
  { terminalId, cols, rows } → { resized: boolean }   // 未知 id 返回 false，不报错

docker/terminal/close
  { terminalId } → { stopped: boolean }              // 幂等，未知 id 返回 false
```

二进制通道：

```text
输入  docker/terminal/in/<terminalId>
      8 字节大端序号 + 原始按键字节（与 ssh/local 终端同封装）
输出  docker/terminal/out/<terminalId>
      原始 PTY 字节（桥保证有序；不再包序号，不做重放）
```

生命周期事件：

```text
docker/terminal/state
  { terminalId, state: "exited", exitCode: number }        // shell 自然退出
  { terminalId, state: "error", error: string }            // 传输/启动后错误
```

打开阶段的失败（并发上限、只读连接、CLI 缺失、SSH 会话不存在）直接以 RPC 错误返回；显式 close 不产生 state 事件。

### 2.4 后端结构

```text
backend/src/features/docker/shell.rs        注册表 + 启动编排 + 事件/审计 + 本机 PTY 泵
backend/src/sessions/ssh/container_terminal.rs  SSH PTY channel 泵
backend/src/app/rpc/docker.rs               docker/terminal/{open,resize,close} 分派
backend/src/app/handler.rs                  docker/terminal/in/<id> 二进制输入
backend/src/app/bootstrap.rs                DockerTerminals 实例
```

- 注册表 `DockerTerminals`：`terminalId → { cancel, inputSender, resizeSender, startedAt }`；并发上限 4；TTL 2 小时（超时 cancel 并释放槽位）；`Drop` 取消全部。
- SSH 泵：复用当前 SSH 会话的已认证 handle，新开 session channel → `request_pty("xterm-256color", cols, rows)` → `exec("… docker exec -it -e TERM=xterm-256color <id> sh -c '<bash 优先，否则 sh>'")`；循环多路复用：channel 数据 → 输出 sink，输入队列 → `channel.data`，resize → `channel.window_change`，cancel → `channel.close`；收集 `ExitStatus`。
- 本机泵：`portable-pty` 开 PTY，`CommandBuilder` 直接执行 `docker exec -i -t -e TERM=xterm-256color <id> sh -c <inner>`；端点经 `DOCKER_HOST` / `CONTAINER_HOST` 环境变量注入；reader 线程 → 有界输出队列 → 事件；writer 写入在阻塞线程执行；child wait 线程回传退出码；cancel/close 走 `ChildKiller`。
- 输出不批处理、不落盘、不重放；每个 PTY read（≤8 KiB）立即发一个二进制事件。
- 输入使用既有 `sessions::input_queue::enqueue_bounded`（满队列短暂重试后丢弃，不当作会话死亡）。

### 2.5 安全与审计

- 容器 id 必须通过 `validate_container_id`；终端命令只有固定形态，用户无法注入命令或参数。
- SSH 连接只读（`ensure_writable` 失败）直接拒绝打开；前端 `canWrite === false` 同时禁用入口。
- 打开时写一条审计意图：`{cli} exec -it <id> sh`（不含引擎端点导出、按键或输出）；失败只记录内容无关的原因。
- 输入输出不进审计、不落盘；容器 id 与终端 id 都不允许出现在错误文案之外的持久化中。
- 单 sidecar 并发上限 4，防止 UI 失控或脚本反复打开耗尽 PTY。

### 2.6 错误与降级

| 场景 | 行为 |
| --- | --- |
| 容器未运行 / paused | 终端页显示「容器未运行，请先启动」，不发起 open；容器状态轮询变化后按钮自动恢复 |
| 只读连接 | 显示「此连接为只读，无法打开容器终端」 |
| 镜像无 `sh`/`bash`（distroless/scratch） | docker 输出错误后退出；显示「容器内没有可用的 sh/bash」+ 重连按钮 |
| SSH 断开 / 会话失效 | state=error；显示错误 + 重连按钮 |
| shell 退出（exit） | state=exited + 退出码；显示「会话已结束」+ 重连按钮 |
| 并发上限 | open RPC 错误「Too many active container terminals (4)…」 |
| 打开超时（SSH 30s） | RPC 错误，UI 显示 + 重连 |

### 2.7 移除旧终端

- 删除 `docker/exec` RPC、`ContainerTerminalTab` 的逐条命令控制台、mock 的 `docker/exec`、对应 i18n（execHint/execPlaceholder/execRun/execClear/execEmpty/execRunning/execFailed/execExitCode/execReadOnly）与测试。
- `terminalHint`（在终端打开按钮说明）保留。

## 三、测试

### 后端

- 命令渲染：固定 `docker exec -it` 形态、id 校验、非法 id 拒绝、引擎端点只进环境变量/导出前缀。
- 注册表：并发上限、resize/close 幂等、TTL 回收、Drop 取消、输入满队列丢弃不报错。
- SSH 泵：用测试假 channel / 既有 russh 夹具验证 PTY 请求参数、数据→sink、输入/resize 转发、退出码、取消关闭。
- 本机泵：真实 `portable-pty` 起短命令（如 `sh -c 'printf ready'`）验证输出与退出码；Docker 可用时跑真实 `docker exec` 冒烟（不可用自动 SKIP）。
- 审计：只写固定意图行，不含输出/按键。

### 前端

- `useContainerTerminal`：open 前订阅、terminalId 沿用、输入封装、resize 去重、state 事件、失败/重试、关闭幂等、只读/未运行不打开。
- `ContainerTerminalTab`：可见时才 open、不可见保持、容器切换重开、xterm 数据写出、retry、只读/未运行文案、卸载关闭。
- `ContainerManagerPane`：首轮自动打开 Logs；收起后轮询不重开；行点击重开；行菜单页签；清空/连接切换。
- mock：`docker/terminal/*` + 输入回显、state 事件、`?docker=off` 错误态。
- 浏览器 fixture：默认列表+日志分栏、终端页输入 `echo hi` 显示输出与提示符、分隔条拖拽后终端重新 fit。

### 全量

- 结构/死代码守卫、typecheck、前后端测试、真实本机 Docker 冒烟、打包哈希。

## 四、兼容与限制

- 只支持 POSIX `sh`/`bash` 容器；Windows 容器（无 sh）不支持交互终端（明确报错）。
- 终端不持久化：关详情/切容器即销毁。
- 本机面在 Windows sidecar 上使用 ConPTY；SSH 面行为与平台无关。
- 并发 4 个终端 + TTL 2 小时为兜底，不提供设置项。
