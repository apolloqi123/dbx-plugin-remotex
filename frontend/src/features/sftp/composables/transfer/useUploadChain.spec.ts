// @vitest-environment happy-dom
// issue #197 回归守卫：文件拖入终端/SFTP 面板点亮「上传」悬浮层后，未放下
// （dragleave/dragend 取消路径）必须恢复拖入前状态。旧实现用 `@dragleave.self`
// 拦截子元素间冒泡的 leave，结果经子元素把文件拖出面板/拖出窗口的那次 leave
// 被吞掉，面板永久停留在上传页。这里用模拟 DragEvent 走真实 DOM 子树验证
// onSftpDragEnter/onSftpDragLeave/onTerminalDragEnter/onTerminalDragLeave 的
// 完整生命周期，不需要真实文件。
import { ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useUploadChain } from "./useUploadChain";
import { RpcUnavailableError } from "../../../../shared/lib/rpc";

// 整文件夹覆盖确认走共享 confirmDialog：本文件用可编程替身按用例返回接受/拒绝。
const { confirmDialogMock } = vi.hoisted(() => ({
  confirmDialogMock: vi.fn(async (_message: string, _options?: { danger?: boolean }) => true),
}));
vi.mock("../../../../shared/lib/confirmDialog", () => ({ confirmDialog: confirmDialogMock }));

const globalWindow = window as unknown as { dbxPlugin?: unknown };

beforeEach(() => {
  confirmDialogMock.mockReset();
  confirmDialogMock.mockResolvedValue(true);
});

afterEach(() => {
  delete globalWindow.dbxPlugin;
});

type UploadChainOptions = Parameters<typeof useUploadChain>[0];
type UploadSourceFn = UploadChainOptions["uploadSource"];

/** 最小 DragEvent 替身：处理器只读 dataTransfer.types / relatedTarget / currentTarget。 */
function fakeDragEvent(init: { types?: string[]; relatedTarget?: EventTarget | null; currentTarget: EventTarget | null }): DragEvent {
  return {
    dataTransfer: init.types ? { types: init.types } : undefined,
    relatedTarget: init.relatedTarget ?? null,
    currentTarget: init.currentTarget,
  } as unknown as DragEvent;
}

/** 面板 > 子元素 > 孙元素 的真实 DOM 子树，模拟 SFTP/终端面板内部结构。 */
function buildPaneTree() {
  const pane = document.createElement("section");
  const child = document.createElement("div");
  const grandchild = document.createElement("span");
  child.appendChild(grandchild);
  pane.appendChild(child);
  document.body.appendChild(pane);
  return { pane, child, grandchild };
}

/** 挂载上传链，drag 高亮 ref 随返回值透出便于断言；overrides 覆盖门禁输入。 */
function mountUploadChain(overrides?: {
  sftpPaneOpen?: boolean;
  session?: { sessionId?: string };
  currentPath?: string;
  sftpNameEncodingState?: string;
  transferDuplicateState?: "rename" | "ask" | "overwrite";
  uploadSource?: UploadSourceFn;
  refreshAfterUpload?: (targetDir?: string) => Promise<void>;
  loadDirectory?: () => Promise<void>;
  showNotice?: (message: string) => void;
  sudoUpload?: () => boolean;
}) {
  const dragActive = ref(false);
  const terminalDragActive = ref(false);
  const showError = vi.fn();
  const showNotice = overrides?.showNotice ?? vi.fn();
  const uploadSource = overrides?.uploadSource ?? vi.fn<UploadSourceFn>(async () => true);
  const loadDirectory = overrides?.loadDirectory ?? vi.fn(async () => undefined);
  const chain = useUploadChain({
    t: (key) => key,
    showNotice,
    showError,
    session: ref(overrides?.session),
    connected: ref(true),
    canWrite: ref(true),
    currentPath: ref(overrides?.currentPath ?? "/root"),
    sftpHomePath: ref("/home/demo"),
    terminalCwd: ref(""),
    sftpNameEncodingState: ref(overrides?.sftpNameEncodingState ?? "auto"),
    dragActive,
    terminalDragActive,
    sftpPaneOpen: ref(overrides?.sftpPaneOpen ?? false),
    terminalTransferBusy: ref(false),
    uploadSource,
    loadTransferConcurrency: () => 3,
    loadTransferMaxActive: () => 3,
    loadTransferDuplicatePolicy: () => "rename",
    transferDuplicateState: ref(overrides?.transferDuplicateState ?? "rename"),
    transferTasks: {},
    terminal: () => undefined,
    loadDirectory,
    openTransferPanel: vi.fn(),
    joinRemote: (parent: string, name: string) => `${parent}/${name}`,
    sudoUpload: overrides?.sudoUpload ?? (() => false),
    refreshAfterUpload: overrides?.refreshAfterUpload,
  });
  return { chain, dragActive, terminalDragActive, showError, showNotice, uploadSource, loadDirectory };
}

describe("useUploadChain folder-row upload target", () => {
  it("uploads host-picked files into the target directory passed by the folder menu", async () => {
    const pick = vi.fn(async () => ({ files: [{ handleId: "h1", name: "a.txt", size: 3 }] }));
    globalWindow.dbxPlugin = {
      invoke: vi.fn(async () => ({})),
      fileTransfer: {
        pick,
        read: vi.fn(async () => ({ dataBase64: "YWJj", length: 3 })),
        cancel: vi.fn(async () => undefined),
      },
      decodeBase64: (value: string) => Uint8Array.from(atob(value), (char) => char.charCodeAt(0)),
    };

    const { chain, uploadSource } = mountUploadChain();
    await chain.chooseUpload("/etc");

    expect(pick).toHaveBeenCalledWith({ multiple: true });
    expect(uploadSource).toHaveBeenCalledTimes(1);
    // uploadSource(name, size, readChunk, resume?, targetDir?, ...)
    const calls = (uploadSource as unknown as { mock: { calls: unknown[][] } }).mock.calls;
    expect(calls[0]?.[4]).toBe("/etc");
  });
});

describe("useUploadChain drag highlight lifecycle (issue #197)", () => {
  it("SFTP pane: highlight survives child-to-child moves and resets once the drag leaves through a child without dropping", () => {
    const { chain, dragActive } = mountUploadChain();
    const { pane, child, grandchild } = buildPaneTree();

    // 拖入面板：点亮「上传」悬浮层。
    chain.onSftpDragEnter(fakeDragEvent({ types: ["Files"], currentTarget: pane }));
    expect(dragActive.value).toBe(true);

    // 在面板子树内移动（面板→子元素、子元素→孙元素）：冒泡 leave 不熄灭、不闪烁。
    chain.onSftpDragLeave(fakeDragEvent({ relatedTarget: child, currentTarget: pane }));
    expect(dragActive.value).toBe(true);
    chain.onSftpDragLeave(fakeDragEvent({ relatedTarget: grandchild, currentTarget: pane }));
    expect(dragActive.value).toBe(true);

    // 未放下拖走（经子元素离开面板）：relatedTarget 指向面板外 → 恢复。
    const outside = document.createElement("button");
    document.body.appendChild(outside);
    chain.onSftpDragLeave(fakeDragEvent({ relatedTarget: outside, currentTarget: pane }));
    expect(dragActive.value).toBe(false);
  });

  it("SFTP pane: leaving the window or cancelling the drag (null relatedTarget) also resets the highlight", () => {
    const { chain, dragActive } = mountUploadChain();
    const { pane } = buildPaneTree();

    chain.onSftpDragEnter(fakeDragEvent({ types: ["Files"], currentTarget: pane }));
    expect(dragActive.value).toBe(true);
    // 拖出浏览器窗口 / Esc 取消：最后一次 dragleave relatedTarget 为空。
    chain.onSftpDragLeave(fakeDragEvent({ currentTarget: pane }));
    expect(dragActive.value).toBe(false);
  });

  it("SFTP pane: non-file drags (text/elements) never light the highlight in the first place", () => {
    const { chain, dragActive } = mountUploadChain();
    const { pane } = buildPaneTree();

    chain.onSftpDragEnter(fakeDragEvent({ types: ["text/plain"], currentTarget: pane }));
    expect(dragActive.value).toBe(false);
  });

  it("terminal pane: highlight follows the same enter/leave lifecycle and respects the drop gate", () => {
    const { chain, terminalDragActive } = mountUploadChain();
    const { pane, child } = buildPaneTree();

    // 门禁通过（connected+canWrite+空闲+面板关闭）：拖入点亮。
    chain.onTerminalDragEnter(fakeDragEvent({ types: ["Files"], currentTarget: pane }));
    expect(terminalDragActive.value).toBe(true);
    // 子元素间路过不熄灭。
    chain.onTerminalDragLeave(fakeDragEvent({ relatedTarget: child, currentTarget: pane }));
    expect(terminalDragActive.value).toBe(true);
    // 未放下拖走：恢复。
    chain.onTerminalDragLeave(fakeDragEvent({ currentTarget: pane }));
    expect(terminalDragActive.value).toBe(false);

    // SFTP 面板打开时终端拒收（可见落点是面板）：拖入不再点亮。
    const paneOpen = mountUploadChain({ sftpPaneOpen: true });
    paneOpen.chain.onTerminalDragEnter(fakeDragEvent({ types: ["Files"], currentTarget: pane }));
    expect(paneOpen.terminalDragActive.value).toBe(false);
  });
});

describe("useUploadChain host folder drop with a partial bridge", () => {
  it("有 fileTransfer 无 invoke：句柄仍在 finally 回收，且错误统一交给 showError", async () => {
    const cancel = vi.fn(async () => undefined);
    // 半桥：fileTransfer 在位（句柄来自宿主），但 rpc 通道缺失。
    globalWindow.dbxPlugin = { fileTransfer: { cancel } };
    const { chain, dragActive, showError } = mountUploadChain({ sftpPaneOpen: true });
    dragActive.value = true;

    await chain.handleHostFileDrop([
      { handleId: "handle-1", name: "a.txt", size: 1, contentType: "text/plain", relativePath: "dir/a.txt" },
    ]);

    expect(cancel).toHaveBeenCalledWith("handle-1");
    expect(showError).toHaveBeenCalledTimes(1);
    expect(showError.mock.calls[0]![0]).toBeInstanceOf(RpcUnavailableError);
    expect(dragActive.value).toBe(false);
  });
});

/** 文件夹管线条目替身：readChunk 从内存字节切片（归档流只按需读取）。 */
function folderEntry(relativePath: string, content = "content") {
  const bytes = new TextEncoder().encode(content);
  return {
    name: relativePath.split("/").pop() as string,
    relativePath,
    size: bytes.byteLength,
    readChunk: async (offset: number, length: number) => bytes.subarray(offset, offset + length),
  };
}

describe("useUploadChain explicit drop target and refresh", () => {
  it("onDrop(event, targetDir) uploads into the tree target and refreshes that directory", async () => {
    const invoke = vi.fn(async (method: string, _params?: unknown) => {
      if (method === "sftp/exists") return { exists: false };
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const refreshAfterUpload = vi.fn(async (_targetDir?: string) => undefined);
    const loadDirectory = vi.fn(async () => undefined);
    const { chain, showNotice } = mountUploadChain({ session: { sessionId: "s1" }, uploadSource, refreshAfterUpload, loadDirectory });

    const event = { dataTransfer: { files: [new File(["hello"], "a.txt")] } } as unknown as DragEvent;
    chain.onDrop(event, "/tree/target");

    await vi.waitFor(() => expect(refreshAfterUpload).toHaveBeenCalledWith("/tree/target"));
    expect(uploadSource).toHaveBeenCalledTimes(1);
    const call = uploadSource.mock.calls[0]!;
    expect(call[0]).toBe("a.txt");
    expect(call[4]).toBe("/tree/target");
    expect(call[5]).toMatchObject({ duplicatePreCheckedAbsent: true });
    // 显式 refreshAfterUpload 生效时不再重复 loadDirectory。
    expect(loadDirectory).not.toHaveBeenCalled();
    expect(showNotice).toHaveBeenCalledWith("uploaded");
  });

  it("falls back to dataTransfer.files when webkitGetAsEntry is present but yields no roots", async () => {
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const refreshAfterUpload = vi.fn(async (_targetDir?: string) => undefined);
    const { chain } = mountUploadChain({ session: { sessionId: "s1" }, uploadSource, refreshAfterUpload });
    const event = {
      dataTransfer: {
        items: [{ kind: "file", webkitGetAsEntry: () => null }],
        files: [new File(["hello"], "fallback.txt")],
      },
    } as unknown as DragEvent;

    chain.onDrop(event, "/tree/target");

    await vi.waitFor(() => expect(uploadSource).toHaveBeenCalledTimes(1));
    expect(uploadSource.mock.calls[0]![0]).toBe("fallback.txt");
    expect(uploadSource.mock.calls[0]![4]).toBe("/tree/target");
    // 预检桥不可用：不能声称「已确认不存在」而绕过 uploadSource 的再检查。
    expect(uploadSource.mock.calls[0]![5]).toMatchObject({ duplicatePreCheckedAbsent: false });
    expect(refreshAfterUpload).toHaveBeenCalledWith("/tree/target");
  });
});

describe("useUploadChain force-ask duplicate policy", () => {
  it("forceAskDuplicates prompts even under the rename policy; overwrite keeps the name", async () => {
    const invoke = vi.fn(async (method: string, _params?: unknown) => {
      if (method === "sftp/exists") return { exists: true };
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const { chain } = mountUploadChain({ session: { sessionId: "s1" }, transferDuplicateState: "rename", uploadSource });

    const pending = chain.uploadLocalFiles([new File(["x"], "a.txt")], "/dir", { forceAskDuplicates: true });
    await vi.waitFor(() => expect(chain.uploadDuplicatePrompt.value).not.toBeNull());
    expect(chain.uploadDuplicatePrompt.value!.fileName).toBe("a.txt");
    expect(chain.uploadDuplicatePrompt.value!.path).toBe("/dir/a.txt");
    chain.resolveUploadDuplicate("overwrite");
    await pending;

    expect(invoke).toHaveBeenCalledWith("sftp/exists", { sessionId: "s1", path: "/dir/a.txt" });
    expect(uploadSource).toHaveBeenCalledTimes(1);
    expect(uploadSource.mock.calls[0]![0]).toBe("a.txt");
    expect(uploadSource.mock.calls[0]![5]).toMatchObject({ duplicatePreCheckedAbsent: true });
  });

  it("forceAskDuplicates rename resolves a unique name through sftp/rename-unique", async () => {
    const invoke = vi.fn(async (method: string, _params?: unknown) => {
      if (method === "sftp/exists") return { exists: true };
      if (method === "sftp/rename-unique") return { name: "a (1).txt" };
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const { chain } = mountUploadChain({ session: { sessionId: "s1" }, transferDuplicateState: "rename", uploadSource });

    const pending = chain.uploadLocalFiles([new File(["x"], "a.txt")], "/dir", { forceAskDuplicates: true });
    await vi.waitFor(() => expect(chain.uploadDuplicatePrompt.value).not.toBeNull());
    chain.resolveUploadDuplicate("rename");
    await pending;

    expect(invoke).toHaveBeenCalledWith("sftp/rename-unique", { sessionId: "s1", dir: "/dir", name: "a.txt", sudo: false });
    expect(uploadSource.mock.calls[0]![0]).toBe("a (1).txt");
  });

  it("keeps a per-file choice scoped when apply-to-all is unchecked", async () => {
    const invoke = vi.fn(async (method: string, params?: unknown) => {
      if (method === "sftp/exists") return { exists: true };
      if (method === "sftp/rename-unique") return { name: `${(params as { name: string }).name} (1)` };
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const { chain } = mountUploadChain({ session: { sessionId: "s1" }, transferDuplicateState: "rename", uploadSource });

    const pending = chain.uploadLocalFiles([new File(["x"], "a.txt"), new File(["y"], "b.txt")], "/dir", { forceAskDuplicates: true });
    await vi.waitFor(() => expect(chain.uploadDuplicatePrompt.value).not.toBeNull());
    const firstName = chain.uploadDuplicatePrompt.value!.fileName;
    chain.resolveUploadDuplicate("overwrite");
    // apply-all 未勾选：第二个同名文件必须再次询问，不能被首个 overwrite 决策吞掉。
    await vi.waitFor(() => expect(chain.uploadDuplicatePrompt.value).not.toBeNull());
    const secondName = chain.uploadDuplicatePrompt.value!.fileName;
    expect(secondName).not.toBe(firstName);
    chain.resolveUploadDuplicate("rename");
    await pending;

    expect(invoke).toHaveBeenCalledWith("sftp/rename-unique", { sessionId: "s1", dir: "/dir", name: secondName, sudo: false });
    const uploadedNames = uploadSource.mock.calls.map((call) => call[0]);
    expect(uploadedNames).toContain(firstName);
    expect(uploadedNames).toContain(`${secondName} (1)`);
  });

  it("without forceAskDuplicates the configured rename policy silently renames (no prompt)", async () => {
    const invoke = vi.fn(async (method: string, _params?: unknown) => {
      if (method === "sftp/exists") return { exists: true };
      if (method === "sftp/rename-unique") return { name: "a (1).txt" };
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const { chain } = mountUploadChain({ session: { sessionId: "s1" }, transferDuplicateState: "rename", uploadSource });

    await chain.uploadLocalFiles([new File(["x"], "a.txt")], "/dir");

    expect(chain.uploadDuplicatePrompt.value).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith("sftp/rename-unique", expect.anything());
    // 非 forceAsk 车道仍交给 uploadSource 内部预检（此处为桩，不真正解析）。
    expect(uploadSource).toHaveBeenCalledTimes(1);
  });
});

describe("useUploadChain folder archive and root overwrite", () => {
  it("uses the archive lane for a one-file folder, extracts remotely, and refreshes the target", async () => {
    const invoke = vi.fn(async (method: string, _params?: unknown) => {
      if (method === "sftp/exists") return { exists: false };
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const refreshAfterUpload = vi.fn(async (_targetDir?: string) => undefined);
    const { chain, showNotice } = mountUploadChain({ session: { sessionId: "s1" }, uploadSource, refreshAfterUpload });

    await chain.uploadFolderFiles([folderEntry("photos/a.jpg")], "/target");

    expect(confirmDialogMock).not.toHaveBeenCalled();
    expect(uploadSource).toHaveBeenCalledTimes(1);
    const call = uploadSource.mock.calls[0]!;
    expect(call[0]).toMatch(/^\.sshx-upload-[a-z0-9-]+\.tar$/);
    expect(call[4]).toBe("/target");
    expect(invoke).toHaveBeenCalledWith(
      "sftp/extract",
      { sessionId: "s1", archivePath: `/target/${call[0]}`, destinationPath: "/target", overwrite: true, sudo: false },
      { timeoutMs: 600_000 },
    );
    expect(refreshAfterUpload).toHaveBeenCalledWith("/target");
    expect(showNotice).toHaveBeenCalledWith("folderArchive.completed");
  });

  it("confirms an existing root once, deletes it recursively, then uploads the archive", async () => {
    confirmDialogMock.mockResolvedValue(true);
    const invoke = vi.fn(async (method: string, params?: unknown) => {
      if (method === "sftp/exists") return { exists: (params as { path?: string }).path === "/target/photos" };
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const { chain } = mountUploadChain({ session: { sessionId: "s1" }, uploadSource });

    await chain.uploadFolderFiles([folderEntry("photos/a.jpg"), folderEntry("photos/sub/b.jpg")], "/target");

    expect(confirmDialogMock).toHaveBeenCalledTimes(1);
    expect(confirmDialogMock).toHaveBeenCalledWith("folderUpload.overwriteFolderConfirm", { danger: true });
    expect(invoke).toHaveBeenCalledWith("sftp/delete", { sessionId: "s1", path: "/target/photos", recursive: true });
    const methods = invoke.mock.calls.map((entry) => entry[0]);
    expect(methods.indexOf("sftp/delete")).toBeLessThan(methods.indexOf("sftp/extract"));
    expect(uploadSource).toHaveBeenCalledTimes(1);
    expect(uploadSource.mock.calls[0]![0]).toMatch(/^\.sshx-upload-[a-z0-9-]+\.tar$/);
  });

  it("declining the root confirmation skips every entry under it and uploads nothing", async () => {
    confirmDialogMock.mockResolvedValue(false);
    const invoke = vi.fn(async (method: string, params?: unknown) => {
      if (method === "sftp/exists") return { exists: (params as { path?: string }).path === "/target/photos" };
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const refreshAfterUpload = vi.fn(async (_targetDir?: string) => undefined);
    const { chain } = mountUploadChain({ session: { sessionId: "s1" }, uploadSource, refreshAfterUpload });

    await chain.uploadFolderFiles([folderEntry("photos/a.jpg")], "/target");

    expect(confirmDialogMock).toHaveBeenCalledTimes(1);
    expect(invoke.mock.calls.some((entry) => entry[0] === "sftp/delete")).toBe(false);
    expect(uploadSource).not.toHaveBeenCalled();
    expect(refreshAfterUpload).not.toHaveBeenCalled();
    expect(chain.folderUploadProgress.value).toBeUndefined();
  });

  it("fails closed when the root existence probe errors", async () => {
    const invoke = vi.fn(async (method: string) => {
      if (method === "sftp/exists") throw new Error("probe failed");
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const { chain, showError } = mountUploadChain({ session: { sessionId: "s1" }, uploadSource });

    await chain.uploadFolderFiles([folderEntry("photos/a.jpg")], "/target");

    expect(showError).toHaveBeenCalled();
    expect(invoke.mock.calls.some((entry) => entry[0] === "sftp/delete")).toBe(false);
    expect(uploadSource).not.toHaveBeenCalled();
  });

  it("applies the root overwrite flow to the per-file fallback (latin-1) as well", async () => {
    confirmDialogMock.mockResolvedValue(true);
    const invoke = vi.fn(async (method: string, params?: unknown) => {
      if (method === "sftp/exists") return { exists: (params as { path?: string }).path === "/target/photos" };
      return {};
    });
    globalWindow.dbxPlugin = { invoke };
    const uploadSource = vi.fn<UploadSourceFn>(async () => true);
    const { chain } = mountUploadChain({ session: { sessionId: "s1" }, sftpNameEncodingState: "latin-1", uploadSource });

    await chain.uploadFolderFiles([folderEntry("photos/a.jpg")], "/target");

    expect(confirmDialogMock).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("sftp/delete", { sessionId: "s1", path: "/target/photos", recursive: true });
    expect(invoke).not.toHaveBeenCalledWith("sftp/extract", expect.anything(), expect.anything());
    expect(uploadSource).toHaveBeenCalledTimes(1);
    expect(uploadSource.mock.calls[0]![0]).toBe("a.jpg");
    expect(uploadSource.mock.calls[0]![4]).toBe("/target/photos");
    expect(uploadSource.mock.calls[0]![5]).toMatchObject({ duplicatePreCheckedAbsent: true });
  });
});
