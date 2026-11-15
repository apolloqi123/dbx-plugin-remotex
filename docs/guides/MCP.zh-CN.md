# MCP 参考

RemoteX 把 SSH/SFTP 能力暴露为 16 个 MCP 工具。本文是工具语义、安全门、权限档、
错误与长任务的参考。第一次接入请先看 [MCP 快速开始](MCP_USAGE.zh-CN.md)。

## 两种运行方式

| 方式 | 适用场景 | 凭据来源 |
| --- | --- | --- |
| DBX MCP 桥 | 已安装 DBX，希望复用保存的连接 | DBX 连接存储，工具参数不含密码 |
| 独立 stdio | 没有 DBX 宿主，或客户端直接拉起进程 | 每次调用内联，或桥接转发 |

两种方式互斥：同一进程只运行插件模式或 MCP 模式。

## DBX MCP 桥

DBX 的 MCP 服务器提供两个通用桥工具：

| 工具 | 作用 |
| --- | --- |
| `dbx_list_plugin_tools` | 列出已加载插件贡献的 MCP 工具与 JSON Schema |
| `dbx_call_plugin_tool` | 调用插件工具；`connectionId` 引用 DBX 保存的连接 |

示例：

```json
{
  "name": "dbx_call_plugin_tool",
  "arguments": {
    "pluginId": "<plugin-id>",
    "tool": "ssh_exec",
    "connectionId": "<DBX connection id>",
    "arguments": { "command": "uptime; df -h /" }
  }
}
```

- 连接凭据由宿主解析后经标准连接生命周期转发，不出现在工具参数里。
- 跳板链、Quick Sudo、TOTP、提示词与认证流程设置与工作台一致。
- 插件侧按 `connectionId` 维护连接池，断线自动重连。
- 主机密钥沿用插件的 known_hosts；首次未知主机需先在或经工作台确认。
- Scoped AI 会话中桥可能禁用变更类工具；以客户端实际可见的工具为准。

客户端配置：

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

## 独立 stdio

```bash
dbx-plugin-sshx --mcp
```

- 协议：MCP `2024-11-05`，换行分隔 JSON-RPC 2.0。
- 无 DBX 连接存储；凭据随调用内联传入，或在 `connectionId` 未注册时转发给运行中的
  DBX 应用。
- 连接按 `username@host:port` 在进程内池化。
- 未知主机密钥使用 TOFU：首次记录，之后变化仍拒绝。
- `initialize` 返回的 `serverInfo.name` 为插件标识，由安装清单决定。

客户端配置示例：

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

### stdio 桥接兜底

在 stdio 会话中用 `connectionId` 调用连接类工具，而该 id 未在本会话注册时：

1. 调用转发给运行中的 DBX 应用的本地 TCP 桥。
2. 应用未运行时会尝试唤起。
3. 凭据由应用侧解析，不进入工具参数。
4. 桥不可用时回落内联凭据路径；无法解析则返回包含三条出路的错误。

注意：转发后命令在应用侧 sidecar 执行，本进程的 `DBX_SSH_MCP_READ_ONLY` 与 sudo
白名单不适用；由应用侧连接自身的只读标志与配置生效。

### 连接 id 从哪来

按暴露面从小到大：

1. `ssh_list_connections`：只返回元数据；密钥字段只返回布尔标志。
2. `connectionName`，重名时补 `host` / `port` / `username`。
3. 完整 endpoint（`host` + `username`，`port` 默认 22）复用已注册连接。
4. 内联凭据：最后兜底，凭据会进入工具参数与模型上下文。

候选超过一个时拒绝并列出候选，不猜选。

## 工具目录（16 个）

### 远程执行

| 工具 | 语义 |
| --- | --- |
| `ssh_exec` | 非交互执行命令。只读连接只放行巡检白名单；灾难模式需 `confirmDestructive`；超过约 10 秒改用 `ssh_run_bg` |
| `ssh_exec_sudo` | sudo 执行；密码经 stdin，TOTP/2FA 自动应答；只读连接直接拒绝 |
| `ssh_multi_exec` | 一条命令在 1–10 个已保存连接上执行，支持并行/顺序与 `stopOnError`；不提供 sudo |
| `ssh_run_bg` | nohup 脱离启动，返回 `taskId` / `pid` / `logPath`；断线不丢 |
| `ssh_task_status` | 轮询后台任务：状态、退出码、输出尾部 |
| `ssh_terminal_input` | 向已打开的 DBX 终端注入输入；不收集输出 |

### 文件传输

| 工具 | 语义 |
| --- | --- |
| `sftp_list_dir` `sftp_stat` | 浏览与检查远端路径；`sftp_stat` 也用于判断存在性 |
| `sftp_read_file` `sftp_write_file` | 读写远端文件，文本或 base64，支持 offset 分页 |
| `sftp_upload` `sftp_download` | 本地 ↔ 远端单文件传输，受大小与本地路径约束 |

### 连接与主机

| 工具 | 语义 |
| --- | --- |
| `ssh_list_connections` | 列出连接元数据；不返回 secret 值 |
| `ssh_list_known_hosts` `ssh_remove_known_host` | 管理插件 known_hosts，不改系统文件 |
| `ssh_close` | 关闭缓存的连接 |

## 安全门

所有门在任何网络 I/O 之前执行，顺序为：

```
未知工具/端口 → connectionScope → 只读连接 → 敏感路径
→ sudo 白名单 → 灾难确认 → 人工审批（若启用）→ 桥转发 → 执行
```

### 只读连接

- 写类工具直接拒绝。
- `ssh_exec` / `ssh_multi_exec` 只接受可证明只读的命令及管道组合。
- 重定向、命令替换、`sudo`、白名单外动词一律拒绝。
- 白名单是允许列表：识别不了就不放行。

若干“形似只读、实可变更”的命令按未知处理，例如 `sort -o`、`find -fprint`、
`ip route flush`、`git branch <name>`、`dmesg -c`、`history -c`。

### 敏感路径

只读连接上，命令参数与读类 SFTP 工具的路径命中凭据位置即拒绝：

- `~/.ssh`、`.gnupg`、`.aws`、`.kube`
- `id_*`、`ssh_host_*_key`、`*.pem`、`*.key`、`*.p12`、`*.pfx`
- `/etc/shadow`、`/etc/sudoers`、`.env`、`.netrc`、`.git-credentials`、`.pgpass`
- 各类 shell history

普通连接不设此限。该门用于降低“提示注入后凭只读连接偷凭据”的风险。

### 灾难命令

任何连接上，命中灾难模式都要求 `confirmDestructive: true`；只读连接直接拒绝，
确认位不能覆盖。

覆盖范围包括：

- `rm -rf` 系统根及其浅层目录
- `mkfs`、`fdisk`、`wipefs`
- `dd of=/dev/...` 与裸设备重定向
- `shutdown`、`reboot`、`init 0/6`
- fork 炸弹、`kill -9 -1`
- 递归修改系统根权限/属主
- 覆盖 `/etc/passwd`、`/etc/shadow`、`/etc/sudoers`、`/etc/fstab`、`/boot`
- `docker prune`、`find -delete`
- SQL `DROP DATABASE` / `DROP TABLE`

判定对包装形态保持保守：

- `sudo` 前缀剥旗标后检查内层
- `sh -c '...'` 递归评估脚本文本
- `$(...)` 与反引号子命令扫描

这些规则只做“收紧”，不会把包装命令提升为只读白名单命令。

### sudo 白名单

连接级 `sudo 命令白名单` 按 sudoers 思路工作，但更严格：

- 每行一条，单行可用 `;` 分隔，`#` 注释。
- 令牌精确匹配；`*` 匹配一个参数；结尾 `*` 匹配剩余且至少一个。
- 不写通配 = 只放行这一条精确命令。
- `sudo` 前缀自动剥离；`-u` 等旗标不建模，永不匹配。
- 白名单非空时，`ssh_exec_sudo` 与 `ssh_exec`/`ssh_run_bg` 中的内联 `sudo` 都必须命中。
- 空 = 门关闭。

### 进程级只读

```bash
DBX_SSH_MCP_READ_ONLY=1 dbx-plugin-sshx --mcp
```

整个进程强制只读，工具无法自行关闭。适合生产环境只读入口。

### 本地传输路径

`sftp_upload`（本地读）与 `sftp_download`（本地写）的本地路径受双重约束：

1. **传输根**：canonical 路径必须落在允许根内。可用 `mcp/settings/set` 配置
   `localTransferRoot`；未配置时默认允许根为系统临时目录 + 插件数据目录。
2. **敏感路径黑名单**：凭据库、shell 启动文件、cron、sudoers、launchd 等一律拒绝，
   上传与下载都生效。

canonical 化由 OS 解析 symlink 与 `..`；配置根不可解析时直接报错，不静默回落。

## 权限档

通过 `mcp/settings/set` 持久化，独立模式可用环境变量覆盖。

### `execPermissionMode`

| 值 | 行为 |
| --- | --- |
| `autonomous`（默认） | 通过既有门后直接执行 |
| `confirm` | 写类工具与 exec 族在既有门之后、执行之前请求人工审批 |

- 审批事件为 `ssh/agent/prompt`，120 秒超时即拒绝。
- 审批弹窗可编辑命令；确认文本替换原文执行。
- stdio 独立会话没有审批通道，gated 调用立即报错，不挂起等待。
- 环境变量：`DBX_SSH_MCP_PERMISSION_MODE=autonomous|confirm`

### `connectionScope`

- 最多 64 条允许条目。
- 条目匹配：连接 id 精确、连接名精确、host（ASCII 大小写不敏感）。
- 非空时，连接类调用解析出的目标越界即整体拒绝。
- 内联凭据（host + username）整体拒绝，避免绕开作用域。
- `ssh_list_connections` 只返回作用域内条目。
- known_hosts 管理等本地工具不受限制。
- 环境变量：`DBX_SSH_MCP_CONNECTION_SCOPE`，逗号分隔；显式空值表示拒绝一切。

## 长任务与断线

1. **stdio 请求并发**：主循环逐请求 spawn，慢命令不阻塞其他调用；stdin 关闭后
   在途请求最多 drain 300 秒。
2. **后台任务**：`ssh_run_bg` + `ssh_task_status`。日志在远端
   `/tmp/.dbx-ssh-tasks/`，宿主放弃等待、连接断开或 sidecar 重启后仍可查询。
3. **pre-exec 重试**：失败发生在命令启动之前时，同一次调用内换新连接重试一次；
   命令已启动后的错误不回重试，避免双执行。
4. **keepalive**：30 秒间隔，3 次容忍，降低 NAT/防火墙掐空闲连接的概率。

## 终端同步执行

连接级 `agentTerminalMode` 决定 MCP 命令是否路由到用户可见的终端：

| 模式 | 行为 |
| --- | --- |
| `off` | 隐藏通道执行，不打开终端 |
| `auto` | 只读命令直接执行；变更与提权命令请求审批 |
| `strict` | 所有命令都请求审批 |

- 命令风险分为 `readonly` / `mutating` / `elevated`。
- 审批弹窗显示完整命令、风险徽标与倒计时，默认 120 秒超时拒绝。
- 可勾选“记住此命令”，写入连接级免审批清单；破坏性命令不会入库。
- `runInTerminal` 参数仅为兼容保留，不再决定路由。
- 终端路径不注入 sudo 密码；由终端内 auto-sudo 或人工输入处理。
- 模式保存在插件数据目录的 `agent-modes.json`，按连接记录。

## 错误码

| 码 | 含义 |
| --- | --- |
| `-32700` | JSON 解析失败 |
| `-32600` | 信封不合法 |
| `-32601` | 未知方法或未知二进制通道 |
| `-32000` | 工具级/应用级错误：门禁拒绝、远端失败、参数错误 |
| `-32001` | 协议版本不匹配 |

stdio 模式下工具错误统一为 `-32000`。桥模式不经 JSON-RPC 信封，业务错误同样
映射为 `-32000`。

### 常见错误

| 文案 | 处理 |
| --- | --- |
| `Unknown tool: '...'` | 检查工具名；提示中会给出 `Did you mean` |
| `Missing required parameters: ...` | 一次补齐列出的参数 |
| `Unknown connection` / 候选列表 | 用 `ssh_list_connections` 取 id，或补 selector |
| `read-only connection` | 连接只读；改用巡检命令或换连接 |
| `sudo command is not allowed` | 命中 sudo 白名单门；调整命令或修改白名单 |
| `confirmDestructive` 提示 | 人工复核后显式传 `true` |
| 本地路径越界 | 配置 `localTransferRoot` 或改到允许目录 |

## 输入容错

参数解析对 LLM 常见写法做了宽容，但仍在网络 I/O 前失败：

- 数字字段接受数字字符串（`"port": "2222"`）。
- 布尔字段接受 `true/false`、`1/0`、`yes/no`、`on/off`（不区分大小写）。
- 端口为 0、越界或非数字时立即报错，不静默回落 22。
- 未知工具名给出大小写/分隔符建议。
- 多个必填参数缺失时一次列出全部。

## 验证

离线 smoke：

```bash
cargo build --release --manifest-path backend/Cargo.toml
python3 scripts/smoke/smoke_mcp.py --binary backend/target/release/dbx-plugin-sshx
```

真机回环：

```bash
DBX_SSH_SMOKE_PASSWORD=... python3 scripts/smoke/smoke_mcp.py \
  --host <host> --port <port> --username <user>
```

环境不可用时用例输出 `SKIP`；离线通过不代表 live 连接通过。
