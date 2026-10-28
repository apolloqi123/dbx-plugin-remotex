// @vitest-environment happy-dom
// 宿主桥缺失（非插件 webview / 旧宿主 / 测试未注入）回归：下载入口必须把
// RpcUnavailableError 交给 showError，而不是在 `window.dbxPlugin.invoke`
// 上抛 "Cannot read properties of undefined" 的裸 TypeError。
import { afterEach, describe, expect, it, vi } from "vitest";
import { ref, type Ref } from "vue";
import { useSftpDownload } from "./useSftpDownload";
import { RpcUnavailableError } from "../../../../shared/lib/rpc";

const globalWindow = window as unknown as { dbxPlugin?: unknown };

afterEach(() => {
  delete globalWindow.dbxPlugin;
});

type DownloadEntry = { uri: string; name: string; kind: "file" | "directory"; size?: number };
type DownloadMenu = { entry: DownloadEntry; selection: string[] };

function mountDownload(options: {
  entries?: Ref<DownloadEntry[]>;
  selectedEntries?: Ref<DownloadEntry[]>;
  fileMenu?: Ref<DownloadMenu | undefined>;
  resolveConflict?: () => Promise<"rename" | "overwrite" | undefined>;
} = {}) {
  const session = ref<{ sessionId?: string } | undefined>({ sessionId: "s1" });
  const entries = options.entries ?? ref<DownloadEntry[]>([
    { uri: "root/a.txt", name: "a.txt", kind: "file" as const, size: 10 },
  ]);
  const showError = vi.fn();
  const localCanSave = ref(false);
  const download = useSftpDownload({
    t: (key) => key,
    showNotice: vi.fn(),
    showError,
    session,
    entries,
    selectedEntries: options.selectedEntries,
    fileMenu: options.fileMenu ?? ref<DownloadMenu | undefined>(undefined),
    localDownloadDir: ref(""),
    localCanSave,
    transferTasks: {},
    cancelledTransferTasks: new Set<string>(),
    downloadChunkWaiters: new Map(),
    waitWhilePaused: () => undefined,
    loadDownloadDir: () => "/downloads",
    loadDownloadUseDefaultDir: () => true,
    askDownloadTarget: vi.fn(async () => undefined),
    applyChosenDirAsDefault: vi.fn(),
    resolveDownloadConflictFor: vi.fn(options.resolveConflict ?? (async () => undefined)),
    openTransferPanel: vi.fn(),
    pathFromUri: (uri) => uri,
    sudoDownload: () => false,
    loadTransferConcurrency: () => 1,
    loadTransferMaxActive: () => 1,
    loadSftpCompatMode: () => false,
  });
  return { download, entries, showError, localCanSave };
}

describe("useSftpDownload with the host bridge absent", () => {
  it("downloadEntry 以 RpcUnavailableError 收口（而非裸 TypeError）", async () => {
    delete globalWindow.dbxPlugin;
    const { download, entries, showError } = mountDownload();

    await download.downloadEntry(entries.value[0]!);

    expect(showError).toHaveBeenCalledTimes(1);
    const [cause, target] = showError.mock.calls[0]!;
    expect(cause).toBeInstanceOf(RpcUnavailableError);
    expect(cause).toMatchObject({ method: "sftp/download/start" });
    expect(target).toBe("sftp");
  });

  it("桥缺失的探测不污染缓存：迟到注入宿主桥后下一次探测可重试", async () => {
    delete globalWindow.dbxPlugin;
    const { download, localCanSave } = mountDownload();

    await expect(download.probeLocalCapabilities()).resolves.toBeUndefined();

    const invoke = vi.fn(async () => ({ canSaveLocal: true, downloadsDir: "/downloads" }));
    globalWindow.dbxPlugin = { invoke };
    await expect(download.probeLocalCapabilities()).resolves.toEqual({ canSaveLocal: true, downloadsDir: "/downloads" });
    expect(invoke).toHaveBeenCalledWith("local/capabilities", undefined);
    expect(localCanSave.value).toBe(true);
  });
});

describe("useSftpDownload downloadRemoteFile", () => {
  it("直接以 remotePath 发起下载，不依赖树条目", async () => {
    const starts: Array<Record<string, unknown>> = [];
    globalWindow.dbxPlugin = {
      invoke: vi.fn(async (method: string, params?: unknown) => {
        if (method === "local/capabilities") return { canSaveLocal: false, downloadsDir: "" };
        if (method === "sftp/download/start") {
          starts.push(params as Record<string, unknown>);
          throw new Error("stop");
        }
        return {};
      }),
    };
    // 同文件前面用例可能已把本机落盘能力缓存为 true：冲突策略返回 rename 保证走到 start。
    const { download, showError } = mountDownload({ entries: ref([]), resolveConflict: async () => "rename" });

    await download.downloadRemoteFile({ remotePath: "/tmp/dbx-sshx.1/report.log", name: "report.log", size: 3 });

    expect(starts).toHaveLength(1);
    expect(starts[0]).toMatchObject({ remotePath: "/tmp/dbx-sshx.1/report.log" });
    expect(showError).toHaveBeenCalledWith(expect.any(Error), "sftp", undefined);
  });
});

describe("useSftpDownload batchDownload", () => {
  const a: DownloadEntry = { uri: "sftp:/a.txt", name: "a.txt", kind: "file", size: 10 };
  const b: DownloadEntry = { uri: "sftp:/sub/b.txt", name: "b.txt", kind: "file", size: 10 };
  const c: DownloadEntry = { uri: "sftp:/c.txt", name: "c.txt", kind: "file", size: 10 };

  /** 记录每个真正发起的下载（start 立即失败中止，避免跑完整分块管线）。 */
  function mountBatch(options: {
    entries: Ref<DownloadEntry[]>;
    selectedEntries?: Ref<DownloadEntry[]>;
    selection: string[];
  }) {
    const started: string[] = [];
    const invoke = vi.fn(async (method: string, params?: unknown) => {
      if (method === "local/capabilities") return { canSaveLocal: false, downloadsDir: "" };
      if (method === "sftp/download/start") {
        started.push((params as { remotePath: string }).remotePath);
        throw new Error("stop");
      }
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    // 本地落盘能力可能已被同文件前面的用例缓存为 true：冲突策略返回 rename，
    // 保证两条缓存路径都会真正发起 download/start。
    const { download, showError } = mountDownload({
      entries: options.entries,
      selectedEntries: options.selectedEntries,
      fileMenu: ref<DownloadMenu | undefined>({ entry: options.entries.value[0]!, selection: options.selection }),
      resolveConflict: async () => "rename",
    });
    return { download, started, showError };
  }

  it("使用跨目录 selectedEntries 命中批量下载目标", async () => {
    // 当前目录 entries 只有 a；选中项来自树里另一个已展开目录的 b。
    const { download, started } = mountBatch({
      entries: ref([a]),
      selectedEntries: ref([a, b]),
      selection: [a.uri, b.uri],
    });

    await download.batchDownload();

    expect(started).toEqual([a.uri, b.uri]);
  });

  it("未提供 selectedEntries 时回退 entries（兼容旧调用方）", async () => {
    const { download, started } = mountBatch({
      entries: ref([a, c]),
      selection: [a.uri, c.uri],
    });

    await download.batchDownload();

    expect(started).toEqual([a.uri, c.uri]);
  });
});
