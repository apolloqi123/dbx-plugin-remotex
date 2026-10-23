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
  splitTerminalLayout,
  TERMINAL_PANE_TAB_LIMIT,
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
});
