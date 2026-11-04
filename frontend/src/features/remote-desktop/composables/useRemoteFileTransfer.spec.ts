// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { effectScope } from "vue";
import { useRemoteFileTransfer, type RemoteFileTransferController } from "./useRemoteFileTransfer";

type RpcHandler = (params: Record<string, unknown>) => unknown;

interface Harness {
  controller: RemoteFileTransferController;
  invoke: ReturnType<typeof vi.fn>;
  showError: ReturnType<typeof vi.fn>;
  showNotice: ReturnType<typeof vi.fn>;
  emit(event: { method: string; params: Record<string, unknown> }): void;
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

function createHarness(
  handlers: Record<string, RpcHandler> = {},
  options: { storedDesktopCredentials?: boolean } = {},
): Harness {
  const listeners: Array<(event: { method: string; params: Record<string, unknown> }) => void> = [];
  const base: Record<string, RpcHandler> = {
    "filetransfer/local/list": () => ({
      path: "/Users/demo",
      parent: "/Users",
      home: "/Users/demo",
      drives: [],
      entries: [
        { name: "docs", path: "/Users/demo/docs", kind: "directory", size: 0, hidden: false, symlink: false },
        { name: "a.txt", path: "/Users/demo/a.txt", kind: "file", size: 5, hidden: false, symlink: false },
      ],
    }),
    "filetransfer/connect": (params) => {
      const password = typeof params.password === "string" ? params.password : "";
      if (!password && !options.storedDesktopCredentials) {
        throw new Error("No stored SSH credentials for this connection; enter an SSH password.");
      }
      return {
        sessionId: "sftp-1",
        home: "/home/demo",
        host: "example.test",
        username: "dev",
      };
    },
    "sftp/list": () => ({
      entries: [
        { name: "logs", uri: "sftp:/home/demo/logs", kind: "directory", size: 0 },
        { name: "b.txt", uri: "sftp:/home/demo/b.txt", kind: "file", size: 9 },
      ],
    }),
    "filetransfer/upload/start": () => ({ taskId: "upload-1" }),
    "filetransfer/download/start": () => ({ taskId: "download-1" }),
    "filetransfer/cancel": () => ({ success: true }),
    "filetransfer/ping": () => ({ alive: true }),
    "filetransfer/disconnect": () => ({ success: true }),
    "filetransfer/credentials/forget": () => ({ success: true }),
    "filetransfer/clipboard/files": () => ({ supported: true, files: ["/tmp/copied-one.txt"] }),
    "filetransfer/clipboard/write": () => ({ success: true, count: 1 }),
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
  (window as unknown as { dbxPlugin?: unknown }).dbxPlugin = {
    invoke,
    onEvent,
    encodeBase64: (value: Uint8Array) => btoa(String.fromCharCode(...value)),
    decodeBase64: (value: string) => Uint8Array.from(atob(value), (char) => char.charCodeAt(0)),
  };
  const showError = vi.fn();
  const showNotice = vi.fn();
  const scope = effectScope();
  const controller = scope.run(() =>
    useRemoteFileTransfer({
      t: (key, values) => `${key}${values?.count !== undefined ? `:${values.count}` : ""}`,
      showError,
      showNotice,
      workbenchId: () => "wb-1",
      connectionId: () => "conn-1",
      initialHost: () => "example.test",
      initialUsername: () => "dev",
    }),
  ) as RemoteFileTransferController;
  cleanups.push(() => scope.stop());
  return {
    controller,
    invoke,
    showError,
    showNotice,
    emit(event) {
      const params =
        event.method.startsWith("filetransfer/")
          ? { sessionId: "sftp-1", ...event.params }
          : event.params;
      for (const listener of listeners) listener({ ...event, params });
    },
    dispose: () => scope.stop(),
  };
}

describe("useRemoteFileTransfer", () => {
  it("connects over SSH and loads both panes", async () => {
    const harness = createHarness();
    const c = harness.controller;
    c.open();
    await flush();
    expect(c.local.path).toBe("/Users/demo");
    expect(c.local.entries.map((entry) => entry.name)).toEqual(["docs", "a.txt"]);
    expect(c.form.host).toBe("example.test");
    expect(c.form.username).toBe("dev");
    c.form.password = "secret";
    await c.connect();
    expect(harness.invoke).toHaveBeenCalledWith(
      "filetransfer/connect",
      expect.objectContaining({ host: "example.test", username: "dev", authentication: "password" }),
      { timeoutMs: 90_000 },
    );
    expect(c.session.value?.sessionId).toBe("sftp-1");
    expect(c.remote.path).toBe("/home/demo");
    expect(c.remote.entries.map((entry) => entry.path)).toEqual([
      "/home/demo/logs",
      "/home/demo/b.txt",
    ]);
  });

  it("requires a password before connecting", async () => {
    const harness = createHarness();
    const c = harness.controller;
    c.open();
    await flush();
    await c.connect();
    expect(c.connectError.value).toBe("remoteFiles.passwordRequired");
    expect(c.session.value).toBeNull();
  });

  it("runs an upload and refreshes the remote pane when it finishes", async () => {
    const harness = createHarness();
    const c = harness.controller;
    c.open();
    await flush();
    c.form.password = "secret";
    await c.connect();
    const listCalls = harness.invoke.mock.calls.filter(([method]) => method === "sftp/list").length;
    c.toggleSelection(c.local, c.local.entries[1]);
    c.upload();
    await flush();
    expect(harness.invoke).toHaveBeenCalledWith(
      "filetransfer/upload/start",
      expect.objectContaining({
        sessionId: "sftp-1",
        localPaths: ["/Users/demo/a.txt"],
        remoteDir: "/home/demo",
      }),
    );
    harness.emit({
      method: "filetransfer/progress",
      params: {
        taskId: "upload-1",
        direction: "upload",
        status: "running",
        file: "a.txt",
        filesDone: 0,
        filesTotal: 1,
        bytesDone: 2,
        bytesTotal: 5,
        percent: 40,
      },
    });
    expect(c.progress.value?.percent).toBe(40);
    harness.emit({
      method: "filetransfer/done",
      params: { taskId: "upload-1", direction: "upload", filesDone: 1, filesTotal: 1, skipped: 0 },
    });
    await flush();
    expect(c.progress.value?.status).toBe("done");
    expect(c.local.selected).toEqual([]);
    expect(harness.showNotice).toHaveBeenCalledWith("remoteFiles.uploaded:1");
    const after = harness.invoke.mock.calls.filter(([method]) => method === "sftp/list").length;
    expect(after).toBe(listCalls + 1);
  });

  it("downloads into the local pane and exposes clipboard actions", async () => {
    const harness = createHarness({
      "filetransfer/download/start": () => ({ taskId: "download-1" }),
    });
    const c = harness.controller;
    c.open();
    await flush();
    c.form.password = "secret";
    await c.connect();
    c.toggleSelection(c.remote, c.remote.entries[1]);
    c.download();
    await flush();
    harness.emit({
      method: "filetransfer/progress",
      params: {
        taskId: "download-1",
        direction: "download",
        status: "running",
        file: "b.txt",
        filesDone: 0,
        filesTotal: 1,
        bytesDone: 9,
        bytesTotal: 9,
        percent: 100,
      },
    });
    harness.emit({
      method: "filetransfer/done",
      params: {
        taskId: "download-1",
        direction: "download",
        filesDone: 1,
        filesTotal: 1,
        localPaths: ["/Users/demo/b.txt"],
      },
    });
    await flush();
    expect(c.lastDownloaded.value).toEqual(["/Users/demo/b.txt"]);
    expect(c.remote.selected).toEqual([]);
    expect(c.clipboardSupported.value).toBe(true);
    await c.writeDownloadedToClipboard();
    expect(harness.invoke).toHaveBeenCalledWith("filetransfer/clipboard/write", {
      paths: ["/Users/demo/b.txt"],
    });
    expect(harness.showNotice).toHaveBeenCalledWith("remoteFiles.clipboardWritten:1");
  });

  it("detects Finder file clipboard paths instead of treating them as text", async () => {
    const harness = createHarness();
    const files = await harness.controller.detectClipboardFiles();
    expect(files).toEqual(["/tmp/copied-one.txt"]);
    expect(harness.controller.clipboardSupported.value).toBe(true);
    harness.controller.queueLocalPaths(["/tmp/copied-one.txt", "/tmp/copied-one.txt"]);
    expect(harness.controller.pendingLocalPaths.value).toEqual(["/tmp/copied-one.txt"]);
  });

  it("keeps a custom SSH host and username instead of overwriting them", async () => {
    const harness = createHarness();
    const c = harness.controller;
    c.form.host = "custom.ssh";
    c.form.username = "ops";
    c.open();
    await flush();
    expect(c.form.host).toBe("custom.ssh");
    expect(c.form.username).toBe("ops");
  });

  it("auto-connects with the desktop connection's stored credentials", async () => {
    const harness = createHarness({}, { storedDesktopCredentials: true });
    const c = harness.controller;
    c.open();
    await flush();
    await flush();
    expect(c.session.value?.sessionId).toBe("sftp-1");
    expect(harness.invoke).toHaveBeenCalledWith(
      "filetransfer/connect",
      expect.objectContaining({ connectionId: "conn-1", password: "" }),
      { timeoutMs: 90_000 },
    );
  });

  it("auto-connects on reopen after a password login in the same workbench", async () => {
    const harness = createHarness();
    const c = harness.controller;
    c.open();
    await flush();
    c.form.password = "secret";
    await c.connect();
    expect(c.session.value?.sessionId).toBe("sftp-1");
    await c.disconnect();
    c.open();
    await flush();
    await flush();
    expect(c.session.value?.sessionId).toBe("sftp-1");
    const connects = harness.invoke.mock.calls.filter(([method]) => method === "filetransfer/connect");
    // First open tries the stored desktop credentials (none), then the manual
    // login, then the reopen auto-connect uses the cached password.
    expect(connects).toHaveLength(3);
  });

  it("forgets a saved SSH password from the form", async () => {
    const harness = createHarness();
    const c = harness.controller;
    c.form.host = "example.test";
    c.form.username = "dev";
    c.form.password = "secret";
    await c.forgetSavedPassword();
    expect(harness.invoke).toHaveBeenCalledWith("filetransfer/credentials/forget", {
      host: "example.test",
      port: 22,
      username: "dev",
    });
    expect(c.form.password).toBe("");
    expect(harness.showNotice).toHaveBeenCalledWith("remoteFiles.passwordForgotten");
  });

  it("disconnects and clears remote state", async () => {
    const harness = createHarness();
    const c = harness.controller;
    c.open();
    await flush();
    c.form.password = "secret";
    await c.connect();
    await c.disconnect();
    expect(harness.invoke).toHaveBeenCalledWith("filetransfer/disconnect", { sessionId: "sftp-1" });
    expect(c.session.value).toBeNull();
    expect(c.remote.entries).toEqual([]);
  });
});
