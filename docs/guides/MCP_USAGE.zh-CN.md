# MCP 快速开始

RemoteX 提供 16 个 MCP 工具，覆盖远程执行、后台任务、SFTP 与连接查询。本文只讲
接入和常用调用；完整语义、权限档与错误处理见 [MCP 参考](MCP.zh-CN.md)。

## 选择接入方式

| 你的情况 | 用哪种 |
| --- | --- |
| 已安装 DBX，想让 AI 使用保存的连接 | **DBX MCP 桥**（推荐） |
| 客户端可以直接拉起进程，没有 DBX | **独立 stdio** |
| 有 DBX，但客户端只支持 stdio | 独立 stdio + 桥接兜底 |

## 方式一：DBX MCP 桥

DBX 的 MCP 服务器内置两个通用桥工具：

| 工具 | 作用 |
| --- | --- |
| `dbx_list_plugin_tools` | 发现已加载插件的工具与参数 Schema |
| `dbx_call_plugin_tool` | 调用插件工具 |

配置你的 MCP 客户端：

```json
{
  "mcpServers": {
    "dbx": {
      "command": "/path/to/dbx",
      "args": ["mcp"]
    }
  }
}
```

调用流程：

```text
dbx_list_connections
  → 找到目标连接 id
dbx_list_plugin_tools
  → 确认工具名与参数
dbx_call_plugin_tool {
  "pluginId": "<plugin-id>",
  "tool": "ssh_exec",
  "connectionId": "<connection id>",
  "arguments": { "command": "uptime" }
}
```

- 凭据由 DBX 解析，不出现在工具参数里。
- 连接配置（跳板、sudo、TOTP）与工作台共用。
- 首次连接未知主机需确认主机密钥。

## 方式二：独立 stdio

```bash
dbx-plugin-sshx --mcp
```

客户端配置：

```json
{
  "mcpServers": {
    "remotex": {
      "type": "stdio",
      "command": "/absolute/path/backend/target/release/dbx-plugin-sshx",
      "args": ["--mcp"]
    }
  }
}
```

特点：

- 无 DBX 连接存储，凭据内联传入。
- 未知主机 TOFU：首次记录，之后变化拒绝。
- 数据目录可用 `DBX_PLUGIN_DATA_DIR` 重定向。
- 传 `connectionId` 但本会话未注册时，会尝试转发给运行中的 DBX 应用。

内联凭据示例：

```json
{
  "host": "10.0.0.12",
  "port": 22,
  "username": "ops",
  "password": "..."
}
```

私钥可用 `privateKeyPath` 或 `privateKey`。凭据会进入工具参数与模型上下文，仅限
本机可信会话。

### 只读入口

```bash
DBX_SSH_MCP_READ_ONLY=1 dbx-plugin-sshx --mcp
```

整个进程强制只读，适合生产环境。

## 工具速览

### 远程执行

| 工具 | 用途 |
| --- | --- |
| `ssh_exec` | 执行非交互命令；只读连接只放行巡检命令 |
| `ssh_exec_sudo` | sudo 执行，自动应答 TOTP/2FA |
| `ssh_multi_exec` | 在 1–10 个连接上执行同一命令 |
| `ssh_run_bg` | 长命令后台运行，返回 taskId/logPath |
| `ssh_task_status` | 轮询后台任务状态与输出 |
| `ssh_terminal_input` | 向已打开的终端注入输入 |

### 文件传输

| 工具 | 用途 |
| --- | --- |
| `sftp_list_dir` | 列目录 |
| `sftp_stat` | 查看路径信息；也用于判断存在性 |
| `sftp_read_file` | 读文件，支持 offset 分页 |
| `sftp_write_file` | 写文件，父目录需已存在 |
| `sftp_upload` | 本地 → 远端单文件 |
| `sftp_download` | 远端 → 本地单文件 |

### 连接与主机

| 工具 | 用途 |
| --- | --- |
| `ssh_list_connections` | 列出已保存连接的元数据 |
| `ssh_list_known_hosts` | 列出插件 known_hosts |
| `ssh_remove_known_host` | 移除 known_hosts 条目 |
| `ssh_close` | 关闭缓存连接 |

## 典型用法

### 服务器巡检

```text
ssh_list_connections
→ ssh_exec { connectionId, command: "uptime; free -m; df -h /data" }
```

### 长任务

```text
ssh_run_bg { connectionId, command: "apt upgrade -y" }
→ 返回 taskId/logPath
→ ssh_task_status { connectionId, logPath }
```

后台命令用 nohup 运行，日志在远端 `/tmp/.dbx-ssh-tasks/`，断线后可继续查询。

### 多机执行

```text
ssh_multi_exec {
  targets: ["<id1>", "<id2>"],
  command: "systemctl is-active nginx",
  mode: "parallel"
}
```

一次最多 10 个目标；不提供 sudo。

### 文件分发

```text
sftp_upload { connectionId, localPath: "/tmp/app.tar.gz", remotePath: "/tmp/app.tar.gz" }
→ ssh_exec { connectionId, command: "tar -xzf /tmp/app.tar.gz -C /opt/app" }
```

本地路径必须落在允许根内；敏感路径一律拒绝。

## 安全边界

调用在执行前经过：

1. **连接范围**：`connectionScope` 允许清单。
2. **只读连接**：写操作与 sudo 拒绝。
3. **只读命令白名单**：只放行可证明只读的巡检命令。
4. **敏感路径**：只读连接上拒绝凭据与私钥位置。
5. **sudo 白名单**：提权命令必须命中连接配置。
6. **危险命令确认**：灾难模式要求 `confirmDestructive: true`。
7. **人工审批**：`execPermissionMode=confirm` 时再请求一次确认。

这些门都在网络 I/O 之前执行。

## 常见问题

### 找不到工具

先调用 `dbx_list_plugin_tools`。确认插件已加载且版本包含 16 个工具。

### 连接 id 解析失败

用 `ssh_list_connections` 取 id；连接名重复时补 `host`/`port`/`username`。

### 工具被拒绝

按错误文案区分只读、白名单、危险命令、路径或权限档。详见
[MCP 参考的常见错误](MCP.zh-CN.md#常见错误)。

### 命令执行很久

改用 `ssh_run_bg` + `ssh_task_status`。宿主等待有上限，后台任务不受影响。

## 验证

```bash
cargo build --release --manifest-path backend/Cargo.toml
python3 scripts/smoke/smoke_mcp.py --binary backend/target/release/dbx-plugin-sshx
```

真机回环：

```bash
DBX_SSH_SMOKE_PASSWORD=... python3 scripts/smoke/smoke_mcp.py \
  --host <host> --port <port> --username <user>
```

## 下一步

- 完整工具语义与权限档：[MCP 参考](MCP.zh-CN.md)
- 产品功能：[功能指南](../features.zh-CN.md)
- 安全模型：[安全模型](../security.zh-CN.md)
