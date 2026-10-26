// @vitest-environment happy-dom
// 容器终端页（PTY）：页签可见且容器运行/可写时才建立 PTY；隐藏期间保持连接；
// 容器切换复位 xterm 并重开；PTY 字节写入 xterm、按键经输入通道下发；
// state 事件驱动结束/错误覆盖层与重连；卸载关闭 PTY 并销毁 xterm。
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { nextTick } from "vue";
import ContainerTerminalTab from "./ContainerTerminalTab.vue";
import type { ContainerTerminalStateEvent } from "../composables/useContainerTerminal";
import type { DockerContainer } from "../lib/dockerPanelModel";

const TERMINAL_ID = "term-12345678";

const { MockTerminal } = vi.hoisted(() => {
  class MockTerminal {
    cols = 80;
    rows = 24;
    options: Record<string, unknown>;
    unicode = { activeVersion: "" };
    writes: Uint8Array[] = [];
    dataHandler?: (data: string) => void;
    resizeHandler?: (size: { cols: number; rows: number }) => void;
    resets = 0;
    focused = 0;
    disposed = false;
    static instances: MockTerminal[] = [];

    constructor(options: Record<string, unknown>) {
      this.options = { ...options };
      MockTerminal.instances.push(this);
    }

    loadAddon(): void {}

    open(host: HTMLElement): void {
      host.dataset.xterm = "ready";
    }

    write(data: Uint8Array): void {
      this.writes.push(data);
    }

    onData(handler: (data: string) => void): { dispose(): void } {
      this.dataHandler = handler;
      return { dispose() {} };
    }

    onResize(handler: (size: { cols: number; rows: number }) => void): { dispose(): void } {
      this.resizeHandler = handler;
      return { dispose() {} };
    }

    attachCustomKeyEventHandler(): void {}

    dispose(): void {
      this.disposed = true;
    }

    focus(): void {
      this.focused += 1;
    }

    reset(): void {
      this.resets += 1;
    }

    hasSelection(): boolean {
      return false;
    }

    static latest(): MockTerminal | undefined {
      return MockTerminal.instances.at(-1);
    }

    static reset(): void {
      MockTerminal.instances.length = 0;
    }
  }
  return { MockTerminal };
});

vi.mock("@xterm/xterm", () => ({ Terminal: MockTerminal }));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit(): void {}
    proposeDimensions(): { cols: number; rows: number } {
      return { cols: 80, rows: 24 };
    }
    dispose(): void {}
  },
}));
vi.mock("@xterm/addon-unicode11", () => ({ Unicode11Addon: class { dispose(): void {} } }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class { dispose(): void {} } }));

interface BinaryEvent {
  channel: string;
  data?: Uint8Array;
  dataBase64?: string;
}

interface HostHarness {
  invoke: ReturnType<typeof vi.fn>;
  notify: ReturnType<typeof vi.fn>;
  sendBinary: ReturnType<typeof vi.fn>;
  binaryListeners: Array<(event: BinaryEvent) => void>;
  emitOutput(terminalId: string, text: string): void;
}

function installHost(
  openResult: () => Promise<unknown> = async () => ({ terminalId: TERMINAL_ID }),
): HostHarness {
  const harness: HostHarness = {
    invoke: vi.fn(async (method: string) => {
      if (method === "docker/terminal/open") return openResult();
      if (method === "docker/terminal/close") return { stopped: true };
      return {};
    }),
    notify: vi.fn(async () => undefined),
    sendBinary: vi.fn(async () => undefined),
    binaryListeners: [],
    emitOutput(terminalId: string, text: string) {
      for (const listener of this.binaryListeners) {
        listener({ channel: `docker/terminal/out/${terminalId}`, data: new TextEncoder().encode(text) });
      }
    },
  };
  vi.stubGlobal("dbxPlugin", {
    invoke: harness.invoke,
    notify: harness.notify,
    sendBinary: harness.sendBinary,
    onBinary: (listener: (event: BinaryEvent) => void) => {
      harness.binaryListeners.push(listener);
      return () => {
        const index = harness.binaryListeners.indexOf(listener);
        if (index >= 0) harness.binaryListeners.splice(index, 1);
      };
    },
    decodeBase64: (value: string) => Uint8Array.from(atob(value), (character) => character.charCodeAt(0)),
  });
  return harness;
}

const t = (key: string, values?: Record<string, string | number>) =>
  values ? `${key} ${Object.values(values).join(" ")}` : key;

function container(overrides: Partial<DockerContainer> = {}): DockerContainer {
  return {
    id: "d4a7c9f1e2b3",
    name: "web",
    image: "nginx:latest",
    state: "running",
    status: "Up 2 hours",
    ports: "0.0.0.0:80->80/tcp",
    createdAt: "2026-10-26T00:00:00Z",
    ...overrides,
  };
}

type MountedTab = ReturnType<typeof mount>;

const wrappers: MountedTab[] = [];

function mountTab(overrides: Record<string, unknown> = {}): MountedTab {
  const wrapper = mount(ContainerTerminalTab, {
    attachTo: document.body,
    props: {
      t,
      container: container(),
      sessionId: "session-1",
      connectionId: "conn-1",
      canWrite: true,
      visible: true,
      theme: { background: "#000000" },
      fontFamily: "Menlo",
      fontSize: 13,
      terminalStateEvents: () => () => undefined,
      ...overrides,
    },
  });
  wrappers.push(wrapper);
  return wrapper;
}

function openCalls(harness: HostHarness): Array<Record<string, unknown>> {
  return harness.invoke.mock.calls
    .filter((call) => call[0] === "docker/terminal/open")
    .map((call) => call[1] as Record<string, unknown>);
}

function terminal(): InstanceType<typeof MockTerminal> | undefined {
  return MockTerminal.latest();
}

afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount();
  MockTerminal.reset();
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

describe("ContainerTerminalTab", () => {
  it("opens only when visible and renders a connected xterm with the workbench appearance", async () => {
    const harness = installHost();
    const wrapper = mountTab();

    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(1));
    expect(openCalls(harness)[0]).toMatchObject({ containerId: "d4a7c9f1e2b3", cols: 80, rows: 24 });
    expect(wrapper.find("[data-xterm-host]").attributes("data-xterm")).toBe("ready");
    expect(terminal()?.options.fontFamily).toBe("Menlo");
    expect(terminal()?.options.fontSize).toBe(13);
    expect(terminal()?.options.theme).toMatchObject({ background: "#000000" });
    expect(terminal()?.options.scrollback).toBe(5_000);
    expect(wrapper.find("[data-terminal-overlay]").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("docker.terminalConnecting");
  });

  it("does not open while hidden and keeps the PTY alive when the tab is hidden", async () => {
    const harness = installHost();
    const wrapper = mountTab({ visible: false });
    await flushPromises();
    await nextTick();
    expect(harness.invoke).not.toHaveBeenCalled();

    await wrapper.setProps({ visible: true });
    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(1));

    await wrapper.setProps({ visible: false });
    await flushPromises();
    await wrapper.setProps({ visible: true });
    await flushPromises();
    expect(openCalls(harness)).toHaveLength(1);
    expect(harness.invoke.mock.calls.filter((call) => call[0] === "docker/terminal/close")).toHaveLength(0);
  });

  it("shows a read-only message without opening a PTY", async () => {
    const harness = installHost();
    const wrapper = mountTab({ canWrite: false });

    await vi.waitFor(() => expect(wrapper.text()).toContain("docker.terminalReadOnly"));
    expect(wrapper.find("[data-terminal-retry]").exists()).toBe(false);
    expect(harness.invoke).not.toHaveBeenCalled();
  });

  it("shows a stopped-container message without opening a PTY", async () => {
    const harness = installHost();
    const wrapper = mountTab({ container: container({ state: "exited", status: "Exited (0)" }) });

    await vi.waitFor(() => expect(wrapper.text()).toContain("docker.terminalNotRunning"));
    expect(wrapper.find("[data-terminal-retry]").exists()).toBe(false);
    expect(harness.invoke).not.toHaveBeenCalled();
  });

  it("reopens after the container starts while the tab was hidden", async () => {
    const harness = installHost();
    const wrapper = mountTab({ container: container({ state: "exited", status: "Exited (0)" }) });
    await vi.waitFor(() => expect(wrapper.text()).toContain("docker.terminalNotRunning"));
    expect(harness.invoke).not.toHaveBeenCalled();

    await wrapper.setProps({ visible: false });
    await wrapper.setProps({ container: container(), visible: true });
    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(1));
    expect(wrapper.text()).not.toContain("docker.terminalNotRunning");
  });

  it("writes PTY output bytes into xterm", async () => {
    const harness = installHost();
    mountTab();
    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(1));

    harness.emitOutput(TERMINAL_ID, "hello");
    await vi.waitFor(() => expect(terminal()?.writes).toHaveLength(1));
    expect(new TextDecoder().decode(terminal()?.writes[0])).toBe("hello");
  });

  it("forwards xterm input and resize to the PTY channels", async () => {
    const harness = installHost();
    mountTab();
    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(1));

    terminal()?.dataHandler?.("ls\r");
    await vi.waitFor(() => expect(harness.sendBinary).toHaveBeenCalled());
    const [channel, payload] = harness.sendBinary.mock.calls[0] as [string, Uint8Array];
    expect(channel).toBe(`docker/terminal/in/${TERMINAL_ID}`);
    expect(new TextDecoder().decode(payload.slice(8))).toBe("ls\r");

    terminal()?.resizeHandler?.({ cols: 100, rows: 30 });
    await vi.waitFor(() =>
      expect(harness.invoke).toHaveBeenCalledWith("docker/terminal/resize", {
        terminalId: TERMINAL_ID,
        cols: 100,
        rows: 30,
      }),
    );
  });

  it("shows the exited state and reconnects from the retry button", async () => {
    const harness = installHost();
    let stateHandler: ((event: ContainerTerminalStateEvent) => void) | undefined;
    const terminalStateEvents = (handler: (event: ContainerTerminalStateEvent) => void) => {
      stateHandler = handler;
      return () => {
        stateHandler = undefined;
      };
    };
    const wrapper = mountTab({ terminalStateEvents });
    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(1));

    stateHandler?.({ terminalId: TERMINAL_ID, state: "exited", exitCode: 137 });
    await nextTick();
    expect(wrapper.text()).toContain("docker.terminalExited");
    expect(wrapper.text()).toContain("137");

    await wrapper.get("[data-terminal-retry]").trigger("click");
    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(2));
  });

  it("shows a transport error with a retry button", async () => {
    const harness = installHost();
    let stateHandler: ((event: ContainerTerminalStateEvent) => void) | undefined;
    const wrapper = mountTab({
      terminalStateEvents: (handler: (event: ContainerTerminalStateEvent) => void) => {
        stateHandler = handler;
        return () => undefined;
      },
    });
    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(1));

    stateHandler?.({ terminalId: TERMINAL_ID, state: "error", error: "session lost" });
    await nextTick();
    expect(wrapper.get("[data-terminal-overlay]").text()).toContain("session lost");
    expect(wrapper.find("[data-terminal-retry]").exists()).toBe(true);
  });

  it("resets xterm, closes the old shell and reopens when the container changes", async () => {
    const harness = installHost();
    const wrapper = mountTab();
    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(1));
    const first = terminal();

    await wrapper.setProps({ container: container({ id: "def456abc789", name: "db" }) });
    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(2));
    expect(openCalls(harness)[1]).toMatchObject({ containerId: "def456abc789" });
    expect(first?.resets).toBeGreaterThan(0);
    expect(harness.invoke).toHaveBeenCalledWith("docker/terminal/close", { terminalId: TERMINAL_ID }, expect.anything());
  });

  it("closes the PTY and disposes xterm on unmount", async () => {
    const harness = installHost();
    const wrapper = mountTab();
    await vi.waitFor(() => expect(openCalls(harness)).toHaveLength(1));
    const instance = terminal();

    wrapper.unmount();
    wrappers.splice(wrappers.indexOf(wrapper), 1);
    expect(harness.invoke).toHaveBeenCalledWith("docker/terminal/close", { terminalId: TERMINAL_ID }, expect.anything());
    expect(instance?.disposed).toBe(true);
  });
});
