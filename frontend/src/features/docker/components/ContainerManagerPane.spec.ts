// @vitest-environment happy-dom
// 容器管理分屏的选择生命周期：列表清空必须收起详情（不能残留已删除容器），
// 轮询的状态变化必须刷新父组件持有的容器快照；详情页头 X 只收起详情，
// 分屏页头 X 才关闭整个分屏。
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ContainerManagerPane from "./ContainerManagerPane.vue";
import { DOCKER_ENGINE_STORE_KEY } from "../lib/dockerEngine";
import { pluginStore } from "../../../shared/lib/pluginStore";

type Row = { id: string; name: string; state: string; image?: string; status?: string; ports?: string };

afterEach(() => {
  pluginStore.removeItem(DOCKER_ENGINE_STORE_KEY);
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

describe("container manager selection", () => {
  it("clears the detail when the list becomes empty", async () => {
    const containers = rows();
    stubHost(containers);
    const wrapper = mountManager();
    try {
      await flushPromises();
      expect(wrapper.get(".container-detail-head").text()).toContain("web-nginx");
      containers.splice(0);
      await wrapper.get(".container-refresh").trigger("click");
      await flushPromises();
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);
      expect(wrapper.get(".container-manager-empty").text()).not.toContain("web-nginx");
    } finally {
      wrapper.unmount();
    }
  });

  it("updates the detail when polling reports a new state", async () => {
    const containers = rows();
    stubHost(containers);
    const wrapper = mountManager();
    try {
      await flushPromises();
      expect(wrapper.get(".container-detail-head").text()).toContain("running");
      containers[0] = { ...containers[0], state: "exited", status: "Exited (0) 1 second ago" };
      await wrapper.get(".container-refresh").trigger("click");
      await flushPromises();
      expect(wrapper.get(".container-detail-head").text()).toContain("exited");
      expect(wrapper.get(".container-detail-head").text()).not.toContain("running");
    } finally {
      wrapper.unmount();
    }
  });

  it("uses the detail X to clear the selection and the manager X to close the pane", async () => {
    stubHost(rows());
    const wrapper = mountManager();
    try {
      await flushPromises();
      await wrapper.get(".container-detail-close").trigger("click");
      expect(wrapper.find(".container-detail-pane").exists()).toBe(false);
      expect(wrapper.emitted("close")).toBeUndefined();

      // 手动收起后再次点击仍是高亮行：必须能重新打开详情。
      await wrapper.get('[data-container-id="web222"]').trigger("click");
      expect(wrapper.find(".container-detail-pane").exists()).toBe(true);
      expect(wrapper.emitted("close")).toBeUndefined();

      await wrapper.get(".container-manager-close").trigger("click");
      expect(wrapper.emitted("close")).toHaveLength(1);
    } finally {
      wrapper.unmount();
    }
  });
});
