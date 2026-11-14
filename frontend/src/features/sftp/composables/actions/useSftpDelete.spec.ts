// @vitest-environment happy-dom
// useSftpDelete 行为测试：单条/批量删除成功后回调 onDeleted（实际删除路径）
// 与 onMutated（刷新树），失败时仍回调 onMutated；缺省回退 loadDirectory()。
// 批量删除自 M34 起为「一次 RPC」：sftp/deleteMany / sudo/removeMany 返回
// { deleted, failed }，前端只关闭成功路径对应的编辑器页签。
import { afterEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import type { Ref } from "vue";
import type { SftpEntryKind } from "../../lib/entries/sftpEntries";
import type { BatchProgressState } from "../../../transfers/lib/sftpBatchProgress";
import { useSftpDelete } from "./useSftpDelete";

type DeleteEntry = { uri: string; name: string; kind: SftpEntryKind };
type BatchDeleteResult = { deleted: string[]; failed: Array<{ path: string; error: string }> };

const BATCH_TIMEOUT_MS = 10 * 60 * 1000;

function mountDelete(options: {
  onDeleted?: (paths: string[]) => void;
  onMutated?: () => void | Promise<void>;
  failPath?: string;
  sudo?: boolean;
  failBatchPaths?: string[];
  rejectBatch?: boolean;
} = {}) {
  const invoke = vi.fn(async (method: string, params: Record<string, unknown> = {}) => {
    if (method === "sftp/deleteMany" || method === "sudo/removeMany") {
      if (options.rejectBatch) throw new Error("batch delete failed");
      const paths = method === "sudo/removeMany"
        ? ((params.paths as string[] | undefined) ?? [])
        : (((params.items as Array<{ path: string }> | undefined) ?? []).map((item) => item.path));
      const failedPaths = options.failBatchPaths ?? (options.failPath ? [options.failPath] : []);
      const failed = paths
        .filter((path) => failedPaths.includes(path))
        .map((path) => ({ path, error: "delete failed" }));
      const deleted = paths.filter((path) => !failedPaths.includes(path));
      return { deleted, failed } satisfies BatchDeleteResult;
    }
    if (params.path === options.failPath) throw new Error("delete failed");
    return {};
  });
  (window as unknown as { dbxPlugin: unknown }).dbxPlugin = { invoke };

  const deleteTarget = ref<DeleteEntry | undefined>(undefined);
  const selectedEntries = ref<DeleteEntry[]>([]);
  const batchDeleteOpen = ref(false);
  const deleteSubmitting = ref(false);
  const batchDeleteSubmitting = ref(false);
  const batchProgress = ref<BatchProgressState | null>(null);
  const showNotice = vi.fn();
  const showError = vi.fn();
  const loadDirectory = vi.fn(async (_path?: string) => {});
  const clearRowSelection = vi.fn();

  const deletion = useSftpDelete({
    t: (key: string, values?: Record<string, string | number>) =>
      values ? `${key}|${JSON.stringify(values)}` : key,
    showNotice,
    showError,
    session: ref({ sessionId: "s1" }),
    sudoMode: ref(options.sudo === true),
    currentPath: ref("/home"),
    deleteTarget,
    deleteSubmitting,
    batchDeleteOpen,
    batchDeleteSubmitting,
    batchProgress,
    selectedEntries: selectedEntries as Ref<DeleteEntry[]>,
    pathFromUri: (uri) => uri.replace(/^sftp:/, ""),
    joinRemote: (parent, name) => `${parent.replace(/\/+$/, "")}/${name}`,
    loadDirectory,
    clearRowSelection,
    archiveBusy: ref(false),
    onDeleted: options.onDeleted,
    onMutated: options.onMutated,
  });

  return {
    deletion,
    invoke,
    deleteTarget,
    selectedEntries,
    batchDeleteOpen,
    deleteSubmitting,
    batchDeleteSubmitting,
    batchProgress,
    showNotice,
    showError,
    loadDirectory,
    clearRowSelection,
  };
}

const file = (path: string): DeleteEntry => ({
  uri: `sftp:${path}`,
  name: path.slice(path.lastIndexOf("/") + 1),
  kind: "file",
});

const dir = (path: string): DeleteEntry => ({
  uri: `sftp:${path}`,
  name: path.slice(path.lastIndexOf("/") + 1),
  kind: "directory",
});

afterEach(() => {
  delete (window as unknown as { dbxPlugin?: unknown }).dbxPlugin;
});

describe("useSftpDelete", () => {
  it("单条删除成功：onDeleted 收到路径，onMutated 触发且不再 loadDirectory", async () => {
    const onDeleted = vi.fn();
    const onMutated = vi.fn();
    const h = mountDelete({ onDeleted, onMutated });
    h.deleteTarget.value = file("/home/a.txt");

    await h.deletion.confirmDelete();

    expect(h.invoke).toHaveBeenCalledWith("sftp/delete", {
      sessionId: "s1",
      path: "/home/a.txt",
      recursive: false,
    });
    expect(onDeleted).toHaveBeenCalledTimes(1);
    expect(onDeleted).toHaveBeenCalledWith(["/home/a.txt"]);
    expect(onMutated).toHaveBeenCalledWith(["/home/a.txt"]);
    expect(onMutated).toHaveBeenCalledTimes(1);
    expect(h.loadDirectory).not.toHaveBeenCalled();
    expect(h.deleteTarget.value).toBeUndefined();
    expect(h.showNotice).toHaveBeenCalledWith("deleted");
    expect(h.deleteSubmitting.value).toBe(false);
  });

  it("单条删除成功且无回调：回退 loadDirectory()（兼容旧调用方）", async () => {
    const h = mountDelete();
    h.deleteTarget.value = file("/home/a.txt");

    await h.deletion.confirmDelete();

    expect(h.loadDirectory).toHaveBeenCalledTimes(1);
    expect(h.showError).not.toHaveBeenCalled();
  });

  it("单条删除失败：onDeleted 收到空列表，仍回调 onMutated 并报错", async () => {
    const onDeleted = vi.fn();
    const onMutated = vi.fn();
    const h = mountDelete({ onDeleted, onMutated, failPath: "/home/bad.txt" });
    h.deleteTarget.value = file("/home/bad.txt");

    await h.deletion.confirmDelete();

    expect(h.showError).toHaveBeenCalledTimes(1);
    expect(onDeleted).toHaveBeenCalledWith([]);
    expect(onMutated).toHaveBeenCalledTimes(1);
    expect(onMutated).toHaveBeenCalledWith();
    expect(h.loadDirectory).not.toHaveBeenCalled();
    expect(h.deleteTarget.value).toBeDefined();
    expect(h.deleteSubmitting.value).toBe(false);
  });

  it("批量删除成功：只调一次 sftp/deleteMany，onDeleted 收到返回的 deleted", async () => {
    const onDeleted = vi.fn();
    const onMutated = vi.fn();
    const h = mountDelete({ onDeleted, onMutated });
    h.selectedEntries.value = [file("/home/a.txt"), dir("/home/logs")];
    h.batchDeleteOpen.value = true;

    await h.deletion.confirmBatchDelete();

    expect(h.invoke).toHaveBeenCalledTimes(1);
    expect(h.invoke).toHaveBeenCalledWith(
      "sftp/deleteMany",
      {
        sessionId: "s1",
        items: [
          { path: "/home/a.txt", recursive: false },
          { path: "/home/logs", recursive: true },
        ],
      },
      { timeoutMs: BATCH_TIMEOUT_MS },
    );
    expect(onDeleted).toHaveBeenCalledWith(["/home/a.txt", "/home/logs"]);
    expect(onMutated).toHaveBeenCalledWith(["/home/a.txt", "/home/logs"]);
    expect(onMutated).toHaveBeenCalledTimes(1);
    expect(h.loadDirectory).not.toHaveBeenCalled();
    expect(h.batchDeleteOpen.value).toBe(false);
    expect(h.clearRowSelection).toHaveBeenCalledTimes(1);
    expect(h.showNotice).toHaveBeenCalledWith("deleted");
    expect(h.showError).not.toHaveBeenCalled();
    expect(h.batchDeleteSubmitting.value).toBe(false);
    expect(h.batchProgress.value).toBeNull();
  });

  it("批量删除部分失败：报第一项错误、对话框保持打开，onDeleted 只含成功路径", async () => {
    const onDeleted = vi.fn();
    const onMutated = vi.fn();
    const h = mountDelete({ onDeleted, onMutated, failBatchPaths: ["/home/bad.txt"] });
    h.selectedEntries.value = [file("/home/ok.txt"), file("/home/bad.txt")];
    h.batchDeleteOpen.value = true;

    await h.deletion.confirmBatchDelete();

    expect(h.invoke).toHaveBeenCalledTimes(1);
    expect(h.showError).toHaveBeenCalledTimes(1);
    expect(h.showError).toHaveBeenCalledWith("/home/bad.txt: delete failed");
    expect(onDeleted).toHaveBeenCalledWith(["/home/ok.txt"]);
    expect(onMutated).toHaveBeenCalledWith(["/home/ok.txt"]);
    expect(onMutated).toHaveBeenCalledTimes(1);
    expect(h.batchDeleteOpen.value).toBe(true);
    expect(h.clearRowSelection).not.toHaveBeenCalled();
    expect(h.showNotice).not.toHaveBeenCalled();
    expect(h.batchDeleteSubmitting.value).toBe(false);
    expect(h.batchProgress.value).toBeNull();
  });

  it("sudo 模式：只调一次 sudo/removeMany，参数为 paths 数组", async () => {
    const onDeleted = vi.fn();
    const h = mountDelete({ onDeleted, sudo: true });
    h.selectedEntries.value = [file("/var/log/a.txt"), dir("/var/log/app")];
    h.batchDeleteOpen.value = true;

    await h.deletion.confirmBatchDelete();

    expect(h.invoke).toHaveBeenCalledTimes(1);
    expect(h.invoke).toHaveBeenCalledWith(
      "sudo/removeMany",
      { sessionId: "s1", paths: ["/var/log/a.txt", "/var/log/app"] },
      { timeoutMs: BATCH_TIMEOUT_MS },
    );
    expect(onDeleted).toHaveBeenCalledWith(["/var/log/a.txt", "/var/log/app"]);
    expect(h.batchDeleteOpen.value).toBe(false);
  });

  it("批量删除整批失败且无回调：回退 loadDirectory()", async () => {
    const h = mountDelete({ failBatchPaths: ["/home/bad.txt"] });
    h.selectedEntries.value = [file("/home/bad.txt")];

    await h.deletion.confirmBatchDelete();

    expect(h.showError).toHaveBeenCalledTimes(1);
    expect(h.loadDirectory).toHaveBeenCalledTimes(1);
  });

  it("批量删除 RPC 异常：无删除清单，onMutated 零参数走全量刷新", async () => {
    const onDeleted = vi.fn();
    const onMutated = vi.fn();
    const h = mountDelete({ onDeleted, onMutated, rejectBatch: true });
    h.selectedEntries.value = [file("/home/a.txt")];

    await h.deletion.confirmBatchDelete();

    expect(h.showError).toHaveBeenCalledTimes(1);
    expect(onDeleted).toHaveBeenCalledWith([]);
    expect(onMutated).toHaveBeenCalledTimes(1);
    expect(onMutated).toHaveBeenCalledWith();
    expect(h.loadDirectory).not.toHaveBeenCalled();
  });
});
