// @vitest-environment happy-dom
// useEditorTabs 行为测试：打开前检查（文本/图片/已知二进制/大文件确认/嗅探）、
// 页签状态隔离、保存分流（sftp/write 与 sudo/writeFile）、关闭脏确认、
// 断连清空页签，以及读取失败落页签错误态的既定选择。
import { nextTick, ref, watch } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SftpEntryKind } from "../../lib/entries/sftpEntries";
import {
  MAX_DIRECT_WRITE_BYTES,
  MAX_IMAGE_PREVIEW_BYTES,
  MAX_INLINE_PREVIEW_BYTES,
  SNIFF_CHUNK_BYTES,
  useEditorTabs,
  type EditorEntry,
} from "./useEditorTabs";

const confirmMock = vi.hoisted(() =>
  vi.fn<(message: string, options?: { danger?: boolean }) => Promise<boolean>>(),
);
vi.mock("../../../../shared/lib/confirmDialog", () => ({ confirmDialog: confirmMock }));

function encodeBase64(bytes: Uint8Array) {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function encodeText(text: string) {
  return encodeBase64(new TextEncoder().encode(text));
}

function decodeText(value: string) {
  const binary = atob(value);
  return new TextDecoder().decode(Uint8Array.from(binary, (char) => char.charCodeAt(0)));
}

/** 组装注入依赖：invoke 按 RPC 方法路由到内存文件表，sink 全量暴露。 */
function setupHarness(overrides: { sudoMode?: boolean; canWrite?: boolean } = {}) {
  const files = new Map<string, string>();
  const session = ref<{ sessionId?: string } | undefined>({ sessionId: "s1" });
  const sudoMode = ref(overrides.sudoMode ?? false);
  const canWrite = ref(overrides.canWrite ?? true);
  const notices: string[] = [];
  const errors: unknown[] = [];
  const onSaved = vi.fn();
  const t = (key: string, values?: Record<string, string | number>) =>
    values ? `${key}|${JSON.stringify(values)}` : key;

  const invoke = vi.fn(async (method: string, params: Record<string, unknown> = {}) => {
    if (method === "sftp/read" || method === "sudo/readFile") {
      const path = String(params.path ?? "");
      const bytes = new TextEncoder().encode(files.get(path) ?? "");
      const offset = typeof params.offset === "number" ? params.offset : 0;
      const limit = typeof params.maxBytes === "number"
        ? params.maxBytes
        : typeof params.length === "number"
          ? params.length
          : MAX_INLINE_PREVIEW_BYTES;
      return {
        dataBase64: encodeBase64(bytes.slice(offset, offset + limit)),
        truncated: offset + limit < bytes.length,
      };
    }
    if (method === "sftp/write") {
      files.set(String(params.remotePath), decodeText(String(params.dataBase64)));
      return { written: decodeText(String(params.dataBase64)).length };
    }
    if (method === "sudo/writeFile") {
      files.set(String(params.path), decodeText(String(params.dataBase64)));
      return { written: decodeText(String(params.dataBase64)).length };
    }
    throw new Error(`unexpected method ${method}`);
  });

  (window as unknown as { dbxPlugin: unknown }).dbxPlugin = {
    invoke,
    encodeBase64,
    decodeBase64: (value: string) => {
      const binary = atob(value);
      return Uint8Array.from(binary, (char) => char.charCodeAt(0));
    },
  };

  const editor = useEditorTabs({
    t,
    showNotice: (message) => notices.push(message),
    showError: (cause) => errors.push(cause),
    session,
    sudoMode,
    canWrite,
    pathFromUri: (uri) => uri.replace(/^sftp:/, ""),
    onSaved,
  });

  return { editor, files, session, sudoMode, canWrite, notices, errors, invoke, onSaved };
}

function entry(name: string, size?: number, kind: SftpEntryKind = "file"): EditorEntry {
  return { uri: `sftp:/home/${name}`, name, kind, size };
}

function entryAt(path: string, size = 0, kind: SftpEntryKind = "file"): EditorEntry {
  return { uri: `sftp:${path}`, name: path.slice(path.lastIndexOf("/") + 1), kind, size };
}

/** 拦住指定 maxBytes 的读取 RPC，返回 release：用于断言 loading 的中间态与
 *  watch/effect 是否真的被触发（旧实现改裸对象，watch 永远收不到后续更新）。 */
function gateRead(
  h: ReturnType<typeof setupHarness>,
  maxBytes: number = MAX_INLINE_PREVIEW_BYTES,
  failWith?: Error,
) {
  const base = h.invoke.getMockImplementation();
  if (!base) throw new Error("invoke has no implementation");
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  h.invoke.mockImplementation(async (method: string, params: Record<string, unknown> = {}) => {
    if (method === "sftp/read" && params.maxBytes === maxBytes) {
      await gate;
      if (failWith) throw failWith;
    }
    return base(method, params);
  });
  return release;
}

beforeEach(() => {
  confirmMock.mockReset();
  confirmMock.mockResolvedValue(true);
});

describe("useEditorTabs", () => {
  it("打开文本：嗅探后创建可编辑页签并激活", async () => {
    const h = setupHarness();
    h.files.set("/home/notes.txt", "hello");

    await h.editor.openEntry(entry("notes.txt", 5));

    expect(h.editor.tabs.value).toHaveLength(1);
    const tab = h.editor.tabs.value[0];
    expect(tab).toMatchObject({
      path: "/home/notes.txt",
      name: "notes.txt",
      kind: "text",
      text: "hello",
      baseline: "hello",
      dirty: false,
      editable: true,
      truncated: false,
      loading: false,
      saving: false,
      size: 5,
    });
    expect(tab.error).toBeUndefined();
    expect(h.editor.activeId.value).toBe(tab.id);
    expect(h.editor.activeTab.value).toBe(tab);
    expect(h.editor.hasTabs.value).toBe(true);
    expect(h.invoke).toHaveBeenNthCalledWith(1, "sftp/read", {
      sessionId: "s1",
      path: "/home/notes.txt",
      maxBytes: SNIFF_CHUNK_BYTES,
    });
    expect(h.invoke).toHaveBeenNthCalledWith(2, "sftp/read", {
      sessionId: "s1",
      path: "/home/notes.txt",
      maxBytes: MAX_INLINE_PREVIEW_BYTES,
    });
  });

  it("打开图片：创建只读图片页签并内联 data URL", async () => {
    const h = setupHarness();
    h.files.set("/home/pic.png", "hi");

    await h.editor.openEntry(entry("pic.png", 2));

    expect(h.editor.tabs.value).toHaveLength(1);
    const tab = h.editor.tabs.value[0];
    expect(tab.kind).toBe("image");
    expect(tab.imageUrl).toBe(`data:image/png;base64,${encodeText("hi")}`);
    expect(tab.editable).toBe(false);
    expect(tab.loading).toBe(false);
    expect(tab.text).toBe("");
    expect(tab.baseline).toBe("");
    expect(tab.dirty).toBe(false);
    expect(h.invoke).toHaveBeenCalledWith("sftp/read", {
      sessionId: "s1",
      path: "/home/pic.png",
      maxBytes: MAX_IMAGE_PREVIEW_BYTES,
    });
  });

  it("已知二进制扩展名：只提示，不建页签也不读取", async () => {
    const h = setupHarness();

    await h.editor.openEntry(entry("archive.zip", 1024));

    expect(h.editor.tabs.value).toHaveLength(0);
    expect(h.notices[0]).toContain("binaryFile.notOpen");
    expect(h.notices[0]).toContain("archive.zip");
    expect(h.invoke).not.toHaveBeenCalled();
  });

  it("无后缀二进制内容：嗅探命中后提示，不建页签", async () => {
    const h = setupHarness();
    h.invoke.mockResolvedValueOnce({
      dataBase64: encodeBase64(new Uint8Array([0, 1, 2, 3])),
      truncated: false,
    });

    // size 未知（缺省）同样嗅探，避免未知大小的改名二进制灌入编辑器。
    await h.editor.openEntry({ uri: "sftp:/home/unknown", name: "unknown", kind: "file" });

    expect(h.editor.tabs.value).toHaveLength(0);
    expect(h.notices[0]).toContain("binaryFile.notOpen");
    expect(h.invoke).toHaveBeenCalledTimes(1);
    expect(h.invoke).toHaveBeenCalledWith("sftp/read", {
      sessionId: "s1",
      path: "/home/unknown",
      maxBytes: SNIFF_CHUNK_BYTES,
    });
  });

  it("图片读取截断：移除占位页签并提示，不留下半张图", async () => {
    const h = setupHarness();
    h.invoke.mockResolvedValueOnce({ dataBase64: encodeText("partial"), truncated: true });

    await h.editor.openEntry(entry("photo.png", 10));

    expect(h.editor.tabs.value).toHaveLength(0);
    expect(h.editor.activeId.value).toBe("");
    expect(h.notices[0]).toContain("previewDialog.truncated");
  });

  it("大文件：确认后只加载头部，页签截断且只读；拒绝则不建页签", async () => {
    const h = setupHarness();
    const size = MAX_INLINE_PREVIEW_BYTES + 1;
    h.files.set("/home/big.log", "x".repeat(size));

    await h.editor.openEntry(entry("big.log", size));

    expect(confirmMock).toHaveBeenCalledWith(
      expect.stringContaining("previewDialog.tooLargeConfirm"),
      { danger: false },
    );
    const tab = h.editor.tabs.value[0];
    expect(tab.truncated).toBe(true);
    expect(tab.editable).toBe(false);
    expect(tab.text).toHaveLength(MAX_INLINE_PREVIEW_BYTES);
    expect(tab.loading).toBe(false);

    const denied = setupHarness();
    confirmMock.mockResolvedValueOnce(false);
    await denied.editor.openEntry(entry("big.log", size));
    expect(denied.editor.tabs.value).toHaveLength(0);
    expect(denied.invoke).not.toHaveBeenCalled();
  });

  it("读取失败：保留页签并置错误态，只读", async () => {
    const h = setupHarness();
    h.invoke.mockImplementation(async (_method: string, params: Record<string, unknown> = {}) => {
      if (params.maxBytes === SNIFF_CHUNK_BYTES) {
        return { dataBase64: encodeText("hello"), truncated: false };
      }
      throw new Error("read failed");
    });

    await h.editor.openEntry(entry("notes.txt", 5));

    expect(h.editor.tabs.value).toHaveLength(1);
    const tab = h.editor.tabs.value[0];
    expect(tab.error).toBe("read failed");
    expect(tab.editable).toBe(false);
    expect(tab.loading).toBe(false);
    expect(tab.text).toBe("");
    expect(tab.dirty).toBe(false);
  });

  it("保存：走 sftp/write 正确参数，清脏并回调 onSaved", async () => {
    const h = setupHarness();
    h.files.set("/home/notes.txt", "hello");
    await h.editor.openEntry(entry("notes.txt", 5));
    const tab = h.editor.tabs.value[0];

    h.editor.updateText(tab.id, "hello world");
    expect(tab.dirty).toBe(true);

    h.invoke.mockClear();
    await h.editor.saveTab(tab.id);

    expect(h.invoke).toHaveBeenCalledWith("sftp/write", {
      sessionId: "s1",
      remotePath: "/home/notes.txt",
      dataBase64: encodeText("hello world"),
    });
    expect(h.files.get("/home/notes.txt")).toBe("hello world");
    expect(tab.baseline).toBe("hello world");
    expect(tab.dirty).toBe(false);
    expect(tab.size).toBe(11);
    expect(tab.saving).toBe(false);
    expect(h.notices.some((notice) => notice.startsWith("editSave.saved"))).toBe(true);
    expect(h.onSaved).toHaveBeenCalledWith(tab);
  });

  it("sudo 保存：走 sudo/writeFile，读取也走 sudo/readFile", async () => {
    const h = setupHarness({ sudoMode: true });
    h.files.set("/root/notes.txt", "hello");
    await h.editor.openEntry({ uri: "sftp:/root/notes.txt", name: "notes.txt", kind: "file", size: 5 });
    const tab = h.editor.tabs.value[0];

    expect(h.invoke).toHaveBeenCalledWith("sudo/readFile", {
      sessionId: "s1",
      path: "/root/notes.txt",
      offset: 0,
      length: MAX_INLINE_PREVIEW_BYTES,
    });

    h.editor.updateText(tab.id, "updated");
    h.invoke.mockClear();
    await h.editor.saveTab(tab.id);

    expect(h.invoke).toHaveBeenCalledWith("sudo/writeFile", {
      sessionId: "s1",
      path: "/root/notes.txt",
      dataBase64: encodeText("updated"),
    });
    expect(tab.dirty).toBe(false);
  });

  it("保存失败：报错并保留脏标记", async () => {
    const h = setupHarness();
    h.files.set("/home/notes.txt", "hello");
    await h.editor.openEntry(entry("notes.txt", 5));
    const tab = h.editor.tabs.value[0];
    h.editor.updateText(tab.id, "changed");

    h.invoke.mockRejectedValueOnce(new Error("permission denied"));
    await h.editor.saveTab(tab.id);

    expect(h.errors).toHaveLength(1);
    expect(tab.dirty).toBe(true);
    expect(tab.saving).toBe(false);
    expect(tab.baseline).toBe("hello");
  });

  it("保存超过直写上限：报 sizeLimit，不调用远端且保留脏标记", async () => {
    const h = setupHarness();
    h.files.set("/home/notes.txt", "hello");
    await h.editor.openEntry(entry("notes.txt", 5));
    const tab = h.editor.tabs.value[0];
    h.editor.updateText(tab.id, "x".repeat(MAX_DIRECT_WRITE_BYTES + 1));

    h.invoke.mockClear();
    await h.editor.saveTab(tab.id);

    expect(h.invoke).not.toHaveBeenCalled();
    expect(h.errors).toHaveLength(1);
    expect((h.errors[0] as Error).message).toBe("sftpAttrs.sizeLimit");
    expect(tab.dirty).toBe(true);
    expect(tab.saving).toBe(false);
  });

  it("关闭脏页签：确认取消则保留，确认后移除并落到相邻页签", async () => {
    const h = setupHarness();
    h.files.set("/home/a.txt", "a");
    h.files.set("/home/b.txt", "b");
    await h.editor.openEntry(entry("a.txt", 1));
    const first = h.editor.tabs.value[0];
    await h.editor.openEntry(entry("b.txt", 1));
    const second = h.editor.tabs.value[1];
    h.editor.updateText(second.id, "b changed");

    confirmMock.mockResolvedValueOnce(false);
    await h.editor.closeTab(second.id);
    expect(confirmMock).toHaveBeenCalledWith("editSave.closeConfirm");
    expect(h.editor.tabs.value).toHaveLength(2);
    expect(h.editor.activeId.value).toBe(second.id);

    confirmMock.mockResolvedValueOnce(true);
    await h.editor.closeTab(second.id);
    expect(h.editor.tabs.value).toHaveLength(1);
    expect(h.editor.activeId.value).toBe(first.id);

    // 干净的页签关闭不弹确认；最后一个关闭后 activeId 清空。
    confirmMock.mockClear();
    await h.editor.closeTab(first.id);
    expect(confirmMock).not.toHaveBeenCalled();
    expect(h.editor.tabs.value).toHaveLength(0);
    expect(h.editor.activeId.value).toBe("");
  });

  it("同一路径重复打开：只激活既有页签，不重复读取", async () => {
    const h = setupHarness();
    h.files.set("/home/notes.txt", "hello");
    await h.editor.openEntry(entry("notes.txt", 5));
    const tab = h.editor.tabs.value[0];
    h.invoke.mockClear();
    h.editor.activeId.value = "";

    await h.editor.openEntry(entry("notes.txt", 5));

    expect(h.editor.tabs.value).toHaveLength(1);
    expect(h.editor.activeId.value).toBe(tab.id);
    expect(h.invoke).not.toHaveBeenCalled();
  });

  it("目录不进入编辑器", async () => {
    const h = setupHarness();
    await h.editor.openEntry(entry("folder", undefined, "directory"));
    expect(h.editor.tabs.value).toHaveLength(0);
    expect(h.invoke).not.toHaveBeenCalled();
  });

  it("session 消失：静默清空所有页签（含脏页签，不弹确认）", async () => {
    const h = setupHarness();
    h.files.set("/home/notes.txt", "hello");
    await h.editor.openEntry(entry("notes.txt", 5));
    h.editor.updateText(h.editor.tabs.value[0].id, "dirty");

    h.session.value = undefined;
    await nextTick();

    expect(h.editor.tabs.value).toHaveLength(0);
    expect(h.editor.activeId.value).toBe("");
    expect(confirmMock).not.toHaveBeenCalled();
  });

  // —— 响应式页签（浏览器实测 bug）：push 进数组的裸对象后续 await 里的写入
  // 必须能被 watch/渲染观察到，而不是只有「事后直接读」才看得到新值。
  it("异步读取：loading true→false 与 text 变更可被 watch 观察到", async () => {
    const h = setupHarness();
    h.files.set("/home/notes.txt", "hello");
    const loadingStates: Array<boolean | undefined> = [];
    const texts: Array<string | undefined> = [];
    watch(() => h.editor.tabs.value[0]?.loading, (value) => loadingStates.push(value), { flush: "sync" });
    watch(() => h.editor.tabs.value[0]?.text, (value) => texts.push(value), { flush: "sync" });
    const release = gateRead(h);

    const opening = h.editor.openEntry(entry("notes.txt", 5));
    await vi.waitFor(() => expect(h.editor.tabs.value[0]?.loading).toBe(true));
    release();
    await opening;

    expect(loadingStates).toContain(true);
    expect(loadingStates.at(-1)).toBe(false);
    expect(loadingStates.lastIndexOf(false)).toBeGreaterThan(loadingStates.indexOf(true));
    expect(texts).toContain("hello");
    expect(h.editor.activeTab.value).toMatchObject({ loading: false, text: "hello" });
  });

  it("读取失败：error 变更可被 watch 观察到", async () => {
    const h = setupHarness();
    h.files.set("/home/notes.txt", "hello");
    const errors: Array<string | undefined> = [];
    watch(() => h.editor.tabs.value[0]?.error, (value) => errors.push(value), { flush: "sync" });
    const release = gateRead(h, MAX_INLINE_PREVIEW_BYTES, new Error("read failed"));

    const opening = h.editor.openEntry(entry("notes.txt", 5));
    await vi.waitFor(() => expect(h.editor.tabs.value[0]?.loading).toBe(true));
    release();
    await opening;

    expect(errors).toContain("read failed");
    expect(h.editor.tabs.value[0]).toMatchObject({ error: "read failed", loading: false });
  });

  it("图片页签：imageUrl 变更可被 watch 观察到", async () => {
    const h = setupHarness();
    h.files.set("/home/pic.png", "hi");
    const imageUrls: Array<string | undefined> = [];
    const loadingStates: Array<boolean | undefined> = [];
    watch(() => h.editor.tabs.value[0]?.imageUrl, (value) => imageUrls.push(value), { flush: "sync" });
    watch(() => h.editor.tabs.value[0]?.loading, (value) => loadingStates.push(value), { flush: "sync" });
    const release = gateRead(h, MAX_IMAGE_PREVIEW_BYTES);

    const opening = h.editor.openEntry(entry("pic.png", 2));
    await vi.waitFor(() => expect(h.editor.tabs.value[0]?.loading).toBe(true));
    release();
    await opening;

    expect(imageUrls).toContain(`data:image/png;base64,${encodeText("hi")}`);
    expect(loadingStates.at(-1)).toBe(false);
  });

  // —— sudo 分流：root-only 文件的内容嗅探与图片读取也必须走 sudo/readFile。
  it("sudo 嗅探：走 sudo/readFile（offset/length），不再调 sftp/read", async () => {
    const h = setupHarness({ sudoMode: true });
    h.files.set("/root/script", "hello");

    await h.editor.openEntry({ uri: "sftp:/root/script", name: "script", kind: "file", size: 5 });

    expect(h.invoke).toHaveBeenNthCalledWith(1, "sudo/readFile", {
      sessionId: "s1",
      path: "/root/script",
      offset: 0,
      length: SNIFF_CHUNK_BYTES,
    });
    expect(h.invoke).toHaveBeenNthCalledWith(2, "sudo/readFile", {
      sessionId: "s1",
      path: "/root/script",
      offset: 0,
      length: MAX_INLINE_PREVIEW_BYTES,
    });
    expect(h.invoke.mock.calls.some(([method]) => method === "sftp/read")).toBe(false);
    expect(h.editor.tabs.value[0]).toMatchObject({ text: "hello", loading: false });
  });

  it("sudo 嗅探命中二进制：提示并不建页签", async () => {
    const h = setupHarness({ sudoMode: true });
    h.invoke.mockResolvedValueOnce({
      dataBase64: encodeBase64(new Uint8Array([0, 1, 2, 3])),
      truncated: false,
    });

    await h.editor.openEntry({ uri: "sftp:/root/blob", name: "blob", kind: "file" });

    expect(h.editor.tabs.value).toHaveLength(0);
    expect(h.notices[0]).toContain("binaryFile.notOpen");
    expect(h.invoke).toHaveBeenCalledTimes(1);
    expect(h.invoke).toHaveBeenCalledWith("sudo/readFile", {
      sessionId: "s1",
      path: "/root/blob",
      offset: 0,
      length: SNIFF_CHUNK_BYTES,
    });
  });

  it("sudo 图片页签：openImageTab 走 sudo/readFile（offset/length）", async () => {
    const h = setupHarness({ sudoMode: true });
    h.files.set("/root/pic.png", "hi");

    await h.editor.openEntry({ uri: "sftp:/root/pic.png", name: "pic.png", kind: "file", size: 2 });

    expect(h.invoke).toHaveBeenCalledTimes(1);
    expect(h.invoke).toHaveBeenCalledWith("sudo/readFile", {
      sessionId: "s1",
      path: "/root/pic.png",
      offset: 0,
      length: MAX_IMAGE_PREVIEW_BYTES,
    });
    expect(h.editor.tabs.value[0]).toMatchObject({
      kind: "image",
      imageUrl: `data:image/png;base64,${encodeText("hi")}`,
      loading: false,
    });
  });

  // —— 路径对账：重命名同步页签路径/文件名，删除静默关闭其下页签。
  it("handleRename：目录前缀下页签改路径与文件名，正文/脏标记保留", async () => {
    const h = setupHarness();
    h.files.set("/home/dir/a.txt", "alpha");
    h.files.set("/home/dir/sub/b.log", "bravo");
    h.files.set("/home/keep.txt", "keep");
    await h.editor.openEntry(entryAt("/home/dir/a.txt", 5));
    await h.editor.openEntry(entryAt("/home/dir/sub/b.log", 5));
    const keepEntry = entryAt("/home/keep.txt", 4);
    await h.editor.openEntry(keepEntry);
    const nested = h.editor.tabs.value[1];
    h.editor.updateText(nested.id, "bravo changed");
    const keep = h.editor.tabs.value[2];

    h.editor.handleRename("/home/dir", "/home/renamed");

    expect(h.editor.tabs.value.map((tab) => tab.path)).toEqual([
      "/home/renamed/a.txt",
      "/home/renamed/sub/b.log",
      "/home/keep.txt",
    ]);
    expect(h.editor.tabs.value.map((tab) => tab.name)).toEqual(["a.txt", "b.log", "keep.txt"]);
    expect(nested).toMatchObject({
      path: "/home/renamed/sub/b.log",
      name: "b.log",
      text: "bravo changed",
      baseline: "bravo",
      dirty: true,
    });
    expect(keep.path).toBe("/home/keep.txt");
  });

  it("handleRename：文件本身重命名同步 path 与 name，正文保留", async () => {
    const h = setupHarness();
    h.files.set("/home/notes.txt", "hello");
    await h.editor.openEntry(entry("notes.txt", 5));
    const tab = h.editor.tabs.value[0];
    h.editor.updateText(tab.id, "hello!");

    h.editor.handleRename("/home/notes.txt", "/home/notes.md");

    expect(tab.path).toBe("/home/notes.md");
    expect(tab.name).toBe("notes.md");
    expect(tab.text).toBe("hello!");
    expect(tab.dirty).toBe(true);
  });

  it("closeTabsUnder：静默关闭目录下页签，活动页签落到相邻幸存页签", async () => {
    const h = setupHarness();
    for (const path of ["/home/a.txt", "/home/dir/b.txt", "/home/dir/sub/c.txt", "/home/z.txt"]) {
      h.files.set(path, path);
    }
    await h.editor.openEntry(entryAt("/home/a.txt", 1));
    const first = h.editor.tabs.value[0];
    await h.editor.openEntry(entryAt("/home/dir/b.txt", 1));
    const removedActive = h.editor.tabs.value[1];
    await h.editor.openEntry(entryAt("/home/dir/sub/c.txt", 1));
    await h.editor.openEntry(entryAt("/home/z.txt", 1));
    const last = h.editor.tabs.value[3];
    h.editor.activate(removedActive.id);
    h.editor.updateText(removedActive.id, "unsaved");
    confirmMock.mockClear();

    h.editor.closeTabsUnder("/home/dir");

    expect(h.editor.tabs.value.map((tab) => tab.path)).toEqual(["/home/a.txt", "/home/z.txt"]);
    // 删除区间之后的幸存页签优先作为落点。
    expect(h.editor.activeId.value).toBe(last.id);
    expect(confirmMock).not.toHaveBeenCalled();

    // 活动页签不在删除范围内：保持不变；无命中/空路径不做任何事。
    h.editor.activate(first.id);
    h.editor.closeTabsUnder("/home/missing");
    h.editor.closeTabsUnder("");
    expect(h.editor.tabs.value.map((tab) => tab.path)).toEqual(["/home/a.txt", "/home/z.txt"]);
    expect(h.editor.activeId.value).toBe(first.id);
  });

  it("closeTabsUnder：删除全部页签后清空 activeId", async () => {
    const h = setupHarness();
    h.files.set("/home/dir/only.txt", "only");
    await h.editor.openEntry(entryAt("/home/dir/only.txt", 4));
    expect(h.editor.activeId.value).not.toBe("");

    h.editor.closeTabsUnder("/home/dir/");

    expect(h.editor.tabs.value).toHaveLength(0);
    expect(h.editor.activeId.value).toBe("");
  });
});
