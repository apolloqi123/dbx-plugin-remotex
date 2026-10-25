/**
 * 本地终端与 local/* 域夹具：local/terminal/*、local/session/*、shell 清单、
 * local/capabilities|saveFile|editors|open-with|metrics、偏好与目录选择器。
 *
 * 本地终端夹具覆盖按钮 → 633/133 标记 → 回显全链路；状态（sessionId/序号/
 * 退出态）与调试入口 __dbxMockExitLocal 共享同一模块级状态。
 */

import { UNHANDLED, type Unhandled, emitHostBinary, emitHostEvent, terminalFrame } from "./shared";
import { nextMetricsRefreshTick } from "./metrics";

// ---- 本地终端夹具：local/terminal/* 全链路（按钮 → 633/133 标记 → 回显）----
let localSessionId = "";
let localSequence = 0;
let localExited = false;

function emitLocalTerminal(text: string) {
  if (!localSessionId) return;
  localSequence += 1;
  const event = { channel: `local/terminal/out/${localSessionId}`, data: terminalFrame(localSequence, text) };
  emitHostBinary(event);
}

function localPromptMarks(cwd: string) {
  // 与真实 shell integration 同形状：提示符只带 prompt-start（633;A/133;A）
  // + cwd；C/D 标记在回显用户命令时成对出现（见 sendBinary 的本地分支）。
  return `\u001b]633;A\u0007\u001b]133;A\u0007\u001b]633;P;Cwd=${cwd}\u0007jinpy@MacBook-Pro ~ % `;
}

function localCommandMarks(command: string) {
  // 与真实 integration 脚本一致：E（命令行）→ C（开始执行），parser 以 E 为
  // "命令开始"信号。
  return `\u001b]633;E;${command}\u0007\u001b]633;C\u0007\u001b]133;C\u0007`;
}

function localDoneMarks(code: number) {
  return `\u001b]633;D;${code}\u0007\u001b]133;D;${code}\u0007`;
}

/** local/terminal/in 通道载荷：inputAck → 回显周期（E/C → echo → D → prompt）。 */
export function handleLocalTerminalInput(channel: string, bytes: Uint8Array): void {
  const inputSequence = Number(new DataView(bytes.buffer, bytes.byteOffset, 8).getBigUint64(0, false));
  emitHostEvent({ method: "local/terminal/inputAck", params: { sessionId: channel.slice("local/terminal/in/".length), sequence: inputSequence } });
  const typed = new TextDecoder().decode(bytes.slice(8));
  setTimeout(() => emitLocalTerminal(`${typed}\r\n${localCommandMarks(typed.trim())}dbx-mock echo: ${typed.trim()}\r\n${localDoneMarks(0)}${localPromptMarks("/Users/demo")}`), 12);
}

/** mock 调试入口 __dbxMockExitLocal：把本地终端打入退出态。 */
export function exitLocalSession(): void {
  if (!localSessionId) return;
  const sessionId = localSessionId;
  localExited = true;
  emitHostEvent({ method: "local/session/state", params: { sessionId, state: "exited", exitCode: 127 } });
}

// ---- 原生拖拽指针探针（local/pointer/sample）----
// 默认与旧宿主一致：没有光标样本，前端不做坐标反解、按原始宿主点命中。
// 夹具用 __dbxMockPointerSample 注入真实光标 + 窗口几何，走查宿主把坐标
// 多除一次缩放比例时的反解路径（见 features/sftp/lib/hostDragMapping.ts）。
export interface MockPointerSampleState {
  platform: string;
  cursor: { x: number; y: number } | null;
  window: { x: number; y: number; width: number; height: number } | null;
}

const EMPTY_POINTER_SAMPLE: MockPointerSampleState = { platform: "mock", cursor: null, window: null };
let pointerSampleState: MockPointerSampleState = EMPTY_POINTER_SAMPLE;

export function setMockPointerSample(state: MockPointerSampleState | null): void {
  pointerSampleState = state ?? EMPTY_POINTER_SAMPLE;
}

// 插件级 UI 偏好（local/preferences/get|set）：镜像 sidecar preferences.json 的合并语义。
const localPrefsState = {
  downloadDir: "",
  downloadUseDefaultDir: true,
  downloadConflictPolicy: "rename",
  localShell: "",
  localShellIntegration: true,
  // 本地 Shell 启动目录/环境变量（镜像 sidecar preferences.json 白名单键；
  // env 存净化后的 [{name, value}]，与 sidecar sanitize_env_pair 边界一致）。
  localShellCwd: "",
  localShellEnv: [] as Array<{ name: string; value: string }>,
  auto_record: false,
  // M14-B 三键镜像（缺省与 sidecar 一致：深度 3 / 兼容关 / 编码 auto）。
  transfer_max_active: 3,
  sftp_compat_mode: false,
  sftp_name_encoding: "auto",
  // M16 连接级覆盖桶（值域 auto/latin-1，与 sidecar 白名单一致；空=全部跟随全局）。
  sftp_name_encoding_overrides: {} as Record<string, string>,
};

/** local/* 域方法。 */
export async function handleLocalInvoke(method: string, params: unknown): Promise<unknown | Unhandled> {
  if (method === "local/terminal/start") {
    localSessionId = `local-session-${Math.random().toString(36).slice(2, 8)}`;
    localSequence = 0;
    localExited = false;
    setTimeout(() => emitLocalTerminal(localPromptMarks("/Users/demo")), 40);
    return {
      sessionId: localSessionId,
      // 真实 sidecar 回显实际 spawn 的 shell；夹具同样回显请求参数。
      shell: (params as { shell?: string } | undefined)?.shell || "/bin/zsh",
      shellIntegration: (params as { shellIntegration?: boolean } | undefined)?.shellIntegration !== false,
    };
  }
  if (method === "local/terminal/resize") return { success: true };
  if (method === "local/terminal/replay") {
    return { frameCount: 0, firstAvailableSequence: localSequence + 1, tailSequence: localSequence, complete: true };
  }
  if (method === "local/session/close") {
    localSessionId = "";
    return { success: true };
  }
  if (method === "local/session/list") {
    return {
      sessions: localSessionId && !localExited
        ? [{ sessionId: localSessionId, workbenchId: "mock-workbench", shell: "/bin/zsh", createdAt: 0 }]
        : [],
    };
  }
  if (method === "local/shells/list") {
    return {
      platform: "macos",
      shells: [
        { program: "/opt/homebrew/bin/fish", name: "Fish", isDefault: false, isUserShell: true },
        { program: "/bin/zsh", name: "Zsh", isDefault: true, isUserShell: false },
        { program: "/bin/bash", name: "Bash", isDefault: false, isUserShell: false },
        { program: "/bin/csh", name: "Csh", isDefault: false, isUserShell: false, injectable: false },
      ],
    };
  }
  if (method === "local/capabilities") return { canSaveLocal: false, downloadsDir: "" };
  if (method === "local/pointer/sample") return { ...pointerSampleState };
  if (method === "local/saveFile") {
    // 通用本机落盘（录制 GIF/transcript、trzsz/zmodem 导出）：镜像 sidecar 的
    // 必填校验与 {localPath,name} 返回形状；夹具不写真实磁盘，只合成下载路径。
    const input = params as Record<string, unknown>;
    const name = String(input.name || "").trim();
    if (!name) throw new Error("Missing name");
    if (!String(input.dataBase64 || "")) throw new Error("Missing dataBase64");
    const dir = String(input.targetDir || "").trim() || localPrefsState.downloadDir.trim() || "/Users/demo/Downloads";
    const safeName = name.split(/[\\/]/).pop() || "download.bin";
    return { localPath: `${dir.replace(/[\\/]+$/, "")}/${safeName}`, name: safeName };
  }
  if (method === "local/editors/list") {
    // 外部编辑器目录（编辑器映射增强）：mock 给一条可用的 VS Code 让
    // 「打开方式」子菜单在无 sidecar 的开发环境也有可见选项。
    return {
      platform: "macos",
      editors: [
        { id: "vscode", name: "Visual Studio Code", available: false, launch: { kind: "openApp", appId: "Visual Studio Code" }, suggestedExtensions: [".txt", ".md", ".json"] },
        { id: "textedit", name: "TextEdit", available: true, launch: { kind: "openApp", appId: "TextEdit" }, suggestedExtensions: [".txt"] },
      ],
    };
  }
  if (method === "local/open-with") return { success: true };
  if (method === "local/metrics") {
    // 本地指标（dock 本地终端系统信息带）：与 ssh/metrics 同形状，但反映
    // sidecar 宿主机本身——无 gpu/npu（SSH-only 语义）、无 osId/osPretty
    // （本地宿主无 os-release，optional 降级缺省）。
    const metricsRefreshTick = nextMetricsRefreshTick();
    const localWave = (base: number, amplitude: number) => Math.max(0, Math.round(base + amplitude * Math.sin(metricsRefreshTick / 2)));
    return {
      hostname: "macbook-local",
      kernel: "darwin 25.6.0",
      uptimeSeconds: 86_400 * 3 + 7_200,
      cpu: { cores: 10, percent: localWave(18, 12), load1: 1.8, load5: 2.1, load15: 2.0 },
      memory: { totalBytes: 17_179_869_184, availableBytes: 8_589_934_592, usedBytes: 8_589_934_592 },
      network: [
        { name: "en0", rxRate: localWave(90_000, 60_000), txRate: localWave(30_000, 20_000), rxTotal: 45_678_901_234, txTotal: 12_345_678_901 },
        { name: "lo0", rxRate: localWave(800, 500), txRate: localWave(800, 500), rxTotal: 9_876_543, txTotal: 9_876_543 },
      ],
    };
  }
  if (method === "local/preferences/get") return { ...localPrefsState };
  if (method === "local/fs/drives") return { drives: ["C:\\", "D:\\"] };
  if (method === "local/fs/exists") return { exists: false, path: "" };
  if (method === "local/fs/browse") {
    // 目录选择器的假目录树：覆盖盘符页 → Downloads 的完整导航路径。
    const input = params as Record<string, unknown>;
    const p = typeof input.path === "string" && input.path.trim() ? input.path.trim() : "C:\\Users\\demo\\Downloads";
    const trees: Record<string, { parent: string | null; dirs: string[] }> = {
      "C:\\": { parent: null, dirs: ["Users", "Windows", "Temp"] },
      "D:\\": { parent: null, dirs: ["Downloads", "Work"] },
      "C:\\Users": { parent: "C:\\", dirs: ["demo"] },
      "C:\\Users\\demo": { parent: "C:\\Users", dirs: ["Downloads", "Documents"] },
      "C:\\Users\\demo\\Downloads": { parent: "C:\\Users\\demo", dirs: [] },
      "C:\\Users\\demo\\Documents": { parent: "C:\\Users\\demo", dirs: [] },
    };
    const node = trees[p];
    if (!node) throw new Error(`Cannot open '${p}': no such directory (mock)`);
    return { path: p, parent: node.parent, entries: node.dirs.map((name) => ({ name, path: `${p.replace(/[\\/]+$/, "")}\\${name}`, is_dir: true })) };
  }
  if (method === "local/preferences/set") {
    const input = params as Record<string, unknown>;
    if (typeof input.downloadDir === "string") localPrefsState.downloadDir = input.downloadDir.trim();
    if (typeof input.downloadUseDefaultDir === "boolean") localPrefsState.downloadUseDefaultDir = input.downloadUseDefaultDir;
    if (typeof input.downloadConflictPolicy === "string" && ["rename", "ask", "overwrite"].includes(input.downloadConflictPolicy)) localPrefsState.downloadConflictPolicy = input.downloadConflictPolicy;
    if (typeof input.auto_record === "boolean") localPrefsState.auto_record = input.auto_record;
    // 本地 Shell 偏好：cwd 字符串裁剪存储（空串=清除）；env 数组镜像 sidecar
    // 净化——空名/含 =/托管键/超限条目静默丢弃，≤32 对。
    if (typeof input.localShellCwd === "string") localPrefsState.localShellCwd = input.localShellCwd.trim();
    if (Array.isArray(input.localShellEnv)) {
      const managed = ["TERM", "COLORTERM", "TERM_PROGRAM"];
      const store: Array<{ name: string; value: string }> = [];
      for (const entry of input.localShellEnv) {
        if (store.length >= 32) break;
        if (typeof entry !== "object" || entry === null) continue;
        const record = entry as Record<string, unknown>;
        if (typeof record.name !== "string") continue;
        // 缺 value 视为空值保留（镜像 sidecar unwrap_or("")）。
        const value = typeof record.value === "string" ? record.value : "";
        const name = record.name.trim();
        if (!name || name.includes("=") || name.length > 128 || value.length > 4096) continue;
        if (managed.some((key) => key.toLowerCase() === name.toLowerCase())) continue;
        store.push({ name, value });
      }
      localPrefsState.localShellEnv = store;
    }
    if (input.transfer_max_active !== undefined) localPrefsState.transfer_max_active = Math.min(8, Math.max(1, Math.floor(Number(input.transfer_max_active) || 3)));
    if (typeof input.sftp_compat_mode === "boolean") localPrefsState.sftp_compat_mode = input.sftp_compat_mode;
    if (input.sftp_name_encoding === "auto" || input.sftp_name_encoding === "latin-1") localPrefsState.sftp_name_encoding = input.sftp_name_encoding;
    // M16 连接级覆盖：整表替换，镜像 sidecar 清洗（非对象忽略、非法值丢弃）。
    if (input.sftp_name_encoding_overrides && typeof input.sftp_name_encoding_overrides === "object" && !Array.isArray(input.sftp_name_encoding_overrides)) {
      const store: Record<string, string> = {};
      for (const [id, value] of Object.entries(input.sftp_name_encoding_overrides as Record<string, unknown>)) {
        if (id.trim().length === 0) continue;
        if (value === "auto" || value === "latin-1") store[id] = value;
      }
      localPrefsState.sftp_name_encoding_overrides = store;
    }
    return { ...localPrefsState };
  }
  return UNHANDLED;
}
