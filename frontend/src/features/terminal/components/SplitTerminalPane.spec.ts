// @vitest-environment happy-dom
// SplitTerminalPane integration: the xterm renderer is replaced with a small
// fake (xterm needs a real canvas/font stack), but the host bridge, session
// lifecycle, frame ordering and input queue are the production implementations.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";

const xtermMock = vi.hoisted(() => {
  class FakeTerminal {
    static instances: FakeTerminal[] = [];
    cols = 80;
    rows = 24;
    options: Record<string, unknown>;
    unicode = { activeVersion: "6" };
    buffer = { active: { type: "normal" } };
    dataHandler: ((data: string) => void) | undefined;
    selectionHandler: (() => void) | undefined;
    keyHandler: ((event: KeyboardEvent) => boolean) | undefined;
    written: Uint8Array[] = [];
    selection = "";
    disposed = false;
    focused = false;
    opened = false;
    pasted = "";

    constructor(options: Record<string, unknown>) {
      this.options = { ...options };
      FakeTerminal.instances.push(this);
    }

    loadAddon() {}
    open() {
      this.opened = true;
    }
    attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean) {
      this.keyHandler = handler;
    }
    onData(handler: (data: string) => void) {
      this.dataHandler = handler;
      return { dispose: () => { this.dataHandler = undefined; } };
    }
    onSelectionChange(handler: () => void) {
      this.selectionHandler = handler;
      return { dispose: () => { this.selectionHandler = undefined; } };
    }
    selectAll() {}
    clear() {}
    paste(data: string) {
      this.pasted += data;
    }
    write(data: Uint8Array) {
      this.written.push(data);
    }
    focus() {
      this.focused = true;
    }
    getSelection() {
      return this.selection;
    }
    hasSelection() {
      return this.selection.length > 0;
    }
    reset() {}
    dispose() {
      this.disposed = true;
    }
  }
  return { FakeTerminal };
});

vi.mock("@xterm/xterm", () => ({ Terminal: xtermMock.FakeTerminal }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} dispose() {} } }));
vi.mock("@xterm/addon-search", () => ({
  SearchAddon: class {
    onDidChangeResults() {}
    findNext() {}
    findPrevious() {}
    clearDecorations() {}
    dispose() {}
  },
}));
vi.mock("@xterm/addon-unicode11", () => ({ Unicode11Addon: class { dispose() {} } }));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class { dispose() {} } }));

import SplitTerminalPane from "./SplitTerminalPane.vue";

interface HostCall {
  method: string;
  params: Record<string, unknown>;
}

interface HostHarness {
  calls: HostCall[];
  notifyCalls: HostCall[];
  sendBinaryCalls: Array<{ channel: string; data: Uint8Array }>;
  binaryListeners: Array<(event: { channel: string; data?: Uint8Array; dataBase64?: string }) => void>;
  closeCalls: string[];
  openError?: string;
  resolveOpen?: (sessionId: string) => void;
}

function installHost(options: { openError?: string; deferOpen?: boolean } = {}): HostHarness {
  const harness: HostHarness = {
    calls: [],
    notifyCalls: [],
    sendBinaryCalls: [],
    binaryListeners: [],
    closeCalls: [],
    openError: options.openError,
  };
  let resolveOpen: ((value: unknown) => void) | undefined;
  const deferredOpen = options.deferOpen
    ? new Promise((resolve) => {
      resolveOpen = resolve;
    })
    : undefined;
  harness.resolveOpen = (sessionId: string) => {
    resolveOpen?.({ sessionId, connectionId: "conn-1", connected: true, sequence: 0, chunkSize: 0 });
  };
  const plugin = {
    ready: Promise.resolve({}),
    locale: "en",
    invoke: vi.fn(async (method: string, params: Record<string, unknown> = {}) => {
      harness.calls.push({ method, params });
      if (method === "ssh/session/open") {
        if (harness.openError) throw new Error(harness.openError);
        if (deferredOpen) return deferredOpen;
        return { sessionId: "server-split", connectionId: "conn-1", connected: true, sequence: 0, chunkSize: 0 };
      }
      if (method === "ssh/terminal/replay") {
        return { frameCount: 0, firstAvailableSequence: 1, tailSequence: 0, complete: true };
      }
      if (method === "ssh/session/close") {
        harness.closeCalls.push(String(params.sessionId));
        return {};
      }
      throw new Error(`Unexpected host call: ${method}`);
    }),
    notify: vi.fn(async (method: string, params: Record<string, unknown> = {}) => {
      harness.notifyCalls.push({ method, params });
    }),
    sendBinary: vi.fn(async (channel: string, data: Uint8Array) => {
      harness.sendBinaryCalls.push({ channel, data });
    }),
    onBinary: vi.fn((listener: (event: { channel: string; data?: Uint8Array }) => void) => {
      harness.binaryListeners.push(listener);
      return () => {
        const index = harness.binaryListeners.indexOf(listener);
        if (index >= 0) harness.binaryListeners.splice(index, 1);
      };
    }),
    onEvent: vi.fn(() => () => {}),
    decodeBase64: (value: string) => Uint8Array.from(atob(value), (char) => char.charCodeAt(0)),
    encodeBase64: (value: Uint8Array) => btoa(String.fromCharCode(...value)),
  };
  vi.stubGlobal("dbxPlugin", plugin);
  return harness;
}

function frame(sequence: number, text: string, stream = 0): Uint8Array {
  const data = new TextEncoder().encode(text);
  const bytes = new Uint8Array(9 + data.byteLength);
  bytes[0] = stream;
  new DataView(bytes.buffer).setBigUint64(1, BigInt(sequence), false);
  bytes.set(data, 9);
  return bytes;
}

function props(overrides: Record<string, unknown> = {}) {
  return {
    connectionId: "conn-1",
    workbenchId: "workbench-1",
    mainSessionId: "main-session-1",
    theme: { background: "#000000" },
    fontFamily: "monospace",
    fontSize: 13,
    t: (key: string) => key,
    locale: "en",
    ...overrides,
  };
}

async function mountPane(
  hostOptions: { openError?: string; deferOpen?: boolean } = {},
  overrides: Record<string, unknown> = {},
) {
  const harness = installHost(hostOptions);
  const wrapper = mount(SplitTerminalPane, { attachTo: document.body, props: props(overrides) });
  await flushPromises();
  await flushPromises();
  return { wrapper, harness, terminal: xtermMock.FakeTerminal.instances.at(-1)! };
}

async function openContextMenu(wrapper: VueWrapper): Promise<HTMLElement[]> {
  wrapper.get(".split-terminal-pane").element.dispatchEvent(
    new MouseEvent("contextmenu", { bubbles: true, cancelable: true }),
  );
  await nextTick();
  await nextTick();
  await flushPromises();
  return Array.from(document.body.querySelectorAll<HTMLElement>('[role="menuitem"]'));
}

beforeEach(() => {
  xtermMock.FakeTerminal.instances.length = 0;
  // happy-dom has no layout; the fake observer keeps the production observer
  // wiring from depending on the environment.
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    unobserve() {}
    disconnect() {}
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

describe("SplitTerminalPane", () => {
  it("opens a second PTY over the main authenticated transport and reports the authoritative session", async () => {
    const { wrapper, harness } = await mountPane();

    const open = harness.calls.find((call) => call.method === "ssh/session/open");
    expect(open).toBeDefined();
    expect(open!.params).toMatchObject({
      connectionId: "conn-1",
      workbenchId: "workbench-1#split",
      reuseAuthenticatedTransport: true,
      reuseAuthenticatedSessionId: "main-session-1",
      cols: 80,
      rows: 24,
    });
    expect(String(open!.params.requestedSessionId)).toMatch(/^[A-Za-z0-9._-]+$/);
    expect(harness.calls.some((call) => call.method === "ssh/terminal/replay" && call.params.afterSequence === 0)).toBe(true);
    expect(wrapper.emitted("opened")?.[0]).toEqual(["server-split"]);
    expect(wrapper.find(".split-terminal-pane").attributes("role")).toBe("region");
    expect(wrapper.find(".split-terminal-pane").attributes("aria-label")).toBe("terminalSplit.title");

    wrapper.unmount();
  });

  it("accepts output for requestedSessionId before the open RPC resolves", async () => {
    // The open RPC can take seconds (PTY spawn); first output may arrive under
    // the caller-provided id while ssh/session/open is still in flight.
    const { wrapper, harness, terminal } = await mountPane({ deferOpen: true });
    const requested = String(harness.calls.find((call) => call.method === "ssh/session/open")!.params.requestedSessionId);
    expect(requested).not.toBe("");

    harness.binaryListeners[0]({ channel: `ssh/terminal/out/${requested}`, data: frame(1, "early") });
    expect(terminal.written.map((bytes) => new TextDecoder().decode(bytes))).toEqual(["early"]);

    harness.resolveOpen?.(requested);
    await flushPromises();
    await flushPromises();

    expect(wrapper.emitted("opened")?.[0]).toEqual([requested]);
    wrapper.unmount();
  });

  it("closes the split PTY when the pane unmounts", async () => {
    const { wrapper, harness } = await mountPane();
    wrapper.unmount();
    await flushPromises();

    expect(harness.closeCalls).toEqual(["server-split"]);
    expect(harness.binaryListeners).toHaveLength(0);
  });

  it("closes and emits closed from the header X button", async () => {
    const { wrapper, harness } = await mountPane();

    await wrapper.get(".split-terminal-close").trigger("click");
    await flushPromises();

    expect(wrapper.emitted("closed")).toHaveLength(1);
    expect(harness.closeCalls).toEqual(["server-split"]);

    wrapper.unmount();
  });

  it("forwards keyboard input through the sequenced SSH input channel", async () => {
    const { wrapper, harness, terminal } = await mountPane();

    terminal.dataHandler?.("ls\n");
    await flushPromises();

    expect(harness.sendBinaryCalls).toHaveLength(1);
    expect(harness.sendBinaryCalls[0].channel).toBe("ssh/terminal/in/server-split");
    expect(Array.from(harness.sendBinaryCalls[0].data.slice(8))).toEqual(Array.from(new TextEncoder().encode("ls\n")));
    expect(Array.from(harness.sendBinaryCalls[0].data.slice(0, 8))).toEqual([0, 0, 0, 0, 0, 0, 0, 1]);

    wrapper.unmount();
  });

  it("writes contiguous output and requests a replay once a sequence gap appears", async () => {
    const { wrapper, harness, terminal } = await mountPane();
    harness.calls.length = 0;

    const listener = harness.binaryListeners[0];
    listener({ channel: "ssh/terminal/out/server-split", data: frame(1, "one") });
    listener({ channel: "ssh/terminal/out/server-split", data: frame(3, "three") });

    expect(terminal.written.map((bytes) => new TextDecoder().decode(bytes))).toEqual(["one"]);
    const replay = harness.calls.find((call) => call.method === "ssh/terminal/replay");
    expect(replay?.params).toEqual({ sessionId: "server-split", afterSequence: 1 });

    listener({ channel: "ssh/terminal/out/server-split", data: frame(2, "two") });
    expect(terminal.written.map((bytes) => new TextDecoder().decode(bytes))).toEqual(["one", "two", "three"]);

    wrapper.unmount();
  });

  it("reports the shell's OSC 7 cwd for directory following", async () => {
    const { wrapper, harness } = await mountPane();
    const listener = harness.binaryListeners[0];

    listener({ channel: "ssh/terminal/out/server-split", data: frame(1, "\u001b]7;file://hostname/var/log\u0007") });
    await flushPromises();

    expect(wrapper.emitted("cwd")?.at(-1)).toEqual(["/var/log"]);
    wrapper.unmount();
  });

  it("keeps the post-hook prompt off the previous prompt line", async () => {
    const { wrapper, harness, terminal } = await mountPane();
    const listener = harness.binaryListeners[0];

    listener({
      channel: "ssh/terminal/out/server-split",
      data: frame(1, "Last login: now\r\n[root@host ~]# "),
    });
    listener({
      channel: "ssh/terminal/out/server-split",
      data: frame(2, "\u001b]7;file://host/root\u0007[root@host ~]# "),
    });

    const output = terminal.written.map((bytes) => new TextDecoder().decode(bytes)).join("");
    // The prompt hook's OSC 7 lands while the pre-arming prompt left the cursor
    // mid-line: the renderer must splice a CRLF in front of it instead of
    // letting the remote prompt draw a second `[root@host ~]# ` on the same line.
    expect(output).toBe(
      "Last login: now\r\n[root@host ~]# \r\n\u001b]7;file://host/root\u0007[root@host ~]# ",
    );
    expect(wrapper.emitted("cwd")?.at(-1)).toEqual(["/root"]);
    wrapper.unmount();
  });

  it("surfaces a transport state frame as an error and stops accepting output", async () => {
    const { wrapper, harness, terminal } = await mountPane();
    const listener = harness.binaryListeners[0];

    listener({ channel: "ssh/terminal/out/server-split", data: frame(1, "ssh-transport-disconnected", 2) });
    listener({ channel: "ssh/terminal/out/server-split", data: frame(2, "after-error") });
    await flushPromises();

    expect(wrapper.emitted("error")?.[0]).toEqual(["transportDisconnected"]);
    expect(wrapper.find(".split-terminal-error").text()).toBe("transportDisconnected");
    expect(wrapper.find(".split-terminal-close").exists()).toBe(true);
    expect(terminal.written).toHaveLength(0);

    wrapper.unmount();
  });

  it("treats directory-tracking-unavailable as informational and keeps the stream alive", async () => {
    const { wrapper, harness, terminal } = await mountPane();
    const listener = harness.binaryListeners[0];

    listener({ channel: "ssh/terminal/out/server-split", data: frame(1, "directory-tracking-unavailable", 2) });
    listener({ channel: "ssh/terminal/out/server-split", data: frame(2, "still alive") });
    await flushPromises();

    expect(wrapper.emitted("error")).toBeUndefined();
    expect(wrapper.find(".split-terminal-error").exists()).toBe(false);
    expect(terminal.written.map((bytes) => new TextDecoder().decode(bytes))).toEqual(["still alive"]);

    wrapper.unmount();
  });

  it("shows an open failure, keeps the close button and emits error", async () => {
    const { wrapper } = await mountPane({ openError: "No live authenticated SSH connection" });

    expect(wrapper.emitted("error")?.[0]).toEqual(["No live authenticated SSH connection"]);
    expect(wrapper.find(".split-terminal-status").text()).toContain("No live authenticated SSH connection");
    expect(wrapper.find(".split-terminal-error").text()).toBe("No live authenticated SSH connection");
    expect(wrapper.find(".split-terminal-close").exists()).toBe(true);

    wrapper.unmount();
  });

  it("updates theme and font options in place without recreating the terminal", async () => {
    const { wrapper, terminal } = await mountPane();
    const count = xtermMock.FakeTerminal.instances.length;

    await wrapper.setProps({ theme: { background: "#112233" }, fontFamily: "Fira Code", fontSize: 15 });

    expect(terminal.options.theme).toEqual({ background: "#112233" });
    expect(terminal.options.fontFamily).toBe("Fira Code");
    expect(terminal.options.fontSize).toBe(15);
    expect(xtermMock.FakeTerminal.instances).toHaveLength(count);

    wrapper.unmount();
  });

  it("closes and reopens on the new main session when mainSessionId changes", async () => {
    const { wrapper, harness } = await mountPane();

    await wrapper.setProps({ mainSessionId: "main-session-2" });
    await flushPromises();
    await flushPromises();

    expect(harness.closeCalls).toEqual(["server-split"]);
    const opens = harness.calls.filter((call) => call.method === "ssh/session/open");
    expect(opens).toHaveLength(2);
    expect(opens[1].params.reuseAuthenticatedSessionId).toBe("main-session-2");

    wrapper.unmount();
  });

  it("can render as an in-page tab body without its own header", async () => {
    installHost();
    const wrapper = mount(SplitTerminalPane, {
      attachTo: document.body,
      props: props({ showHeader: false, active: true }),
    });
    await flushPromises();
    await flushPromises();

    expect(wrapper.find(".split-terminal-header").exists()).toBe(false);
    expect(wrapper.find(".split-terminal-body").exists()).toBe(true);

    wrapper.unmount();
  });

  it("opens its own context menu on right click instead of the host native menu", async () => {
    const { wrapper } = await mountPane({}, { canCreateTab: true, closeKind: "tab" });
    const pane = wrapper.get(".split-terminal-pane");
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });

    pane.element.dispatchEvent(event);
    await nextTick();
    await nextTick();
    await flushPromises();

    const text = document.body.textContent ?? "";
    expect(event.defaultPrevented).toBe(true);
    expect(text).toContain("terminalSelectAll");
    expect(text).toContain("terminalTab.new");
    expect(text).toContain("terminalTab.close");

    wrapper.unmount();
  });

  it("emits new-tab when selecting the menu entry", async () => {
    const { wrapper } = await mountPane({}, { canCreateTab: true });
    wrapper.get(".split-terminal-pane").element.dispatchEvent(
      new MouseEvent("contextmenu", { bubbles: true, cancelable: true }),
    );
    await nextTick();
    await nextTick();
    await flushPromises();

    const entry = Array.from(document.body.querySelectorAll<HTMLElement>('[role="menuitem"]')).find((el) =>
      el.textContent?.includes("terminalTab.new"),
    );
    expect(entry).toBeTruthy();
    entry!.click();
    await flushPromises();

    expect(wrapper.emitted("new-tab")).toHaveLength(1);
    wrapper.unmount();
  });

  it("mirrors the main terminal menu options, including disabled rz/trz entries", async () => {
    const engines = [{ name: "Google", urlTemplate: "https://www.google.com/search?q=%s" }];
    const { wrapper } = await mountPane(
      {},
      { connected: true, canWrite: true, canSplit: true, canCreateTab: true, closeKind: "tab", engines },
    );

    const items = await openContextMenu(wrapper);
    const text = items.map((item) => item.textContent ?? "").join("\n");
    for (const key of [
      "terminalCopy",
      "terminalPaste",
      "ctxSearch.menuLabel",
      "terminalSelectAll",
      "terminalSearch.open",
      "terminalClear",
      "sudoRefresh.title",
      "terminalTab.new",
      "terminalSplit.leftRight",
      "terminalSplit.topBottom",
      "zmodemUpload",
      "trzszUpload",
      "terminalTab.close",
    ]) {
      expect(text).toContain(key);
    }
    const upload = items.find((item) => item.textContent?.includes("zmodemUpload"));
    expect(upload?.getAttribute("aria-disabled") ?? upload?.getAttribute("data-disabled")).toBeTruthy();

    wrapper.unmount();
  });

  it("sends sudo -v to its own split session from the menu", async () => {
    const { wrapper, harness, terminal } = await mountPane({}, { connected: true });
    const items = await openContextMenu(wrapper);
    const entry = items.find((item) => item.textContent?.includes("sudoRefresh.title"));

    entry!.click();
    await flushPromises();

    const sent = harness.sendBinaryCalls.map((call) => new TextDecoder().decode(call.data.slice(8)));
    expect(sent).toContain("sudo -v\r");
    expect(terminal.focused).toBe(true);
    wrapper.unmount();
  });

  it("opens a pane-local terminal search panel from the menu", async () => {
    const { wrapper } = await mountPane();
    const items = await openContextMenu(wrapper);

    items.find((item) => item.textContent?.includes("terminalSearch.open"))!.click();
    await nextTick();
    await flushPromises();

    expect(wrapper.find(".terminal-search-panel").exists()).toBe(true);
    wrapper.unmount();
  });

  it("emits search-online with the pane selection", async () => {
    const engines = [{ name: "Google", urlTemplate: "https://www.google.com/search?q=%s" }];
    const { wrapper, terminal } = await mountPane({}, { engines });
    terminal.selection = "needle";
    terminal.selectionHandler?.();
    await nextTick();
    const items = await openContextMenu(wrapper);

    items.find((item) => item.textContent?.includes("ctxSearch.menuLabel"))!.click();
    await nextTick();
    await flushPromises();
    const engine = Array.from(document.body.querySelectorAll<HTMLElement>('[role="menuitem"]')).find((el) =>
      el.textContent?.includes("Google"),
    );
    engine!.click();
    await flushPromises();

    expect(wrapper.emitted("search-online")?.[0]).toEqual([engines[0], "needle"]);
    wrapper.unmount();
  });

  it("does not render a new-tab button in the split header", async () => {
    const { wrapper } = await mountPane({}, { canCreateTab: true });

    expect(wrapper.find(".split-terminal-add").exists()).toBe(false);
    expect(wrapper.find(".split-terminal-close").exists()).toBe(true);
    wrapper.unmount();
  });

  it("offers split orientation switches from the split pane menu", async () => {
    const { wrapper } = await mountPane({}, { canSplit: true, closeKind: "split" });
    const pane = wrapper.get(".split-terminal-pane");

    pane.element.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await nextTick();
    await nextTick();
    await flushPromises();

    const text = document.body.textContent ?? "";
    expect(text).toContain("terminalSplit.leftRight");
    expect(text).toContain("terminalSplit.topBottom");
    expect(text).toContain("terminalSplit.close");

    wrapper.unmount();
  });


  it("exposes focus() and focuses the terminal on body click", async () => {
    const { wrapper, terminal } = await mountPane();

    await wrapper.get(".split-terminal-body").trigger("click");
    expect(terminal.focused).toBe(true);

    terminal.focused = false;
    (wrapper.vm as unknown as { focus: () => void }).focus();
    expect(terminal.focused).toBe(true);

    wrapper.unmount();
  });
});
