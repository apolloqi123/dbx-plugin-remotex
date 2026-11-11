// @vitest-environment happy-dom
// 日志页的跟随交互：默认跟随、上滚暂停但流不断、按钮/回底恢复、tail 重启清空、
// 结束/权限不足横幅、容器切换与卸载收流。流由组件内部 useDockerLogStream 持有，
// 因此测试经真实 RPC stub + dispatchDockerLogEvent 驱动，不 mock composable。
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";
import ContainerLogsTab from "./ContainerLogsTab.vue";
import type { DockerContainer } from "../lib/dockerPanelModel";
import { dispatchDockerLogEvent } from "../lib/dockerLogEvents";
import { pluginStore } from "../../../shared/lib/pluginStore";
import { DOCKER_ENGINE_STORE_KEY, saveDockerEngineSettings } from "../lib/dockerEngine";
import { resetDockerChannelQueues } from "../lib/dockerChannelQueue";
import { resetDockerLogStreamRegistry } from "../lib/dockerLogStreamRegistry";

const t = (key: string) => key;

type Call = { method: string; params: Record<string, unknown> };
type MountedTab = ReturnType<typeof mount>;

const wrappers: MountedTab[] = [];

function stubHost(): Call[] {
  const calls: Call[] = [];
  let next = 1;
  vi.stubGlobal("dbxPlugin", {
    invoke: async (method: string, params: Record<string, unknown>) => {
      calls.push({ method, params });
      if (method === "docker/logs/start") return { streamId: `stream-${next++}` };
      if (method === "docker/logs/stop") return { stopped: true };
      if (method === "docker/logs/close") return { closed: true };
      throw new Error(`unexpected rpc ${method}`);
    },
  });
  return calls;
}

function container(overrides: Partial<DockerContainer> = {}): DockerContainer {
  return {
    id: "abc123def456",
    name: "web-nginx",
    image: "nginx:1.27",
    state: "running",
    status: "Up 3 minutes",
    ports: "8080->80/tcp",
    createdAt: "2026-10-26T00:00:00Z",
    ...overrides,
  };
}

async function mountTab(
  props: {
    container?: DockerContainer;
    canWrite?: boolean;
    sessionId?: string;
    connectionId?: string;
    visible?: boolean;
  } = {},
) {
  const wrapper = mount(ContainerLogsTab, {
    props: {
      t,
      container: props.container ?? container(),
      canWrite: props.canWrite,
      sessionId: props.sessionId ?? "sess-1",
      connectionId: props.connectionId,
      visible: props.visible,
    },
  });
  wrappers.push(wrapper);
  await flushPromises();
  return wrapper;
}

/** happy-dom 不做真实布局：给滚动容器注入固定尺寸。 */
function sizeScroller(wrapper: MountedTab, scrollHeight = 1000, clientHeight = 100): HTMLElement {
  const element = wrapper.get(".docker-log-scroll").element as HTMLElement;
  Object.defineProperty(element, "scrollHeight", { value: scrollHeight, configurable: true });
  Object.defineProperty(element, "clientHeight", { value: clientHeight, configurable: true });
  return element;
}

/** 日志渲染按 100ms 节拍合并：断言 DOM 前等一拍。 */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 120));
  await flushPromises();
  await nextTick();
}

afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount();
  resetDockerChannelQueues();
  resetDockerLogStreamRegistry();
  pluginStore.removeItem(DOCKER_ENGINE_STORE_KEY);
  vi.unstubAllGlobals();
});

describe("ContainerLogsTab streaming", () => {
  it("colors levels, timestamps and ANSI output without changing the copied text", async () => {
    stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", {
      streamId: "stream-1",
      text: "2026-10-29T12:00:00Z INFO booting\nERROR <script>boom</script>\n\u001b[31mred\u001b[0m plain\n",
    });
    await settle();

    const scroller = wrapper.get(".docker-log-scroll");
    expect(scroller.text()).toBe(
      "2026-10-29T12:00:00Z INFO booting\nERROR <script>boom</script>\nred plain",
    );
    expect(wrapper.find(".log-time").text()).toBe("2026-10-29T12:00:00Z");
    expect(wrapper.find(".log-level-info").text()).toBe("INFO");
    expect(wrapper.find(".log-level-error").text()).toBe("ERROR");
    expect(wrapper.find(".log-ansi-red").text()).toBe("red");
    // 日志里的尖括号按文本渲染，不允许注入标记。
    expect(scroller.html()).not.toContain("<script>");
  });

  it("appends rows incrementally without rebuilding existing lines", async () => {
    stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "first\nsecond\n" });
    await settle();
    const firstNode = wrapper.get(".docker-log-line").element;
    expect(wrapper.findAll(".docker-log-line")).toHaveLength(2);

    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "third\n" });
    await settle();
    expect(wrapper.findAll(".docker-log-line")).toHaveLength(3);
    expect(wrapper.get(".docker-log-line").element).toBe(firstNode);
    expect(wrapper.get(".docker-log-scroll").text()).toBe("first\nsecond\nthird");
  });

  it("renders only the newest window while following (1000-line buffer)", async () => {
    stubHost();
    const wrapper = await mountTab();
    const total = 10_050;
    const bulk = Array.from({ length: total }, (_, index) => `line-${index}`).join("\n") + "\n";
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: bulk });
    // DOM 只保留当前窗口（200 行）；1000 行缓冲供换窗回看。
    await settle();
    expect(wrapper.findAll(".docker-log-line")).toHaveLength(200);
    expect(wrapper.get(".docker-log-line").text()).toBe("line-9850");
    expect(wrapper.get(".docker-log-scroll").text()).not.toContain("line-0\n");
    expect(wrapper.find('[data-action="older"]').exists()).toBe(false);
  });

  it("keeps the render window and the scroll position while paused", async () => {
    stubHost();
    const wrapper = await mountTab();
    const element = sizeScroller(wrapper, 100_000, 100);
    element.scrollTop = 0;
    await wrapper.get(".docker-log-scroll").trigger("scroll");
    await vi.waitFor(() => expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("false"));

    const bulk = Array.from({ length: 1_500 }, (_, index) => `line-${index}`).join("\n") + "\n";
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: bulk });
    await settle();
    expect(wrapper.findAll(".docker-log-line")).toHaveLength(200);
    expect(wrapper.get(".docker-log-line").text()).toBe("line-500");
    expect(element.scrollTop).toBe(0);

    // 恢复跟随 → 跳回最新一屏。
    await wrapper.get('[data-action="follow"]').trigger("click");
    await settle();
    expect(wrapper.findAll(".docker-log-line")).toHaveLength(200);
    expect(wrapper.get(".docker-log-line").text()).toBe("line-1300");
    expect(element.scrollTop).toBe(element.scrollHeight);
  });

  it("slides the render window when scrolling to the top edge", async () => {
    stubHost();
    const wrapper = await mountTab();
    const bulk = Array.from({ length: 300 }, (_, index) => `line-${index}`).join("\n") + "\n";
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: bulk });
    await settle();
    // 跟随时窗口是最后 200 行（line-100..line-299）。
    expect(wrapper.get(".docker-log-line").text()).toBe("line-100");
    expect(wrapper.findAll(".docker-log-line")).toHaveLength(200);

    sizeScroller(wrapper, 10_000, 100);
    const element = wrapper.get(".docker-log-scroll").element as HTMLElement;
    element.scrollTop = 0;
    await wrapper.get(".docker-log-scroll").trigger("scroll");
    await vi.waitFor(() => expect(wrapper.get(".docker-log-line").text()).toBe("line-0"));
    expect(wrapper.findAll(".docker-log-line")).toHaveLength(200);
  });

  it("keeps plain text for lines without any color tokens", async () => {
    stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "just words\n" });
    await settle();
    expect(wrapper.get(".docker-log-scroll").text()).toBe("just words");
    expect(wrapper.find(".docker-log-scroll span").exists()).toBe(true);
  });

  it("starts the stream on mount and renders chunks", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    expect(calls[0]).toMatchObject({
      method: "docker/logs/start",
      params: { sessionId: "sess-1", containerId: "abc123def456", tail: 200 },
    });
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "boot\nready\n" });
    await settle();
    expect(wrapper.get(".docker-log-scroll").text()).toBe("boot\nready");
  });

  it("carries the per-connection engine settings into the start payload", async () => {
    saveDockerEngineSettings("conn-1", { cli: "podman", socket: "/run/podman.sock", host: "" });
    const calls = stubHost();
    await mountTab({ connectionId: "conn-1" });
    expect(calls[0]).toEqual({
      method: "docker/logs/start",
      params: {
        sessionId: "sess-1",
        owner: expect.any(String),
        containerId: "abc123def456",
        tail: 200,
        cli: "podman",
        socket: "/run/podman.sock",
      },
    });
  });

  it("follows by default and pins to the bottom when text arrives", async () => {
    stubHost();
    const wrapper = await mountTab();
    const element = sizeScroller(wrapper);
    expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("true");

    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "line-1\nline-2\n" });
    await settle();
    expect(element.scrollTop).toBe(element.scrollHeight);
  });

  it("pauses follow when scrolled up and resumes from the follow button", async () => {
    stubHost();
    const wrapper = await mountTab();
    const element = sizeScroller(wrapper);
    element.scrollTop = 500;
    await wrapper.get(".docker-log-scroll").trigger("scroll");
    await vi.waitFor(() => expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("false"));
    expect(wrapper.text()).toContain("docker.followPaused");
    expect(wrapper.get('[data-action="follow"]').text()).toBe("docker.followResume");

    await wrapper.get('[data-action="follow"]').trigger("click");
    await settle();
    expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("true");
    expect(element.scrollTop).toBe(element.scrollHeight);
  });

  it("resumes automatically when the user scrolls back to the bottom", async () => {
    stubHost();
    const wrapper = await mountTab();
    const element = sizeScroller(wrapper);
    element.scrollTop = 500;
    await wrapper.get(".docker-log-scroll").trigger("scroll");
    await vi.waitFor(() => expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("false"));

    element.scrollTop = 1000;
    await wrapper.get(".docker-log-scroll").trigger("scroll");
    await vi.waitFor(() => expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("true"));
  });

  it("stops auto-scrolling from the button without stopping the stream", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    sizeScroller(wrapper);
    await wrapper.get('[data-action="follow"]').trigger("click");
    expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("false");

    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "still-arriving\n" });
    await settle();
    expect(wrapper.get(".docker-log-scroll").text()).toBe("still-arriving");
    expect(calls.some((call) => call.method === "docker/logs/stop")).toBe(false);
  });

  it("keeps consuming while paused", async () => {
    stubHost();
    const wrapper = await mountTab();
    const element = sizeScroller(wrapper);
    element.scrollTop = 0;
    await wrapper.get(".docker-log-scroll").trigger("scroll");
    await vi.waitFor(() => expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("false"));

    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "while-paused\n" });
    await settle();
    expect(wrapper.get(".docker-log-scroll").text()).toBe("while-paused");
    expect(element.scrollTop).toBe(0);
  });

  it("clears the view without stopping the stream", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "one\ntwo\n" });
    await settle();
    await wrapper.get('[data-action="clear"]').trigger("click");
    expect(wrapper.get(".docker-log-scroll").text()).toBe("");
    expect(calls.some((call) => call.method === "docker/logs/stop")).toBe(false);
  });

  it("restarts with the new tail and clears the buffer", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "old-output\n" });
    await settle();

    await wrapper.get('[data-action="tail"]').setValue("500");
    await flushPromises();
    await settle();

    expect(wrapper.get(".docker-log-scroll").text()).toBe("");
    expect(calls.some((call) => call.method === "docker/logs/stop" && call.params.streamId === "stream-1")).toBe(true);
    const starts = calls.filter((call) => call.method === "docker/logs/start");
    expect(starts.at(-1)?.params).toMatchObject({ containerId: "abc123def456", tail: 500 });
  });

  it("restarts the stream when the container changes", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "first\n" });
    await settle();

    await wrapper.setProps({ container: container({ id: "def456abc123", name: "web-next" }) });
    await flushPromises();

    expect(wrapper.get(".docker-log-scroll").text()).toBe("");
    expect(calls.some((call) => call.method === "docker/logs/stop" && call.params.streamId === "stream-1")).toBe(true);
    const starts = calls.filter((call) => call.method === "docker/logs/start");
    expect(starts.at(-1)?.params).toMatchObject({ containerId: "def456abc123" });
  });

  it("stops the remote stream after the DBX window stays hidden", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    expect(calls.filter((call) => call.method === "docker/logs/start")).toHaveLength(1);

    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    try {
      vi.useFakeTimers();
      document.dispatchEvent(new Event("visibilitychange"));
      await vi.advanceTimersByTimeAsync(10_000);
      await flushPromises();
      // 后台超过宽限期：立即收掉远端 docker logs -f，避免服务器 CPU 被常驻跟随占用。
      expect(calls.some((call) => call.method === "docker/logs/stop" && call.params.streamId === "stream-1")).toBe(true);

      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
      document.dispatchEvent(new Event("visibilitychange"));
      await flushPromises();
      const starts = calls.filter((call) => call.method === "docker/logs/start");
      expect(starts).toHaveLength(2);
      expect(starts.at(-1)?.params).toMatchObject({ containerId: "abc123def456", tail: 200 });
    } finally {
      vi.useRealTimers();
      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    }
  });

  it("does not open a log channel while the tab is hidden", async () => {
    const calls = stubHost();
    await mountTab({ visible: false });
    expect(calls.some((call) => call.method === "docker/logs/start")).toBe(false);
  });

  it("stops following while hidden and restarts when visible again", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "visible\n" });
    await settle();

    await wrapper.setProps({ visible: false });
    await flushPromises();
    expect(calls.some((call) => call.method === "docker/logs/stop" && call.params.streamId === "stream-1")).toBe(true);

    await wrapper.setProps({ visible: true });
    await flushPromises();
    const starts = calls.filter((call) => call.method === "docker/logs/start");
    expect(starts).toHaveLength(2);
    expect(starts.at(-1)?.params).toMatchObject({ containerId: "abc123def456", tail: 200 });
    // 恢复时重新跟随最新日志：旧缓冲不重复展示。
    expect(wrapper.get(".docker-log-scroll").text()).toBe("");
  });

  it("defers a hidden container switch until the tab is visible again", async () => {
    const calls = stubHost();
    const wrapper = await mountTab({ visible: false });
    await wrapper.setProps({ container: container({ id: "def456abc123", name: "web-next" }) });
    await flushPromises();
    expect(calls.some((call) => call.method === "docker/logs/start")).toBe(false);

    await wrapper.setProps({ visible: true });
    await flushPromises();
    const starts = calls.filter((call) => call.method === "docker/logs/start");
    expect(starts.at(-1)?.params).toMatchObject({ containerId: "def456abc123" });
  });

  it("scrolls back to the bottom when the tab becomes visible again", async () => {
    stubHost();
    const wrapper = await mountTab({ visible: false });
    const element = sizeScroller(wrapper);
    await wrapper.setProps({ visible: true });
    await flushPromises();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "while-hidden\n" });
    await settle();
    element.scrollTop = 0;
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "more\n" });
    await settle();
    expect(element.scrollTop).toBe(element.scrollHeight);
  });

  it("does not show the no-output empty state while the stream is starting", async () => {
    let resolveStart: ((value: unknown) => void) | undefined;
    vi.stubGlobal("dbxPlugin", {
      invoke: (method: string) => {
        if (method === "docker/logs/start") {
          return new Promise((resolve) => {
            resolveStart = resolve;
          });
        }
        return Promise.resolve({ stopped: true });
      },
    });
    const wrapper = mount(ContainerLogsTab, { props: { t, container: container(), sessionId: "sess-1" } });
    wrappers.push(wrapper);
    await settle();
    expect(wrapper.find(".docker-log-empty").exists()).toBe(false);
    // 释放连接级队列，避免后续用例排在未完成的 start 后面。
    resolveStart?.({ streamId: "stream-starting" });
    await flushPromises();
  });

  it("shows the cleared hint in the empty state after clearing", async () => {
    stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "one\n" });
    await settle();
    await wrapper.get('[data-action="clear"]').trigger("click");
    expect(wrapper.get('[data-action="clear"]').text()).toBe("docker.logsClear");
    expect(wrapper.get(".docker-log-empty").text()).toBe("docker.logsCleared");
  });

  it("stops the stream on unmount", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    wrapper.unmount();
    wrappers.splice(wrappers.indexOf(wrapper), 1);
    // 收流 RPC 直连（不排连接队列），等它落地后再断言。
    await vi.waitFor(() =>
      expect(calls.some((call) => call.method === "docker/logs/stop" && call.params.streamId === "stream-1")).toBe(true),
    );
  });

  it("closes the owner scope when unmounted while the start is still in flight", async () => {
    const calls: Call[] = [];
    let resolveStart: ((value: unknown) => void) | undefined;
    vi.stubGlobal("dbxPlugin", {
      invoke: (method: string, params: Record<string, unknown>) => {
        calls.push({ method, params });
        if (method === "docker/logs/start") {
          return new Promise((resolve) => {
            resolveStart = resolve;
          });
        }
        return Promise.resolve({ stopped: true, closed: true });
      },
    });
    const wrapper = mount(ContainerLogsTab, { props: { t, container: container(), sessionId: "sess-1" } });
    wrappers.push(wrapper);
    await nextTick();

    wrapper.unmount();
    wrappers.splice(wrappers.indexOf(wrapper), 1);
    // 还没拿到 streamId：按 owner 收口，后端会拒绝这条迟到的 start。
    await vi.waitFor(() =>
      expect(calls).toContainEqual({
        method: "docker/logs/close",
        params: { owner: calls[0].params.owner },
      }),
    );

    resolveStart?.({ streamId: "late-stream" });
    await flushPromises();
    // 本实例已卸载且 start 晚了：响应回来时仍必须把后端流收掉。
    await vi.waitFor(() =>
      expect(calls).toContainEqual({ method: "docker/logs/stop", params: { streamId: "late-stream" } }),
    );
  });
});

describe("ContainerLogsTab end banners", () => {
  it("keeps the log and offers a restart after a natural end", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "bye\n" });
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", exitCode: 137 });
    await settle();

    expect(wrapper.get(".docker-log-scroll").text()).toBe("bye");
    const banner = wrapper.get('[data-role="stream-end"]');
    expect(banner.text()).toContain("docker.streamEnded");
    expect(banner.text()).toContain("137");
    // 非零退出才是失败：横幅用红色语义档，退出码同时标红。
    expect(banner.classes()).toContain("is-error");
    expect(wrapper.get(".docker-log-exit").classes()).toContain("is-failed");

    await wrapper.get('[data-action="restart"]').trigger("click");
    await flushPromises();
    await settle();
    expect(calls.filter((call) => call.method === "docker/logs/start")).toHaveLength(2);
    // 重新开始会清掉上一轮输出，避免 tail 行重复。
    expect(wrapper.get(".docker-log-scroll").text()).toBe("");
  });

  it("does not style a clean exit as an error", async () => {
    stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", exitCode: 0 });
    await settle();

    const banner = wrapper.get('[data-role="stream-end"]');
    expect(banner.classes()).not.toContain("is-error");
    expect(banner.classes()).not.toContain("is-warning");
    expect(wrapper.get(".docker-log-exit").classes()).not.toContain("is-failed");
    // 正常结束仍保留重开入口，只是不再用告警色。
    expect(wrapper.find('[data-action="restart"]').exists()).toBe(true);
  });

  it("offers a sudo retry that restarts with sudo", async () => {    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", error: "permission denied", needsSudo: true });
    await settle();

    const banner = wrapper.get('[data-role="stream-end"]');
    expect(banner.text()).toContain("permission denied");
    const sudoButton = wrapper.get('[data-action="sudo-retry"]');
    expect(sudoButton.text()).toBe("docker.sudoRetry");
    expect(sudoButton.attributes("disabled")).toBeUndefined();

    await sudoButton.trigger("click");
    await flushPromises();
    expect(calls.filter((call) => call.method === "docker/logs/start").at(-1)?.params).toMatchObject({
      containerId: "abc123def456",
      sudo: true,
    });
  });

  it("disables the sudo retry on a read-only connection", async () => {
    stubHost();
    const wrapper = await mountTab({ canWrite: false });
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", needsSudo: true });
    await settle();
    expect(wrapper.get('[data-action="sudo-retry"]').attributes("disabled")).toBeDefined();
  });

  it("shows a timeout error with the restart action", async () => {
    stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", error: "stream timed out" });
    await settle();
    expect(wrapper.get('[data-role="stream-end"]').text()).toContain("stream timed out");
    expect(wrapper.find('[data-action="restart"]').exists()).toBe(true);
  });
});
