import { describe, expect, it } from "vitest";
import {
  activateTerminalTab,
  addTerminalTab,
  closeTerminalLayoutPane,
  collectPaneTabIds,
  collectSplitPaneIds,
  createTerminalLayout,
  findTerminalPane,
  hasTerminalSplit,
  nextTerminalTabLabel,
  removeSplitAroundPane,
  removeTerminalTab,
  resolveActiveTabCwd,
  shouldFollowTabCwd,
  splitTerminalLayout,
  TERMINAL_PANE_TAB_LIMIT,
  terminalLayoutNeedsTabs,
  type TerminalLayoutNode,
} from "./terminalLayout";

function shape(node: TerminalLayoutNode): unknown {
  if (node.kind === "pane") return node.id;
  return { [node.orientation]: [shape(node.first), shape(node.second)] };
}

describe("terminalLayout", () => {
  it("starts with the main pane and its main tab", () => {
    const layout = createTerminalLayout();
    expect(shape(layout)).toBe("main");
    expect(layout.kind).toBe("pane");
    if (layout.kind !== "pane") return;
    expect(layout.tabs).toEqual([{ id: "main", kind: "main", labelIndex: 0 }]);
    expect(layout.activeTabId).toBe("main");
    expect(hasTerminalSplit(layout)).toBe(false);
    expect(collectSplitPaneIds(layout)).toEqual([]);
  });

  it("needs the pane tab strip only for splits or extra shell tabs", () => {
    const layout = createTerminalLayout();
    // 新建布局只有主终端一个页签 → 不需要页签栏。
    expect(terminalLayoutNeedsTabs(layout)).toBe(false);
    // 追加 shell 页签后需要页签栏（切换/关闭页签）。
    expect(terminalLayoutNeedsTabs(addTerminalTab(layout, "main", "tab1"))).toBe(true);
    // 分屏后同样需要页签栏。
    expect(terminalLayoutNeedsTabs(splitTerminalLayout(layout, "main", "row", "B", "tabB"))).toBe(true);
  });

  it("splits only the selected pane, keeping nested orientation intact", () => {
    const vertical = splitTerminalLayout(createTerminalLayout(), "main", "column", "B", "tabB");
    expect(shape(vertical)).toEqual({ column: ["main", "B"] });

    const nested = splitTerminalLayout(vertical, "B", "row", "C", "tabC");
    expect(shape(nested)).toEqual({ column: ["main", { row: ["B", "C"] }] });
    expect(findTerminalPane(nested, "C")?.tabs.map((tab) => tab.id)).toEqual(["tabC"]);
  });

  it("does not mutate the original tree", () => {
    const vertical = splitTerminalLayout(createTerminalLayout(), "main", "column", "B", "tabB");
    splitTerminalLayout(vertical, "B", "row", "C", "tabC");
    expect(shape(vertical)).toEqual({ column: ["main", "B"] });
  });

  it("closes a leaf and collapses its parent to the sibling subtree", () => {
    const nested = splitTerminalLayout(
      splitTerminalLayout(createTerminalLayout(), "main", "column", "B", "tabB"),
      "B",
      "row",
      "C",
      "tabC",
    );
    expect(shape(closeTerminalLayoutPane(nested, "C")!)).toEqual({ column: ["main", "B"] });
    expect(shape(closeTerminalLayoutPane(nested, "B")!)).toEqual({ column: ["main", "C"] });
  });

  it("never closes the main pane", () => {
    const layout = splitTerminalLayout(createTerminalLayout(), "main", "row", "B", "tabB");
    expect(closeTerminalLayoutPane(layout, "main")).toBe(layout);
  });

  it("removes the split around a pane but keeps that pane", () => {
    const nested = splitTerminalLayout(
      splitTerminalLayout(createTerminalLayout(), "main", "column", "B", "tabB"),
      "B",
      "row",
      "C",
      "tabC",
    );
    expect(shape(removeSplitAroundPane(nested, "main"))).toBe("main");
    expect(shape(removeSplitAroundPane(nested, "B"))).toEqual({ column: ["main", "B"] });
  });

  it("collects every split leaf id", () => {
    const nested = splitTerminalLayout(
      splitTerminalLayout(createTerminalLayout(), "main", "column", "B", "tabB"),
      "B",
      "row",
      "C",
      "tabC",
    );
    expect(collectSplitPaneIds(nested).sort()).toEqual(["B", "C"]);
  });

  it("adds tabs to a single pane with global labels and activates the new one", () => {
    const layout = addTerminalTab(createTerminalLayout(), "main", "tab1");
    expect(nextTerminalTabLabel(layout)).toBe(2);
    const withSecond = addTerminalTab(layout, "main", "tab2");
    const pane = findTerminalPane(withSecond, "main")!;
    expect(pane.tabs.map((tab) => [tab.id, tab.labelIndex])).toEqual([
      ["main", 0],
      ["tab1", 1],
      ["tab2", 2],
    ]);
    expect(pane.activeTabId).toBe("tab2");
  });

  it("moves the active tab to the right/left neighbour when closing", () => {
    let layout = addTerminalTab(createTerminalLayout(), "main", "tab1");
    layout = addTerminalTab(layout, "main", "tab2");
    layout = activateTerminalTab(layout, "main", "tab1");
    layout = removeTerminalTab(layout, "main", "tab1");
    expect(findTerminalPane(layout, "main")?.activeTabId).toBe("tab2");
    layout = removeTerminalTab(layout, "main", "tab2");
    expect(findTerminalPane(layout, "main")?.activeTabId).toBe("main");
  });

  it("closes a split pane when its last tab is removed and keeps the main tab", () => {
    const layout = splitTerminalLayout(createTerminalLayout(), "main", "row", "B", "tabB");
    const closed = removeTerminalTab(layout, "B", "tabB");
    expect(shape(closed)).toBe("main");
    expect(removeTerminalTab(closed, "main", "main")).toBe(closed);
  });

  it("enforces the per-pane tab limit", () => {
    let layout: TerminalLayoutNode = createTerminalLayout();
    for (let index = 0; index < TERMINAL_PANE_TAB_LIMIT + 2; index += 1) {
      layout = addTerminalTab(layout, "main", `tab${index}`);
    }
    expect(findTerminalPane(layout, "main")?.tabs).toHaveLength(TERMINAL_PANE_TAB_LIMIT);
  });

  it("collects the tab ids of a pane for session cleanup", () => {
    const layout = addTerminalTab(
      splitTerminalLayout(createTerminalLayout(), "main", "row", "B", "tabB"),
      "main",
      "tab1",
    );
    expect(collectPaneTabIds(layout, "main").sort()).toEqual(["main", "tab1"]);
    expect(collectPaneTabIds(layout, "B")).toEqual(["tabB"]);
  });

  it("resolves the followed cwd from the active pane's active tab", () => {
    const shellCwd = new Map([["tab1", "/srv/app"], ["tabB", "/var/log"]]);
    const split = splitTerminalLayout(createTerminalLayout(), "main", "row", "B", "tabB");
    // 主终端页签 → 主终端 cwd。
    expect(resolveActiveTabCwd(split, "main", "/home/user", shellCwd)).toBe("/home/user");
    // 主窗格的额外页签 → 该页签自己的 cwd。
    const withTab = activateTerminalTab(addTerminalTab(split, "main", "tab1"), "main", "tab1");
    expect(resolveActiveTabCwd(withTab, "main", "/home/user", shellCwd)).toBe("/srv/app");
    // 分屏窗格的活动页签 → 分屏 cwd。
    expect(resolveActiveTabCwd(withTab, "B", "/home/user", shellCwd)).toBe("/var/log");
    // 活动窗格已被关闭 → 回退主窗格（主窗格当前活动页签为 tab1）。
    expect(resolveActiveTabCwd(withTab, "gone", "/home/user", shellCwd)).toBe("/srv/app");
    // 未上报 cwd 的页签 → 空串（调用方跳过跟随，不做错误跳转）。
    expect(resolveActiveTabCwd(split, "B", "/home/user", new Map())).toBe("");
  });

  it("follows only a genuinely new cwd, not the initial prompt or repeated reports", () => {
    // 普通新建分屏的初始提示符：页签不是当前活动页签 → 只记录，不导航。
    expect(shouldFollowTabCwd(undefined, "/data", false)).toBe(false);
    // 活动分屏的 cwd 迟到（打开面板时还停在分屏里）→ 跟随。
    expect(shouldFollowTabCwd(undefined, "/data", true)).toBe(true);
    // cd 到新目录 → 跟随。
    expect(shouldFollowTabCwd("/data", "/data/jenkins", false)).toBe(true);
    // 同一提示符的重复 OSC 7（回车/resize 重绘）→ 不重复导航。
    expect(shouldFollowTabCwd("/data/jenkins", "/data/jenkins", true)).toBe(false);
    // 空路径 = 尚未上报 → 不跟随。
    expect(shouldFollowTabCwd("/data", "", true)).toBe(false);
  });
});
