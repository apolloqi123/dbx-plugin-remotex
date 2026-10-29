// useHistoryPanel 按键接线回归：Enter 恒由面板消费（仅回填、不执行），
// Tab 在命令行路径放行给 shell 补全。纯逻辑映射在 historyPanel.spec，
// 这里锁住「面板按键消费返回值」这一层——它决定 useTerminalKeys 是否把键
// 放进 xterm（返回 false 会把回车发往 PTY 直接执行命令）。
import { describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { useHistoryPanel } from "./useHistoryPanel";

function setup() {
  const replaceTerminalLineWith = vi.fn();
  const terminal = { buffer: { active: { type: "normal" } }, focus: vi.fn() };
  const historyPanel = useHistoryPanel({
    terminal: () => terminal as never,
    commandHistory: () => ["git status", "ls"],
    commandHistoryTimes: () => ({}),
    commandHistoryMeta: () => ({}),
    getPendingTerminalInput: () => "ls",
    getSessionId: () => undefined,
    isLocalMode: () => true,
    commandRunning: () => false,
    terminalTransferBusy: () => false,
    completionOpen: () => false,
    suggestionOpen: () => false,
    quickSelectOpen: () => false,
    searchOpen: () => false,
    interactivePromptPending: () => false,
    terminalMenuOpen: ref(false),
    loadRemoteShellHistory: vi.fn(),
    replaceTerminalLineWith,
    readEchoTextBeforeCursor: () => null,
    readTerminalSuggestionAnchor: () => ({ x: 0, y: 100, cellHeight: 17, cellWidth: 8 }),
    hideGhostSuggestion: vi.fn(),
    closeSuggestions: vi.fn(),
    dismissPromptHints: vi.fn(),
  });
  return { historyPanel, terminal, replaceTerminalLineWith };
}

function keydown(key: string): KeyboardEvent {
  return { type: "keydown", key, isComposing: false, keyCode: key === "Enter" ? 13 : key === "Tab" ? 9 : 0 } as KeyboardEvent;
}

describe("handleHistoryPanelKey (Enter 仅回填)", () => {
  it("swallows Enter and closes without sending it to the shell", () => {
    const { historyPanel, terminal } = setup();
    historyPanel.openHistoryPanel();
    expect(historyPanel.historyPanelOpen.value).toBe(true);
    // true = 消费：useTerminalKeys 不会把 Enter 交给 xterm，命令只留在输入行。
    expect(historyPanel.handleHistoryPanelKey(keydown("Enter"))).toBe(true);
    expect(historyPanel.historyPanelOpen.value).toBe(false);
    expect(terminal.focus).toHaveBeenCalled();
  });

  it("still lets Tab through to shell completion from the terminal path", () => {
    const { historyPanel } = setup();
    historyPanel.openHistoryPanel();
    // false = 放行：面板收起，Tab 交给 shell 原生补全（不是回填执行）。
    expect(historyPanel.handleHistoryPanelKey(keydown("Tab"))).toBe(false);
    expect(historyPanel.historyPanelOpen.value).toBe(false);
  });

  it("consumes Enter from the search box as well", () => {
    const { historyPanel } = setup();
    historyPanel.openHistoryPanel(true);
    expect(historyPanel.handleHistoryPanelKey(keydown("Enter"), true)).toBe(true);
    expect(historyPanel.historyPanelOpen.value).toBe(false);
  });
});
