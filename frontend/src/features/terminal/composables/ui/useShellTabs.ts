import { computed, nextTick, reactive, ref, type Ref } from "vue";
import {
  activateTerminalTab,
  addTerminalTab,
  closeTerminalLayoutPane,
  collectPaneTabIds,
  createTerminalLayout,
  findTerminalPane,
  MAIN_TERMINAL_PANE_ID,
  removeSplitAroundPane,
  removeTerminalTab,
  resolveActiveTabCwd,
  splitTerminalLayout,
  terminalTabs,
  type TerminalLayoutOrientation,
  type TerminalLayoutNode,
} from "../../lib/terminalLayout";
import { randomUUID } from "../../../../shared/lib/uuid";

/**
 * 终端分屏/页签的布局树状态与窗格操作。
 *
 * 布局树里每个叶子都能继续左右/上下分屏（A/B 上下分屏后从 B 再左右分屏得到
 * column[A, row[B, C]]）；分屏窗格各自复用已认证 transport 开独立 PTY，宿主
 * openWorkbench 只支持 tab 落点，所以布局完全在插件内。
 *
 * 目录跟随的「页签上报 cwd」与「窗格激活」两个入口仍留在 App（行为不变量
 * 由结构测试锁定字符串），它们读写这里返回的 shellTabCwd/activeTerminalPaneId。
 */
export function useShellTabs(options: {
  terminalCwd: Ref<string>;
  canSplit: () => boolean;
  canCreateTab: () => boolean;
  scheduleFit: () => void;
  syncSftpToActivePane: () => void;
}) {
  const { terminalCwd, canSplit, canCreateTab, scheduleFit, syncSftpToActivePane } = options;

  const terminalLayout = ref<TerminalLayoutNode>(createTerminalLayout());
  const shellTabSessions = new Map<string, string>();
  // 目录跟随由各终端真实 cwd 变化驱动；activeTerminalPaneId 只决定面板重开时
  // 的初始目标。主窗格主终端用 terminalCwd，其余页签用各自 OSC 7 上报的 cwd。
  // 非响应式 Map 的会话映射仍用 shellTabSessions；cwd 需要驱动 computed，单独
  // 用响应式 Map。
  const activeTerminalPaneId = ref(MAIN_TERMINAL_PANE_ID);
  const shellTabCwd = reactive(new Map<string, string>());
  const activePaneCwd = computed(() =>
    resolveActiveTabCwd(terminalLayout.value, activeTerminalPaneId.value, terminalCwd.value, shellTabCwd),
  );

  /** 主终端菜单：把主终端窗格再分一层。 */
  function openTerminalSplit(orientation: TerminalLayoutOrientation) {
    splitTerminalPane("main", orientation);
  }

  function splitTerminalPane(paneId: string, orientation: TerminalLayoutOrientation) {
    if (!canSplit()) return;
    const newPaneId = randomUUID();
    const newTabId = randomUUID();
    terminalLayout.value = splitTerminalLayout(terminalLayout.value, paneId, orientation, newPaneId, newTabId);
    void nextTick(scheduleFit);
  }

  /** 主终端菜单的「关闭分屏」：移除主终端旁边的分屏，主终端本身保留。 */
  function closeTerminalSplit() {
    terminalLayout.value = removeSplitAroundPane(terminalLayout.value, "main");
    pruneShellTabSessions();
    void nextTick(scheduleFit);
  }

  function closePane(paneId: string) {
    for (const tabId of collectPaneTabIds(terminalLayout.value, paneId)) {
      shellTabSessions.delete(tabId);
      shellTabCwd.delete(tabId);
    }
    terminalLayout.value = closeTerminalLayoutPane(terminalLayout.value, paneId) ?? createTerminalLayout();
    if (!findTerminalPane(terminalLayout.value, activeTerminalPaneId.value)) {
      activeTerminalPaneId.value = MAIN_TERMINAL_PANE_ID;
    }
    void nextTick(scheduleFit);
  }

  function createPaneTab(paneId: string) {
    if (!canCreateTab()) return;
    if (!findTerminalPane(terminalLayout.value, paneId)) return;
    activeTerminalPaneId.value = paneId;
    const tabId = randomUUID();
    terminalLayout.value = addTerminalTab(terminalLayout.value, paneId, tabId);
    void nextTick(scheduleFit);
  }

  function activatePaneTab(paneId: string, tabId: string) {
    const pane = findTerminalPane(terminalLayout.value, paneId);
    const tabChanged = pane?.activeTabId !== tabId;
    const paneChanged = activeTerminalPaneId.value !== paneId;
    activeTerminalPaneId.value = paneId;
    terminalLayout.value = activateTerminalTab(terminalLayout.value, paneId, tabId);
    if (tabChanged || paneChanged) syncSftpToActivePane();
    void nextTick(scheduleFit);
  }

  function closePaneTab(paneId: string, tabId: string) {
    shellTabSessions.delete(tabId);
    shellTabCwd.delete(tabId);
    terminalLayout.value = removeTerminalTab(terminalLayout.value, paneId, tabId);
    if (!findTerminalPane(terminalLayout.value, activeTerminalPaneId.value)) {
      activeTerminalPaneId.value = MAIN_TERMINAL_PANE_ID;
    }
    void nextTick(scheduleFit);
  }

  function onShellTabOpened(tabId: string, sessionId: string) {
    shellTabSessions.set(tabId, sessionId);
    // 分屏 PTY 在 ssh/session/open 时已由 sidecar 无条件安装 per-prompt 钩子
    // （InstallPromptHook，静默、不写行），此处不需要再发 directoryTracking。
  }

  /** 布局变化后回收已不在树上的标签会话登记（PTY 由组件卸载负责关闭）。 */
  function pruneShellTabSessions() {
    const live = new Set(terminalTabs(terminalLayout.value).map((tab) => tab.id));
    for (const tabId of shellTabSessions.keys()) {
      if (!live.has(tabId)) shellTabSessions.delete(tabId);
    }
    for (const tabId of shellTabCwd.keys()) {
      if (!live.has(tabId)) shellTabCwd.delete(tabId);
    }
  }

  function resetTerminalLayout() {
    shellTabSessions.clear();
    shellTabCwd.clear();
    activeTerminalPaneId.value = MAIN_TERMINAL_PANE_ID;
    terminalLayout.value = createTerminalLayout();
    void nextTick(scheduleFit);
  }

  return {
    terminalLayout,
    shellTabSessions,
    activeTerminalPaneId,
    shellTabCwd,
    activePaneCwd,
    openTerminalSplit,
    splitTerminalPane,
    closeTerminalSplit,
    closePane,
    createPaneTab,
    activatePaneTab,
    closePaneTab,
    onShellTabOpened,
    pruneShellTabSessions,
    resetTerminalLayout,
  };
}
