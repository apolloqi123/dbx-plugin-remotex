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
    await wrapper.get('[data-tab="files"]').trigger("click");
    expect(wrapper.emitted("update:activeTab")?.[0]).toEqual(["files"]);
    expect(wrapper.find('[data-tab="logs"]').exists()).toBe(true);
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
    expect(wrapper.findComponent(ContainerFilesTab).exists()).toBe(false);
  });

  it("shows the container identity and emits close from the header", async () => {
    const wrapper = mount(ContainerDetailPane, { props: { t, container, activeTab: "files" } });
    expect(wrapper.text()).toContain("web-nginx");
    await wrapper.get(".container-detail-close").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });
});
