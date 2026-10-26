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

afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount();
  pluginStore.removeItem(DOCKER_ENGINE_STORE_KEY);
  vi.unstubAllGlobals();
});

describe("ContainerLogsTab streaming", () => {
  it("starts the stream on mount and renders chunks", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    expect(calls[0]).toMatchObject({
      method: "docker/logs/start",
      params: { sessionId: "sess-1", containerId: "abc123def456", tail: 200 },
    });
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "boot\nready\n" });
    await nextTick();
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
    await nextTick();
    await nextTick();
    expect(element.scrollTop).toBe(element.scrollHeight);
  });

  it("pauses follow when scrolled up and resumes from the follow button", async () => {
    stubHost();
    const wrapper = await mountTab();
    const element = sizeScroller(wrapper);
    element.scrollTop = 500;
    await wrapper.get(".docker-log-scroll").trigger("scroll");

    expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("false");
    expect(wrapper.text()).toContain("docker.followPaused");
    expect(wrapper.get('[data-action="follow"]').text()).toBe("docker.followResume");

    await wrapper.get('[data-action="follow"]').trigger("click");
    await nextTick();
    expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("true");
    expect(element.scrollTop).toBe(element.scrollHeight);
  });

  it("resumes automatically when the user scrolls back to the bottom", async () => {
    stubHost();
    const wrapper = await mountTab();
    const element = sizeScroller(wrapper);
    element.scrollTop = 500;
    await wrapper.get(".docker-log-scroll").trigger("scroll");
    expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("false");

    element.scrollTop = 1000;
    await wrapper.get(".docker-log-scroll").trigger("scroll");
    expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("true");
  });

  it("stops auto-scrolling from the button without stopping the stream", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    sizeScroller(wrapper);
    await wrapper.get('[data-action="follow"]').trigger("click");
    expect(wrapper.get('[data-action="follow"]').attributes("aria-pressed")).toBe("false");

    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "still-arriving\n" });
    await nextTick();
    expect(wrapper.get(".docker-log-scroll").text()).toBe("still-arriving");
    expect(calls.some((call) => call.method === "docker/logs/stop")).toBe(false);
  });

  it("keeps consuming while paused", async () => {
    stubHost();
    const wrapper = await mountTab();
    const element = sizeScroller(wrapper);
    element.scrollTop = 0;
    await wrapper.get(".docker-log-scroll").trigger("scroll");

    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "while-paused\n" });
    await nextTick();
    expect(wrapper.get(".docker-log-scroll").text()).toBe("while-paused");
    expect(element.scrollTop).toBe(0);
  });

  it("clears the view without stopping the stream", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "one\ntwo\n" });
    await nextTick();
    await wrapper.get('[data-action="clear"]').trigger("click");
    expect(wrapper.get(".docker-log-scroll").text()).toBe("");
    expect(calls.some((call) => call.method === "docker/logs/stop")).toBe(false);
  });

  it("restarts with the new tail and clears the buffer", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "old-output\n" });
    await nextTick();

    await wrapper.get('[data-action="tail"]').setValue("500");
    await flushPromises();
    await nextTick();

    expect(wrapper.get(".docker-log-scroll").text()).toBe("");
    expect(calls.some((call) => call.method === "docker/logs/stop" && call.params.streamId === "stream-1")).toBe(true);
    const starts = calls.filter((call) => call.method === "docker/logs/start");
    expect(starts.at(-1)?.params).toMatchObject({ containerId: "abc123def456", tail: 500 });
  });

  it("restarts the stream when the container changes", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "first\n" });
    await nextTick();

    await wrapper.setProps({ container: container({ id: "def456abc123", name: "web-next" }) });
    await flushPromises();

    expect(wrapper.get(".docker-log-scroll").text()).toBe("");
    expect(calls.some((call) => call.method === "docker/logs/stop" && call.params.streamId === "stream-1")).toBe(true);
    const starts = calls.filter((call) => call.method === "docker/logs/start");
    expect(starts.at(-1)?.params).toMatchObject({ containerId: "def456abc123" });
  });

  it("scrolls back to the bottom when the tab becomes visible again", async () => {
    stubHost();
    const wrapper = await mountTab({ visible: false });
    const element = sizeScroller(wrapper);
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "while-hidden\n" });
    await nextTick();
    // v-show 隐藏期间没有布局，滚动位置停在原处。
    element.scrollTop = 0;

    await wrapper.setProps({ visible: true });
    await nextTick();
    await nextTick();
    expect(element.scrollTop).toBe(element.scrollHeight);
  });

  it("does not show the no-output empty state while the stream is starting", async () => {
    vi.stubGlobal("dbxPlugin", {
      invoke: (method: string) => {
        if (method === "docker/logs/start") return new Promise(() => undefined);
        return Promise.resolve({ stopped: true });
      },
    });
    const wrapper = mount(ContainerLogsTab, { props: { t, container: container(), sessionId: "sess-1" } });
    wrappers.push(wrapper);
    await nextTick();
    expect(wrapper.find(".docker-log-empty").exists()).toBe(false);
  });

  it("shows the cleared hint in the empty state after clearing", async () => {
    stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "one\n" });
    await nextTick();
    await wrapper.get('[data-action="clear"]').trigger("click");
    expect(wrapper.get('[data-action="clear"]').text()).toBe("docker.logsClear");
    expect(wrapper.get(".docker-log-empty").text()).toBe("docker.logsCleared");
  });

  it("stops the stream on unmount", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    wrapper.unmount();
    wrappers.splice(wrappers.indexOf(wrapper), 1);
    expect(calls.some((call) => call.method === "docker/logs/stop" && call.params.streamId === "stream-1")).toBe(true);
  });
});

describe("ContainerLogsTab end banners", () => {
  it("keeps the log and offers a restart after a natural end", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "bye\n" });
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", exitCode: 137 });
    await nextTick();

    expect(wrapper.get(".docker-log-scroll").text()).toBe("bye");
    const banner = wrapper.get('[data-role="stream-end"]');
    expect(banner.text()).toContain("docker.streamEnded");
    expect(banner.text()).toContain("137");

    await wrapper.get('[data-action="restart"]').trigger("click");
    await flushPromises();
    await nextTick();
    expect(calls.filter((call) => call.method === "docker/logs/start")).toHaveLength(2);
    // 重新开始会清掉上一轮输出，避免 tail 行重复。
    expect(wrapper.get(".docker-log-scroll").text()).toBe("");
  });

  it("offers a sudo retry that restarts with sudo", async () => {
    const calls = stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", error: "permission denied", needsSudo: true });
    await nextTick();

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
    await nextTick();
    expect(wrapper.get('[data-action="sudo-retry"]').attributes("disabled")).toBeDefined();
  });

  it("shows a timeout error with the restart action", async () => {
    stubHost();
    const wrapper = await mountTab();
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", error: "stream timed out" });
    await nextTick();
    expect(wrapper.get('[data-role="stream-end"]').text()).toContain("stream timed out");
    expect(wrapper.find('[data-action="restart"]').exists()).toBe(true);
  });
});
