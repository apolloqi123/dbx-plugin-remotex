// @vitest-environment happy-dom
// renameTargetPath 单测：树可同时展开多个目录，重命名目标必须取条目自身
// 绝对路径的父目录，而不是当前浏览目录（currentPath）。
// useSftpRename 行为测试：成功/失败都触发 onMutated（或回退 loadDirectory），
// 成功时 onRenamed 收到实际 source/target 绝对路径。
import { afterEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { renameTargetPath, useSftpRename } from "./useSftpRename";

const confirmMock = vi.hoisted(() =>
  vi.fn<(message: string, options?: { danger?: boolean }) => Promise<boolean>>(),
);
vi.mock("../../../shared/lib/confirmDialog", () => ({ confirmDialog: confirmMock }));

const joinRemote = (parent: string, name: string) =>
  `${parent === "/" ? "" : parent.replace(/\/+$/, "")}/${name.replace(/^\/+/, "")}`;

type RenameEntry = { uri: string; name: string };

function mountRename(options: {
  onRenamed?: (oldPath: string, newPath: string) => void;
  onMutated?: () => void | Promise<void>;
  sudoMode?: boolean;
  renameFails?: boolean;
} = {}) {
  const invoke = vi.fn(async (method: string, params: Record<string, unknown> = {}) => {
    if (method === "sftp/exists") return { exists: false };
    if (method === "sftp/rename" || method === "sudo/rename") {
      if (options.renameFails) throw new Error("rename failed");
      return {};
    }
    throw new Error(`unexpected method ${method} (${JSON.stringify(params)})`);
  });
  (window as unknown as { dbxPlugin: unknown }).dbxPlugin = { invoke };
  const loadDirectory = vi.fn(async (_path?: string) => {});
  const showError = vi.fn();
  const rename = useSftpRename({
    t: (key: string, values?: Record<string, string | number>) =>
      values ? `${key}|${JSON.stringify(values)}` : key,
    showError,
    session: ref({ sessionId: "s1" }),
    canWrite: ref(true),
    sudoMode: ref(options.sudoMode ?? false),
    selectedPath: ref(""),
    pathFromUri: (uri) => uri.replace(/^sftp:/, ""),
    joinRemote,
    loadDirectory,
    onRenamed: options.onRenamed,
    onMutated: options.onMutated,
  });
  return { rename, invoke, loadDirectory, showError };
}

async function commit(
  rename: ReturnType<typeof mountRename>["rename"],
  entry: RenameEntry,
  name: string,
) {
  rename.renamingPath.value = entry.uri;
  rename.renameDraft.value = name;
  await rename.commitRename(entry);
}

afterEach(() => {
  delete (window as unknown as { dbxPlugin?: unknown }).dbxPlugin;
  confirmMock.mockReset();
  confirmMock.mockResolvedValue(true);
});

describe("renameTargetPath", () => {
  it("keeps the target inside the entry's own directory", () => {
    expect(renameTargetPath("/home/demo/server.log", "server.log.bak", joinRemote)).toBe(
      "/home/demo/server.log.bak",
    );
  });

  it("uses the expanded child directory, not the current listing path", () => {
    // 当前目录是 /home/demo，但被重命名条目位于展开的 /home/demo/sub。
    expect(renameTargetPath("/home/demo/sub/notes.txt", "notes.md", joinRemote)).toBe(
      "/home/demo/sub/notes.md",
    );
  });

  it("handles files directly under the remote root", () => {
    expect(renameTargetPath("/readme.txt", "readme.md", joinRemote)).toBe("/readme.md");
  });

  it("tolerates a trailing slash on the source path", () => {
    expect(renameTargetPath("/home/demo/sub/", "renamed", joinRemote)).toBe("/home/demo/renamed");
  });
});

describe("useSftpRename commitRename", () => {
  const entry: RenameEntry = { uri: "sftp:/home/demo/sub/notes.txt", name: "notes.txt" };

  it("成功：onRenamed 先收到源/目标绝对路径，onMutated 随后触发且不再 loadDirectory", async () => {
    const order: string[] = [];
    const onRenamed = vi.fn((_oldPath: string, _newPath: string) => {
      order.push("renamed");
    });
    const onMutated = vi.fn(async () => {
      order.push("mutated");
    });
    const h = mountRename({ onRenamed, onMutated });

    await commit(h.rename, entry, "notes.md");

    expect(h.invoke).toHaveBeenCalledWith("sftp/rename", {
      sessionId: "s1",
      sourcePath: "/home/demo/sub/notes.txt",
      targetPath: "/home/demo/sub/notes.md",
    });
    expect(onRenamed).toHaveBeenCalledTimes(1);
    expect(onRenamed).toHaveBeenCalledWith("/home/demo/sub/notes.txt", "/home/demo/sub/notes.md");
    expect(onMutated).toHaveBeenCalledTimes(1);
    expect(order).toEqual(["renamed", "mutated"]);
    expect(h.loadDirectory).not.toHaveBeenCalled();
    expect(h.rename.renamingPath.value).toBe("");
    expect(h.rename.renameSubmitting.value).toBe(false);
  });

  it("成功且未提供回调：回退 loadDirectory()（兼容旧调用方）", async () => {
    const h = mountRename();

    await commit(h.rename, entry, "notes.md");

    expect(h.loadDirectory).toHaveBeenCalledTimes(1);
    expect(h.showError).not.toHaveBeenCalled();
  });

  it("失败：仍回调 onMutated（不调 onRenamed）并报错", async () => {
    const onRenamed = vi.fn();
    const onMutated = vi.fn();
    const h = mountRename({ onRenamed, onMutated, renameFails: true });

    await commit(h.rename, entry, "notes.md");

    expect(h.showError).toHaveBeenCalledTimes(1);
    expect(onRenamed).not.toHaveBeenCalled();
    expect(onMutated).toHaveBeenCalledTimes(1);
    expect(h.loadDirectory).not.toHaveBeenCalled();
    expect(h.rename.renamingPath.value).toBe("");
    expect(h.rename.renameSubmitting.value).toBe(false);
  });

  it("失败且未提供回调：回退 loadDirectory()", async () => {
    const h = mountRename({ renameFails: true });

    await commit(h.rename, entry, "notes.md");

    expect(h.showError).toHaveBeenCalledTimes(1);
    expect(h.loadDirectory).toHaveBeenCalledTimes(1);
  });
});
