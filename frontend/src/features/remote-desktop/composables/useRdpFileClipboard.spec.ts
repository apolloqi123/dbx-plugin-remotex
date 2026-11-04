// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { effectScope, ref } from "vue";
import {
  CLIPBOARD_FILE_LIMIT,
  CLIPBOARD_TOTAL_BYTES_LIMIT,
  base64FromBytes,
  bytesFromBase64,
  normalizeHostFiles,
} from "../lib/clipboardFiles";
import { useRdpFileClipboard, type RdpFileClipboardApi } from "./useRdpFileClipboard";

const cleanups: Array<() => void> = [];

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  delete (window as unknown as { dbxPlugin?: unknown }).dbxPlugin;
  vi.restoreAllMocks();
});

function encodeText(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function fromBase64(text: string): Uint8Array {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

function textOf(base64: string): string {
  return new TextDecoder().decode(fromBase64(base64));
}

function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

type RpcHandler = (params: Record<string, unknown>) => unknown;

interface Harness {
  api: RdpFileClipboardApi;
  sessionId: ReturnType<typeof ref<string | null>>;
  invoke: ReturnType<typeof vi.fn>;
  pick: ReturnType<typeof vi.fn>;
  onDrop: ReturnType<typeof vi.fn>;
  unsubscribeDrop: ReturnType<typeof vi.fn>;
  beginSave: ReturnType<typeof vi.fn>;
  write: ReturnType<typeof vi.fn>;
  finish: ReturnType<typeof vi.fn>;
  cancel: ReturnType<typeof vi.fn>;
  showError: ReturnType<typeof vi.fn>;
  showNotice: ReturnType<typeof vi.fn>;
  dropListener(): ((files: unknown[]) => void) | undefined;
  setHostFile(handleId: string, bytes: Uint8Array): void;
  setActive(value: boolean): void;
  setEnabled(value: boolean): void;
}

function createHarness(options: { handlers?: Record<string, RpcHandler> } = {}): Harness {
  let active = true;
  let enabled = true;
  const sessionId = ref<string | null>("session-1");
  const hostFiles = new Map<string, Uint8Array>();
  const dropListeners: Array<(files: unknown[]) => void> = [];
  const unsubscribeDrop = vi.fn();

  const baseHandlers: Record<string, RpcHandler> = {
    "rdp/clipboard-files/begin": () => ({ uploadId: "upload-1", chunkBytes: 4 }),
    "rdp/clipboard-files/chunk": () => ({ received: 0 }),
    "rdp/clipboard-files/commit": () => ({ success: true, fileCount: 1, totalBytes: 0 }),
    "rdp/clipboard-files/abort": () => ({ success: true }),
    "rdp/clipboard-files/clear": () => ({ success: true }),
    "rdp/clipboard-files/read": () => ({ dataBase64: "", eof: true, total: 0 }),
    "rdp/clipboard-files/download": () => ({ success: true, downloading: true }),
    "rdp/clipboard-files/status": () => ({
      remoteFiles: [],
      remoteTotalBytes: 0,
      archiveReady: false,
    }),
    ...options.handlers,
  };

  const invoke = vi.fn(async (method: string, params: Record<string, unknown>) => {
    const handler = baseHandlers[method];
    if (!handler) throw new Error(`unexpected rpc: ${method}`);
    return handler(params);
  });

  const read = vi.fn(async (handleId: string, offset: number, length: number) => {
    const bytes = hostFiles.get(handleId) ?? new Uint8Array(0);
    const slice = bytes.subarray(offset, offset + length);
    return { dataBase64: toBase64(slice), length: slice.length, eof: offset + slice.length >= bytes.length };
  });
  const pick = vi.fn(async (): Promise<unknown> => ({ files: [] }));
  const onDrop = vi.fn((listener: (files: unknown[]) => void) => {
    dropListeners.push(listener);
    return unsubscribeDrop;
  });
  const beginSave = vi.fn(async (): Promise<{ handleId: string; chunkBytes?: number } | null> => null);
  const write = vi.fn(async (_handleId: string, offset: number, data: Uint8Array) => ({
    written: data.length,
    nextOffset: offset + data.length,
  }));
  const finish = vi.fn(async () => undefined);
  const cancel = vi.fn(async () => undefined);

  (window as unknown as { dbxPlugin?: unknown }).dbxPlugin = {
    invoke,
    fileTransfer: { read, pick, onDrop, beginSave, write, finish, cancel },
    encodeBase64: toBase64,
    decodeBase64: fromBase64,
  };

  const showError = vi.fn();
  const showNotice = vi.fn();
  const t = vi.fn((key: string) => key);
  const scope = effectScope();
  const api = scope.run(() =>
    useRdpFileClipboard({
      t,
      showError,
      showNotice,
      sessionId,
      active: () => active,
      enabled: () => enabled,
    }),
  );
  if (!api) throw new Error("useRdpFileClipboard did not initialize");
  cleanups.push(() => scope.stop());

  return {
    api,
    sessionId,
    invoke,
    pick,
    onDrop,
    unsubscribeDrop,
    beginSave,
    write,
    finish,
    cancel,
    showError,
    showNotice,
    dropListener: () => dropListeners[dropListeners.length - 1],
    setHostFile: (handleId, bytes) => {
      hostFiles.set(handleId, bytes);
    },
    setActive: (value) => {
      active = value;
    },
    setEnabled: (value) => {
      enabled = value;
    },
  };
}

function callsTo(harness: Harness, method: string): Array<Record<string, unknown>> {
  return harness.invoke.mock.calls
    .filter(([name]) => name === method)
    .map(([, params]) => params as Record<string, unknown>);
}

describe("normalizeHostFiles", () => {
  it("drops unsafe names and handles, sanitizes relative paths and clamps sizes", () => {
    const files = normalizeHostFiles([
      { handleId: "h1", name: "../../etc/passwd", relativePath: "../a/./b.txt", size: -5, contentType: "text/plain" },
      { handleId: "", name: "missing-handle.txt", size: 1 },
      { handleId: "h2", name: "", size: 1 },
      { handleId: "h3", name: "C:\\temp\\file.txt", size: 1.9 },
    ]);

    expect(files).toEqual([
      { handleId: "h1", name: "passwd", relativePath: "a/b.txt", size: 0, contentType: "text/plain" },
      { handleId: "h3", name: "file.txt", size: 1 },
    ]);
  });

  it("enforces the file count and total size caps", () => {
    const tooMany = Array.from({ length: CLIPBOARD_FILE_LIMIT + 1 }, (_, index) => ({
      handleId: `h${index}`,
      name: `f${index}.txt`,
      size: 0,
    }));
    expect(() => normalizeHostFiles(tooMany)).toThrow(String(CLIPBOARD_FILE_LIMIT));

    expect(() =>
      normalizeHostFiles([{ handleId: "big", name: "big.bin", size: CLIPBOARD_TOTAL_BYTES_LIMIT + 1 }]),
    ).toThrow(/512 MiB/);
  });
});

describe("base64 helpers", () => {
  it("round-trips payloads larger than one chunk without the host bridge", () => {
    delete (window as unknown as { dbxPlugin?: unknown }).dbxPlugin;
    const bytes = new Uint8Array(200_000);
    for (let index = 0; index < bytes.length; index += 1) bytes[index] = index % 251;

    expect(bytesFromBase64(base64FromBytes(bytes))).toEqual(bytes);
  });
});

describe("useRdpFileClipboard upload", () => {
  it("streams each host file with per-file offsets and commits", async () => {
    const progressAtChunk: Array<unknown> = [];
    let harness: Harness;
    harness = createHarness({
      handlers: {
        "rdp/clipboard-files/chunk": () => {
          progressAtChunk.push(harness.api.progress.value ? { ...harness.api.progress.value } : null);
          return { received: 1 };
        },
      },
    });
    harness.setHostFile("h1", encodeText("abcdefghij"));
    harness.setHostFile("h2", encodeText("XYZ"));

    const sent = await harness.api.sendFiles([
      { handleId: "h1", name: "a.txt", size: 10 },
      { handleId: "h2", name: "b.txt", size: 3, relativePath: "dir/b.txt" },
    ]);

    expect(sent).toBe(true);
    expect(callsTo(harness, "rdp/clipboard-files/begin")).toEqual([
      {
        sessionId: "session-1",
        files: [
          { name: "a.txt", size: 10 },
          { name: "b.txt", size: 3, relativePath: "dir/b.txt" },
        ],
      },
    ]);
    const chunks = callsTo(harness, "rdp/clipboard-files/chunk");
    expect(chunks.map((params) => [params.index, params.offset, textOf(String(params.dataBase64))])).toEqual([
      [0, 0, "abcd"],
      [0, 4, "efgh"],
      [0, 8, "ij"],
      [1, 0, "XYZ"],
    ]);
    expect(callsTo(harness, "rdp/clipboard-files/commit")).toEqual([
      { sessionId: "session-1", uploadId: "upload-1" },
    ]);
    expect(callsTo(harness, "rdp/clipboard-files/abort")).toEqual([]);
    expect(harness.showNotice).toHaveBeenCalledWith("rdp.clipboardFiles.sent");
    expect(progressAtChunk[0]).toMatchObject({ phase: "uploading", done: 0, total: 13, filesDone: 0, filesTotal: 2 });
    expect(progressAtChunk[progressAtChunk.length - 1]).toMatchObject({ done: 10, total: 13, filesDone: 1, filesTotal: 2 });
    expect(harness.api.progress.value).toBeNull();
    expect(harness.api.busy.value).toBe(false);
  });

  it("aborts the upload and reports the failure when a chunk RPC fails", async () => {
    const harness = createHarness({
      handlers: {
        "rdp/clipboard-files/chunk": () => {
          throw new Error("chunk rejected");
        },
      },
    });
    harness.setHostFile("h1", encodeText("abcdef"));

    const sent = await harness.api.sendFiles([{ handleId: "h1", name: "a.txt", size: 6 }]);

    expect(sent).toBe(false);
    expect(callsTo(harness, "rdp/clipboard-files/abort")).toEqual([
      { sessionId: "session-1", uploadId: "upload-1" },
    ]);
    expect(callsTo(harness, "rdp/clipboard-files/commit")).toEqual([]);
    expect(harness.showError).toHaveBeenCalledTimes(1);
    expect(harness.api.error.value).toBe("chunk rejected");
    expect(harness.api.progress.value).toBeNull();
    expect(harness.api.busy.value).toBe(false);
  });

  it("refuses to send while inactive or disabled", async () => {
    const harness = createHarness();
    harness.setHostFile("h1", encodeText("x"));

    harness.setActive(false);
    expect(await harness.api.sendFiles([{ handleId: "h1", name: "a.txt", size: 1 }])).toBe(false);
    harness.setActive(true);
    harness.setEnabled(false);
    expect(await harness.api.sendFiles([{ handleId: "h1", name: "a.txt", size: 1 }])).toBe(false);
    expect(callsTo(harness, "rdp/clipboard-files/begin")).toEqual([]);
  });

});

describe("useRdpFileClipboard save", () => {
  const archive = encodeText("archive-data");

  function prepareSaveTransport(harness: Harness): void {
    harness.beginSave.mockResolvedValue({ handleId: "save-1", chunkBytes: 4 });
    const readHandler: RpcHandler = (params) => {
      const offset = Number(params.offset);
      const slice = archive.subarray(offset, offset + Number(params.length));
      return {
        dataBase64: toBase64(slice),
        eof: offset + slice.length >= archive.length,
        total: archive.length,
      };
    };
    const invoke = harness.invoke.getMockImplementation() as
      | ((method: string, params: Record<string, unknown>) => unknown)
      | undefined;
    harness.invoke.mockImplementation(async (method: string, params: Record<string, unknown>) => {
      if (method === "rdp/clipboard-files/read") return readHandler(params);
      if (method === "rdp/clipboard-files/archive") {
        return { archiveName: "clipboard.zip", archiveSize: archive.length };
      }
      if (!invoke) throw new Error(`unexpected rpc: ${method}`);
      return invoke(method, params);
    });
  }

  function prepareArchive(harness: Harness): void {
    harness.api.handleEvent("rdp/clipboard-files/ready", {
      sessionId: "session-1",
      archiveName: "clipboard.zip",
      archiveSize: archive.length,
      fileCount: 2,
      files: [],
    });
    prepareSaveTransport(harness);
  }

  it("builds the save archive on demand after an automatic download", async () => {
    const harness = createHarness();
    harness.api.handleEvent("rdp/clipboard-files/ready", {
      sessionId: "session-1",
      fileCount: 2,
      files: [],
      pasteboard: true,
    });
    expect(harness.api.remoteReady.value).toEqual({ fileCount: 2, pasteboard: true });
    expect(harness.api.archiveReady.value).toBeNull();
    expect(harness.showNotice).toHaveBeenCalledWith("rdp.clipboardFiles.pasteboardReady");

    prepareSaveTransport(harness);
    expect(await harness.api.saveRemoteArchive()).toBe(true);
    expect(callsTo(harness, "rdp/clipboard-files/archive")).toHaveLength(1);
    expect(harness.beginSave).toHaveBeenCalledWith({
      name: "clipboard.zip",
      contentType: "application/zip",
      size: archive.length,
    });
    expect(harness.api.remoteReady.value).toBeNull();
  });

  it("streams the remote archive into a host save handle", async () => {
    const harness = createHarness();
    prepareArchive(harness);
    const phases: string[] = [];
    harness.write.mockImplementation(async (_handleId: string, offset: number, data: Uint8Array) => {
      phases.push(String(harness.api.progress.value?.phase));
      return { written: data.length, nextOffset: offset + data.length };
    });

    expect(await harness.api.saveRemoteArchive()).toBe(true);

    expect(harness.beginSave).toHaveBeenCalledWith({
      name: "clipboard.zip",
      contentType: "application/zip",
      size: archive.length,
    });
    expect(harness.write.mock.calls.map(([, offset]) => offset)).toEqual([0, 4, 8]);
    expect(harness.finish).toHaveBeenCalledWith("save-1");
    expect(harness.cancel).not.toHaveBeenCalled();
    expect(phases.every((phase) => phase === "saving")).toBe(true);
    expect(harness.showNotice).toHaveBeenCalledWith("rdp.clipboardFiles.saved");
    expect(harness.api.archiveReady.value).toBeNull();
    expect(harness.api.progress.value).toBeNull();
    await flush();
    expect(callsTo(harness, "rdp/clipboard-files/clear")).toEqual([{ sessionId: "session-1" }]);
  });

  it("cancels the host handle when streaming fails", async () => {
    const harness = createHarness();
    prepareArchive(harness);
    harness.write.mockRejectedValueOnce(new Error("disk full"));

    expect(await harness.api.saveRemoteArchive()).toBe(false);

    expect(harness.cancel).toHaveBeenCalledWith("save-1");
    expect(harness.finish).not.toHaveBeenCalled();
    expect(harness.showError).toHaveBeenCalledTimes(1);
    expect(harness.api.error.value).toBe("disk full");
    expect(harness.api.archiveReady.value).not.toBeNull();
    expect(harness.api.progress.value).toBeNull();
    expect(harness.api.busy.value).toBe(false);
  });

  it("aborts when the backend archive changes mid-save", async () => {
    const harness = createHarness();
    prepareArchive(harness);
    const invoke = harness.invoke.getMockImplementation() as
      | ((method: string, params: Record<string, unknown>) => unknown)
      | undefined;
    harness.invoke.mockImplementation(async (method: string, params: Record<string, unknown>) => {
      if (method === "rdp/clipboard-files/read") {
        return { dataBase64: "", eof: true, total: 4, archiveName: "other.zip" };
      }
      if (!invoke) throw new Error(`unexpected rpc: ${method}`);
      return invoke(method, params);
    });

    expect(await harness.api.saveRemoteArchive()).toBe(false);
    expect(harness.cancel).toHaveBeenCalledWith("save-1");
    expect(harness.showError).toHaveBeenCalledTimes(1);
    expect(harness.api.archiveReady.value).not.toBeNull();
  });

  it("returns false when the host save dialog is dismissed", async () => {
    const harness = createHarness();
    prepareArchive(harness);
    harness.beginSave.mockResolvedValue(null);

    expect(await harness.api.saveRemoteArchive()).toBe(false);
    expect(harness.write).not.toHaveBeenCalled();
    expect(harness.cancel).not.toHaveBeenCalled();
    expect(harness.api.busy.value).toBe(false);
  });
});

describe("useRdpFileClipboard events", () => {
  it("ignores events from another session", () => {
    const harness = createHarness();
    harness.api.handleEvent("rdp/clipboard-files/available", {
      sessionId: "other",
      files: [{ name: "a.txt", size: 3, directory: false }],
      totalBytes: 3,
    });

    expect(harness.api.remoteFiles.value).toBeNull();
  });

  it("applies available, progress, ready and error events", () => {
    const harness = createHarness();

    harness.api.handleEvent("rdp/clipboard-files/available", {
      sessionId: "session-1",
      files: [
        { name: "a.txt", size: 3, directory: false },
        { name: "folder", size: 0, directory: true, relativePath: "" },
      ],
      totalBytes: 3,
    });
    expect(harness.api.remoteFiles.value).toEqual({
      files: [
        { name: "a.txt", size: 3, directory: false },
        { name: "folder", size: 0, directory: true },
      ],
      totalBytes: 3,
    });

    harness.api.handleEvent("rdp/clipboard-files/progress", {
      sessionId: "session-1",
      filesDone: 1,
      filesTotal: 2,
      bytesDone: 2,
      bytesTotal: 4,
    });
    expect(harness.api.progress.value).toMatchObject({
      phase: "downloading",
      done: 2,
      total: 4,
      percent: 50,
      filesDone: 1,
      filesTotal: 2,
    });

    harness.api.handleEvent("rdp/clipboard-files/ready", {
      sessionId: "session-1",
      archiveName: "clipboard.zip",
      archiveSize: 4,
      fileCount: 2,
      files: [],
    });
    expect(harness.api.archiveReady.value).toEqual({ name: "clipboard.zip", size: 4, fileCount: 2 });

    harness.api.handleEvent("rdp/clipboard-files/error", {
      sessionId: "session-1",
      message: "remote staging failed",
    });
    expect(harness.api.error.value).toBe("remote staging failed");
    expect(harness.api.progress.value).toBeNull();
    expect(harness.showError).toHaveBeenCalledTimes(1);
    const [cause, target] = harness.showError.mock.calls[0];
    expect(cause).toBeInstanceOf(Error);
    expect((cause as Error).message).toBe("remote staging failed");
    expect(target).toBe("terminal");
  });
});

describe("useRdpFileClipboard drop listener", () => {
  it("forwards drops only while active and enabled", async () => {
    const harness = createHarness();
    harness.setHostFile("h1", encodeText("abc"));
    harness.api.startDropListener();
    const listener = harness.dropListener();
    expect(listener).toBeTypeOf("function");

    harness.setActive(false);
    listener?.([{ handleId: "h1", name: "a.txt", size: 3 }]);
    await flush();
    expect(callsTo(harness, "rdp/clipboard-files/begin")).toEqual([]);

    harness.setActive(true);
    harness.setEnabled(false);
    listener?.([{ handleId: "h1", name: "a.txt", size: 3 }]);
    await flush();
    expect(callsTo(harness, "rdp/clipboard-files/begin")).toEqual([]);

    harness.setEnabled(true);
    listener?.([{ handleId: "h1", name: "a.txt", size: 3 }]);
    await flush();
    expect(callsTo(harness, "rdp/clipboard-files/begin")).toHaveLength(1);
    expect(callsTo(harness, "rdp/clipboard-files/commit")).toHaveLength(1);
  });

  it("subscribes once and releases the listener on stop", () => {
    const harness = createHarness();
    harness.api.startDropListener();
    harness.api.startDropListener();
    expect(harness.onDrop).toHaveBeenCalledTimes(1);
    expect(callsTo(harness, "rdp/clipboard-files/status")).toHaveLength(1);

    harness.api.stopDropListener();
    expect(harness.unsubscribeDrop).toHaveBeenCalledTimes(1);
    harness.api.stopDropListener();
    expect(harness.unsubscribeDrop).toHaveBeenCalledTimes(1);
  });

  it("starts the remote download only on explicit request", async () => {
    const harness = createHarness();
    harness.api.handleEvent("rdp/clipboard-files/available", {
      sessionId: "session-1",
      files: [{ name: "a.txt", size: 5, directory: false }],
      totalBytes: 5,
    });
    expect(harness.api.remoteFiles.value?.files).toHaveLength(1);
    expect(harness.api.progress.value).toBeNull();

    expect(await harness.api.startRemoteDownload()).toBe(true);
    expect(callsTo(harness, "rdp/clipboard-files/download")).toEqual([{ sessionId: "session-1" }]);
    expect(harness.api.progress.value).toMatchObject({ phase: "downloading", done: 0, total: 5 });
  });

  it("does not start a download without remote file metadata", async () => {
    const harness = createHarness();
    expect(await harness.api.startRemoteDownload()).toBe(false);
    expect(callsTo(harness, "rdp/clipboard-files/download")).toEqual([]);
  });

  it("restores status when started for an active session", async () => {
    const harness = createHarness({
      handlers: {
        "rdp/clipboard-files/status": () => ({
          remoteFiles: [{ name: "a.txt", size: 5, directory: false }],
          remoteTotalBytes: 5,
          archiveReady: true,
          archiveName: "clipboard.zip",
          archiveSize: 5,
          fileCount: 1,
          nativeFiles: true,
          serverFiles: true,
          fileClipboardReady: true,
        }),
      },
    });

    harness.api.startDropListener();
    await flush();

    expect(callsTo(harness, "rdp/clipboard-files/status")).toEqual([{ sessionId: "session-1" }]);
    expect(harness.api.remoteFiles.value).toEqual({
      files: [{ name: "a.txt", size: 5, directory: false }],
      totalBytes: 5,
    });
    expect(harness.api.archiveReady.value).toEqual({ name: "clipboard.zip", size: 5, fileCount: 1 });
    expect(harness.api.nativeSupported.value).toBe(true);
    expect(harness.api.serverFiles.value).toBe(true);
    expect(harness.api.serverFilesKnown.value).toBe(true);
  });

  it("marks a text-only server as lacking file streams", () => {
    const harness = createHarness();
    harness.api.setServerCapability(false);
    expect(harness.api.serverFiles.value).toBe(false);
    expect(harness.api.serverFilesKnown.value).toBe(true);
  });

  it("does not restore status while inactive", async () => {
    const harness = createHarness();
    harness.setActive(false);

    harness.api.startDropListener();
    await flush();

    expect(callsTo(harness, "rdp/clipboard-files/status")).toEqual([]);
  });
});