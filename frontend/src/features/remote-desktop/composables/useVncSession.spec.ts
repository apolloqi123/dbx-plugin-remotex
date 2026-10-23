// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi, type Mock } from "vitest";
import { effectScope, nextTick, ref } from "vue";
import { useVncSession, type VncSurfaceHandle } from "./useVncSession";

/** useVncSession 隔离单测：不挂载 App.vue，仅用 effectScope 提供组件作用域
 * （组合式函数内的 onScopeDispose 清理在 scope.stop() 时触发），RPC 全部
 * 经 window.dbxPlugin.invoke 桩观察，断言方法字符串/载荷与状态机不变。 */

const originalBridge = (window as unknown as { dbxPlugin?: unknown }).dbxPlugin;

function installBridge(invoke: ReturnType<typeof vi.fn>) {
  (window as unknown as { dbxPlugin: { invoke: typeof invoke } }).dbxPlugin = { invoke };
}

type SurfaceMock = VncSurfaceHandle & {
  acceptFrame: Mock<(frame: Uint8Array) => boolean>;
  reset: Mock<() => void>;
  canvasFocus: Mock<() => void>;
};

function buildSurface(): SurfaceMock {
  const canvasFocus = vi.fn();
  return {
    acceptFrame: vi.fn(() => true),
    reset: vi.fn(),
    canvasFocus,
    $el: { querySelector: vi.fn(() => ({ focus: canvasFocus })) },
  };
}

function createHarness() {
  const invoke = vi.fn();
  installBridge(invoke);
  const showError = vi.fn();
  const showNotice = vi.fn();
  const terminalFocus = vi.fn();
  let disposed = false;
  const scope = effectScope();
  const api = scope.run(() =>
    useVncSession({
      t: (key: string) => key,
      showError,
      showNotice,
      connectionId: ref("connection-1"),
      workbenchId: ref("workbench-1"),
      runtimeEndpoint: ref({ host: "runtime.local", port: 5901 }),
      isDisposed: () => disposed,
      getTerminal: () => ({ focus: terminalFocus }),
    }),
  );
  if (!api) throw new Error("useVncSession did not initialize");
  return {
    api,
    invoke,
    showError,
    showNotice,
    terminalFocus,
    scope,
    setDisposed: (value: boolean) => {
      disposed = value;
    },
  };
}

function connectOptions(overrides: Record<string, unknown> = {}) {
  return { host: "10.0.0.7", port: 5900, scaleMode: "fit" as const, ...overrides };
}

function startResult() {
  return { sessionId: "vnc-1", host: "10.0.0.7", port: 5900 };
}

afterEach(() => {
  (window as unknown as { dbxPlugin?: unknown }).dbxPlugin = originalBridge;
});

describe("useVncSession state machine", () => {
  it("starts idle with no session, target or error", () => {
    const { api, scope } = createHarness();
    expect(api.vncSession.value).toBeNull();
    expect(api.vncState.value).toBe("idle");
    expect(api.vncError.value).toBe("");
    expect(api.isVncMode.value).toBe(false);
    expect(api.vncTarget.value).toBe("");
    expect(api.vncScaleMode.value).toBe("fit");
    scope.stop();
  });

  it("connect success: sets connecting state, session, scale mode and focuses the canvas", async () => {
    const { api, invoke, scope } = createHarness();
    const surface = buildSurface();
    api.vncSurface.value = surface;
    invoke.mockResolvedValueOnce(startResult());

    const ok = await api.startVncSession(connectOptions({ scaleMode: "stretch", password: "s3cret" }));

    expect(ok).toBe(true);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("vnc/start", {
      connectionId: "connection-1",
      workbenchId: "workbench-1",
      host: "10.0.0.7",
      port: 5900,
      runtimeHost: "runtime.local",
      runtimePort: 5901,
      scaleMode: "stretch",
      password: "s3cret",
    });
    expect(api.vncSession.value).toEqual(startResult());
    expect(api.vncState.value).toBe("connecting");
    expect(api.vncError.value).toBe("");
    expect(api.vncScaleMode.value).toBe("stretch");
    expect(api.isVncMode.value).toBe(true);
    expect(api.vncTarget.value).toBe("10.0.0.7:5900");
    expect(surface.reset).toHaveBeenCalledTimes(1);
    await nextTick();
    expect(surface.canvasFocus).toHaveBeenCalledTimes(1);
    scope.stop();
  });

  it("connect success: runtime endpoint falls back to form host/port when the context is empty", async () => {
    const invoke = vi.fn().mockResolvedValueOnce(startResult());
    installBridge(invoke);
    const scope = effectScope();
    const api = scope.run(() =>
      useVncSession({
        t: (key: string) => key,
        showError: vi.fn(),
        showNotice: vi.fn(),
        connectionId: ref(""),
        workbenchId: ref("workbench-1"),
        runtimeEndpoint: ref({}),
        isDisposed: () => false,
        getTerminal: () => undefined,
      }),
    )!;

    await api.startVncSession(connectOptions());

    expect(invoke).toHaveBeenCalledWith("vnc/start", {
      connectionId: undefined,
      workbenchId: "workbench-1",
      host: "10.0.0.7",
      port: 5900,
      runtimeHost: "10.0.0.7",
      runtimePort: 5900,
      scaleMode: "fit",
    });
    scope.stop();
  });

  it("connect failure: reports through showError and leaves idle state", async () => {
    const { api, invoke, showError, scope } = createHarness();
    const cause = new Error("connection refused");
    invoke.mockRejectedValueOnce(cause);

    const ok = await api.startVncSession(connectOptions());

    expect(ok).toBe(false);
    expect(showError).toHaveBeenCalledWith(cause, "terminal");
    expect(api.vncSession.value).toBeNull();
    expect(api.vncState.value).toBe("idle");
  });

  it("connect race with unmount: closes the newly created sidecar session and stays idle", async () => {
    const { api, invoke, setDisposed, scope } = createHarness();
    invoke.mockResolvedValueOnce(startResult());

    setDisposed(true);
    const ok = await api.startVncSession(connectOptions());

    expect(ok).toBe(false);
    expect(api.vncSession.value).toBeNull();
    expect(api.vncState.value).toBe("idle");
    expect(invoke).toHaveBeenCalledWith("vnc/close", { sessionId: "vnc-1" });
    scope.stop();
  });

  it("replaces a live session before reconnecting (implicit close of the previous session)", async () => {
    const { api, invoke, scope } = createHarness();
    invoke
      .mockResolvedValueOnce(startResult())
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce({ ...startResult(), sessionId: "vnc-2" });

    await api.startVncSession(connectOptions());
    const ok = await api.startVncSession(connectOptions({ host: "10.0.0.8" }));

    expect(ok).toBe(true);
    expect(invoke).toHaveBeenNthCalledWith(2, "vnc/close", { sessionId: "vnc-1" });
    expect(api.vncSession.value?.sessionId).toBe("vnc-2");
    expect(api.vncState.value).toBe("connecting");
    scope.stop();
  });
});

describe("useVncSession close and cleanup", () => {
  it("close: clears session/state/error, sends vnc/close and returns focus to the terminal", async () => {
    const { api, invoke, terminalFocus, scope } = createHarness();
    invoke.mockResolvedValueOnce(startResult()).mockResolvedValueOnce(undefined);
    await api.startVncSession(connectOptions());
    api.markVncClosed("auth failed");

    await api.closeVncSession();

    expect(invoke).toHaveBeenNthCalledWith(2, "vnc/close", { sessionId: "vnc-1" });
    expect(api.vncSession.value).toBeNull();
    expect(api.vncState.value).toBe("idle");
    expect(api.vncError.value).toBe("");
    expect(api.isVncMode.value).toBe(false);
    expect(terminalFocus).toHaveBeenCalledTimes(1);
    scope.stop();
  });

  it("close without a session: no RPC and no focus change (matches the original early return)", async () => {
    const { api, invoke, terminalFocus, scope } = createHarness();
    await api.closeVncSession();
    expect(invoke).not.toHaveBeenCalled();
    expect(terminalFocus).not.toHaveBeenCalled();
    scope.stop();
  });

  it("markVncClosed: closed is sticky and keeps the first error text", async () => {
    const { api, invoke, scope } = createHarness();
    invoke.mockResolvedValueOnce(startResult());
    await api.startVncSession(connectOptions());

    api.markVncClosed("tight-only server");
    expect(api.vncState.value).toBe("closed");
    expect(api.vncError.value).toBe("tight-only server");

    api.markVncClosed("second reason");
    expect(api.vncError.value).toBe("tight-only server");

    api.markVncClosed(null);
    expect(api.vncError.value).toBe("tight-only server");
    scope.stop();
  });

  it("markVncClosed: ignored when no session is bound", () => {
    const { api, scope } = createHarness();
    api.markVncClosed("orphan error");
    expect(api.vncState.value).toBe("idle");
    expect(api.vncError.value).toBe("");
    scope.stop();
  });

  it("unmount cleanup releases the surface handle", async () => {
    const { api, scope } = createHarness();
    api.vncSurface.value = buildSurface();
    scope.stop();
    expect(api.vncSurface.value).toBeNull();
  });
});

describe("useVncSession without the host bridge", () => {
  const bridgeWindow = window as unknown as { dbxPlugin?: unknown };

  it("close: tears the session down and restores terminal focus without a sync TypeError", async () => {
    const { api, invoke, terminalFocus, scope } = createHarness();
    invoke.mockResolvedValueOnce(startResult());
    await api.startVncSession(connectOptions());
    delete bridgeWindow.dbxPlugin;

    await expect(api.closeVncSession()).resolves.toBeUndefined();

    expect(api.vncSession.value).toBeNull();
    expect(api.vncState.value).toBe("idle");
    expect(terminalFocus).toHaveBeenCalledTimes(1);
    scope.stop();
  });

  it("input/clipboard: non-fatal (rejection swallowed) and clipboard still reports the notice", async () => {
    const { api, invoke, showNotice, scope } = createHarness();
    invoke.mockResolvedValueOnce(startResult());
    await api.startVncSession(connectOptions());
    delete bridgeWindow.dbxPlugin;

    expect(() => api.sendVncInput({ kind: "key", keysym: 65, pressed: true })).not.toThrow();
    expect(() => api.sendVncClipboard("hello")).not.toThrow();
    // 让 rpc 的 RpcUnavailableError 拒绝落到 .catch（未处理拒绝会让 vitest 失败）。
    await Promise.resolve();
    await Promise.resolve();

    expect(showNotice).toHaveBeenCalledWith("vnc.clipboardSent");
    scope.stop();
  });
});

describe("useVncSession frames, reattach and input", () => {
  it("frame hook: ignores frames from other sessions and promotes the matching session to running", async () => {
    const { api, invoke, scope } = createHarness();
    const surface = buildSurface();
    api.vncSurface.value = surface;
    invoke.mockResolvedValueOnce(startResult());
    await api.startVncSession(connectOptions());
    expect(api.vncState.value).toBe("connecting");

    const payload = new Uint8Array([1, 2, 3]);
    api.handleVncFrame("vnc-other", payload);
    expect(api.vncState.value).toBe("connecting");
    expect(surface.acceptFrame).not.toHaveBeenCalled();

    api.handleVncFrame("vnc-1", payload);
    expect(api.vncState.value).toBe("running");
    expect(surface.acceptFrame).toHaveBeenCalledWith(payload);
    scope.stop();
  });

  it("reattach: restores the live sidecar session and requests replay", async () => {
    const { api, invoke, scope } = createHarness();
    invoke.mockResolvedValueOnce(undefined);

    await api.reattachVncSession({ sessionId: "vnc-live" });

    expect(api.vncSession.value).toEqual({ sessionId: "vnc-live", host: "", port: 5900 });
    expect(api.vncState.value).toBe("running");
    expect(invoke).toHaveBeenCalledWith("vnc/replay", { sessionId: "vnc-live" });
    scope.stop();
  });

  it("reattach keeps live host/port when provided", async () => {
    const { api, invoke, scope } = createHarness();
    invoke.mockResolvedValueOnce(undefined);
    await api.reattachVncSession({ sessionId: "vnc-live", host: "10.0.0.9", port: 5902 });
    expect(api.vncSession.value).toEqual({ sessionId: "vnc-live", host: "10.0.0.9", port: 5902 });
    scope.stop();
  });

  it("input/clipboard: forwards the exact VncInputEvent and reports clipboard sends", async () => {
    const { api, invoke, showNotice, scope } = createHarness();
    invoke.mockResolvedValueOnce(startResult()).mockResolvedValue(undefined);
    await api.startVncSession(connectOptions());

    const event = { kind: "pointer" as const, x: 12, y: 34, buttonMask: 1 };
    api.sendVncInput(event);
    expect(invoke).toHaveBeenNthCalledWith(2, "vnc/input", { sessionId: "vnc-1", ...event });

    api.sendVncClipboard("hello");
    expect(invoke).toHaveBeenNthCalledWith(3, "vnc/set-clipboard", { sessionId: "vnc-1", text: "hello" });
    expect(showNotice).toHaveBeenCalledWith("vnc.clipboardSent");

    api.sendVncClipboard("");
    expect(invoke).toHaveBeenCalledTimes(3);
    scope.stop();
  });

  it("input/clipboard without a session stays a no-op", () => {
    const { api, invoke, showNotice, scope } = createHarness();
    api.sendVncInput({ kind: "key", keysym: 65, pressed: true });
    api.sendVncClipboard("hello");
    expect(invoke).not.toHaveBeenCalled();
    expect(showNotice).not.toHaveBeenCalled();
    scope.stop();
  });
});
