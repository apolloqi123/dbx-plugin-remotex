// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { effectScope } from "vue";
import { useVncRemoteClipboard } from "./useVncRemoteClipboard";

type RpcHandler = (params: Record<string, unknown>) => unknown;

interface Harness {
  controller: ReturnType<typeof useVncRemoteClipboard>;
  invoke: ReturnType<typeof vi.fn>;
  showNotice: ReturnType<typeof vi.fn>;
  emit(method: string, params: Record<string, unknown>): void;
  dispose(): void;
}

const cleanups: Array<() => void> = [];

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  delete (window as unknown as { dbxPlugin?: unknown }).dbxPlugin;
  vi.restoreAllMocks();
});

function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

async function blur(): Promise<void> {
  window.dispatchEvent(new Event("blur"));
  await flush();
  await flush();
}

function createHarness(handlers: Record<string, RpcHandler> = {}): Harness {
  const listeners: Array<(event: { method: string; params: Record<string, unknown> }) => void> = [];
  const base: Record<string, RpcHandler> = {
    "filetransfer/clipboard/remote": () => ({ supported: true, paths: ["/home/demo/.bash_history"] }),
    "filetransfer/clipboard/fetch": () => ({ taskId: "clipboard-task" }),
    ...handlers,
  };
  const invoke = vi.fn(async (method: string, params: Record<string, unknown>) => {
    const handler = base[method];
    if (!handler) throw new Error(`unexpected rpc: ${method}`);
    return handler(params);
  });
  const onEvent = vi.fn((listener: (event: { method: string; params: Record<string, unknown> }) => void) => {
    listeners.push(listener);
    return () => undefined;
  });
  (window as unknown as { dbxPlugin?: unknown }).dbxPlugin = { invoke, onEvent };
  const showError = vi.fn();
  const showNotice = vi.fn();
  const scope = effectScope();
  const controller = scope.run(() =>
    useVncRemoteClipboard({
      t: (key, values) => `${key}${values?.count !== undefined ? `:${values.count}` : ""}`,
      showError,
      showNotice,
      sessionId: () => "sftp-1",
      active: () => true,
      busy: () => false,
      pollIntervalMs: 1_000_000,
      probeDelayMs: 0,
    }),
  )!;
  cleanups.push(() => scope.stop());
  return {
    controller,
    invoke,
    showNotice,
    emit(method, params) {
      for (const listener of listeners) listener({ method, params });
    },
    dispose: () => scope.stop(),
  };
}

describe("useVncRemoteClipboard", () => {
  it("fetches copied remote files on window blur and reports the clipboard-ready result", async () => {
    const harness = createHarness();
    await harness.controller.check();
    // Copying inside the VNC desktop alone must not touch the Mac clipboard.
    expect(harness.invoke).not.toHaveBeenCalledWith("filetransfer/clipboard/fetch", expect.anything());
    await blur();
    expect(harness.invoke).toHaveBeenCalledWith(
      "filetransfer/clipboard/fetch",
      expect.objectContaining({
        sessionId: "sftp-1",
        remotePaths: ["/home/demo/.bash_history"],
      }),
    );
    const requestId = harness.invoke.mock.calls.find(([method]) => method === "filetransfer/clipboard/fetch")![1].requestId as string;
    harness.emit("filetransfer/clipboard/ready", {
      requestId,
      sessionId: "sftp-1",
      localPaths: ["/Users/demo/Downloads/DBX Remote Clipboard/.bash_history"],
      clipboard: true,
    });
    await flush();
    expect(harness.showNotice).toHaveBeenCalledWith("remoteFiles.remoteCopyReady:1");
  });

  it("uses a URI list forwarded by the VNC server and waits for blur", async () => {
    const harness = createHarness();
    harness.emit("vnc/clipboard-files", {
      sessionId: "vnc-1",
      paths: ["/home/demo/.bash_history"],
    });
    await flush();
    expect(harness.invoke).not.toHaveBeenCalledWith("filetransfer/clipboard/fetch", expect.anything());
    await blur();
    expect(harness.invoke).toHaveBeenCalledWith(
      "filetransfer/clipboard/fetch",
      expect.objectContaining({ remotePaths: ["/home/demo/.bash_history"] }),
    );
  });

  it("does not fetch the same remote clipboard twice", async () => {
    const harness = createHarness();
    await harness.controller.check();
    await blur();
    await harness.controller.check();
    await blur();
    const fetches = harness.invoke.mock.calls.filter(([method]) => method === "filetransfer/clipboard/fetch");
    expect(fetches).toHaveLength(1);
  });

  it("drops a pending file copy when the remote clipboard becomes text", async () => {
    const harness = createHarness();
    await harness.controller.check();
    harness.emit("vnc/clipboard", { sessionId: "vnc-1", text: "plain text" });
    await blur();
    expect(harness.invoke).not.toHaveBeenCalledWith("filetransfer/clipboard/fetch", expect.anything());
  });

  it("surfaces a missing xclip/xsel only on an explicit copy probe", async () => {
    const harness = createHarness({
      "filetransfer/clipboard/remote": () => ({ supported: false, paths: [] }),
    });
    await harness.controller.check();
    expect(harness.showNotice).not.toHaveBeenCalled();
    harness.controller.probe();
    await flush();
    await flush();
    expect(harness.showNotice).toHaveBeenCalledWith("remoteFiles.remoteClipboardUnavailable");
  });

  it("reports a failed fetch and allows a later retry", async () => {
    const harness = createHarness({
      "filetransfer/clipboard/fetch": () => {
        throw new Error("download rejected");
      },
    });
    await harness.controller.check();
    await blur();
    expect(harness.controller.fetching.value).toBe(false);
    await harness.controller.check();
    await blur();
    const fetches = harness.invoke.mock.calls.filter(([method]) => method === "filetransfer/clipboard/fetch");
    expect(fetches).toHaveLength(2);
  });
});
