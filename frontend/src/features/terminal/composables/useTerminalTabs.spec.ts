import { describe, expect, it } from "vitest";
import { TERMINAL_TAB_LIMIT, useTerminalTabs } from "./useTerminalTabs";

describe("useTerminalTabs", () => {
  it("starts with the main tab and adds aux tabs in order", () => {
    let next = 0;
    const state = useTerminalTabs({ createId: () => `tab-${++next}` });
    expect(state.tabs.value.map((tab) => tab.kind)).toEqual(["main"]);
    expect(state.activeId.value).toBe("main");

    const first = state.create();
    const second = state.create();
    expect(first).toMatchObject({ id: "tab-1", kind: "aux", index: 1 });
    expect(second).toMatchObject({ id: "tab-2", kind: "aux", index: 2 });
    expect(state.tabs.value.map((tab) => tab.id)).toEqual(["main", "tab-1", "tab-2"]);
    expect(state.activeId.value).toBe("tab-2");
    expect(state.auxTabs.value.map((tab) => tab.id)).toEqual(["tab-1", "tab-2"]);
  });

  it("falls back to the right neighbour, then left, when closing the active tab", () => {
    let next = 0;
    const state = useTerminalTabs({ createId: () => `tab-${++next}` });
    state.create();
    state.create();
    state.create();
    state.activate("tab-2");

    state.close("tab-2");
    expect(state.activeId.value).toBe("tab-3");

    state.close("tab-3");
    expect(state.activeId.value).toBe("tab-1");

    state.close("tab-1");
    expect(state.activeId.value).toBe("main");
  });

  it("never closes the main tab and caps the number of tabs", () => {
    let next = 0;
    const state = useTerminalTabs({ createId: () => `tab-${++next}`, limit: 3 });
    expect(state.close("main")).toBe(false);
    expect(state.create()).toBeDefined();
    expect(state.create()).toBeDefined();
    expect(state.canAdd.value).toBe(false);
    expect(state.create()).toBeUndefined();
    expect(state.tabs.value).toHaveLength(3);
  });

  it("reset returns to the main tab and keeps numbering monotonic", () => {
    let next = 0;
    const state = useTerminalTabs({ createId: () => `tab-${++next}` });
    state.create();
    state.create();
    state.reset();
    expect(state.tabs.value).toHaveLength(1);
    expect(state.activeId.value).toBe("main");
    expect(state.create()?.index).toBe(3);
  });

  it("exposes the default cap", () => {
    expect(TERMINAL_TAB_LIMIT).toBe(8);
  });
});
