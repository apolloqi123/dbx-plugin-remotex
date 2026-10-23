//! MCP catalog helpers.

use super::*;

/// Parameter descriptions shared by the connection property family and the
/// Quick Sudo profile tool (schema round 6: every advertised parameter must
/// carry an accurate description; defined once here, referenced by both
/// schema sites so the wording can never drift apart).
pub(super) const AUTHENTICATION_DESCRIPTION: &str = "Auth for inline dials: password (default), private-key, private-key-password, or agent; omitted infers from privateKey*/password";

pub(super) const CONNECT_TIMEOUT_DESCRIPTION: &str =
    "TCP connect timeout in seconds for inline dials (default 30, min 1)";

pub(super) const AUTH_FLOW_MODE_DESCRIPTION: &str = "2FA sudo flow: password_only (no OTP), password_plus_otp (combined prompt), password_then_otp (separate; default); unknown values fall back to the default";

/// Tool-specific parameters layered on the shared connection properties.
/// Each entry is `(key, json type, description)`; the type is part of the
/// contract because the dispatcher parses numbers and booleans from these
/// fields (`maxBytes`, `overwrite`, …).
pub(super) fn connection_properties(extra: &[(&str, &str, &str)]) -> Value {
    let mut properties = json!({
        // Declared here so strict MCP hosts forward it: the dispatcher reads
        // it for terminal routing (runInTerminal) and stored-connection
        // resolution, and an undeclared argument is dropped by schema
        // validation before the sidecar ever sees the call.
        //
        // 描述文本按 #172/#203 预算刻意精简（属性名/类型/枚举齐全，语义保留）：
        // 16 个工具 × 21 参数逐字重复，description 文本是 tools/list 尺寸的
        // 绝对大头；自明参数（host/port/username/路径类）整条去掉
        // description，改写时只删铺垫，不删模型正确调用所需的行为语义与
        // 安全警告。超预算先砍最长描述（见 tool_definitions_stay_under 测试）。
        "connectionId": { "type": "string", "description": "Saved DBX connection id (via ssh_list_connections)" },
        "connectionName": { "type": "string", "description": "Saved connection name (may replace connectionId); ambiguous matches are refused with candidates" },
        "host": { "type": "string" },
        "port": { "type": "integer" },
        "username": { "type": "string" },
        "password": { "type": "string", "description": "Login password (password auth, or sudo fallback)" },
        "privateKeyPath": { "type": "string" },
        "privateKeyContent": { "type": "string", "description": "Private key contents (OpenSSH/PEM/PPK); overrides privateKeyPath" },
        "privateKeyPassphrase": { "type": "string" },
        "agentSocket": { "type": "string" },
        "authentication": { "type": "string", "enum": ["password", "private-key", "private-key-password", "agent"], "description": AUTHENTICATION_DESCRIPTION },
        "connectTimeoutSecs": { "type": "integer", "description": CONNECT_TIMEOUT_DESCRIPTION },
        "sudoPassword": { "type": "string", "description": "Sudo password override (defaults to password)" },
        "totpSecret": { "type": "string", "description": "TOTP secret (otpauth:// URI, base32, or static code) for 2FA auto-answer; newline/semicolon separated secrets auto-rotate" },
        "authFlowMode": { "type": "string", "enum": ["password_only", "password_plus_otp", "password_then_otp"], "description": AUTH_FLOW_MODE_DESCRIPTION },
        "passwordPromptHint": { "type": "string" },
        "totpPromptHint": { "type": "string" },
        "jumpHosts": { "type": "array", "description": "ProxyJump chain (up to 3): [{\"host\":\"bastion\",\"port\":22,\"username\":\"ops\",\"password\":\"…\"}]; replaces direct dialing" },
        "triggers": { "type": "string", "description": "Expect-style triggers, JSON or tssh text, e.g. {\"stages\":[{\"pattern\":\"(?i)code\",\"sendText\":\"654321\\\\r\"}]}; each stage replies when its regex matches PTY output; invalid rules fail the dial. A malicious server can fake prompts to harvest replies" },
        "passwordCommand": { "type": "string", "description": "Local command; stdout (minus newline) is the login password. Placeholders: %h %u %p %%; explicit args win" },
        "passphraseCommand": { "type": "string", "description": "Local command; stdout is the key passphrase. Same placeholders as passwordCommand" },
    });
    if let Some(map) = properties.as_object_mut() {
        for (key, kind, description) in extra {
            map.insert(
                key.to_string(),
                json!({ "type": kind, "description": description }),
            );
        }
    }
    properties
}

pub(super) fn connection_selector_requirements() -> Value {
    json!([
        { "required": ["connectionId"] },
        { "required": ["connectionName"] },
        { "required": ["host", "username"] },
    ])
}

/// Advisory MCP tool annotations (spec "Tool annotations"): per-tool hints
/// clients may use to pre-approve read-only tools or gate destructive ones.
/// Advisory only — the server-side gates (mcp_safety whitelist, confirm
/// gate, read-only connection gate) stay authoritative regardless of what
/// a client does with these hints. The classification deliberately mirrors
/// the internal gate sets (`is_write_tool` / `is_confirm_gated_tool` /
/// read-only sparing) so hints and enforcement cannot drift apart in
/// opposite directions: the exec family and the overwrite-capable transfer
/// tools are destructive=true, everything the confirm gate spares as
/// provably read-only is readOnly=true, and `ssh_close` sits
/// destructive=false between the two.
pub(super) fn tool_annotations(name: &str) -> Value {
    // Provably no mutation of remote or local state (matches the set the
    // confirm gate spares).
    const READ_ONLY: &[&str] = &[
        "ssh_list_connections",
        "ssh_list_known_hosts",
        "ssh_task_status",
        "sftp_list_dir",
        "sftp_stat",
        "sftp_read_file",
    ];
    // May destroy or replace existing state: the exec family (arbitrary
    // remote commands), store deletions, and overwrite-capable writes
    // (matches the confirm-gated set).
    const DESTRUCTIVE: &[&str] = &[
        "ssh_exec",
        "ssh_exec_sudo",
        "ssh_multi_exec",
        "ssh_terminal_input",
        "ssh_run_bg",
        "ssh_remove_known_host",
        "sftp_upload",
        "sftp_download",
        "sftp_write_file",
    ];
    // Mutating but non-destructive, and repeating them converges to the
    // same state instead of compounding (idempotentHint).
    const IDEMPOTENT_MUTATING: &[&str] = &["ssh_close"];
    let read_only = READ_ONLY.contains(&name);
    let destructive = DESTRUCTIVE.contains(&name);
    let mut annotations = json!({
        "readOnlyHint": read_only,
        "destructiveHint": destructive,
        "idempotentHint": read_only || IDEMPOTENT_MUTATING.contains(&name),
        "openWorldHint": true,
    });
    if let Some(title) = tool_title(name) {
        annotations["title"] = json!(title);
    }
    annotations
}

/// Human-readable display names (spec `annotations.title`) for MCP client
/// tool pickers. Unknown names yield no title and keep the conservative
/// default hints from [`tool_annotations`].
pub(super) fn tool_title(name: &str) -> Option<&'static str> {
    Some(match name {
        "ssh_exec" => "Run SSH command",
        "ssh_exec_sudo" => "Run SSH command with sudo",
        "ssh_multi_exec" => "Run SSH command on several connections",
        "ssh_terminal_input" => "Inject terminal input",
        "ssh_run_bg" => "Start background SSH task",
        "ssh_task_status" => "Poll background SSH task",
        "ssh_close" => "Close SSH connection",
        "ssh_list_known_hosts" => "List known hosts",
        "ssh_list_connections" => "List SSH connections",
        "ssh_remove_known_host" => "Remove known host",
        "sftp_list_dir" => "List remote directory",
        "sftp_stat" => "Stat remote path",
        "sftp_upload" => "Upload file over SFTP",
        "sftp_download" => "Download file over SFTP",
        "sftp_read_file" => "Read remote file",
        "sftp_write_file" => "Write remote file",
        _ => return None,
    })
}

/// Every MCP tool name this sidecar registers, kept in sync with
/// `tool_definitions` (a unit test pins the two together). Backs the
/// unknown-tool check with its did-you-mean suggestion.
pub const TOOL_NAMES: &[&str] = &[
    "ssh_exec",
    "ssh_exec_sudo",
    "ssh_multi_exec",
    "ssh_terminal_input",
    "ssh_run_bg",
    "ssh_task_status",
    "ssh_close",
    "ssh_list_known_hosts",
    "ssh_list_connections",
    "ssh_remove_known_host",
    "sftp_list_dir",
    "sftp_stat",
    "sftp_upload",
    "sftp_download",
    "sftp_read_file",
    "sftp_write_file",
];

pub fn tool_definitions() -> Value {
    let mut tools = json!([
        {
            "name": "ssh_exec",
            "description": "Run a non-interactive remote shell command over SSH. Quick Sudo is NOT applied; use ssh_exec_sudo for privileged commands. Catastrophic patterns (disk formatting, recursive system deletes, shutdown, raw device writes, SQL DROP) require confirmDestructive: true; read-only connections only accept whitelisted inspection commands (ls, cat, df, ps, systemctl status, journalctl, docker ps, ...). Hosts may abandon the wait after ~15s while the command keeps running remotely; for anything slow use ssh_run_bg + ssh_task_status. After a timeout the command may STILL be running: verify before rerunning.",
            "inputSchema": {
                "type": "object",
                "properties": connection_properties(&[
                    ("command", "string", "Shell command to execute"),
                    ("timeoutSecs", "integer", "Plugin-side wait cap in seconds (5-300, default 60); the host may abandon the wait earlier (~15s) while the command keeps running"),
                    ("confirmDestructive", "boolean", "Set true to allow a command recognized as destructive after human review"),
                    ("runInTerminal", "boolean", "Run inside the user's visible DBX terminal so the command and its output are visible and interruptible (requires a saved connectionId). Set true when the task needs visibility, human oversight, or interactivity; elevated commands run only after the user approves them in the terminal. The connection-level terminal MCP mode (toggled in the DBX terminal toolbar) decides the default when this flag is omitted; off keeps the silent hidden channel"),
                ]),
                "required": ["command"],
                "anyOf": connection_selector_requirements(),
            },
        },
        {
            "name": "ssh_exec_sudo",
            "description": "Run a remote shell command with sudo. The sudo password is piped over stdin and 2FA/TOTP prompts are answered automatically when a TOTP secret is available (inline arguments, or a shared Quick Sudo profile referenced by quickSudoProfile; explicit arguments win). Catastrophic patterns require confirmDestructive: true; refused outright on read-only connections. Same wait-cap caveat as ssh_exec: keep long privileged jobs under ssh_run_bg.",
            "inputSchema": {
                "type": "object",
                "properties": connection_properties(&[
                    ("command", "string", "Shell command to execute with sudo"),
                    ("timeoutSecs", "integer", "Plugin-side wait cap in seconds (5-300, default 90); the host may abandon the wait earlier (~15s) while the command keeps running"),
                    ("quickSudoProfile", "string", "Global Quick Sudo profile id or exact name supplying sudo password/TOTP/prompt defaults"),
                    ("confirmDestructive", "boolean", "Set true to allow a command recognized as destructive after human review"),
                    ("runInTerminal", "boolean", "Run inside the user's visible DBX terminal so the command and its output are visible and interruptible (requires a saved connectionId). Set true when the task needs visibility, human oversight, or interactivity; elevated commands run only after the user approves them in the terminal. The connection-level terminal MCP mode (toggled in the DBX terminal toolbar) decides the default when this flag is omitted; off keeps the silent hidden channel"),
                ]),
                "required": ["command"],
                "anyOf": connection_selector_requirements(),
            },
        },
        {
            "name": "ssh_multi_exec",
            "description": "Run one command on several saved connections and collect per-target results (hidden channel; never a visible terminal). targets: 1-10 connection references (connectionId or connectionName), deduplicated in order. mode: parallel (default) or sequential; stopOnError (sequential only) stops at the first failure. confirmDestructive: true covers catastrophic commands on every target; sudo is NOT available here - use single-target ssh_exec_sudo for escalation. Per target: read-only connections only accept whitelisted inspection commands; the host may abandon the wait after ~15s while commands keep running - for anything slow use ssh_run_bg.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "targets": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": 10,
                                 "description": "Connection references (connectionId or connectionName); deduplicated preserving order" },
                    "command": { "type": "string", "description": "Shell command to run on every target" },
                    "mode": { "type": "string", "enum": ["parallel", "sequential"],
                              "description": "parallel (default) runs all targets concurrently; sequential walks them in order" },
                    "stopOnError": { "type": "boolean", "description": "Sequential mode only: stop before the next target once one fails (default false)" },
                    "timeoutSecs": { "type": "integer", "description": "Plugin-side wait cap per target (5-300)" },
                    "confirmDestructive": { "type": "boolean", "description": "Set true to allow a command recognized as catastrophic on all targets" },
                },
                "required": ["targets", "command"],
            },
        },
        {
            "name": "ssh_terminal_input",
            "description": "Inject raw input (interactive answers, Ctrl+C, escape sequences) into the connection's LIVE DBX terminal so the human can watch it happen; the terminal must already be open. Newlines fold to Enter (\\r), NUL is stripped, input capped at 8 KiB. appendNewline (default false) appends one Enter - set it to run a typed command, leave it off for prompt answers or control keys. Output is NOT collected; for captured output use ssh_exec with runInTerminal: true. Gates: read-only connections only accept control-only input; catastrophic lines require confirmDestructive: true; sudo lines must match the connection's sudo whitelist when one is configured.",
            "inputSchema": {
                "type": "object",
                "properties": connection_properties(&[
                    ("input", "string", "Raw terminal input to inject; newlines fold to Enter (\\r)"),
                    ("appendNewline", "boolean", "Append one Enter after the input (default false; interactive answers usually carry their own \\r)"),
                    ("confirmDestructive", "boolean", "Set true to allow a line recognized as a catastrophic pattern"),
                ]),
                "required": ["input"],
                "anyOf": connection_selector_requirements(),
            },
        },
        {
            "name": "ssh_run_bg",
            "description": "Start a long-running command detached on the remote host (nohup; output appended to ~/.dbx-ssh-tasks/<taskId>.log) and return immediately with taskId/pid/logPath. Survives disconnects, host wait caps, and session restarts because the output lives on the server. Poll with ssh_task_status(logPath). Same safety gates as ssh_exec: destructive patterns need confirmDestructive: true, read-only connections refuse it.",
            "inputSchema": {
                "type": "object",
                "properties": connection_properties(&[
                    ("command", "string", "Shell command to run detached"),
                    ("confirmDestructive", "boolean", "Set true to allow a command recognized as destructive after human review"),
                ]),
                "required": ["command"],
                "anyOf": connection_selector_requirements(),
            },
        },
        {
            "name": "ssh_task_status",
            "description": "Poll a background task started with ssh_run_bg: returns state (running/done/missing), exit code once finished, pid liveness, and the trailing bytes of the server-side log file. Reconnects transparently, so it works after disconnects or from a later session.",
            "inputSchema": {
                "type": "object",
                "properties": connection_properties(&[
                    ("logPath", "string", "logPath returned by ssh_run_bg"),
                    ("tailBytes", "integer", "Trailing bytes of output to return (200-16000, default 4000)"),
                ]),
                "required": ["logPath"],
                "anyOf": connection_selector_requirements(),
            },
        },
        {
            "name": "ssh_close",
            "description": "Close the cached SSH/SFTP connection for a host after finishing work.",
            "inputSchema": {
                "type": "object",
                "properties": connection_properties(&[]),
                "anyOf": connection_selector_requirements(),
            },
        },
        {
            "name": "ssh_list_known_hosts",
            "description": "List entries of the plugin's known_hosts store (the system ~/.ssh/known_hosts is never modified).",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "ssh_list_connections",
            "description": "List the DBX app's saved SSH connections for this plugin (id, name, host, port, username, authentication method, read-only flag) merged with connections registered in this MCP session. Metadata only: credentials are never included. Use an entry's id as connectionId, its name as connectionName, or its host+port+username as an endpoint reference; a unique endpoint match reuses the saved connection without inline credentials.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": "ssh_remove_known_host",
            "description": "Remove entries for host:port from the plugin's known_hosts store; use after a legitimate server reinstall.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "host": { "type": "string", "description": "Host name as recorded" },
                    "port": { "type": "integer", "description": "Port (default 22)" },
                },
                "required": ["host"],
            },
        },
        {
            "name": "sftp_list_dir",
            "description": "List a remote directory over SFTP. Names follow the connection's file-name encoding: on latin-1 connections entries are decoded from raw server bytes to display form, and a returned path passed back to other SFTP tools addresses the same server bytes.",
            "inputSchema": { "type": "object", "properties": connection_properties(&[("path", "string", "Remote directory path")]), "required": ["path"], "anyOf": connection_selector_requirements() },
        },
        {
            "name": "sftp_stat",
            "description": "Inspect a remote path over SFTP (size, permissions, timestamps).",
            "inputSchema": { "type": "object", "properties": connection_properties(&[("path", "string", "Remote path")]), "required": ["path"], "anyOf": connection_selector_requirements() },
        },
        {
            "name": "sftp_upload",
            "description": "Upload a local file to the remote server over SFTP (single file, no recursion). The local file must be readable by the MCP server process; size is capped by maxUploadBytes.",
            "inputSchema": { "type": "object", "properties": connection_properties(&[
                ("localPath", "string", "Local file path to upload"),
                ("remotePath", "string", "Remote file path to create"),
                ("overwrite", "boolean", "Set true to replace an existing remote file"),
            ]), "required": ["localPath", "remotePath"] },
        },
        {
            "name": "sftp_download",
            "description": "Download a remote file to a local path over SFTP (single file). Remote size is capped by maxDownloadBytes; missing local parent directories are created. Shell/cron/systemd bootstrap paths are refused as local targets; on read-only connections credential paths are refused as remote sources.",
            "inputSchema": { "type": "object", "properties": connection_properties(&[
                ("remotePath", "string", "Remote file path to download"),
                ("localPath", "string", "Local target file path"),
                ("overwrite", "boolean", "Set true to replace an existing local file"),
            ]), "required": ["remotePath", "localPath"] },
        },
        {
            "name": "sftp_read_file",
            "description": "Read a remote file (text by default, or base64). Truncates at maxBytes; default and ceiling come from the plugin's MCP size settings. Pass offset to resume from a byte position (empty result at/after EOF).",
            "inputSchema": { "type": "object", "properties": connection_properties(&[
                ("path", "string", "Remote file path"),
                ("maxBytes", "integer", "Maximum bytes to read; clamped to maxDownloadBytes (base64 field ignored)"),
                ("offset", "integer", "Byte offset to start reading from (default 0)"),
                ("base64", "boolean", "Set true to return base64 instead of UTF-8 text"),
            ]), "required": ["path"], "anyOf": connection_selector_requirements() },
        },
        {
            "name": "sftp_write_file",
            "description": "Write a remote file with UTF-8 content. Existing files require overwrite=true; content is capped by maxUploadBytes.",
            "inputSchema": { "type": "object", "properties": connection_properties(&[
                ("path", "string", "Remote file path"),
                ("content", "string", "File content (UTF-8 text)"),
                ("overwrite", "boolean", "Set true to replace an existing file"),
            ]), "required": ["path", "content"] },
        },
    ]);
    // Connection-bound tools share the selector alternatives; add them here
    // after the compact JSON declarations so path/command required fields
    // stay independent from connection addressing. Every tool additionally
    // gets its advisory annotations (see `tool_annotations`).
    if let Some(tool_list) = tools.as_array_mut() {
        for tool in tool_list {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(schema) = tool.get_mut("inputSchema").and_then(Value::as_object_mut) {
                schema.insert("anyOf".to_string(), connection_selector_requirements());
            }
            tool["annotations"] = tool_annotations(&name);
        }
    }
    tools
}
