# RPC 协议

本文定义 UI 与 sidecar 之间的传输格式、方法总表、事件、二进制通道与错误码。
`backend/src/app/rpc/` 的 `match` 分派是方法集合的唯一事实源；
`scripts/verify/mock_host_registry_check.mjs` 会把本文的方法表与代码比对。

## 传输

宿主与插件使用 framed stdio：

```
┌────────┬──────────────────┬─────────────────────┐
│ kind:1 │ length:4 (BE u32)│ payload:length 字节 │
└────────┴──────────────────┴─────────────────────┘
```

| kind | 内容 | 上限 |
| --- | --- | --- |
| `0` | UTF-8 JSON 文本 | 8 MiB |
| `1` | 二进制数据 | 64 MiB |

- JSON 帧承载请求、响应与事件。
- 二进制帧承载终端输出、文件块、画面帧。
- 帧长度超限或 kind 未知时，连接终止并记录错误。

## JSON-RPC 信封

请求：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "ssh/exec",
  "params": { "connectionId": "conn-1", "command": "uptime" }
}
```

成功响应：

```json
{ "jsonrpc": "2.0", "id": 1, "result": { "stdout": "...", "exitCode": 0 } }
```

错误响应：

```json
{ "jsonrpc": "2.0", "id": 1, "error": { "code": -32000, "message": "..." } }
```

事件（无 id，由 sidecar 主动推送）：

```json
{
  "jsonrpc": "2.0",
  "method": "ssh/terminal/state",
  "params": { "sessionId": "s-1", "state": "connected" }
}
```

### 初始化

首个请求为 `plugin/initialize`：

```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "method": "plugin/initialize",
  "params": {
    "host": { "protocolVersions": [1], "version": "..." },
    "locale": "zh-CN"
  }
}
```

响应返回 `protocolVersion`、能力列表与插件版本。宿主与插件没有共同协议版本时返回
`-32001`。

## 方法总表

方法名格式为 `<namespace>/<action>`，可有多级。表中同一行的多个方法共享说明。

### SSH 会话与执行

| 方法 | 说明 |
| --- | --- |
| `ssh/session/open` `ssh/session/attach` `ssh/session/close` `ssh/session/alive` | 打开 PTY、附加已有会话、关闭、探活 |
| `ssh/sessions/list` | 列出当前 sidecar 中的 SSH 会话 |
| `ssh/status` | 连接与认证状态摘要 |
| `ssh/exec` `ssh/exec/cancel` | 非交互执行、取消并返回已收集输出 |
| `ssh/terminal/replay` `ssh/terminal/resize` `ssh/terminal/batchInput` `ssh/terminal/directoryTracking` | 按序号回放、窗口调整、批量输入、目录跟随 |
| `ssh/metrics` `ssh/metrics/history` | 服务器指标快照与历史序列 |
| `ssh/processes/list` `ssh/processes/kill` | 进程列表与结束进程 |
| `ssh/recording/start` `ssh/recording/stop` `ssh/recording/get` `ssh/recording/list` `ssh/recording/search` `ssh/recording/delete` `ssh/recording/clear` `ssh/recording/reveal` | 录制控制、分页读取、搜索、删除、清空、在文件夹中显示 |
| `ssh/forward/interfaces` `ssh/forward/list` `ssh/forward/start` `ssh/forward/stop` | 可绑定地址、转发列表、启动、停止 |
| `ssh/agent/mode/get` `ssh/agent/resolve` | Agent 模式与 socket 解析 |
| `ssh/audit/list` `ssh/audit/clear` | 审计分页查询与清空 |
| `ssh/host-key/check` `ssh/host-key/resolve` `ssh/knownHosts/list` `ssh/knownHosts/remove` | 主机密钥校验、变更处理与插件 known_hosts 管理 |
| `ssh/settings/get` `ssh/settings/set` | 连接级设置读写 |
| `ssh/quickCommands/list` `ssh/quickCommands/save` `ssh/quickCommands/delete` | 快速命令 CRUD |
| `ssh/highlightRules/list` `ssh/highlightRules/save` `ssh/highlightRules/delete` | 输出高亮规则 CRUD |
| `ssh/batchBar/state` | 批量输入条状态 |

### SFTP

| 方法 | 说明 |
| --- | --- |
| `sftp/list` `sftp/stat` `sftp/exists` `sftp/home` `sftp/diskUsage` `sftp/read` `sftp/write` | 目录与文件读写、存在性、主目录、磁盘用量 |
| `sftp/createDirectory` `sftp/rename` `sftp/rename-unique` `sftp/move` `sftp/copy` `sftp/delete` `sftp/deleteMany` `sftp/chmod` `sftp/touch` | 目录与文件操作；`deleteMany` 执行删除计划 |
| `sftp/symlink-create` `sftp/symlink-read` `sftp/symlink-update` | 符号链接创建、读取、替换 |
| `sftp/archive` `sftp/extract` | 压缩与解压 |
| `sftp/bookmarks/list` `sftp/bookmarks/save` `sftp/bookmarks/delete` | 路径书签 CRUD |
| `sftp/upload/start` `sftp/upload/finish` `sftp/upload-local` | 上传开始、完成、从本地路径上传 |
| `sftp/download/start` `sftp/download/next` `sftp/download/finish` `sftp/download/tree/start` | 下载开始、拉取下一块、完成、目录树下载 |
| `sftp/transfer/list` `sftp/transfer/status` `sftp/transfer/cancel` `sftp/transfer/resumable` `sftp/transfer/history` `sftp/transfer/history/clear` | 传输队列、状态、取消、续传检查、按连接历史与清空 |

### 本机终端与文件

| 方法 | 说明 |
| --- | --- |
| `local/terminal/start` `local/terminal/resize` `local/terminal/replay` `local/terminal/launch-options` | 本机 PTY 启动、调整、回放、启动选项 |
| `local/session/list` `local/session/close` | 本机会话列表与关闭 |
| `local/shells/list` `local/capabilities` `local/saveFile` `local/pointer/sample` | 可用 shell、能力探测、保存文件对话框、指针采样（拖放定位） |
| `local/preferences/get` `local/preferences/set` | 本地偏好读写 |
| `local/recording/start` `local/recording/stop` | 本机终端录制 |
| `local/fs/browse` `local/fs/drives` `local/fs/exists` | 本地文件浏览、盘符、存在性 |
| `local/editors/list` `local/reveal` `local/open` `local/open-with` | 外部编辑器、在文件夹中显示、打开、指定应用打开 |
| `local/wallpaper/get` `local/wallpaper/set` `local/wallpaper/clear` | 终端壁纸读写与清除 |
| `local/metrics` | 本机指标 |

### Docker

| 方法 | 说明 |
| --- | --- |
| `docker/list` `docker/action` `docker/stats` `docker/stats/limits` | 容器列表、生命周期操作、资源统计与限制 |
| `docker/logs` `docker/logs/start` `docker/logs/stop` `docker/logs/close` | 日志拉取与跟随 |
| `docker/terminal/open` `docker/terminal/resize` `docker/terminal/close` | 容器 PTY |
| `docker/files/list` `docker/files/stat` `docker/files/cleanup` `docker/files/export` `docker/files/import` `docker/files/prepareUpload` `docker/files/saveLocal` | 容器文件浏览与导入导出 |
| `docker/files/upload/begin` `docker/files/upload/chunk` `docker/files/upload/finish` `docker/files/upload/cancel` | 容器文件分块上传 |

### sudo

| 方法 | 说明 |
| --- | --- |
| `sudo/listDir` `sudo/stat` `sudo/exists` `sudo/readFile` `sudo/writeFile` `sudo/chmod` `sudo/touch` `sudo/mkdir` `sudo/rename` `sudo/remove` `sudo/removeAll` `sudo/removeMany` | 提权文件操作 |
| `sudo/upload/start` `sudo/upload/finish` `sudo/download/start` `sudo/download/cancel` | 提权传输 |
| `sudo/profiles/list` `sudo/profiles/save` `sudo/profiles/delete` `sudo/profiles/options` `sudo/profiles/reveal` | sudo 档案 CRUD 与选项 |

### 远程桌面

| 方法 | 说明 |
| --- | --- |
| `rdp/start` `rdp/close` `rdp/reconnect` `rdp/resize` `rdp/write` `rdp/input` `rdp/replay` `rdp/list` | RDP 会话生命周期、输入、回放 |
| `rdp/certificate/resolve` | 证书确认与信任决策 |
| `rdp/set-clipboard` | 文本剪贴板写入 |
| `rdp/clipboard-files/begin` `rdp/clipboard-files/chunk` `rdp/clipboard-files/commit` `rdp/clipboard-files/archive` `rdp/clipboard-files/status` `rdp/clipboard-files/read` `rdp/clipboard-files/download` `rdp/clipboard-files/clear` `rdp/clipboard-files/abort` | 文件剪贴板分块传输 |
| `vnc/start` `vnc/close` `vnc/reconnect` `vnc/resize` `vnc/write` `vnc/input` `vnc/replay` `vnc/list` `vnc/set-clipboard` | VNC 会话生命周期、输入、回放、剪贴板 |

### 文件传输服务

| 方法 | 说明 |
| --- | --- |
| `filetransfer/connect` `filetransfer/disconnect` `filetransfer/ping` | 独立文件传输连接 |
| `filetransfer/local/list` | 本地文件列表 |
| `filetransfer/credentials/forget` | 清除缓存的传输凭据 |
| `filetransfer/upload/start` `filetransfer/download/start` `filetransfer/cancel` | 上传、下载、取消 |
| `filetransfer/clipboard/fetch` `filetransfer/clipboard/files` `filetransfer/clipboard/remote` `filetransfer/clipboard/write` | 宿主剪贴板文件读写 |

### 串口、Telnet 与自动化

| 方法 | 说明 |
| --- | --- |
| `serial/start` `serial/close` `serial/write` `serial/replay` `serial/list` `serial/ports/list` | 串口会话与端口枚举 |
| `serial/upload/start` `serial/upload/data` `serial/upload/cancel` | 串口文件上传 |
| `telnet/start` `telnet/close` `telnet/write` `telnet/resize` `telnet/replay` `telnet/list` | Telnet 会话 |
| `task/execute` `task/start` `task/status` `task/stop` `task/validate` | Scheduler Task Provider 固定 RPC 面 |
| `triggers/validate` | 终端触发器校验 |
| `watch/start` `watch/stop` `watch/stop-all` `watch/upload` | 外部编辑器文件监视与回传 |

### 连接、设置与集成

| 方法 | 说明 |
| --- | --- |
| `connection/protocol` `connection/action` `connection/test` `connection/connect` `connection/disconnect` `connection/challenge/resolve` | 协议信息、连接操作、测试、连接/断开、认证挑战应答 |
| `keys/discover` `keys/discover/options` | 私钥发现与候选列表 |
| `otp/list` `otp/save` `otp/bind` `otp/unbind` `otp/delete` `otp/generate` `otp/import-qr` | OTP 管理 |
| `filesystem/list` `filesystem/read` `filesystem/write` `filesystem/delete` `filesystem/rename` `filesystem/createDirectory` | 宿主 filesystem-provider 贡献点 |
| `mcp/tools` `mcp/call` `mcp/settings/get` `mcp/settings/set` | MCP 工具发现、执行与设置 |
| `contextMenu/manage-tunnels` `contextMenu/resolve/manage-tunnels` | 宿主连接右键菜单：管理转发 |
| `completion/execute` | 命令补全执行面 |
| `workbench/close` | 关闭工作台 |
| `import/preview/start` `import/preview/finish` `import/preview/cancel` | 第三方连接导入预览 |

## 事件

事件通过 JSON 帧推送，`method` 为事件名，`params` 为负载。常见事件：

| 事件 | 负载要点 |
| --- | --- |
| `ssh/terminal/state` | `sessionId`、`state`、可选错误 |
| `ssh/terminal/inputAck` | `sessionId`、`sequence` |
| `ssh/terminal/error` | `sessionId`、`error` |
| `local/terminal/state` | 本机会话状态 |
| `local/terminal/inputAck` | 本机输入序号确认 |
| `local/terminal/error` | 本机写入失败 |
| `telnet/session/state` | Telnet 会话状态 |
| `telnet/terminal/inputAck` | Telnet 输入确认 |
| `serial/session/state` | 串口会话状态 |
| `serial/terminal/inputAck` | 串口输入确认 |
| `serial/upload/progress` | 上传进度 |
| `vnc/session/state` | VNC 状态 |
| `vnc/pointer` | 远端指针位置 |
| `rdp/session/state` | RDP 状态 |
| `rdp/clipboard` | 剪贴板内容 |
| `sftp/transfer/progress` | 传输进度 |
| `sftp/upload/error` | 上传块写入失败 |
| `filetransfer/progress` | 独立文件传输进度 |
| `filetransfer/done` | 传输完成 |
| `docker/logs/end` | 日志流结束 |
| `docker/terminal/state` | 容器终端状态 |
| `ssh/forward/*` | 转发启动、停止、错误 |
| `ssh/auto-sudo` | 自动提权阶段与结果 |
| `ssh/batchBar/state` | 批量输入状态 |
| `watch/file-modified` | 被监视文件变化 |
| `task/state` `task/progress` `task/log` | 任务状态、进度、日志 |

事件可能携带 `sessionId`/`taskId`/`terminalId`，前端据此路由；未知事件应忽略而不是崩溃。

## 二进制通道

| 通道 | 方向 | 内容 |
| --- | --- | --- |
| `ssh/terminal/out/<sessionId>` | 插件 → UI | SSH 终端输出 |
| `ssh/terminal/in/<sessionId>` | UI → 插件 | SSH 终端输入 |
| `local/terminal/out/<sessionId>` | 插件 → UI | 本机终端输出 |
| `local/terminal/in/<sessionId>` | UI → 插件 | 本机终端输入 |
| `telnet/terminal/out/<sessionId>` | 插件 → UI | Telnet 输出 |
| `telnet/terminal/in/<sessionId>` | UI → 插件 | Telnet 输入 |
| `serial/terminal/out/<sessionId>` | 插件 → UI | 串口输出 |
| `serial/terminal/in/<sessionId>` | UI → 插件 | 串口输入 |
| `docker/terminal/out/<terminalId>` | 插件 → UI | 容器终端输出 |
| `docker/terminal/in/<terminalId>` | UI → 插件 | 容器终端输入 |
| `vnc/frame/<sessionId>` | 插件 → UI | VNC 画面帧 |
| `rdp/frame/<sessionId>` | 插件 → UI | RDP 画面帧 |
| `sftp/upload/<taskId>` | UI → 插件 | 上传数据块 |
| `sftp/download/<taskId>` | 插件 → UI | 下载数据块 |
| `import/preview/<taskId>/<part>` | UI → 插件 | 导入预览文件块 |

### 终端帧格式

终端输入与输出使用同一布局：

```
┌───────────┬──────────────────┬───────────────┐
│ stream:u8 │ sequence:u64 (BE)│ data: 原始字节 │
└───────────┴──────────────────┴───────────────┘
```

| stream | 含义 |
| --- | --- |
| `0` | stdout |
| `1` | stderr |
| `2` | 带内状态 |
| `3` | 仅串口输入通道（stdin） |

共 9 字节头。输入方向以 `[stream][sequence][data]` 发送，输出方向同样带序号。
前端发现序号缺口时调用对应 `.../terminal/replay` 补拉。

### 上传/下载块

- `sftp/upload/<taskId>` 按块追加，序号与偏移由任务状态维护。
- `sftp/download/<taskId>` 由后端推送块，前端写临时文件。
- 块大小由实现决定；不要假设固定值。

## 错误码

| 码 | 含义 |
| --- | --- |
| `-32700` | JSON 解析失败 |
| `-32600` | 请求信封不合法 |
| `-32601` | 方法或二进制通道不存在 |
| `-32602` | 参数不合法 |
| `-32000` | 业务错误：连接、认证、远端命令、门禁拒绝、传输失败 |
| `-32001` | 协议版本不匹配 |

工具级错误统一使用 `-32000`，`message` 直接面向用户，包含可操作的原因。
不要依赖 `message` 文本做程序判断；需要分支时使用结果字段中的结构化状态。

## 兼容性

- `PROTOCOL_VERSION = 1`。
- 新增方法向后兼容；删除或重命名方法必须同步前端、mock 与本文档。
- 事件负载新增字段允许；删除字段视为破坏性变更。
- 二进制帧格式变更必须提升协议版本，或在通道名中体现新格式。

## 校验

```bash
node scripts/verify/mock_host_registry_check.mjs
```

校验内容：

1. 后端分派臂的方法集合
2. mock host 实现的方法
3. 前端 `.invoke("...")` / `rpc("...")` 字面量
4. 本文档表格第一列与列表项中的方法

前三项不一致会让脚本失败；第四项滞后会给出 report-only 警告。
