// @vitest-environment happy-dom
// useSftpDelete 行为测试：单条/批量删除成功后回调 onDeleted（实际删除路径）
// 与 onMutated（刷新树），失败时仍回调 onMutated；缺省回退 loadDirectory()。
import { afterEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import type { Ref } from "vue";
import type { SftpEntryKind } from "../lib/sftpEntries";
import type { BatchProgressState } from "../../transfers/lib/sftpBatchProgress";
import { useSftpDelete } from "./useSftpDelete";

type DeleteEntry = { uri: string; name: string; kind: SftpEntryKind };

function mountDelete(options: {
  onDeleted?: (paths: string[]) => void;
  onMutated?: () => void | Promise<void>;
  failPath?: string;
} = {}) {
  const invoke = vi.fn(async (_method: string, params: Record<string, unknown> = {}) => {
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
    sudoMode: ref(false),
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
    expect(h.loadDirectory).not.toHaveBeenCalled();
    expect(h.deleteTarget.value).toBeDefined();
    expect(h.deleteSubmitting.value).toBe(false);
  });

  it("批量删除成功：onDeleted 收到全部路径，onMutated 触发且不再 loadDirectory", async () => {
    const onDeleted = vi.fn();
    const onMutated = vi.fn();
    const h = mountDelete({ onDeleted, onMutated });
    h.selectedEntries.value = [file("/home/a.txt"), file("/home/dir/b.txt")];
    h.batchDeleteOpen.value = true;

    await h.deletion.confirmBatchDelete();

    expect(onDeleted).toHaveBeenCalledWith(["/home/a.txt", "/home/dir/b.txt"]);
    expect(onMutated).toHaveBeenCalledTimes(1);
    expect(h.loadDirectory).not.toHaveBeenCalled();
    expect(h.batchDeleteOpen.value).toBe(false);
    expect(h.clearRowSelection).toHaveBeenCalledTimes(1);
    expect(h.showNotice).toHaveBeenCalledWith("deleted");
    expect(h.batchDeleteSubmitting.value).toBe(false);
    expect(h.batchProgress.value).toBeNull();
  });

  it("批量删除中途失败：onDeleted 只含已成功路径，仍触发 onMutated", async () => {
    const onDeleted = vi.fn();
    const onMutated = vi.fn();
    const h = mountDelete({ onDeleted, onMutated, failPath: "/home/bad.txt" });
    h.selectedEntries.value = [file("/home/ok.txt"), file("/home/bad.txt"), file("/home/later.txt")];

    await h.deletion.confirmBatchDelete();

    expect(h.showError).toHaveBeenCalledTimes(1);
    expect(onDeleted).toHaveBeenCalledWith(["/home/ok.txt"]);
    expect(onMutated).toHaveBeenCalledTimes(1);
    expect(h.loadDirectory).not.toHaveBeenCalled();
    expect(h.batchDeleteSubmitting.value).toBe(false);
    expect(h.batchProgress.value).toBeNull();
  });

  it("批量删除失败且无回调：回退 loadDirectory()", async () => {
    const h = mountDelete({ failPath: "/home/bad.txt" });
    h.selectedEntries.value = [file("/home/bad.txt")];

    await h.deletion.confirmBatchDelete();

    expect(h.showError).toHaveBeenCalledTimes(1);
    expect(h.loadDirectory).toHaveBeenCalledTimes(1);
  });
});
