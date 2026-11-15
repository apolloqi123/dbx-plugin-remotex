# 架构说明

RemoteX 由两部分组成：

- **UI**：Vue 3 + TypeScript 工作台，运行在 DBX 的插件 webview 中。
- **Sidecar**：Rust 进程，通过 framed JSON-RPC 与 DBX 宿主通信，负责所有连接、
  协议、文件和系统操作。

两端共享一份协议：UI 调用 sidecar 的 RPC 方法，sidecar 通过 JSON 事件与二进制
通道推送输出。完整的信封、方法与通道定义见
[RPC 协议](architecture/PROTOCOL.zh-CN.md)。

## 进程与边界

```
┌─────────────────────────────────────────────────────────────┐
│ DBX 宿主                                                     │
│  · 插件生命周期、workbench 容器                              │
│  · secret binding / connection store / Scheduler Task Provider│
│  · MCP 桥（dbx_list_plugin_tools / dbx_call_plugin_tool）     │
└───────────────┬─────────────────────────────┬───────────────┘
                │ framed JSON-RPC + binary     │ host API
┌───────────────▼──────────────┐   ┌──────────▼───────────────┐
│ UI（webview）                │   │ Sidecar（Rust）           │
│ · 工作台、终端、SFTP、面板    │◄──┤ · RPC 分派                │
│ · 帧缓冲与回放调度            │   │ · 会话与协议栈            │
│ · 本地状态与 i18n            │   │ · 文件、传输、Docker      │
└──────────────────────────────┘   │ · 录制、指标、安全、MCP    │
                                   └──────────┬───────────────┘
                                              │ TCP / serial / PTY
                                   ┌──────────▼───────────────┐
                                   │ 远端主机 / 本机           │
                                   └──────────────────────────┘
```

关键边界：

- UI 不直接持有 SSH 连接，也不读写远端文件；所有远端操作经 RPC。
- 凭据由宿主解析后以连接生命周期参数交给 sidecar；UI 只传连接 id 与操作参数。
- 终端输出走二进制通道，避免 base64 膨胀；控制面走 JSON。
- sidecar 崩溃或重启时，UI 依靠会话状态事件与回放重建画面。

## 后端模块

```
backend/src
├── app/             进程入口、生命周期、RPC 分派
│   ├── bootstrap.rs 插件元数据、能力声明、宿主初始化
│   ├── handler.rs   请求分派、二进制通道、输入序号确认
│   └── rpc/         按域拆分的 RPC handler
├── core/            跨功能基础类型
│   ├── model/       连接、终端、路径、名称编码
│   ├── keys/        私钥发现、解码、known_hosts
│   ├── otp/         TOTP 生成与校验
│   ├── otp_store/   OTP 种子存储、二维码解码
│   ├── preferences/ 设置、传输偏好、壁纸、启动命令
│   └── vault.rs     宿主 secret 访问封装
├── sessions/        协议会话
│   ├── ssh/         连接、认证、PTY、SFTP、转发、传输
│   ├── local/       本机 PTY
│   ├── telnet/      Telnet 协商与终端
│   ├── serial/      串口与上传
│   ├── vnc/         VNC 帧缓冲
│   └── rdp/         RDP 会话与剪贴板
├── features/        跨会话能力
│   ├── terminal/    终端帧、复制模式、批量输入
│   ├── remote_files/ 远程文件复制与剪贴板文件
│   ├── transfer/    传输历史、压缩、续传
│   ├── docker.rs    Docker 引擎与容器工作台
│   ├── forwarding/  本地/远程/SOCKS5 转发
│   ├── metrics/     指标采集与历史
│   ├── recording.rs 录制器与回放
│   ├── security/    只读、审计、告警
│   ├── automation/  定时任务、触发器、多机执行
│   ├── import/      第三方客户端导入
│   ├── mcp/         MCP 工具目录、策略、stdio 与桥接
│   ├── agent/       宿主 AI 终端与审批
│   └── completion/  命令补全
├── filesystem/      宿主 filesystem-provider 与本地文件系统
├── platform/        路径、剪贴板、诊断、进程作业对象
└── structure_tests.rs 模块边界与依赖方向检查
```

### 分派路径

1. sidecar 收到 JSON 请求。
2. `app/rpc/mod.rs` 按 namespace 与 group 路由到域 handler。
3. handler 解析参数、调用会话或功能模块。
4. 同步结果作为 JSON response 返回；持续输出通过 emitter 推送。

`app/rpc/` 的 `match` 臂是 RPC 方法的唯一事实源。新增方法必须同时更新：

- 前端调用点
- mock host（`frontend/src/app/mock/host/`）
- 本文档与 [RPC 协议](architecture/PROTOCOL.zh-CN.md)

`scripts/verify/mock_host_registry_check.mjs` 会做四路比对。

### 会话模型

- SSH 会话以 `sessionId` 标识，绑定一条已保存连接。
- 认证后的 transport 可被多个 PTY 复用（“复制会话”）。
- 每个 PTY 有独立输入序号与输出序号；输出帧为
  `[stream:u8][sequence:u64][data]`。
- 会话关闭时先捕获连接归属，再清理传输记录，保证失败行仍能按连接归档。
- 远端命令执行、SFTP、转发共享同一条 transport，但使用独立 channel。

## 前端模块

```
frontend/src
├── app/
│   ├── workbench/   工作台外壳、标签、布局、工具条
│   ├── components/  通用壳组件
│   ├── mock/        mock host，按 RPC 域拆分
│   ├── lib/         RPC 客户端、事件订阅、帧缓冲
│   └── styles/      主题与全局样式
├── features/
│   ├── terminal/    终端视图、标签、分屏、搜索、复制模式
│   ├── sftp/        SFTP 双栏/树、预览、编辑器、删除计划
│   ├── transfers/   传输面板、队列、历史
│   ├── recording/   录制入口、记录面板、回放、导出
│   ├── forwarding/  转发列表与表单
│   ├── docker/      容器列表、日志、统计、终端、文件
│   ├── metrics/     指标卡片与曲线
│   ├── remote-desktop/ VNC/RDP 画布与输入
│   ├── serial/      串口设置与终端
│   ├── telnet/      Telnet 终端
│   ├── security/    只读、审计、告警视图
│   └── settings/    设置页
└── shared/
    ├── lib/         RPC、i18n、主题、协议帧、格式化
    ├── composables/ 通用组合式逻辑
    ├── components/  共享组件
    └── ui/          基础控件
```

### 帧缓冲与回放

终端输出是高吞吐流，前端使用有界帧缓冲：

1. 二进制帧到达后按序号入队。
2. 按序 drain 到 xterm；发现缺口时记录缺口位置。
3. 缓冲超过上限时，先记下有限的重同步目标，再主动请求
   `ssh/terminal/replay`（本地终端为 `local/terminal/replay`，Telnet/串口同理）。
4. 回放补齐后继续 drain，避免洪峰后画面永久停在旧位置。

Telnet、串口、本地终端复用同一套帧缓冲策略。

### 状态与设置

- 连接列表与凭据由宿主提供，UI 只读元数据。
- 界面偏好通过 `ssh/settings/*`、`local/preferences/*` 保存。
- 传输并发、限速、压缩偏好保存在 sidecar，跨窗口一致。
- i18n 支持七种语言，文案按 feature 拆分在 `shared/lib/i18n/messages/`。

## 数据流示例：一次 SFTP 下载

```
UI                  Sidecar                        远端
│ sftp/download/start │                              │
├────────────────────►│ 打开 SFTP channel            │
│                     ├─────────────────────────────►│
│                     │ 读取分块                     │
│ binary sftp/download/<taskId>
│◄────────────────────┤                              │
│ 写入本地临时文件     │                              │
│                     │ 完成后 rename 原子替换        │
│ sftp/transfer/progress (JSON event)                │
│◄────────────────────┤                              │
│ sftp/download/finish│                              │
├────────────────────►│ 校验、记录历史               │
│                     ├─────────────────────────────►│
```

- 进度事件是节流的 JSON；数据块走二进制通道。
- 本地写入使用临时文件，完成后原子替换。
- 终态写入传输历史；历史按连接 id 查询与清空。

## 数据与持久化

| 数据 | 归属 | 说明 |
| --- | --- | --- |
| 连接、密码、私钥 | DBX 宿主 | UI 不落盘；sidecar 按连接生命周期读取 |
| 插件偏好 | 插件数据目录 | JSON，带版本与清洗逻辑 |
| Known Hosts | 插件数据目录 | 独立文件，不修改系统文件 |
| 录制 | 插件数据目录 | asciicast v2，权限按敏感文件处理 |
| 传输历史 | 插件数据目录 | 全局 200 条环形记录，按连接查询 |
| MCP 设置 | 插件数据目录 | 权限档、只读、路径约束 |
| 主机密钥 | 插件 known_hosts | 独立文件，不修改系统文件 |
| OTP 种子、sudo 密码 | 插件 vault | 字段级 AEAD 加密后落盘 |

`DBX_PLUGIN_DATA_DIR` 可重定向数据目录，测试与便携部署使用。

## 打包结构

```
plugin/
├── manifest.json        插件元数据、贡献点、权限、本地化
├── dbx-plugin.toml      后端语言、目录、二进制名、打包包含项
├── assets/              图标
├── ui/                  前端构建产物
└── bin/dbx-plugin-sshx  sidecar 二进制
```

`.dbxp` 是上述目录的压缩包。`scripts/build.sh` 流程：

1. 前端安装、类型检查、测试、构建到 `ui/`。
2. 构建 Rust sidecar。
3. 调用 `dbx-plugin package` 生成 `.dbxp`。
4. 校验打包内的 SDK 与二进制。

发布包按平台与架构命名；CI 在 Linux 上使用较低的 glibc 基线构建 sidecar。

## 设计约束

- **结构测试**：`backend/src/structure_tests.rs` 检查模块边界与禁止依赖。
- **单一方法事实源**：RPC 分派、mock、前端调用与协议文档必须一致。
- **凭据不跨界**：UI 与日志不接触明文密码；MCP 工具参数默认不带凭据。
- **输出可回放**：所有终端类会话使用序号帧，支持缺口补拉。
- **失败可见**：会话、传输、桥接失败都必须转成 UI 可显示的状态或事件。

## 延伸阅读

- [RPC 协议](architecture/PROTOCOL.zh-CN.md)
- [开发指南](development.zh-CN.md)
- [安全模型](security.zh-CN.md)
