/**
 * 工作台终端窗格布局树。
 *
 * 分屏不是「一个全局方向 + 一个附加 PTY」：每个窗格都能继续左右/上下分屏，
 * 且每个窗格拥有自己的标签页集合（主窗格的第一页是主终端，其余页与分屏
 * 页都是独立 PTY）。用二叉树表达布局，叶子内保存 tabs/activeTabId。
 */

export const MAIN_TERMINAL_PANE_ID = "main";
export const MAIN_TERMINAL_TAB_ID = "main";
export const TERMINAL_PANE_TAB_LIMIT = 8;

export type TerminalLayoutOrientation = "row" | "column";

export interface TerminalPaneTab {
  id: string;
  kind: "main" | "shell";
  /** 0 = 主终端；其余为全局递增的显示序号。 */
  labelIndex: number;
}

export interface TerminalPaneLeaf {
  kind: "pane";
  id: string;
  paneKind: "main" | "split";
  tabs: TerminalPaneTab[];
  activeTabId: string;
}

export interface TerminalSplitNode {
  kind: "split";
  orientation: TerminalLayoutOrientation;
  first: TerminalLayoutNode;
  second: TerminalLayoutNode;
}

export type TerminalLayoutNode = TerminalPaneLeaf | TerminalSplitNode;

function createMainTab(): TerminalPaneTab {
  return { id: MAIN_TERMINAL_TAB_ID, kind: "main", labelIndex: 0 };
}

function createShellTab(id: string, labelIndex: number): TerminalPaneTab {
  return { id, kind: "shell", labelIndex };
}

export function createTerminalLayout(): TerminalPaneLeaf {
  return {
    kind: "pane",
    id: MAIN_TERMINAL_PANE_ID,
    paneKind: "main",
    tabs: [createMainTab()],
    activeTabId: MAIN_TERMINAL_TAB_ID,
  };
}

export function findTerminalPane(
  node: TerminalLayoutNode,
  paneId: string,
): TerminalPaneLeaf | undefined {
  if (node.kind === "pane") return node.id === paneId ? node : undefined;
  return findTerminalPane(node.first, paneId) ?? findTerminalPane(node.second, paneId);
}

/**
 * 目录跟随的 cwd 选择：以「最后交互窗格的活动页签」为准——主终端页签用主
 * 终端上报的 cwd，其余页签用各自 OSC 7 记录的 cwd；活动窗格已被关闭时回退
 * 到主窗格，未知页签返回空串（调用方跳过跟随，避免把 SFTP 带到错误目录）。
 */
export function resolveActiveTabCwd(
  node: TerminalLayoutNode,
  activePaneId: string,
  mainCwd: string,
  shellTabCwd: ReadonlyMap<string, string>,
): string {
  const pane = findTerminalPane(node, activePaneId) ?? findTerminalPane(node, MAIN_TERMINAL_PANE_ID);
  const tabId = pane?.activeTabId ?? MAIN_TERMINAL_TAB_ID;
  return tabId === MAIN_TERMINAL_TAB_ID ? mainCwd : shellTabCwd.get(tabId) ?? "";
}

/**
 * 目录跟随的触发判定。页签的首次上报是初始提示符（打开分屏/新标签），只有
 * 该页签本来就是活动窗格的活动页签（cwd 迟到、旧 watcher 覆盖的窗口）才
 * 跟随；其余情况只记录，供面板重开目标使用。之后只有真正换了目录才跟随，
 * 同一提示符的重复上报（回车/resize 重绘）不触发。窗格聚焦切换不经过这里，
 * 因此不会把 SFTP 反复带回各窗格目录。
 */
export function shouldFollowTabCwd(
  previous: string | undefined,
  next: string,
  tabIsActive: boolean,
): boolean {
  if (!next) return false;
  if (previous === undefined) return tabIsActive;
  return next !== previous;
}

export function terminalTabs(node: TerminalLayoutNode): TerminalPaneTab[] {
  return node.kind === "pane" ? node.tabs : [...terminalTabs(node.first), ...terminalTabs(node.second)];
}

/** 全局显示序号：跨窗格递增，关闭后不复用（与截图中的 终端 1..N 一致）。 */
export function nextTerminalTabLabel(node: TerminalLayoutNode): number {
  return terminalTabs(node).reduce((max, tab) => Math.max(max, tab.labelIndex), 0) + 1;
}

export function collectPaneTabIds(node: TerminalLayoutNode, paneId: string): string[] {
  const pane = findTerminalPane(node, paneId);
  return pane ? pane.tabs.map((tab) => tab.id) : [];
}

/**
 * 把指定叶子替换为「原叶子 + 新分屏叶子」的二叉节点。新叶子带一个自己的
 * 标签页；原叶子的标签页全部保留。
 */
export function splitTerminalLayout(
  node: TerminalLayoutNode,
  paneId: string,
  orientation: TerminalLayoutOrientation,
  newPaneId: string,
  newTabId: string,
): TerminalLayoutNode {
  if (!findTerminalPane(node, paneId)) return node;
  const labelIndex = nextTerminalTabLabel(node);
  const insert = (current: TerminalLayoutNode): TerminalLayoutNode => {
    if (current.kind === "pane") {
      if (current.id !== paneId) return current;
      return {
        kind: "split",
        orientation,
        first: current,
        second: {
          kind: "pane",
          id: newPaneId,
          paneKind: "split",
          tabs: [createShellTab(newTabId, labelIndex)],
          activeTabId: newTabId,
        },
      };
    }
    return { ...current, first: insert(current.first), second: insert(current.second) };
  };
  return insert(node);
}

/** 在指定窗格追加一个标签页并激活。 */
export function addTerminalTab(
  node: TerminalLayoutNode,
  paneId: string,
  tabId: string,
): TerminalLayoutNode {
  const pane = findTerminalPane(node, paneId);
  if (!pane) return node;
  if (pane.tabs.length >= TERMINAL_PANE_TAB_LIMIT) return node;
  const labelIndex = nextTerminalTabLabel(node);
  const update = (current: TerminalLayoutNode): TerminalLayoutNode => {
    if (current.kind === "pane") {
      if (current.id !== paneId) return current;
      return { ...current, tabs: [...current.tabs, createShellTab(tabId, labelIndex)], activeTabId: tabId };
    }
    return { ...current, first: update(current.first), second: update(current.second) };
  };
  return update(node);
}

/**
 * 关闭指定窗格里的标签页；主终端的 main 标签不可关闭。
 * 分屏窗格的最后一个标签关闭时整棵父节点折叠成兄弟子树。
 */
export function removeTerminalTab(
  node: TerminalLayoutNode,
  paneId: string,
  tabId: string,
): TerminalLayoutNode {
  const remove = (current: TerminalLayoutNode): TerminalLayoutNode | null => {
    if (current.kind === "pane") {
      if (current.id !== paneId) return current;
      const index = current.tabs.findIndex((tab) => tab.id === tabId);
      if (index < 0) return current;
      const tab = current.tabs[index];
      if (tab.kind === "main") return current;
      const tabs = current.tabs.filter((item) => item.id !== tabId);
      if (!tabs.length) return current.paneKind === "split" ? null : current;
      const activeTabId =
        current.activeTabId === tabId
          ? (tabs[index] ?? tabs[index - 1]).id
          : current.activeTabId;
      return { ...current, tabs, activeTabId };
    }
    const first = remove(current.first);
    const second = remove(current.second);
    if (!first) return second;
    if (!second) return first;
    if (first === current.first && second === current.second) return current;
    return { ...current, first, second };
  };
  return remove(node) ?? createTerminalLayout();
}

export function activateTerminalTab(
  node: TerminalLayoutNode,
  paneId: string,
  tabId: string,
): TerminalLayoutNode {
  const pane = findTerminalPane(node, paneId);
  if (!pane || !pane.tabs.some((tab) => tab.id === tabId)) return node;
  if (pane.activeTabId === tabId) return node;
  const update = (current: TerminalLayoutNode): TerminalLayoutNode => {
    if (current.kind === "pane") {
      return current.id === paneId ? { ...current, activeTabId: tabId } : current;
    }
    return { ...current, first: update(current.first), second: update(current.second) };
  };
  return update(node);
}

/**
 * 关闭指定分屏窗格：父节点折叠为另一个子树。主终端窗格不可关闭。
 */
export function closeTerminalLayoutPane(
  node: TerminalLayoutNode,
  paneId: string,
): TerminalLayoutNode | null {
  if (node.kind === "pane") {
    if (node.id !== paneId || node.paneKind === "main") return node;
    return null;
  }
  const first = closeTerminalLayoutPane(node.first, paneId);
  const second = closeTerminalLayoutPane(node.second, paneId);
  if (!first) return second;
  if (!second) return first;
  if (first === node.first && second === node.second) return node;
  return { ...node, first, second };
}

/**
 * 移除包含指定叶子的最近一层分屏（连同兄弟子树），保留该叶子本身。
 * 主终端菜单的「关闭分屏」用它：关闭主终端旁边的分屏，而不是某个子窗格。
 */
export function removeSplitAroundPane(
  node: TerminalLayoutNode,
  paneId: string,
): TerminalLayoutNode {
  if (node.kind === "pane") return node;
  if (node.first.kind === "pane" && node.first.id === paneId) return node.first;
  if (node.second.kind === "pane" && node.second.id === paneId) return node.second;
  if (findTerminalPane(node.first, paneId)) {
    return { ...node, first: removeSplitAroundPane(node.first, paneId) };
  }
  if (findTerminalPane(node.second, paneId)) {
    return { ...node, second: removeSplitAroundPane(node.second, paneId) };
  }
  return node;
}

/** 所有分屏叶子 id（不含 main），用于工作台关闭时回收 PTY。 */
export function collectSplitPaneIds(node: TerminalLayoutNode): string[] {
  if (node.kind === "pane") {
    return node.paneKind === "split" ? [node.id] : [];
  }
  return [...collectSplitPaneIds(node.first), ...collectSplitPaneIds(node.second)];
}

export function hasTerminalSplit(node: TerminalLayoutNode): boolean {
  return node.kind === "split";
}

/** 终端布局是否需要窗格页签栏：存在分屏，或窗格内除主终端外还有 shell 页签。 */
export function terminalLayoutNeedsTabs(node: TerminalLayoutNode): boolean {
  if (node.kind === "split") return true;
  return node.tabs.length > 1;
}
