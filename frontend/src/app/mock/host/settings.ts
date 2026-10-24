/**
 * 设置域夹具：ssh/settings/get|set、快捷命令、关键词高亮规则、审计列表、
 * MCP 设置。状态与校验语义镜像 sidecar 的持久化形状（上限/默认规则/脱敏回显）。
 */

import { UNHANDLED, type Unhandled } from "./shared";

const QUICK_COMMANDS_LIMIT = 20;
const quickCommandsState: { id: string; name: string; command: string; createdAt: number; updatedAt: number }[] = [];
const settingsState = { quickSudo: true, sudoUsePty: false, sudoPasswordSet: true, totpConfigured: false, authFlowMode: "password_then_otp", passwordPromptHint: "", totpPromptHint: "", agentTerminalMode: "off", rememberedCommands: [] as string[] };

// 关键词高亮规则（ssh/highlightRules/*）mock 状态：镜像 sidecar 存储
// （highlight-rules.json，0600）的响应形状/上限/默认值语义，并镜像后端
// 首次初始化播种的 22 条默认规则（highlight_rules::DEFAULT_RULE_SPECS：
// 严重度分色，长短语精确、词干兜底），防可视化夹具脱节。
const HIGHLIGHT_RULES_LIMIT = 30;
const HIGHLIGHT_PATTERN_MAX = 200;
const HIGHLIGHT_COLOR_RE = /^#[0-9a-fA-F]{6}$/;
const HIGHLIGHT_DEFAULT_SEEDS: Array<{ id: string; pattern: string; color: string; caseSensitive?: boolean; isRegex?: boolean }> = [
  { id: "default-error", pattern: "ERROR", color: "#ef4444" },
  { id: "default-fatal", pattern: "FATAL", color: "#ef4444" },
  { id: "default-permission-denied", pattern: "Permission denied", color: "#ef4444" },
  { id: "default-no-such-file", pattern: "No such file or directory", color: "#ef4444" },
  { id: "default-command-not-found", pattern: "command not found", color: "#ef4444" },
  { id: "default-connection-refused", pattern: "Connection refused", color: "#ef4444" },
  { id: "default-no-space-left", pattern: "No space left on device", color: "#ef4444" },
  { id: "default-failed-to", pattern: "Failed to", color: "#ef4444" },
  { id: "default-cannot", pattern: "cannot", color: "#ef4444" },
  { id: "default-exception", pattern: "Exception", color: "#ef4444" },
  { id: "default-traceback", pattern: "Traceback (most recent call last)", color: "#ef4444" },
  { id: "default-fail", pattern: "FAIL", color: "#f59e0b" },
  { id: "default-denied", pattern: "denied", color: "#f59e0b" },
  { id: "default-timed-out", pattern: "timed out", color: "#f59e0b" },
  { id: "default-warn", pattern: "WARN", color: "#facc15" },
  { id: "default-deprecated", pattern: "deprecated", color: "#facc15" },
  { id: "default-success", pattern: "SUCCESS", color: "#22c55e" },
  { id: "default-active-running", pattern: "active (running)", color: "#22c55e" },
  { id: "default-done", pattern: "done", color: "#22c55e" },
  { id: "default-check", pattern: "✓", color: "#22c55e" },
  { id: "default-passed", pattern: "PASSED", color: "#22c55e", caseSensitive: true },
  { id: "default-ipv4", pattern: "\\b\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\b", color: "#3b82f6", isRegex: true },
];
let highlightRuleSeq = 0;
interface MockHighlightRule { id: string; pattern: string; isRegex: boolean; color: string; caseSensitive: boolean; enabled: boolean; createdAt: number; updatedAt: number }
const highlightRulesState: MockHighlightRule[] = HIGHLIGHT_DEFAULT_SEEDS.map(({ id, pattern, color, caseSensitive = false, isRegex = false }) => ({
  id, pattern, isRegex, color, caseSensitive, enabled: true, createdAt: 1786262400, updatedAt: 1786262400,
}));
const highlightRuleViews = () => [...highlightRulesState].sort((a, b) => a.createdAt - b.createdAt);

// MCP 设置（mcp/settings/get|set）新字段（IMPL_PLAN_NETCATTY_PARITY §1.3）：
// 权限档与连接作用域，镜像持久化 + 校验语义。
const mcpSettingsState = { execPermissionMode: "autonomous", connectionScope: [] as string[] };

// 镜像并行批次 ssh/audit/list 的真实形状（AuditEntry：tsMs/tool/connectionId/
// gate/approval/outcome/exitCode/durationMs/mode/command/output/error，
// 0.4.77 起带 command/output 尾部）；末条保留计划 §1.1 旧形状（ts 秒 + kind +
// command）验证前端多形状容忍。
const AUDIT_FIXTURE: Array<Record<string, unknown>> = [
  { tsMs: 1786262400000, tool: "ssh_exec", connectionId: "Production SSH", gate: "pass", approval: "none", outcome: "ok", exitCode: 0, durationMs: 812, mode: "stdio", command: "df -h /data", output: "Filesystem Size Used Avail Capacity Mounted on\n/dev/vda1 98G 41G 52G 45% /data" },
  { tsMs: 1786262460000, tool: "ssh_exec_sudo", connectionId: "Production SSH", gate: "pass", approval: "prompt", outcome: "ok", exitCode: 0, durationMs: 2310, mode: "terminal", command: "systemctl restart nginx" },
  { tsMs: 1786262520000, tool: "ssh_exec", connectionId: "Production SSH", gate: "destructive-unconfirmed", approval: "none", outcome: "error", error: "destructive command requires confirmDestructive: true", durationMs: 3, mode: "stdio", command: "rm -rf /var/log/old" },
  { tsMs: 1786262580000, tool: "ssh_terminal_input", connectionId: "Production SSH", gate: "write-denied", approval: "none", outcome: "error", durationMs: 1, mode: "embedded" },
  { ts: 1786262640, kind: "agent.challenge", sessionId: "visual-session", command: "rm -rf /tmp/scratch", decision: "approved", risk: "elevated" },
];
let auditEntriesState: Array<Record<string, unknown>> = AUDIT_FIXTURE.map((entry) => ({ ...entry }));

export async function handleSettingsInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "ssh/settings/get") {
    const base = { quickSudo: true, sudoUsePty: false, sudoPasswordSet: true, totpConfigured: false, authFlowMode: "password_then_otp", passwordPromptHint: "", totpPromptHint: "", agentTerminalMode: settingsState.agentTerminalMode, rememberedCommands: [...settingsState.rememberedCommands] };
    // revealSecrets: true 时镜像真实桥的回显形状（mock 不存原值，回空串）。
    return (params as Record<string, unknown>).revealSecrets === true
      ? { ...base, sudoPassword: "", totpSecret: "" }
      : base;
  }
  if (method === "ssh/settings/set") {
    const input = params as Record<string, unknown>;
    settingsState.quickSudo = typeof input.quickSudo === "boolean" ? input.quickSudo : settingsState.quickSudo;
    settingsState.authFlowMode = typeof input.authFlowMode === "string" ? input.authFlowMode : settingsState.authFlowMode;
    settingsState.passwordPromptHint = typeof input.passwordPromptHint === "string" ? input.passwordPromptHint : settingsState.passwordPromptHint;
    settingsState.totpPromptHint = typeof input.totpPromptHint === "string" ? input.totpPromptHint : settingsState.totpPromptHint;
    settingsState.sudoPasswordSet = typeof input.sudoPassword === "string" ? input.sudoPassword.length > 0 : settingsState.sudoPasswordSet;
    settingsState.totpConfigured = typeof input.totpSecret === "string" ? input.totpSecret.trim().length > 0 : settingsState.totpConfigured;
    if (typeof input.agentTerminalMode === "string") settingsState.agentTerminalMode = input.agentTerminalMode;
    if (Array.isArray(input.rememberedCommands)) {
      settingsState.rememberedCommands = (input.rememberedCommands as unknown[])
        .map((line) => String(line).trim()).filter((line) => line.length > 0).slice(0, 50);
    }
    return { ...settingsState };
  }
  if (method === "ssh/quickCommands/list") return { commands: quickCommandsState };
  if (method === "ssh/quickCommands/save") {
    const input = params as Record<string, unknown>;
    const command = String(input.command || "").trim();
    if (!command) throw new Error("Missing command");
    if (command.length > 500) throw new Error("Quick command is limited to 500 characters");
    const name = (String(input.name || "").trim() || command.slice(0, 60)).slice(0, 60);
    const id = String(input.id || "").trim();
    const now = Math.floor(Date.now() / 1000);
    const existing = quickCommandsState.findIndex((entry) => entry.id === id);
    if (existing >= 0) {
      quickCommandsState[existing] = { ...quickCommandsState[existing], name, command, updatedAt: now };
      return { quickCommand: quickCommandsState[existing], created: false, commands: [...quickCommandsState] };
    }
    if (quickCommandsState.length >= QUICK_COMMANDS_LIMIT) throw new Error(`At most ${QUICK_COMMANDS_LIMIT} quick commands are supported`);
    const entry = { id: `mock-qc-${quickCommandsState.length + 1}-${now}`, name, command, createdAt: now, updatedAt: now };
    quickCommandsState.push(entry);
    return { quickCommand: entry, created: true, commands: [...quickCommandsState] };
  }
  if (method === "ssh/quickCommands/delete") {
    const id = String((params as Record<string, unknown>)?.id || "");
    const index = quickCommandsState.findIndex((entry) => entry.id === id);
    if (index >= 0) quickCommandsState.splice(index, 1);
    return { removed: index >= 0, commands: [...quickCommandsState] };
  }
  if (method === "ssh/highlightRules/list") return { rules: highlightRuleViews() };
  if (method === "ssh/highlightRules/save") {
    const input = params as Record<string, unknown>;
    const pattern = String(input.pattern || "").trim();
    if (!pattern) throw new Error("Missing pattern");
    if (pattern.length > HIGHLIGHT_PATTERN_MAX) throw new Error(`Pattern is limited to ${HIGHLIGHT_PATTERN_MAX} characters`);
    const color = typeof input.color === "string" && HIGHLIGHT_COLOR_RE.test(input.color) ? input.color : "#f59e0b";
    const id = String(input.id || "").trim();
    const now = Math.floor(Date.now() / 1000);
    const existing = highlightRulesState.findIndex((entry) => entry.id === id);
    if (existing >= 0) {
      highlightRulesState[existing] = {
        ...highlightRulesState[existing],
        pattern,
        isRegex: input.isRegex === true,
        color,
        caseSensitive: input.caseSensitive === true,
        enabled: input.enabled !== false,
        updatedAt: now,
      };
      return { rule: highlightRulesState[existing], created: false, rules: highlightRuleViews() };
    }
    if (highlightRulesState.length >= HIGHLIGHT_RULES_LIMIT) throw new Error(`At most ${HIGHLIGHT_RULES_LIMIT} highlight rules are supported`);
    const entry: MockHighlightRule = {
      id: `mock-hr-${++highlightRuleSeq}-${now}`,
      pattern,
      isRegex: input.isRegex === true,
      color,
      caseSensitive: input.caseSensitive === true,
      enabled: input.enabled !== false,
      createdAt: now,
      updatedAt: now,
    };
    highlightRulesState.push(entry);
    return { rule: entry, created: true, rules: highlightRuleViews() };
  }
  if (method === "ssh/highlightRules/delete") {
    const id = String((params as Record<string, unknown>)?.id || "");
    const index = highlightRulesState.findIndex((entry) => entry.id === id);
    if (index >= 0) highlightRulesState.splice(index, 1);
    // 未知 id 不报错（幂等），镜像真实 sidecar 的 removed:false 语义。
    return { removed: index >= 0, rules: highlightRuleViews() };
  }
  if (method === "ssh/audit/list") {
    // 镜像并行批次契约：limit ∈ [1,500] 缺省 100；行序保持文件序（旧→新）取
    // 最新 limit 条；`kind` 参数后端不识别（过滤由前端客户端做）。
    const input = (params || {}) as Record<string, unknown>;
    const limitRaw = Number(input.limit);
    const limit = Number.isFinite(limitRaw) && limitRaw > 0 ? Math.min(500, Math.floor(limitRaw)) : 100;
    const truncated = auditEntriesState.length > limit;
    return { entries: auditEntriesState.slice(-limit), truncated };
  }
  if (method === "ssh/audit/clear") {
    auditEntriesState = [];
    return { cleared: true };
  }
  if (method === "mcp/settings/get") return { maxReadBytes: 8 * 1024 * 1024, maxUploadBytes: 64 * 1024 * 1024, maxDownloadBytes: 256 * 1024 * 1024, execPermissionMode: mcpSettingsState.execPermissionMode, connectionScope: [...mcpSettingsState.connectionScope] };
  if (method === "mcp/settings/set") {
    const input = (params || {}) as Record<string, unknown>;
    if (typeof input.execPermissionMode === "string") {
      if (input.execPermissionMode !== "autonomous" && input.execPermissionMode !== "confirm") throw new Error("Invalid execPermissionMode: expected autonomous or confirm");
      mcpSettingsState.execPermissionMode = input.execPermissionMode;
    }
    if (Array.isArray(input.connectionScope)) {
      mcpSettingsState.connectionScope = input.connectionScope
        .map((entry) => String(entry).trim())
        .filter((entry) => entry.length > 0)
        .slice(0, 20);
    }
    return { maxReadBytes: 8 * 1024 * 1024, maxUploadBytes: 64 * 1024 * 1024, maxDownloadBytes: 256 * 1024 * 1024, execPermissionMode: mcpSettingsState.execPermissionMode, connectionScope: [...mcpSettingsState.connectionScope] };
  }
  return UNHANDLED;
}
