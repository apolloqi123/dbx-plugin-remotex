// @vitest-environment happy-dom
// 容器管理分屏的布局生命周期：默认只显示列表（无详情、无分隔条）；行点击只
// 高亮不建详情；只有行菜单 Logs/Files 才打开右分栏；详情打开后列表/详情之间
// 是可拖拽/方向键调整的分隔条（25%–70%，默认 40%）；详情关闭回到 100%；
// 轮询刷新打开的详情快照但绝不重开已关闭的详情；列表清空收起详情。
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";
import ContainerManagerPane from "./ContainerManagerPane.vue";
import ContainerListPane from "./ContainerListPane.vue";
import { DOCKER_ENGINE_STORE_KEY } from "../lib/dockerEngine";
import { pluginStore } from "../../../shared/lib/pluginStore";

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
  it("shows only the full-width list until a detail is opened", async () => {
    stubHost(rows());
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

  it("highlights a clicked row without opening the detail", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      await wrapper.get('[data-container-id="web222"]').trigger("click");
      expect(wrapper.find(".container-row.is-selected").exists()).toBe(true);
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);
      expect(wrapper.find(".container-manager-divider").exists()).toBe(false);
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
      expect(listBasis(wrapper)).toBe("40%");
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

  it("switches an open detail to a clicked row but never opens a closed one", async () => {
    const containers: Row[] = [
      ...rows(),
      { id: "db111", name: "db", state: "exited", image: "postgres:16" },
    ];
    stubHost(containers);
    const wrapper = mountManager();
    try {
      await flushPromises();
      await wrapper.get('[data-container-id="db111"]').trigger("click");
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);

      openFromList(wrapper, "logs");
      await nextTick();
      await wrapper.get('[data-container-id="db111"]').trigger("click");
      await nextTick();
      expect(wrapper.get(".container-detail-head").text()).toContain("db");
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
      expect(divider.attributes("aria-valuenow")).toBe("40");

      await divider.trigger("pointerdown", { clientX: 400, button: 0 });
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 600 }));
      await nextTick();
      expect(divider.attributes("aria-valuenow")).toBe("60");
      expect(listBasis(wrapper)).toBe("60%");

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
      expect(divider.attributes("aria-valuenow")).toBe("44");
      await divider.trigger("keydown", { key: "ArrowLeft" });
      expect(divider.attributes("aria-valuenow")).toBe("40");
      await divider.trigger("keydown", { key: "ArrowRight", shiftKey: true });
      expect(divider.attributes("aria-valuenow")).toBe("52");
      expect(listBasis(wrapper)).toBe("52%");

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
      containers[0] = { ...containers[0], status: "Up 2 minutes" };
      await wrapper.get(".container-refresh").trigger("click");
      await flushPromises();
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);
      expect(wrapper.find(".container-manager-divider").exists()).toBe(false);
      expect(wrapper.find(".container-row.is-selected").exists()).toBe(true);
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
});
