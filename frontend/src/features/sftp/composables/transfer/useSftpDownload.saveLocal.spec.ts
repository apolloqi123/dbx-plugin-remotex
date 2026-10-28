// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { useSftpDownload } from "./useSftpDownload";

const globalWindow = window as unknown as { dbxPlugin?: unknown };
afterEach(() => { delete globalWindow.dbxPlugin; });

function mount() {
  return useSftpDownload({
    t: (key) => key,
    showNotice: vi.fn(),
    showError: vi.fn(),
    session: ref({ sessionId: "s1" }),
    entries: ref<Array<{ uri: string; name: string; kind: "file" | "directory"; size?: number }>>([]),
    fileMenu: ref(undefined),
    localDownloadDir: ref("/downloads"),
    localCanSave: ref(true),
    transferTasks: {},
    cancelledTransferTasks: new Set<string>(),
    downloadChunkWaiters: new Map(),
    waitWhilePaused: () => undefined,
    loadDownloadDir: () => "/downloads",
    loadDownloadUseDefaultDir: () => true,
    askDownloadTarget: vi.fn(async () => undefined),
    applyChosenDirAsDefault: vi.fn(),
    resolveDownloadConflictFor: vi.fn(async () => "rename" as const),
    openTransferPanel: vi.fn(),
    pathFromUri: (uri) => uri,
    sudoDownload: () => false,
    loadTransferConcurrency: () => 3,
    loadTransferMaxActive: () => 3,
    loadSftpCompatMode: () => false,
  });
}

describe("saveToLocal downloads declare emitChunks=false", () => {
  it("single-file start", async () => {
    const starts: Array<Record<string, unknown>> = [];
    globalWindow.dbxPlugin = {
      invoke: vi.fn(async (method: string, params?: unknown) => {
        if (method === "local/capabilities") return { canSaveLocal: true, downloadsDir: "/downloads" };
        if (method === "sftp/download/start") {
          starts.push(params as Record<string, unknown>);
          throw new Error("stop");
        }
        return {};
      }),
    };
    const download = mount();
    await download.downloadRemoteFile({ remotePath: "/tmp/a.bin", name: "a.bin", size: 3 });
    expect(starts[0]).toMatchObject({ saveToLocal: true, emitChunks: false });
  });

  it("folder start", async () => {
    const starts: Array<Record<string, unknown>> = [];
    globalWindow.dbxPlugin = {
      invoke: vi.fn(async (method: string, params?: unknown) => {
        if (method === "local/capabilities") return { canSaveLocal: true, downloadsDir: "/downloads" };
        if (method === "sftp/download/tree/start") {
          starts.push(params as Record<string, unknown>);
          throw new Error("stop");
        }
        return {};
      }),
    };
    const download = mount();
    await download.downloadEntry({ uri: "sftp:/dir", name: "dir", kind: "directory" });
    expect(starts[0]).toMatchObject({ emitChunks: false });
  });
});
