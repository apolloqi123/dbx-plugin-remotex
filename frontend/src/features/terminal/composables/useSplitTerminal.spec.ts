// @vitest-environment happy-dom
// 分屏 PTY 尺寸去重：ResizeObserver 的初始回调会在 open 之后补发同尺寸
// resize，远端 shell 收到 window-change 会重绘提示符（一行两个 prompt）。
import { afterEach, describe, expect, it, vi } from "vitest";
import { useSplitTerminal } from "./useSplitTerminal";

function installHost() {
  const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
  const notifyCalls: Array<{ method: string; params: Record<string, unknown> }> = [];
  vi.stubGlobal("dbxPlugin", {
    invoke: vi.fn(async (method: string, params: Record<string, unknown> = {}) => {
      calls.push({ method, params });
      if (method === "ssh/session/open") return { sessionId: "server-split" };
      if (method === "ssh/terminal/replay") return { complete: true };
      return {};
    }),
    notify: vi.fn(async (method: string, params: Record<string, unknown> = {}) => {
      notifyCalls.push({ method, params });
    }),
    sendBinary: vi.fn(async () => undefined),
    onBinary: vi.fn(() => () => undefined),
  });
  return { calls, notifyCalls };
}

function createSession(host: ReturnType<typeof installHost>) {
  return useSplitTerminal({
    connectionId: () => "conn-1",
    workbenchId: () => "workbench-1",
    mainSessionId: () => "main-session-1",
    t: (key) => key,
    onData: () => undefined,
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useSplitTerminal resize dedupe", () => {
  it("does not resend the spawn size and only forwards real changes", async () => {
    const host = installHost();
    const session = createSession(host);
    await session.open(80, 24);

    const open = host.calls.find((call) => call.method === "ssh/session/open");
    expect(open?.params).toMatchObject({ cols: 80, rows: 24 });

    // ResizeObserver 初始回调：与 spawn 同尺寸，不得发 window-change。
    session.resize(80, 24);
    expect(host.notifyCalls).toHaveLength(0);

    session.resize(100, 30);
    expect(host.notifyCalls).toEqual([
      { method: "ssh/terminal/resize", params: { sessionId: "server-split", cols: 100, rows: 30 } },
    ]);

    session.resize(100, 30);
    expect(host.notifyCalls).toHaveLength(1);

    session.close();
  });
});
