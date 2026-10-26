// @vitest-environment happy-dom
// happy-dom 没有 canvas/测量 API：终端页签挂载时 xterm 会尝试创建 DOM 渲染器，
// 这里用最小桩替换 xterm 模块，只验证页签挂载/显隐与 props 下传。
import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import ContainerDetailPane from "./ContainerDetailPane.vue";
import ContainerLogsTab from "./ContainerLogsTab.vue";
import ContainerFilesTab from "./ContainerFilesTab.vue";
import ContainerTerminalTab from "./ContainerTerminalTab.vue";
import type { DockerContainer } from "../lib/dockerPanelModel";

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

const t = (key: string) => key;
const container: DockerContainer = {
  id: "web222",
  name: "web-nginx",
  image: "nginx:1.27",
  state: "running",
  status: "Up 3 minutes",
  ports: "0.0.0.0:8080->80/tcp",
  createdAt: "2026-10-26T00:00:00Z",
};

describe("container detail pane", () => {
  it("switches tabs and keeps the tab strip mounted", async () => {
    const wrapper = mount(ContainerDetailPane, { props: { t, container, activeTab: "logs" } });
    expect(wrapper.get('[data-tab="logs"]').attributes("aria-label")).toBe("docker.logsTab");
    expect(wrapper.get('[data-tab="logs"]').find("svg").exists()).toBe(true);
    expect(wrapper.get('[data-tab="files"]').attributes("aria-label")).toBe("docker.filesTab");
    expect(wrapper.get('[data-tab="files"]').find("svg").exists()).toBe(true);
    // 第三个页签只有图标，title/aria-label 指向 docker.terminalTab。
    expect(wrapper.get('[data-tab="terminal"]').attributes("aria-label")).toBe("docker.terminalTab");
    expect(wrapper.get('[data-tab="terminal"]').attributes("title")).toBe("docker.terminalTab");
    expect(wrapper.get('[data-tab="terminal"]').find("svg").exists()).toBe(true);
    await wrapper.get('[data-tab="files"]').trigger("click");
    expect(wrapper.emitted("update:activeTab")?.[0]).toEqual(["files"]);
    await wrapper.get('[data-tab="terminal"]').trigger("click");
    expect(wrapper.emitted("update:activeTab")?.[1]).toEqual(["terminal"]);
    expect(wrapper.find('[data-tab="logs"]').exists()).toBe(true);
  });

  it("shows the terminal tab panel and hides it when another tab is active", async () => {
    const wrapper = mount(ContainerDetailPane, { props: { t, container, activeTab: "terminal" } });
    expect((wrapper.findComponent(ContainerTerminalTab).element as HTMLElement).style.display).toBe("");
    expect((wrapper.findComponent(ContainerLogsTab).element as HTMLElement).style.display).toBe("none");
    expect((wrapper.findComponent(ContainerFilesTab).element as HTMLElement).style.display).toBe("none");

    await wrapper.setProps({ activeTab: "logs" });
    expect((wrapper.findComponent(ContainerTerminalTab).element as HTMLElement).style.display).toBe("none");
  });

  it("tells the logs tab when it becomes visible again after Files → Logs", async () => {
    const wrapper = mount(ContainerDetailPane, { props: { t, container, activeTab: "logs" } });
    expect(wrapper.findComponent(ContainerLogsTab).props("visible")).toBe(true);

    await wrapper.setProps({ activeTab: "files" });
    expect(wrapper.findComponent(ContainerLogsTab).props("visible")).toBe(false);

    await wrapper.setProps({ activeTab: "logs" });
    expect(wrapper.findComponent(ContainerLogsTab).props("visible")).toBe(true);
  });

  it("passes the selected container down to the active tab", () => {
    const wrapper = mount(ContainerDetailPane, {
      props: { t, container, activeTab: "logs", sessionId: "s", connectionId: "c", canWrite: false },
    });
    expect(wrapper.findComponent(ContainerLogsTab).props()).toMatchObject({
      container,
      sessionId: "s",
      connectionId: "c",
      canWrite: false,
    });
    // 三个页签都挂载（v-show），但只有活动页签可见。
    expect(wrapper.findComponent(ContainerFilesTab).props()).toMatchObject({ container, canWrite: false });
    expect(wrapper.findComponent(ContainerTerminalTab).props()).toMatchObject({
      container,
      sessionId: "s",
      connectionId: "c",
      canWrite: false,
    });
    expect((wrapper.findComponent(ContainerLogsTab).element as HTMLElement).style.display).toBe("");
    expect((wrapper.findComponent(ContainerFilesTab).element as HTMLElement).style.display).toBe("none");
    expect((wrapper.findComponent(ContainerTerminalTab).element as HTMLElement).style.display).toBe("none");
  });

  it("keeps the logs tab mounted while the files tab is shown", async () => {
    const wrapper = mount(ContainerDetailPane, { props: { t, container, activeTab: "logs" } });
    await wrapper.get('[data-tab="files"]').trigger("click");
    await wrapper.setProps({ activeTab: "files" });
    // 页签切换不卸载日志页：流不中断（停止只发生在容器切换/关闭/卸载）。
    expect(wrapper.findComponent(ContainerLogsTab).exists()).toBe(true);
    expect((wrapper.findComponent(ContainerLogsTab).element as HTMLElement).style.display).toBe("none");
    expect((wrapper.findComponent(ContainerFilesTab).element as HTMLElement).style.display).toBe("");
  });

  it("shows the container identity and emits close from the header", async () => {
    const wrapper = mount(ContainerDetailPane, { props: { t, container, activeTab: "files" } });
    expect(wrapper.text()).toContain("web-nginx");
    await wrapper.get(".container-detail-close").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });
});
