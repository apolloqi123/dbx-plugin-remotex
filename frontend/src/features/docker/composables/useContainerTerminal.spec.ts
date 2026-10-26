// @vitest-environment happy-dom
// 容器 PTY 传输层：宿主桥、二进制输入输出、resize 去重与 state 事件的单测。
// xterm 实例不在本层，onData 只接收原始字节（与 SplitTerminalPane.spec 的
// 宿主 harness 同形，简化掉了分屏的帧序号/回放能力）。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useContainerTerminal, type UseContainerTerminalOptions } from "./useContainerTerminal";

interface BinaryEvent {
  channel: string;
  data?: Uint8Array;
  dataBase64?: string;
}

interface Harness {
  invoke: ReturnType<typeof vi.fn>;
  notify: ReturnType<typeof vi.fn>;
  sendBinary: ReturnType<typeof vi.fn>;
  binaryListeners: Array<(event: BinaryEvent) => void>;
  emitOutput(terminalId: string, text: string): void;
}

function installHarness(openResult: () => Promise<unknown> = async () => ({ terminalId: "term-12345678" })): Harness {
  const harness = {
    invoke: vi.fn(async (method: string) => {
      if (method === "docker/terminal/open") return openResult();
      if (method === "docker/terminal/resize") return { resized: true };
      if (method === "docker/terminal/close") return { stopped: true };
      return {};
    }),
    notify: vi.fn(async () => undefined),
    sendBinary: vi.fn(async () => undefined),
    binaryListeners: [] as Harness["binaryListeners"],
    emitOutput(terminalId: string, text: string) {
      for (const listener of this.binaryListeners) {
        listener({ channel: `docker/terminal/out/${terminalId}`, data: new TextEncoder().encode(text) });
      }
    },
  };
  (window as unknown as { dbxPlugin: unknown }).dbxPlugin = {
    invoke: harness.invoke,
    notify: harness.notify,
    sendBinary: harness.sendBinary,
    onBinary: (listener: Harness["binaryListeners"][number]) => {
      harness.binaryListeners.push(listener);
      return () => {
        const index = harness.binaryListeners.indexOf(listener);
        if (index >= 0) harness.binaryListeners.splice(index, 1);
      };
    },
    decodeBase64: (value: string) => Uint8Array.from(atob(value), (character) => character.charCodeAt(0)),
  };
  return harness;
}

// 生产翻译会替换 {error} 等占位符；这里的 t 保留键名并把值拼进去，既能让
// "只读/未运行" 断言测试前缀，也能让打开失败断言真实错误文本。
function translate(key: string, values?: Record<string, string | number>): string {
  if (!values) return key;
  const pairs = Object.entries(values).map(([name, value]) => `${name}=${value}`);
  return `${key}(${pairs.join(",")})`;
}

function options(overrides: Partial<UseContainerTerminalOptions> = {}): UseContainerTerminalOptions {
  return {
    containerId: () => "d4a7c9f1e2b3",
    sessionId: () => "session-1",
    connectionId: () => "conn-1",
    canWrite: () => true,
    running: () => true,
    t: translate,
    onData: vi.fn(),
    createTerminalId: () => "term-12345678",
    ...overrides,
  };
}

function resizeCalls(harness: Harness): Array<[string, Record<string, unknown>]> {
  return harness.invoke.mock.calls.filter((call) => call[0] === "docker/terminal/resize") as Array<
    [string, Record<string, unknown>]
  >;
}

describe("useContainerTerminal", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => {
    delete (window as unknown as { dbxPlugin?: unknown }).dbxPlugin;
  });

  it("subscribes before open and forwards output bytes", async () => {
    const harness = installHarness();
    const onData = vi.fn();
    const terminal = useContainerTerminal(options({ onData }));

    const opening = terminal.open(80, 24);
    // open 之前必须已经订阅：open RPC 返回前的首屏输出（提示符）不能丢。
    expect(harness.binaryListeners).toHaveLength(1);
    await opening;

    expect(harness.invoke).toHaveBeenCalledWith(
      "docker/terminal/open",
      expect.objectContaining({ terminalId: "term-12345678", containerId: "d4a7c9f1e2b3", cols: 80, rows: 24 }),
      expect.anything(),
    );
    harness.emitOutput("term-12345678", "hello");
    expect(onData).toHaveBeenCalledWith(new TextEncoder().encode("hello"));
    // 其它终端的输出不得串进来。
    harness.emitOutput("other-terminal", "nope");
    expect(onData).toHaveBeenCalledTimes(1);
    expect(terminal.status.value).toBe("connected");
  });

  it("sends input on the docker channel with a sequence prefix", async () => {
    const harness = installHarness();
    const terminal = useContainerTerminal(options());
    await terminal.open(80, 24);

    terminal.sendInput("ls\r");
    await vi.waitFor(() => expect(harness.sendBinary).toHaveBeenCalled());
    const [channel, payload] = harness.sendBinary.mock.calls[0] as [string, Uint8Array];
    expect(channel).toBe("docker/terminal/in/term-12345678");
    expect(new TextDecoder().decode(payload.slice(8))).toBe("ls\r");
  });

  it("deduplicates unchanged resize requests", async () => {
    const harness = installHarness();
    const terminal = useContainerTerminal(options());
    await terminal.open(80, 24);

    // Same geometry as the open request: no resize RPC.
    terminal.resize(80, 24);
    await Promise.resolve();
    expect(resizeCalls(harness)).toHaveLength(0);

    terminal.resize(100, 30);
    terminal.resize(100, 30);
    await vi.waitFor(() => expect(resizeCalls(harness)).toHaveLength(1));
    expect(resizeCalls(harness)[0][1]).toEqual({
      terminalId: "term-12345678",
      cols: 100,
      rows: 30,
    });
  });

  it("retries a resize the backend dropped once", async () => {
    const harness = installHarness();
    let resizeAttempts = 0;
    harness.invoke.mockImplementation(async (method: string) => {
      if (method === "docker/terminal/open") return { terminalId: "term-12345678" };
      if (method === "docker/terminal/resize") {
        resizeAttempts += 1;
        return { resized: resizeAttempts > 1 };
      }
      return {};
    });
    const terminal = useContainerTerminal(options());
    await terminal.open(80, 24);

    terminal.resize(100, 30);
    await vi.waitFor(() => expect(resizeAttempts).toBe(2), { timeout: 2_000 });
  });

  it("reports errors and retries with a fresh terminal id", async () => {
    let attempt = 0;
    const harness = installHarness(async () => {
      attempt += 1;
      if (attempt === 1) throw new Error("no shell");
      return { terminalId: "term-12345678" };
    });
    const terminal = useContainerTerminal(options());

    await terminal.open(80, 24);
    expect(terminal.status.value).toBe("error");
    expect(terminal.errorMessage.value).toContain("no shell");
    // A lost response may hide an admitted PTY: the speculative id is closed.
    expect(harness.invoke).toHaveBeenCalledWith("docker/terminal/close", { terminalId: "term-12345678" });

    await terminal.retry(80, 24);
    expect(terminal.status.value).toBe("connected");
    expect(terminal.errorMessage.value).toBe("");
  });

  it("maps state events for the current terminal only", async () => {
    const harness = installHarness();
    const terminal = useContainerTerminal(options());
    await terminal.open(80, 24);

    terminal.handleStateEvent({ terminalId: "other", state: "exited", exitCode: 1 });
    expect(terminal.status.value).toBe("connected");

    terminal.handleStateEvent({ terminalId: "term-12345678", state: "exited", exitCode: 0 });
    expect(terminal.status.value).toBe("exited");
    expect(harness.invoke).toHaveBeenCalledTimes(1);
  });

  it("maps an error state to the error message and status", async () => {
    installHarness();
    const terminal = useContainerTerminal(options());
    await terminal.open(80, 24);

    terminal.handleStateEvent({ terminalId: "term-12345678", state: "error", error: "ssh transport lost" });
    expect(terminal.status.value).toBe("error");
    expect(terminal.errorMessage.value).toBe("ssh transport lost");
  });

  it("closes the session once and resets the status", async () => {
    const harness = installHarness();
    const terminal = useContainerTerminal(options());
    await terminal.open(80, 24);

    terminal.close();
    terminal.close();
    await vi.waitFor(() => expect(harness.invoke).toHaveBeenCalledTimes(2));
    expect(harness.invoke).toHaveBeenLastCalledWith("docker/terminal/close", { terminalId: "term-12345678" });
    expect(terminal.status.value).toBe("idle");
    expect(harness.binaryListeners).toHaveLength(0);
  });

  it("opens the local target when no SSH session is bound", async () => {
    const harness = installHarness();
    const terminal = useContainerTerminal(options({ sessionId: () => undefined }));
    await terminal.open(80, 24);

    const [, params] = harness.invoke.mock.calls[0] as [string, Record<string, unknown>];
    expect(params.sessionId).toBeUndefined();
    expect(params.target).toBe("local");
    expect(terminal.status.value).toBe("connected");
  });

  it("does not open for read-only connections or stopped containers", async () => {
    const harness = installHarness();

    const readOnly = useContainerTerminal(
      options({ canWrite: () => false, t: (key) => `ro:${key}` }),
    );
    await readOnly.open(80, 24);
    expect(harness.invoke).not.toHaveBeenCalled();
    expect(readOnly.status.value).toBe("disabled");
    expect(readOnly.errorMessage.value).toBe("ro:docker.terminalReadOnly");

    const stopped = useContainerTerminal(
      options({ running: () => false, t: (key) => `stopped:${key}` }),
    );
    await stopped.open(80, 24);
    expect(harness.invoke).not.toHaveBeenCalled();
    expect(stopped.status.value).toBe("disabled");
    expect(stopped.errorMessage.value).toBe("stopped:docker.terminalNotRunning");
  });
});
