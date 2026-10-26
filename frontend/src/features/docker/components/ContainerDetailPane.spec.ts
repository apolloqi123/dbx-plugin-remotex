// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import ContainerDetailPane from "./ContainerDetailPane.vue";
import ContainerLogsTab from "./ContainerLogsTab.vue";
import ContainerFilesTab from "./ContainerFilesTab.vue";
import type { DockerContainer } from "../lib/dockerPanelModel";

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
    expect(wrapper.get('[data-tab="logs"]').text()).toBe("docker.logsTab");
    expect(wrapper.get('[data-tab="files"]').text()).toBe("docker.filesTab");
    await wrapper.get('[data-tab="files"]').trigger("click");
    expect(wrapper.emitted("update:activeTab")?.[0]).toEqual(["files"]);
    expect(wrapper.find('[data-tab="logs"]').exists()).toBe(true);
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
    // 两个页签都挂载（v-show），但只有活动页签可见。
    expect(wrapper.findComponent(ContainerFilesTab).props()).toMatchObject({ container, canWrite: false });
    expect((wrapper.findComponent(ContainerLogsTab).element as HTMLElement).style.display).toBe("");
    expect((wrapper.findComponent(ContainerFilesTab).element as HTMLElement).style.display).toBe("none");
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
