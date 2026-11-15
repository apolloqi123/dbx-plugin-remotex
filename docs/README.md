# RemoteX 文档

RemoteX 是 DBX 的远程连接工作台，支持 SSH、Telnet、VNC、RDP、串口与本机终端。
本目录按“先上手、再查功能、最后看实现”的顺序组织。

## 推荐阅读顺序

| 顺序 | 文档 | 适合谁 |
| --- | --- | --- |
| 1 | [快速开始](getting-started.zh-CN.md) | 第一次安装和使用 RemoteX |
| 2 | [功能指南](features.zh-CN.md) | 想了解每个面板能做什么 |
| 3 | [安全模型](security.zh-CN.md) | 需要开启只读、审计或接入生产环境 |
| 4 | [故障排查](troubleshooting.zh-CN.md) | 遇到连接、传输或构建问题 |
| 5 | [架构说明](architecture.zh-CN.md) | 需要改代码或排查跨进程问题 |
| 6 | [RPC 协议](architecture/PROTOCOL.zh-CN.md) | 需要调用后端方法或调试事件 |
| 7 | [开发指南](development.zh-CN.md) | 本地开发、测试、打包与发布 |

## 按任务查找

### 使用

- 安装、第一条连接、界面导览：[快速开始](getting-started.zh-CN.md)
- 终端、SFTP、端口转发、容器、监控、录制：[功能指南](features.zh-CN.md)
- 导入已有连接：[快速开始 · 导入连接](getting-started.zh-CN.md#import)
- 只读模式、sudo、主机密钥、审计：[安全模型](security.zh-CN.md)
- 常见报错与恢复步骤：[故障排查](troubleshooting.zh-CN.md)

### 集成与自动化

- MCP 快速开始（中文）：[MCP_USAGE.zh-CN.md](guides/MCP_USAGE.zh-CN.md)
- MCP quick start (English): [MCP_USAGE.en.md](guides/MCP_USAGE.en.md)
- MCP 工具语义、权限档、错误码、长任务：[MCP.zh-CN.md](guides/MCP.zh-CN.md)

### 开发

- 环境、测试、打包、发布检查：[开发指南](development.zh-CN.md)
- 进程边界、模块划分、数据流：[架构说明](architecture.zh-CN.md)
- JSON-RPC 信封、方法总表、二进制通道：[RPC 协议](architecture/PROTOCOL.zh-CN.md)

## 文档约定

- 产品名统一写作 **RemoteX**。
- 插件在宿主中的技术标识由安装清单决定，文档不重复硬编码；集成配置中使用
  `<plugin-id>` 占位符。
- 命令默认在仓库根目录执行。
- 路径中的 `$DBX_PLUGIN_DATA_DIR` 表示插件数据目录；未设置时由宿主分配的插件
  数据目录决定。
- “本地终端”与“RemoteX 终端”指同一功能：运行在 DBX 所在机器上的登录 Shell。

## 反馈

- 问题与功能建议：GitHub Issues
- 发布包与版本说明：GitHub Releases
- 安全问题：请勿公开提交包含凭据、私钥或主机信息的复现材料
