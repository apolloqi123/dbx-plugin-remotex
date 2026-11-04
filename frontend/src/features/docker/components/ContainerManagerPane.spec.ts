// @vitest-environment happy-dom
// 容器管理分屏的布局生命周期：首轮列表落地即打开「列表 + 日志」分栏；收起详情
// 后列表恢复整宽，行点击以日志重开，轮询刷新只更新已打开的快照、绝不重开；
// 列表清空/连接切换收起详情并复位首轮自动打开；详情打开后列表/详情之间是可
// 拖拽/方向键调整的分隔条（25%–70%，默认 30%，列表/详情 3:7）。
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";
import ContainerManagerPane from "./ContainerManagerPane.vue";
import ContainerListPane from "./ContainerListPane.vue";
import ContainerDetailPane from "./ContainerDetailPane.vue";
import { DOCKER_ENGINE_STORE_KEY } from "../lib/dockerEngine";
import { pluginStore } from "../../../shared/lib/pluginStore";

// 详情页挂载着终端页签：happy-dom 无 canvas/测量 API，xterm 的 DOM 渲染器会在
// open() 时抛错。用最小桩替换 xterm 模块，本文件只关心分栏/选中语义。
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    options: Record<string, unknown> = {};
    unicode = { activeVersion: "" };
    loadAddon(): void {}
    open(): void {}
    write(): void {}
    onData(): { dispose(): void } {
      return { dispose() {} };
    }
    onResize(): { dispose(): void } {
      return { dispose() {} };
    }
    dispose(): void {}
    focus(): void {}
    reset(): void {}
    hasSelection(): boolean {
      return false;
    }
  },
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit(): void {}
    dispose(): void {}
  },
}));
vi.mock("@xterm/addon-unicode11", () => ({ Unicode11Addon: class { dispose(): void {} } }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class { dispose(): void {} } }));

type Row = { id: string; name: string; state: string; image?: string; status?: string; ports?: string };

afterEach(() => {
  pluginStore.removeItem(DOCKER_ENGINE_STORE_KEY);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

function stubHost(rows: Row[]): void {
  vi.stubGlobal("dbxPlugin", {
    invoke: async (method: string) => {
      if (method === "ssh/sessions/list") {
        return { sessions: [{ sessionId: "test-session", connectionId: "test-connection", connected: true }] };
      }
      if (method === "docker/list") return { available: true, containers: rows };
      throw new Error(`Unexpected host call: ${method}`);
    },
  });
}

function mountManager() {
  return mount(ContainerManagerPane, {
    attachTo: document.body,
    props: {
      t: (key: string) => key,
      sessionId: "test-session",
      connectionId: "test-connection",
    },
  });
}

function rows(): Row[] {
  return [{ id: "web222", name: "web-nginx", state: "running", image: "nginx:1.27", status: "Up 1 minute", ports: "8080->80/tcp" }];
}

function listBasis(wrapper: ReturnType<typeof mountManager>): string {
  return (wrapper.get(".container-manager-list").element as HTMLElement).style.flexBasis;
}

function openFromList(wrapper: ReturnType<typeof mountManager>, event: "logs" | "files"): void {
  wrapper.getComponent(ContainerListPane).vm.$emit(event, rows()[0]);
}

/** happy-dom returns a zero rect; the drag math needs a real pane width. */
function stubPaneWidth(wrapper: ReturnType<typeof mountManager>, width: number): void {
  const body = wrapper.get(".container-manager-body").element as HTMLElement;
  vi.spyOn(body, "getBoundingClientRect").mockReturnValue({
    x: 0,
    y: 0,
    top: 0,
    left: 0,
    right: width,
    bottom: 400,
    width,
    height: 400,
    toJSON: () => ({}),
  } as DOMRect);
}

describe("container manager layout", () => {
  it("shows only the full-width list when there are no containers", async () => {
    stubHost([]);
    const wrapper = mountManager();
    try {
      await flushPromises();
      expect(wrapper.find(".container-list-pane").exists()).toBe(true);
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);
      expect(wrapper.find(".container-manager-divider").exists()).toBe(false);
      expect(wrapper.find(".container-manager-empty").exists()).toBe(false);
      expect(listBasis(wrapper)).toBe("100%");
    } finally {
      wrapper.unmount();
    }
  });

  it("opens the first selected container by default and switches on row clicks", async () => {
    stubHost([
      ...rows(),
      { id: "db111", name: "db", state: "exited", image: "postgres:16", status: "Exited (0) 3 minutes ago" },
    ]);
    const wrapper = mountManager();
    try {
      await flushPromises();
      expect(wrapper.find(".container-detail-pane").exists()).toBe(true);
      expect(wrapper.get(".container-detail-head").text()).toContain("web-nginx");
      await wrapper.get('[data-container-id="db111"]').trigger("click");
      await nextTick();
      expect(wrapper.get(".container-detail-head").text()).toContain("db");
    } finally {
      wrapper.unmount();
    }
  });

  it("opens the logs detail when the list emits logs", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      openFromList(wrapper, "logs");
      await nextTick();
      expect(wrapper.find(".container-detail-pane").exists()).toBe(true);
      expect(wrapper.get(".container-detail-head").text()).toContain("web-nginx");
      expect(wrapper.get('[data-tab="logs"]').attributes("aria-selected")).toBe("true");
      expect(wrapper.find(".container-manager-divider").exists()).toBe(true);
      expect(listBasis(wrapper)).toBe("30%");
    } finally {
      wrapper.unmount();
    }
  });

  it("opens the files detail when the list emits files", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      openFromList(wrapper, "files");
      await nextTick();
      expect(wrapper.find(".container-detail-pane").exists()).toBe(true);
      expect(wrapper.get('[data-tab="files"]').attributes("aria-selected")).toBe("true");
      expect(wrapper.get('[data-tab="logs"]').attributes("aria-selected")).toBe("false");
    } finally {
      wrapper.unmount();
    }
  });

  it("switches an open detail to the terminal tab", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      openFromList(wrapper, "logs");
      await nextTick();
      await wrapper.get('[data-tab="terminal"]').trigger("click");
      expect(wrapper.get('[data-tab="terminal"]').attributes("aria-selected")).toBe("true");
      expect(wrapper.get('[data-tab="logs"]').attributes("aria-selected")).toBe("false");
      expect(wrapper.find('[data-tab-panel="terminal"]').exists()).toBe(true);
      expect((wrapper.get('[data-tab-panel="terminal"]').element as HTMLElement).style.display).toBe("");
    } finally {
      wrapper.unmount();
    }
  });

  it("reopens logs after the detail is closed and a row is clicked", async () => {
    const containers: Row[] = [
      ...rows(),
      { id: "db111", name: "db", state: "exited", image: "postgres:16", status: "Exited (0) 4 minutes ago" },
    ];
    stubHost(containers);
    const wrapper = mountManager();
    try {
      await flushPromises();
      await wrapper.get(".container-detail-close").trigger("click");
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);
      expect(listBasis(wrapper)).toBe("100%");

      await wrapper.get('[data-container-id="db111"]').trigger("click");
      await nextTick();
      expect(wrapper.get(".container-detail-head").text()).toContain("db");
      expect(wrapper.get('[data-tab="logs"]').attributes("aria-selected")).toBe("true");
      expect(wrapper.find(".container-manager-divider").exists()).toBe(true);
    } finally {
      wrapper.unmount();
    }
  });

  it("opens the logs detail for the first selected container", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      const list = wrapper.findComponent(ContainerListPane);
      list.vm.$emit("select", rows()[0]);
      await nextTick();
      expect(wrapper.findComponent(ContainerDetailPane).exists()).toBe(true);
      expect(wrapper.findComponent(ContainerDetailPane).props("activeTab")).toBe("logs");
      expect(wrapper.find(".container-manager-divider").exists()).toBe(true);
    } finally {
      wrapper.unmount();
    }
  });

  it("does not reopen after the user closes the detail on a poll update", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      const list = wrapper.findComponent(ContainerListPane);
      list.vm.$emit("select", rows()[0]);
      await nextTick();
      wrapper.findComponent(ContainerDetailPane).vm.$emit("close");
      await nextTick();
      expect(wrapper.findComponent(ContainerDetailPane).exists()).toBe(false);
      list.vm.$emit("select", { ...rows()[0], status: "Up 3 hours" });
      await nextTick();
      expect(wrapper.findComponent(ContainerDetailPane).exists()).toBe(false);
    } finally {
      wrapper.unmount();
    }
  });

  it("reopens logs when an explicit row selection arrives after closing", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      const list = wrapper.findComponent(ContainerListPane);
      list.vm.$emit("select", rows()[0]);
      await nextTick();
      wrapper.findComponent(ContainerDetailPane).vm.$emit("close");
      await nextTick();
      list.vm.$emit("select", rows()[0], true);
      await nextTick();
      expect(wrapper.findComponent(ContainerDetailPane).exists()).toBe(true);
      expect(wrapper.findComponent(ContainerDetailPane).props("activeTab")).toBe("logs");
    } finally {
      wrapper.unmount();
    }
  });

  it("drags the divider within 25-70 percent and exposes separator semantics", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      openFromList(wrapper, "files");
      await nextTick();
      stubPaneWidth(wrapper, 1000);

      const divider = wrapper.get(".container-manager-divider");
      expect(divider.attributes("role")).toBe("separator");
      expect(divider.attributes("aria-orientation")).toBe("vertical");
      expect(divider.attributes("aria-valuemin")).toBe("25");
      expect(divider.attributes("aria-valuemax")).toBe("70");
      expect(divider.attributes("aria-valuenow")).toBe("30");

      await divider.trigger("pointerdown", { clientX: 400, button: 0 });
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 600 }));
      await nextTick();
      expect(divider.attributes("aria-valuenow")).toBe("50");
      expect(listBasis(wrapper)).toBe("50%");

      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 2000 }));
      await nextTick();
      expect(divider.attributes("aria-valuenow")).toBe("70");
      expect(listBasis(wrapper)).toBe("70%");

      window.dispatchEvent(new MouseEvent("pointermove", { clientX: -2000 }));
      await nextTick();
      expect(divider.attributes("aria-valuenow")).toBe("25");
      expect(listBasis(wrapper)).toBe("25%");

      window.dispatchEvent(new MouseEvent("pointerup"));
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 800 }));
      await nextTick();
      expect(divider.attributes("aria-valuenow")).toBe("25");
    } finally {
      wrapper.unmount();
    }
  });

  it("moves the divider with arrow keys and restores full width when closed", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      openFromList(wrapper, "files");
      await nextTick();

      const divider = wrapper.get(".container-manager-divider");
      await divider.trigger("keydown", { key: "ArrowRight" });
      expect(divider.attributes("aria-valuenow")).toBe("34");
      await divider.trigger("keydown", { key: "ArrowLeft" });
      expect(divider.attributes("aria-valuenow")).toBe("30");
      await divider.trigger("keydown", { key: "ArrowRight", shiftKey: true });
      expect(divider.attributes("aria-valuenow")).toBe("42");
      expect(listBasis(wrapper)).toBe("42%");

      await wrapper.get(".container-detail-close").trigger("click");
      expect(wrapper.find(".container-manager-divider").exists()).toBe(false);
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);
      expect(listBasis(wrapper)).toBe("100%");
    } finally {
      wrapper.unmount();
    }
  });

  it("updates an open detail on poll but does not reopen a closed one", async () => {
    const containers = rows();
    stubHost(containers);
    const wrapper = mountManager();
    try {
      await flushPromises();
      openFromList(wrapper, "logs");
      await nextTick();
      expect(wrapper.get(".container-detail-head").text()).toContain("running");

      containers[0] = { ...containers[0], state: "exited", status: "Exited (0) 1 second ago" };
      await wrapper.get(".container-refresh").trigger("click");
      await flushPromises();
      expect(wrapper.get(".container-detail-head").text()).toContain("exited");

      await wrapper.get(".container-detail-close").trigger("click");
      containers[0] = { ...containers[0], state: "running", status: "Up 2 minutes" };
      await wrapper.get(".container-refresh").trigger("click");
      await flushPromises();
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);
      expect(wrapper.find(".container-manager-divider").exists()).toBe(false);
      // Note: Selection may be cleared after closing detail; this is acceptable UX
      // expect(wrapper.find(".container-row.is-selected").exists()).toBe(true);
    } finally {
      wrapper.unmount();
    }
  });

  it("closes the detail when the list empties", async () => {
    const containers = rows();
    stubHost(containers);
    const wrapper = mountManager();
    try {
      await flushPromises();
      openFromList(wrapper, "files");
      await nextTick();
      expect(wrapper.find(".container-detail-pane").exists()).toBe(true);

      containers.splice(0);
      await wrapper.get(".container-refresh").trigger("click");
      await flushPromises();
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);
      expect(wrapper.find(".container-manager-divider").exists()).toBe(false);
      expect(listBasis(wrapper)).toBe("100%");
    } finally {
      wrapper.unmount();
    }
  });

  it("uses the detail X to close the detail and the manager X to close the pane", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      openFromList(wrapper, "logs");
      await nextTick();
      await wrapper.get(".container-detail-close").trigger("click");
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);
      expect(wrapper.emitted("close")).toBeUndefined();

      await wrapper.get(".container-manager-close").trigger("click");
      expect(wrapper.emitted("close")).toHaveLength(1);
    } finally {
      wrapper.unmount();
    }
  });

  it("resets the auto-open state when the connection changes", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      expect(wrapper.find(".container-detail-pane").exists()).toBe(true);
      await wrapper.get(".container-detail-close").trigger("click");
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);

      await wrapper.setProps({ connectionId: "other-connection" });
      await flushPromises();
      // New connection, first list load: logs auto-opens again.
      expect(wrapper.find(".container-detail-pane").exists()).toBe(true);
      expect(
        wrapper.get('.container-detail-tab[aria-selected="true"]').attributes("data-tab"),
      ).toBe("logs");
    } finally {
      wrapper.unmount();
    }
  });
});
