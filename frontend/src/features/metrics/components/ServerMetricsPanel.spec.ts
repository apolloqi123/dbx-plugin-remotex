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
      cpu: {
        cores: 4,
        percent: 25,
        load1: 0.1,
        load5: 0.2,
        load15: 0.3,
        perCore: [
          { index: 0, percent: 10 },
          { index: 3, percent: 90 },
        ],
      },
      memory: { totalBytes: 1024, usedBytes: 512, cacheBytes: 128, freeBytes: 512 },
      disks: [{ filesystem: "/dev/sda1", mount: "/", totalBytes: 2048, usedBytes: 1024, availableBytes: 1024, percentUsed: 50, inodeUsePercent: 12 }],
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
    disks: [{ filesystem: "/dev/sda1", mount: "/", totalBytes: 2048, usedBytes: 1024, availableBytes: 1024, percentUsed: 50, inodeUsePercent: 12 }],
    networks: [{ name: "eth0", rxRate: 10, txRate: 5, rxTotal: 100, txTotal: 50 }],
    showAllDisks: false,
    showAllNics: false,
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
    // Card dashboard sections.
    expect(wrapper.text()).toContain("metricsSystem");
    expect(wrapper.text()).toContain("metricsPerCore");
    expect(wrapper.text()).toContain("metricsUsed");
    expect(wrapper.text()).toContain("metricsInodes");
  });

  it("renders per-core bars and splits cache out of the used donut segment", async () => {
    const wrapper = mount(ServerMetricsPanel, { props: { view: view(), t, locale: "en-US" } });
    const cores = wrapper.findAll(".metric-core-row");
    expect(cores).toHaveLength(2);
    expect(cores[0].text()).toContain("10.0%");
    expect(cores[1].text()).toContain("90.0%");
    expect(cores[1].find(".metric-core-value").classes()).toContain("is-hot");
    // 512 used - 128 cache = 384 used; free = 1024 - 512 = 512.
    const legend = wrapper.findAll(".metric-memory-legend li").map((row) => row.text());
    expect(legend[0]).toContain("384 B");
    expect(legend[1]).toContain("128 B");
    expect(legend[2]).toContain("512 B");
  });

  it("puts one density toggle in each of the network and disk cards", async () => {
    const wrapper = mount(ServerMetricsPanel, { props: { view: view(), t, locale: "en-US" } });
    const toggles = wrapper.findAll(".metrics-density-toggle");
    expect(toggles).toHaveLength(2);
    expect(toggles[0].attributes("aria-label")).toBe("metricsShowAllNics");
    expect(toggles[1].attributes("aria-label")).toBe("metricsShowAllDisks");
    await toggles[0].trigger("click");
    await toggles[1].trigger("click");
    expect(wrapper.emitted("toggle-nics-density")).toHaveLength(1);
    expect(wrapper.emitted("toggle-disks-density")).toHaveLength(1);

    const all = mount(ServerMetricsPanel, {
      props: { view: view({ showAllDisks: true, showAllNics: true }), t, locale: "en-US" },
    });
    const active = all.findAll(".metrics-density-toggle");
    expect(active[0].attributes("aria-label")).toBe("metricsOnlyMainNics");
    expect(active[0].attributes("aria-pressed")).toBe("true");
    expect(active[0].classes()).toContain("is-active");
    expect(active[1].attributes("aria-label")).toBe("metricsOnlyMainDisks");
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
