# MCP Quick Start

RemoteX exposes 16 MCP tools for remote execution, background tasks, SFTP, and
connection discovery. This guide covers setup and common calls. For full tool
semantics, permission profiles, and error handling, see the
[MCP reference](MCP.zh-CN.md).

## Pick a connection mode

| Your setup | Use |
| --- | --- |
| DBX is installed and you want AI to reuse saved connections | **DBX MCP bridge** (recommended) |
| Your client can spawn a process and DBX is not installed | **Standalone stdio** |
| DBX is installed but the client only supports stdio | Standalone stdio + bridge fallback |

## Option 1: DBX MCP bridge

DBX's MCP server provides two generic bridge tools:

| Tool | Purpose |
| --- | --- |
| `dbx_list_plugin_tools` | Discover plugin tools and their JSON schemas |
| `dbx_call_plugin_tool` | Call a plugin tool |

Client configuration:

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

Call flow:

```text
dbx_list_connections
  → find the target connection id
dbx_list_plugin_tools
  → confirm tool names and arguments
dbx_call_plugin_tool {
  "pluginId": "<plugin-id>",
  "tool": "ssh_exec",
  "connectionId": "<connection id>",
  "arguments": { "command": "uptime" }
}
```

- DBX resolves credentials; passwords never appear in tool arguments.
- Jump hosts, sudo profiles, and TOTP settings are shared with the workbench.
- Confirm the host key the first time you connect to an unknown host.

## Option 2: standalone stdio

```bash
dbx-plugin-sshx --mcp
```

Client configuration:

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

Behavior:

- No DBX connection store; credentials are passed inline.
- Unknown hosts use TOFU: recorded on first use, rejected if the key changes later.
- Override the data directory with `DBX_PLUGIN_DATA_DIR`.
- A `connectionId` that is not registered in this session is forwarded to a
  running DBX app when available.

Inline credential example:

```json
{
  "host": "10.0.0.12",
  "port": 22,
  "username": "ops",
  "password": "..."
}
```

Use `privateKeyPath` or `privateKey` for key authentication. Credentials become
part of the tool arguments and the model context; use this only in trusted local
sessions.

### Read-only entry point

```bash
DBX_SSH_MCP_READ_ONLY=1 dbx-plugin-sshx --mcp
```

The whole process is forced read-only. This is a good fit for production access.

## Tool catalog

### Remote execution

| Tool | Purpose |
| --- | --- |
| `ssh_exec` | Run a non-interactive command; read-only connections accept inspection commands only |
| `ssh_exec_sudo` | Run with sudo; answers TOTP/2FA automatically |
| `ssh_multi_exec` | Run one command on 1–10 connections |
| `ssh_run_bg` | Start a long command in the background; returns taskId/logPath |
| `ssh_task_status` | Poll a background task and its output tail |
| `ssh_terminal_input` | Inject input into an already-open terminal |

### File transfer

| Tool | Purpose |
| --- | --- |
| `sftp_list_dir` | List a remote directory |
| `sftp_stat` | Inspect a path; also used to test existence |
| `sftp_read_file` | Read a file with offset pagination |
| `sftp_write_file` | Write a file; parent directory must exist |
| `sftp_upload` | Upload one local file |
| `sftp_download` | Download one remote file |

### Connections and hosts

| Tool | Purpose |
| --- | --- |
| `ssh_list_connections` | List saved connection metadata |
| `ssh_list_known_hosts` | List the plugin's known_hosts entries |
| `ssh_remove_known_host` | Remove a known_hosts entry |
| `ssh_close` | Close a cached connection |

## Typical workflows

### Inspect a server

```text
ssh_list_connections
→ ssh_exec { connectionId, command: "uptime; free -m; df -h /data" }
```

### Long-running task

```text
ssh_run_bg { connectionId, command: "apt upgrade -y" }
→ returns taskId/logPath
→ ssh_task_status { connectionId, logPath }
```

The command runs under nohup; its log lives in `/tmp/.dbx-ssh-tasks/` on the
remote host and remains queryable after a disconnect.

### Multi-host check

```text
ssh_multi_exec {
  targets: ["<id1>", "<id2>"],
  command: "systemctl is-active nginx",
  mode: "parallel"
}
```

Up to 10 targets. `ssh_multi_exec` does not provide sudo.

### Distribute a file

```text
sftp_upload { connectionId, localPath: "/tmp/app.tar.gz", remotePath: "/tmp/app.tar.gz" }
→ ssh_exec { connectionId, command: "tar -xzf /tmp/app.tar.gz -C /opt/app" }
```

The local path must be inside an allowed transfer root; sensitive paths are
rejected.

## Safety gates

Before any network I/O, a call passes:

1. **Connection scope**: the `connectionScope` allowlist.
2. **Read-only connection**: writes and sudo are rejected.
3. **Read-only command allowlist**: only provably read-only inspection commands.
4. **Sensitive paths**: credential and private-key locations are rejected on read-only connections.
5. **sudo allowlist**: privileged commands must match the connection's list.
6. **Destructive-command confirmation**: catastrophic patterns require `confirmDestructive: true`.
7. **Human approval**: an extra confirmation when `execPermissionMode=confirm`.

## Troubleshooting

### Tools are missing

Call `dbx_list_plugin_tools` first. Confirm the plugin is loaded and its version
includes all 16 tools.

### A connection id cannot be resolved

Use `ssh_list_connections` to obtain the id. If connection names are duplicated,
add `host`/`port`/`username`.

### A call is rejected

Read the message to distinguish read-only, allowlist, destructive-command, path,
or permission-profile rejection. See the
[MCP reference](MCP.zh-CN.md#常见错误).

### A command takes too long

Use `ssh_run_bg` + `ssh_task_status`. Host wait limits no longer matter once the
command is detached.

## Verification

```bash
cargo build --release --manifest-path backend/Cargo.toml
python3 scripts/smoke/smoke_mcp.py --binary backend/target/release/dbx-plugin-sshx
```

Live loopback:

```bash
DBX_SSH_SMOKE_PASSWORD=... python3 scripts/smoke/smoke_mcp.py \
  --host <host> --port <port> --username <user>
```

## Next steps

- Full tool semantics and permission profiles: [MCP reference](MCP.zh-CN.md)
- Product features: [Feature guide](../features.zh-CN.md)
- Security model: [Security model](../security.zh-CN.md)
