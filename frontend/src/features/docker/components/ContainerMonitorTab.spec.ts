// @vitest-environment happy-dom
// 监控页签：真实 useContainerStats + 宿主桩。覆盖首轮采样与 3s 轮询、页签隐藏
// 停轮询、时间段切换、停止/无权限/失败三种空态与重试。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ContainerMonitorTab from "./ContainerMonitorTab.vue";
import type { DockerContainer } from "../lib/dockerPanelModel";

const t = (key: string, values?: Record<string, string | number>) =>
  values ? `${key}:${JSON.stringify(values)}` : key;

function container(patch: Partial<DockerContainer> = {}): DockerContainer {
  return {
    id: "d4a7c9f1e2b3a1c0d4e5f6",
    name: "web-nginx",
    image: "nginx:1.27",
    state: "running",
    status: "Up 3 days",
    ports: "0.0.0.0:8080->80/tcp",
    createdAt: "2026-09-25 08:15:04 +0000 UTC",
    ...patch,
  };
}

function runningPayload(patch: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    available: true,
    needsSudo: false,
    running: true,
    cpuPercent: 12.5,
    memPercent: 3.2,
    memUsedBytes: 210_000_000,
    memLimitBytes: 8_160_437_862,
    netRxBytes: 1_200_000,
    netTxBytes: 400_000,
    blockReadBytes: 2_000_000,
    blockWriteBytes: 800_000,
    pids: 7,
    ...patch,
  };
}

type Call = { method: string; params: Record<string, unknown> };

function stubHost(handler: (call: Call, index: number) => unknown): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal("dbxPlugin", {
    invoke: async (method: string, params: Record<string, unknown>) => {
      const call = { method, params: params ?? {} };
      calls.push(call);
      return handler(call, calls.length - 1);
    },
  });
  return calls;
}

async function mountTab(options: {
  rows?: DockerContainer;
  visible?: boolean;
  sessionId?: string;
} = {}) {
  const wrapper = mount(ContainerMonitorTab, {
    props: {
      t,
      container: options.rows ?? container(),
      visible: options.visible ?? true,
      sessionId: options.sessionId ?? "sess-1",
    },
  });
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("ContainerMonitorTab", () => {
  it("samples immediately and every 3 seconds while visible", async () => {
    const calls = stubHost((_call, index) => runningPayload({ netRxBytes: 1_200_000 + index * 90_000, netTxBytes: 400_000 + index * 30_000 }));
    const wrapper = await mountTab();
    try {
      expect(calls.filter((call) => call.method === "docker/stats")).toHaveLength(1);
      expect(calls[0].params).toMatchObject({ containerId: "d4a7c9f1e2b3a1c0d4e5f6", sessionId: "sess-1" });

      await vi.advanceTimersByTimeAsync(3_000);
      await flushPromises();
      expect(calls.filter((call) => call.method === "docker/stats")).toHaveLength(2);

      const cpu = wrapper.get('[data-metric="cpu"]');
      expect(cpu.text()).toContain("12.5%");
      const network = wrapper.get('[data-metric="network"]');
      expect(network.text()).toContain("↓");
      const memory = wrapper.get('[data-metric="memory"]');
      expect(memory.text()).toContain("200.3 MiB"); // formatBytes(210_000_000)
    } finally {
      wrapper.unmount();
    }
  });

  it("draws y-axis ticks on all three charts", async () => {
    let sampled = 0;
    stubHost((call) =>
      call.method === "docker/stats/limits"
        ? { hostCpus: 8, cpuLimitCores: null }
        : runningPayload({
            netRxBytes: 1_200_000 + sampled * 90_000,
            netTxBytes: 400_000 + (sampled++) * 45_000,
          }),
    );
    const wrapper = await mountTab();
    try {
      await vi.advanceTimersByTimeAsync(3_000);
      await flushPromises();
      const axes = wrapper.findAll(".metric-trend-yaxis");
      expect(axes).toHaveLength(3);
      const labels = (index: number) => axes[index].findAll("span").map((span) => span.text());
      expect(labels(0)).toEqual(["100%", "50%", "0%"]);
      expect(labels(1)).toEqual(["100%", "50%", "0%"]);
      // 90 KB / 3s = 30 KB/s，取整到 30 KiB/s 档；中间刻度减半，底部归零。
      expect(labels(2)).toEqual(["30.0 KiB/s", "15.0 KiB/s", "0 B/s"]);

      // 上下行共用 30 KiB/s 上限：30 KB/s 顶到顶边，15 KB/s 落在中线。
      const lines = wrapper.get('[data-metric="network"]').findAll("polyline");
      const lastY = (index: number) =>
        Number(lines[index].attributes("points")!.trim().split(" ").at(-1)!.split(",")[1]);
      expect(lastY(0)).toBe(0);
      expect(lastY(1)).toBe(9);
    } finally {
      wrapper.unmount();
    }
  });

  it("falls back to the local daemon without a session id", async () => {
    const calls = stubHost(() => runningPayload());
    const wrapper = mount(ContainerMonitorTab, { props: { t, container: container(), visible: true } });
    await flushPromises();
    try {
      expect(calls[0].params).toMatchObject({ target: "local" });
      expect(calls[0].params).not.toHaveProperty("sessionId");
    } finally {
      wrapper.unmount();
    }
  });

  it("stops polling while the tab is hidden", async () => {
    const calls = stubHost(() => runningPayload());
    const wrapper = await mountTab({ visible: false });
    try {
      expect(calls).toHaveLength(0);
      await vi.advanceTimersByTimeAsync(9_000);
      expect(calls).toHaveLength(0);
    } finally {
      wrapper.unmount();
    }
  });

  it("keeps samples when the same container snapshot is refreshed", async () => {
    const calls = stubHost(() => runningPayload());
    const wrapper = await mountTab();
    try {
      await vi.advanceTimersByTimeAsync(3_000);
      await flushPromises();
      const sampled = calls.filter((call) => call.method === "docker/stats").length;

      // 列表轮询替换同 id 容器对象（status 变化）：不能清空曲线或立刻重采。
      await wrapper.setProps({ container: { ...container(), status: "Up 4 minutes" } });
      await flushPromises();

      expect(calls.filter((call) => call.method === "docker/stats")).toHaveLength(sampled);
      expect(wrapper.find('[data-metric="cpu"]').exists()).toBe(true);
    } finally {
      wrapper.unmount();
    }
  });

  it("keeps collected samples across tab visibility changes", async () => {
    const calls = stubHost(() => runningPayload());
    const wrapper = await mountTab();
    try {
      await vi.advanceTimersByTimeAsync(3_000);
      await flushPromises();
      const sampled = calls.filter((call) => call.method === "docker/stats").length;
      expect(sampled).toBe(2);
      expect(wrapper.find('[data-metric="cpu"]').exists()).toBe(true);

      await wrapper.setProps({ visible: false });
      await vi.advanceTimersByTimeAsync(9_000);
      expect(calls.filter((call) => call.method === "docker/stats")).toHaveLength(sampled);

      // 切回监控页：保留原采样，立即补一拍，不回到“采集不到数据”的空态。
      await wrapper.setProps({ visible: true });
      await flushPromises();
      expect(calls.filter((call) => call.method === "docker/stats")).toHaveLength(sampled + 1);
      expect(wrapper.find('[data-metric="cpu"]').exists()).toBe(true);
    } finally {
      wrapper.unmount();
    }
  });

  it("shows CPU in millicores below one core and in cores above", async () => {
    const calls = stubHost((_call, index) =>
      runningPayload({ cpuPercent: index === 0 ? 12.5 : 157.2 }),
    );
    const wrapper = await mountTab();
    try {
      const cpu = () => wrapper.get('[data-metric="cpu"]');
      expect(cpu().text()).toContain("125m");
      expect(cpu().text()).toContain("12.5%");

      await vi.advanceTimersByTimeAsync(3_000);
      await flushPromises();
      expect(cpu().text()).toContain("1.57");
      expect(cpu().text()).toContain("157.2%");
    } finally {
      wrapper.unmount();
    }
    expect(calls.filter((call) => call.method === "docker/stats")).toHaveLength(2);
  });

  it("shows usage over the host total when the container is unlimited", async () => {
    const calls = stubHost((call) =>
      call.method === "docker/stats/limits"
        ? { hostCpus: 16, cpuLimitCores: null }
        : runningPayload({ cpuPercent: 12.5 }),
    );
    const wrapper = await mountTab();
    await flushPromises();
    try {
      const cpu = wrapper.get('[data-metric="cpu"]');
      expect(cpu.text()).toContain("125m");
      expect(cpu.text()).toContain('"value":"16"');
      // 与内存卡一致：头部是百分比，底部是“用量 / 总量”；进程数退到标题旁。
      // 头部百分比按 16 核总容量折算：12.5% 单核 → 0.8%。
      expect(cpu.get(".container-monitor-card-head").text()).toContain("0.8%");
      expect(cpu.get(".container-monitor-card-head").text()).toContain("docker.monitorPids");
      expect(cpu.get(".container-monitor-card-value").attributes("title")).toContain(
        "docker.monitorCpuPercentHint",
      );
      expect(cpu.get(".container-monitor-card-value").attributes("title")).toContain('"total":"16"');
      expect(cpu.get(".container-monitor-card-value").attributes("title")).toContain('"percent":"12.5%"');
      expect(cpu.get(".container-monitor-card-foot").text()).toContain("125m");
      expect(cpu.get(".container-monitor-card-foot .mono").attributes("title")).toContain(
        "docker.monitorCpuHostHint",
      );
      const limitCalls = () => calls.filter((call) => call.method === "docker/stats/limits");
      expect(limitCalls()).toHaveLength(1);
      expect(limitCalls()[0].params).toMatchObject({
        containerId: "d4a7c9f1e2b3a1c0d4e5f6",
        sessionId: "sess-1",
      });

      // 总量只查一次，不跟着 3s 采样重复 info/inspect。
      await vi.advanceTimersByTimeAsync(6_000);
      expect(limitCalls()).toHaveLength(1);
    } finally {
      wrapper.unmount();
    }
  });

  it("shows the configured CPU limit instead of the host total", async () => {
    stubHost((call) =>
      call.method === "docker/stats/limits"
        ? { hostCpus: 16, cpuLimitCores: 2 }
        : runningPayload({ cpuPercent: 157.2 }),
    );
    const wrapper = await mountTab();
    await flushPromises();
    try {
      const cpu = wrapper.get('[data-metric="cpu"]');
      expect(cpu.text()).toContain("1.57");
      expect(cpu.text()).toContain('"value":"2"');
      expect(cpu.text()).not.toContain('"value":"16"');
      // 157.2% 是 Docker 单核口径；相对 2 核限制是 78.6%。
      expect(cpu.get(".container-monitor-card-head").text()).toContain("78.6%");
      expect(cpu.get(".container-monitor-card-value").attributes("title")).toContain(
        "docker.monitorCpuPercentHint",
      );
      expect(cpu.get(".container-monitor-card-value").attributes("title")).toContain('"total":"2"');
      expect(cpu.get(".container-monitor-card-foot").text()).toContain("1.57");
      expect(cpu.get(".container-monitor-card-foot .mono").attributes("title")).toContain(
        "docker.monitorCpuLimitHint",
      );
    } finally {
      wrapper.unmount();
    }
  });

  it("shows the share of all cores instead of Docker's per-core percentage", async () => {
    stubHost((call) =>
      call.method === "docker/stats/limits"
        ? { hostCpus: 7, cpuLimitCores: null }
        : runningPayload({ cpuPercent: 102.5 }),
    );
    const wrapper = await mountTab();
    await flushPromises();
    try {
      const cpu = wrapper.get('[data-metric="cpu"]');
      // 7 核上跑约 1.02 核 = 14.6%，头部不再出现 >100% 的“超载”错觉。
      expect(cpu.get(".container-monitor-card-head").text()).toContain("14.6%");
      expect(cpu.get(".container-monitor-card-foot").text()).toContain("1.02");
      expect(cpu.get(".container-monitor-card-foot").text()).toContain('"value":"7"');
      expect(cpu.get(".container-monitor-card-value").attributes("title")).toContain('"percent":"102.5%"');
      expect(cpu.get(".metric-trend-yaxis").findAll("span").map((span) => span.text())).toEqual([
        "100%",
        "50%",
        "0%",
      ]);

      await vi.advanceTimersByTimeAsync(3_000);
      await flushPromises();
      // 曲线按占比绘制：14.6% 落在约 15/18 的高度，而不是顶死在 100%。
      const points = cpu.get(".metric-trend polyline").attributes("points")!.trim().split(" ");
      expect(Number(points.at(-1)!.split(",")[1])).toBe(15);
    } finally {
      wrapper.unmount();
    }
  });

  it("keeps a readable CPU scale when the core total is unknown", async () => {
    stubHost((call) => (call.method === "docker/stats/limits" ? {} : runningPayload({ cpuPercent: 157.2 })));
    const wrapper = await mountTab();
    await flushPromises();
    try {
      const cpu = wrapper.get('[data-metric="cpu"]');
      expect(cpu.get(".container-monitor-card-head").text()).toContain("157.2%");
      expect(cpu.get(".container-monitor-card-value").attributes("title")).toContain(
        "docker.monitorCpuRawHint",
      );
      expect(cpu.get(".metric-trend-yaxis").findAll("span").map((span) => span.text())).toEqual([
        "160%",
        "80%",
        "0%",
      ]);
    } finally {
      wrapper.unmount();
    }
  });

  it("switches the visible range without new requests", async () => {
    const calls = stubHost(() => runningPayload());
    const wrapper = await mountTab();
    try {
      const before = calls.length;
      const ranges = wrapper.findAll(".container-monitor-range");
      expect(ranges).toHaveLength(4);
      expect(ranges[1].classes()).toContain("is-active");
      await ranges[0].trigger("click");
      expect(ranges[0].classes()).toContain("is-active");
      expect(ranges[1].classes()).not.toContain("is-active");
      expect(calls.length).toBe(before);
    } finally {
      wrapper.unmount();
    }
  });

  it("shows the stopped state without further polling", async () => {
    const calls = stubHost(() => ({ available: true, needsSudo: false, running: false }));
    const wrapper = await mountTab({ rows: container({ state: "exited" }) });
    try {
      // running()=false → 不发请求，直接展示停止态。
      expect(calls).toHaveLength(0);
      expect(wrapper.get('[data-role="monitor-empty"]').text()).toContain("docker.monitorStopped");
    } finally {
      wrapper.unmount();
    }
  });

  it("shows the sudo hint when the daemon denies access", async () => {
    stubHost(() => ({ available: false, needsSudo: true, running: false }));
    const wrapper = await mountTab();
    try {
      const empty = wrapper.get('[data-role="monitor-empty"]');
      expect(empty.text()).toContain("docker.monitorDenied");
      expect(empty.text()).toContain("docker.monitorDeniedHint");
    } finally {
      wrapper.unmount();
    }
  });

  it("pauses sampling while the document is hidden and resumes when visible", async () => {
    const calls = stubHost(() => runningPayload());
    const wrapper = await mountTab();
    try {
      const statsCalls = () => calls.filter((call) => call.method === "docker/stats").length;
      expect(statsCalls()).toBe(1);

      Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
      document.dispatchEvent(new Event("visibilitychange"));
      await vi.advanceTimersByTimeAsync(9_000);
      expect(statsCalls()).toBe(1);

      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
      document.dispatchEvent(new Event("visibilitychange"));
      await flushPromises();
      expect(statsCalls()).toBe(2);
      await vi.advanceTimersByTimeAsync(3_000);
      await flushPromises();
      expect(statsCalls()).toBe(3);
    } finally {
      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
      wrapper.unmount();
    }
  });

  it("offers a retry after a failed sample", async () => {
    let fail = true;
    stubHost(() => {
      if (fail) throw new Error("stats timeout");
      return runningPayload();
    });
    const wrapper = await mountTab();
    try {
      expect(wrapper.get('[data-role="monitor-empty"]').text()).toContain("docker.monitorFailed");
      fail = false;
      await wrapper.get('[data-action="monitor-retry"]').trigger("click");
      await flushPromises();
      expect(wrapper.find('[data-metric="cpu"]').exists()).toBe(true);
    } finally {
      wrapper.unmount();
    }
  });
});
