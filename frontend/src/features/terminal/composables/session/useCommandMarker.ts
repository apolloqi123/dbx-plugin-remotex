import { computed, reactive, ref, type Ref } from "vue";
import type { PromptNewlineTransformer } from "../../lib/emulator/terminalPromptNewline";
import {
  commandMarkerTooltip,
  runningCommandElapsedMs,
  type Osc633CommandParser,
  type Osc633StreamUpdates,
} from "../../lib/panels/terminalCommandMarkers";

export interface CommandMarkerOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  // OSC 633 shell-integration 解析链（会话级单例，随 reset 复位）
  commandMarkerParser: Osc633CommandParser;
  promptNewline: PromptNewlineTransformer;
  aiSearch: { reset(): void };
  setUserVarCwdEvent: (event: { path: string; at: number } | null) => void;
  resetSuggestionsForSession: () => void;
  closeSuggestions: () => void;
  closeHistoryPanel: () => void;
  resetGhostSuggestion: () => void;
  resetPromptHintsQuiet: () => void;
  resetAiFixForSession: () => void;
  beginAiCommand: () => void;
  lastTerminalCommand: Ref<string | null>;
  localRecentCommands: Ref<string[]>;
  terminalCwd: Ref<string>;
  localLastCwd: Ref<string>;
  isLocalMode: Readonly<Ref<boolean>>;
  pushTerminalCommandHistory: (command: string) => void;
  recordCommandExecutionMeta: (command: string, patch: { exitCode?: number | null; durationMs?: number | null }) => void;
}

/**
 * 命令标记条（OSC 633 Shell Integration）：E/C/D/A 帧驱动的运行/完成状态、
 * 1s 进度计时与富元数据归档。状态本体随会话切换由 resetCommandMarker 复位，
 * 帧消费入口是 applyCommandMarker（writeTerminalOutput 同 chunk 调用）。
 */
export function useCommandMarker(options: CommandMarkerOptions) {
  const {
    t,
    commandMarkerParser,
    promptNewline,
    aiSearch,
    setUserVarCwdEvent,
    resetSuggestionsForSession,
    closeSuggestions,
    closeHistoryPanel,
    resetGhostSuggestion,
    resetPromptHintsQuiet,
    resetAiFixForSession,
    beginAiCommand,
    lastTerminalCommand,
    localRecentCommands,
    terminalCwd,
    localLastCwd,
    isLocalMode,
    pushTerminalCommandHistory,
    recordCommandExecutionMeta,
  } = options;

const commandMarker = reactive({
  installed: false,
  active: false,
  command: "",
  exitCode: null as number | null,
  durationMs: null as number | null,
  cwd: "",
  // Start timestamp of the currently running command; drives the 1s tick that
  // keeps the marker strip duration live while a command is in flight.
  startedAt: null as number | null,
});
// Live elapsed milliseconds for the running marker (null when idle or finished).
const commandMarkerElapsed = ref<number | null>(null);
let commandMarkerTimer = 0;
// Hover tooltip for the terminal command marker strip: full command, exit
// code, duration and working directory (localized, multi-line).
const commandMarkerDetails = computed(() => commandMarkerTooltip(
  {
    command: commandMarker.command,
    exitCode: commandMarker.exitCode,
    durationMs: commandMarker.durationMs,
    elapsedMs: commandMarkerElapsed.value,
    cwd: commandMarker.cwd,
  },
  {
    command: t("terminalCommand.tooltipCommand"),
    exitCode: t("terminalCommand.tooltipExitCode"),
    duration: t("terminalCommand.tooltipDuration"),
    directory: t("terminalCommand.tooltipDirectory"),
  },
));

function stopCommandMarkerTick() {
  if (commandMarkerTimer) {
    window.clearInterval(commandMarkerTimer);
    commandMarkerTimer = 0;
  }
  commandMarkerElapsed.value = null;
}

function startCommandMarkerTick(startedAt: number) {
  stopCommandMarkerTick();
  commandMarkerTimer = window.setInterval(() => {
    commandMarkerElapsed.value = runningCommandElapsedMs(startedAt, Date.now());
  }, 1000);
  commandMarkerElapsed.value = runningCommandElapsedMs(startedAt, Date.now());
}

function resetCommandMarker() {
  commandMarkerParser.reset();
  // 提示符换行状态同属会话流：不把上一会话的行首游标与不完整序列带进新流。
  promptNewline.reset();
  stopCommandMarkerTick();
  // 会话切换/断开连带复位 SetUserVar cwd 优先通道：不把上一会话的目录裁决
  // 带进新会话（OSC 7 回落立即恢复权威）。
  setUserVarCwdEvent(null);
  commandMarker.installed = false;
  commandMarker.active = false;
  commandMarker.command = "";
  commandMarker.exitCode = null;
  commandMarker.durationMs = null;
  commandMarker.cwd = "";
  commandMarker.startedAt = null;
  // 会话切换/断开：建议浮层与抑制门锁存一并复位（P1-1）；ghost 门同步复位。
  resetSuggestionsForSession();
  closeHistoryPanel();
  resetGhostSuggestion();
  // AI 助手状态随会话复位（Warp AI 对齐批）：# 模式/修复条/推荐卡/输出采集
  // 缓冲（解码器重建，避免跨会话的多字节残态）。
  aiSearch.reset();
  resetPromptHintsQuiet();
  resetAiFixForSession();
}

function applyCommandMarker(updates: Osc633StreamUpdates) {
  if (updates.shellIntegrationInstalled !== undefined) commandMarker.installed = updates.shellIntegrationInstalled;
  if (updates.commandActive !== undefined) commandMarker.active = updates.commandActive;
  if (updates.command !== undefined) commandMarker.command = updates.command;
  // A fresh "E" frame starts a new command: clear the previous result so the
  // strip flips to the running state. The "A" frame's lastExitCode=null reset
  // is ignored on purpose — the finished result stays visible at the prompt
  // until the next command starts.
  // 最近命令收集：仅 E 帧写 updates.command，故以其存在为准——不能挂在
  // commandActive 上，E 与 D 常在同一段输出里（命令快进快出时合并后的终值
  // 是 false），挂在 phase 上会漏采（与 D/A 退出码覆写同源的合并陷阱）。
  if (updates.command !== undefined && updates.command.trim()) {
    const command = updates.command.trim();
    lastTerminalCommand.value = command;
    // 命令历史采集（P1-1）：shell integration 会话（本地 + SSH）把 E 帧命令行
    // 写入 commandHistory 环形；持久化沿用 SECRET_LIKE/长度过滤，命令执行中
    // 顺带收起浮层。与 onData 回车行采集去重由 pushCommandHistory 保证。
    pushTerminalCommandHistory(command);
    if (isLocalMode.value && command !== localRecentCommands.value[0]) {
      localRecentCommands.value = [command, ...localRecentCommands.value.filter((c) => c !== command)].slice(0, 20);
    }
    closeSuggestions();
    // 命令开始执行：history 面板一并收起（面板开启时 ↑↓/Enter 均被吞键，
    // 正常到不了这里；E 帧旁路（粘贴多行/快速命令）执行时兜底关闭）。
    closeHistoryPanel();
    // AI 修复（Warp AI 对齐批）：E 帧 = 新命令边界——快进快出命令的
    // commandActive 合并终值是 false（E/C/D 同 chunk），清零不能挂在它上。
    // 采集缓冲清零 + 旧修复条让位 + 推荐卡撤下。
    beginAiCommand();
  }
  if (updates.commandActive === true) {
    commandMarker.exitCode = null;
    commandMarker.durationMs = null;
    // A fresh "E" frame also starts the 1s tick so the marker strip shows a
    // live duration while the command runs; the final durationMs from the
    // "D" frame takes over once the tick stops.
    commandMarker.startedAt = Date.now();
    startCommandMarkerTick(commandMarker.startedAt);
  } else if (updates.commandActive === false) {
    stopCommandMarkerTick();
  }
  if (updates.lastExitCode !== undefined && updates.lastExitCode !== null) commandMarker.exitCode = updates.lastExitCode;
  if (updates.lastCommandDuration !== undefined) commandMarker.durationMs = updates.lastCommandDuration;
  // 富元数据采集（批 4d）：D 帧的退出码/时长归档到命令 → 元数据映射（Warp
  // command search 的时长/退出码列）；合并写（两值分帧到达），随时间映射同款
  // 存续集淘汰 + 持久化。
  if ((updates.lastExitCode !== undefined && updates.lastExitCode !== null) || updates.lastCommandDuration !== undefined) {
    const command = commandMarker.command;
    if (command) {
      recordCommandExecutionMeta(command, {
        exitCode: updates.lastExitCode !== undefined && updates.lastExitCode !== null ? updates.lastExitCode : undefined,
        durationMs: updates.lastCommandDuration ?? undefined,
      });
    }
  }
  if (updates.cwd !== undefined) {
    commandMarker.cwd = updates.cwd;
    if (updates.cwd) terminalCwd.value = updates.cwd;
    if (isLocalMode.value) localLastCwd.value = updates.cwd;
  }
}

  return {
    commandMarker,
    commandMarkerElapsed,
    commandMarkerDetails,
    resetCommandMarker,
    startCommandMarkerTick,
    stopCommandMarkerTick,
    applyCommandMarker,
  };
}
