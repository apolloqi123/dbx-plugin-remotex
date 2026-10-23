// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import ServerMetricsPanel, { type ServerMetricsView } from "./ServerMetricsPanel.vue";

const t = (key: string, values?: Record<string, string | number>) => {
  if (key === "procTitle") return `processes:${values?.count}`;
  return key;
};

function view(overrides: Partial<ServerMetricsView> = {}): ServerMetricsView {
  return {
    metrics: {
      hostname: "srv",
      cpu: { cores: 4, percent: 25, load1: 0.1, load5: 0.2, load15: 0.3 },
      memory: { totalBytes: 1024, usedBytes: 512 },
      disks: [{ filesystem: "/dev/sda1", mount: "/", totalBytes: 2048, usedBytes: 1024, availableBytes: 1024, percentUsed: 50 }],
      network: [{ name: "eth0", rxRate: 10, txRate: 5, rxTotal: 100, txTotal: 50 }],
      processes: [{ pid: 1, user: "root", cpuPercent: 2, memPercent: 3, command: "systemd" }],
    },
    loading: false,
    error: "",
    distroBadge: null,
    cpuSparkline: "",
    memSparkline: "",
    rxSparkline: "",
    txSparkline: "",
    cpuLast: 25,
    memLast: 50,
    disks: [{ filesystem: "/dev/sda1", mount: "/", totalBytes: 2048, usedBytes: 1024, availableBytes: 1024, percentUsed: 50 }],
    networks: [{ name: "eth0", rxRate: 10, txRate: 5, rxTotal: 100, txTotal: 50 }],
    processesOpen: true,
    processRows: [{ pid: 1, ppid: 0, user: "root", cpuPercent: 2, memPercent: 3, etime: "1d", state: "S", command: "systemd" }],
    processLoading: false,
    processSortKey: "cpu",
    sortedProcessTotal: 1,
    visibleProcessRows: [{ pid: 1, ppid: 0, user: "root", cpuPercent: 2, memPercent: 3, etime: "1d", state: "S", command: "systemd" }],
    ...overrides,
  };
}

describe("ServerMetricsPanel", () => {
  it("renders overview, disk, network and process rows", () => {
    const wrapper = mount(ServerMetricsPanel, { props: { view: view(), t, locale: "en-US" } });
    expect(wrapper.text()).toContain("25%");
    expect(wrapper.text()).toContain("/");
    expect(wrapper.text()).toContain("eth0");
    expect(wrapper.text()).toContain("systemd");
    expect(wrapper.text()).toContain("processes:1");
  });

  it("forwards refresh, sort and kill actions", async () => {
    const failed = mount(ServerMetricsPanel, {
      props: { view: view({ metrics: undefined, error: "boom" }), t, locale: "en-US" },
    });
    await failed.findAll("button").find((button) => button.text() === "refresh")!.trigger("click");
    expect(failed.emitted("refresh")).toHaveLength(1);

    const wrapper = mount(ServerMetricsPanel, { props: { view: view(), t, locale: "en-US" } });
    const sort = wrapper.findAll('input[type="radio"]').find((input) => (input.element as HTMLInputElement).value === "mem")!;
    await sort.trigger("change");
    expect(wrapper.emitted("update:sort")?.[0]).toEqual(["mem"]);

    await wrapper.findAll("button").find((button) => button.text() === "procKill")!.trigger("click");
    expect(wrapper.emitted("kill-process")?.[0]?.[1]).toBe(15);
  });
});
