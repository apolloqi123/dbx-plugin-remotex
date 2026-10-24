// @vitest-environment happy-dom
// SftpTreePane 组件测试：文件夹/文件单击语义、caret 阻断、双击打开、右键、
// 键盘（Enter/Space/方向键/roving focus）、改名提交/取消、选中/当前态与空态；
// 以及 Info 详情卡（250ms 悬停/聚焦、懒加载缓存）与拖放目标（高亮、2s 悬停
// 展开、drop-on 同步上抛）。
import { afterEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { mount } from "@vue/test-utils";
import SftpTreePane from "./SftpTreePane.vue";
import type { SftpEntryKind, SftpSanitizedEntry } from "../lib/sftpEntries";
import type { SftpFileTreeNode, SftpTreeRow } from "../lib/sftpFileTree";
import type { SftpDetailInfo } from "../lib/sftpDetails";

const t = (key: string, values?: Record<string, string | number>) => {
  if (key === "sftpSide.root") return "ROOT";
  const template =
    key === "sftpSide.expandNode"
      ? "expand {name}"
      : key === "sftpSide.collapseNode"
        ? "collapse {name}"
        : key;
  return template.replace(/\{(\w+)\}/g, (_match, name: string) => String(values?.[name] ?? `{${name}}`));
};

function makeEntry(path: string, kind: SftpEntryKind, lossy = false): SftpSanitizedEntry {
  const name = path === "/" ? "/" : path.slice(path.lastIndexOf("/") + 1);
  return { name, uri: `sftp:${path}`, kind, ...(lossy ? { lossy: true } : {}) };
}

function makeNode(
  path: string,
  kind: SftpEntryKind = "file",
  overrides: Partial<SftpFileTreeNode> = {},
): SftpFileTreeNode {
  const entry = makeEntry(path, kind, overrides.entry?.lossy);
  return {
    path,
    name: entry.name,
    kind,
    entry: { ...entry, ...overrides.entry },
    expanded: false,
    loaded: false,
    loading: false,
    children: [],
    ...overrides,
  };
}

function row(node: SftpFileTreeNode, depth = 0): SftpTreeRow {
  return { node, depth };
}

interface PaneProps {
  rows: SftpTreeRow[];
  selectedUris: Set<string>;
  selectedPath: string;
  currentPath: string;
  renamingPath: string;
  renameDraft: string;
  renameSubmitting: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
  locale: string;
  virtualize: boolean;
  emptyText: string;
  loadDetails?: (entry: SftpSanitizedEntry) => Promise<SftpDetailInfo | null>;
  externalDragActive?: boolean;
  followPadding?: boolean;
}

function mountPane(rows: SftpTreeRow[], overrides: Partial<Omit<PaneProps, "rows">> = {}) {
  const props: PaneProps = {
    rows,
    selectedUris: new Set<string>(),
    selectedPath: "",
    currentPath: "/",
    renamingPath: "",
    renameDraft: "",
    renameSubmitting: false,
    t,
    locale: "en",
    virtualize: false,
    emptyText: "",
    externalDragActive: false,
    ...overrides,
  };
  return mount(SftpTreePane, { attachTo: document.body, props });
}

afterEach(() => {
  document.body.innerHTML = "";
  vi.useRealTimers();
});

/** 详情卡里 loadDetails 的 promise 在 fake timers 下需要手动冲洗微任务。 */
async function flushMicrotasks(times = 6): Promise<void> {
  for (let index = 0; index < times; index += 1) await Promise.resolve();
  await nextTick();
}

/**
 * DragEvent-like 事件：happy-dom 的 dataTransfer 只读，直接 defineProperty
 * 注入最小桩；dispatch 时 currentTarget 由浏览器补齐。
 */
function dispatchDrag(
  target: Element,
  type: string,
  options: { types?: string[]; relatedTarget?: Node | null } = {},
): DragEvent {
  const event = new Event(type, {
    bubbles: true,
    cancelable: type !== "dragleave" && type !== "dragend",
  }) as DragEvent;
  Object.defineProperty(event, "dataTransfer", {
    value: {
      types: options.types ?? ["Files"],
      items: [],
      files: [],
      getData: () => "",
      setData: () => undefined,
    },
  });
  Object.defineProperty(event, "relatedTarget", { value: options.relatedTarget ?? null });
  target.dispatchEvent(event);
  return event;
}

function detailsCard(): HTMLElement | null {
  return document.body.querySelector<HTMLElement>(".sftp-detail-card");
}

describe("SftpTreePane", () => {
  it("renders folder/file rows, localizes the root and indents by depth", () => {
    const root = makeNode("/", "directory", { expanded: true });
    const file = makeNode("/readme.txt");
    const wrapper = mountPane([row(root, 0), row(file, 1)]);

    const rows = wrapper.findAll(".sftp-tree-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].find(".sftp-tree-name").text()).toBe("/");
    expect(rows[1].find(".sftp-tree-name").text()).toBe("readme.txt");
    expect(rows[0].attributes("style")).toContain("padding-left: 8px");
    expect(rows[1].attributes("style")).toContain("padding-left: 22px");
    expect(rows[0].attributes("role")).toBe("treeitem");
    expect(rows[0].attributes("aria-expanded")).toBe("true");
    expect(rows[1].attributes("aria-expanded")).toBeUndefined();
    expect(wrapper.find(".sftp-tree-caret").attributes("aria-label")).toBe("collapse /");
    expect(rows[0].find("svg.lucide-folder-open").exists()).toBe(true);
    expect(rows[1].find(".sftp-tree-file-icon").exists()).toBe(true);
  });

  it("renders a link icon for symlinks and a lossy warning", () => {
    const link = makeNode("/shortcut", "symlink", { entry: { ...makeEntry("/shortcut", "symlink", true) } });
    const wrapper = mountPane([row(link)]);
    expect(wrapper.find(".sftp-tree-file-icon.is-link").exists()).toBe(true);
    expect(wrapper.find(".sftp-lossy-icon").exists()).toBe(true);
  });

  it("emits toggle when a folder row is clicked and does not select it", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    await wrapper.find(".sftp-tree-row").trigger("click");
    expect(wrapper.emitted("toggle")?.[0]).toEqual([folder]);
    expect(wrapper.emitted("select")).toBeUndefined();
  });

  it("selects a folder instead of toggling it when Ctrl/Cmd/Shift is held", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const el = wrapper.find(".sftp-tree-row");

    await el.trigger("click", { ctrlKey: true });
    await el.trigger("click", { metaKey: true });
    await el.trigger("click", { shiftKey: true });

    expect(wrapper.emitted("toggle")).toBeUndefined();
    const selected = wrapper.emitted("select") ?? [];
    expect(selected).toHaveLength(3);
    expect(selected[0]?.[0]).toEqual(folder.entry);
    expect(selected[0]?.[1]).toBeInstanceOf(MouseEvent);
  });

  it("emits select with the entry and mouse event when a file row is clicked", async () => {
    const file = makeNode("/readme.txt");
    const wrapper = mountPane([row(file)]);
    await wrapper.find(".sftp-tree-row").trigger("click");
    const payload = wrapper.emitted("select")?.[0];
    expect(payload?.[0]).toEqual(file.entry);
    expect(payload?.[1]).toBeInstanceOf(MouseEvent);
    expect(wrapper.emitted("toggle")).toBeUndefined();
  });

  it("opens a file or folder on double click", async () => {
    const folder = makeNode("/var", "directory");
    const file = makeNode("/readme.txt");
    const wrapper = mountPane([row(folder), row(file)]);
    const rows = wrapper.findAll(".sftp-tree-row");
    await rows[0].trigger("dblclick");
    await rows[1].trigger("dblclick");
    expect(wrapper.emitted("open")?.[0]).toEqual([folder]);
    expect(wrapper.emitted("open")?.[1]).toEqual([file]);
  });

  it("does not toggle a folder twice when the second click of a double click arrives", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const el = wrapper.find(".sftp-tree-row");

    await el.trigger("click", { detail: 1 });
    await el.trigger("click", { detail: 2 });
    await el.trigger("dblclick");

    // 首次 click 只 toggle 一次；detail > 1 的重复 click 被忽略，随后 dblclick 打开。
    expect(wrapper.emitted("toggle")).toHaveLength(1);
    expect(wrapper.emitted("open")?.[0]).toEqual([folder]);
  });

  it("toggles from the caret without also firing the row click", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    await wrapper.find(".sftp-tree-caret").trigger("click");
    expect(wrapper.emitted("toggle")?.[0]).toEqual([folder]);
    expect(wrapper.emitted("toggle")).toHaveLength(1);
    expect(wrapper.emitted("open")).toBeUndefined();
    expect(wrapper.emitted("select")).toBeUndefined();
  });

  it("does not toggle twice when the caret receives the second click of a double click", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const caret = wrapper.find(".sftp-tree-caret");

    await caret.trigger("click", { detail: 1 });
    await caret.trigger("click", { detail: 2 });
    await caret.trigger("dblclick");

    expect(wrapper.emitted("toggle")).toHaveLength(1);
    expect(wrapper.emitted("open")).toBeUndefined();
  });

  it("emits context with the node and mouse event on right click", async () => {
    const file = makeNode("/readme.txt");
    const wrapper = mountPane([row(file)]);
    await wrapper.find(".sftp-tree-row").trigger("contextmenu", { clientX: 12, clientY: 34 });
    const payload = wrapper.emitted("context")?.[0];
    expect(payload?.[0]).toEqual(file);
    expect(payload?.[1]).toBeInstanceOf(MouseEvent);
  });

  it("opens on Enter and toggles folders on Space", async () => {
    const folder = makeNode("/var", "directory");
    const file = makeNode("/readme.txt");
    const wrapper = mountPane([row(folder), row(file)]);
    const rows = wrapper.findAll(".sftp-tree-row");

    await rows[0].trigger("keydown", { key: "Enter" });
    await rows[0].trigger("keydown", { key: " " });
    await rows[1].trigger("keydown", { key: "Enter" });
    await rows[1].trigger("keydown", { key: " " });

    expect(wrapper.emitted("open")?.[0]).toEqual([folder]);
    expect(wrapper.emitted("open")?.[1]).toEqual([file]);
    expect(wrapper.emitted("toggle")?.[0]).toEqual([folder]);
    expect(wrapper.emitted("toggle")).toHaveLength(1);
    // Space 在文件行上等同于选中，不再落入 row-keydown 的 no-op 分支。
    expect(wrapper.emitted("select")?.[0]).toEqual([file.entry, expect.any(KeyboardEvent)]);
    expect(wrapper.emitted("row-keydown")).toBeUndefined();
  });

  it("expands with ArrowRight and collapses with ArrowLeft", async () => {
    const collapsed = makeNode("/var", "directory");
    const expanded = makeNode("/etc", "directory", { expanded: true });
    const wrapper = mountPane([row(collapsed), row(expanded)]);
    const rows = wrapper.findAll(".sftp-tree-row");

    await rows[0].trigger("keydown", { key: "ArrowRight" });
    await rows[1].trigger("keydown", { key: "ArrowLeft" });

    expect(wrapper.emitted("toggle")?.[0]).toEqual([collapsed]);
    expect(wrapper.emitted("toggle")?.[1]).toEqual([expanded]);
  });

  it("moves focus with ArrowDown/ArrowUp between rendered rows", async () => {
    const nodes = [makeNode("/a", "directory"), makeNode("/b", "directory"), makeNode("/c", "directory")];
    const wrapper = mountPane(nodes.map((node) => row(node)));
    const rows = wrapper.findAll(".sftp-tree-row");

    await rows[0].trigger("keydown", { key: "ArrowDown" });
    expect(document.activeElement).toBe(rows[1].element);
    await rows[1].trigger("keydown", { key: "ArrowDown" });
    expect(document.activeElement).toBe(rows[2].element);
    await rows[2].trigger("keydown", { key: "ArrowUp" });
    expect(document.activeElement).toBe(rows[1].element);
  });

  it("moves to the parent row with ArrowLeft from a child", async () => {
    const parent = makeNode("/var", "directory");
    const child = makeNode("/var/log.txt");
    const wrapper = mountPane([row(parent, 0), row(child, 1)]);
    const rows = wrapper.findAll(".sftp-tree-row");

    await rows[1].trigger("keydown", { key: "ArrowLeft" });
    expect(document.activeElement).toBe(rows[0].element);
  });

  it("commits rename on Enter and blur, and cancels on Escape", async () => {
    const file = makeNode("/readme.txt");
    const enterWrapper = mountPane([row(file)], {
      renamingPath: file.entry.uri,
      renameDraft: "readme.txt",
    });
    const enterInput = enterWrapper.find("input.rename-input");
    expect((enterInput.element as HTMLInputElement).value).toBe("readme.txt");
    await enterInput.setValue("renamed.txt");
    await enterInput.trigger("keydown", { key: "Enter" });
    expect(enterWrapper.emitted("rename-commit")?.[0]).toEqual([file, "renamed.txt"]);

    const blurWrapper = mountPane([row(file)], {
      renamingPath: file.entry.uri,
      renameDraft: "readme.txt",
    });
    const blurInput = blurWrapper.find("input.rename-input");
    await blurInput.setValue("blurred.txt");
    await blurInput.trigger("blur");
    expect(blurWrapper.emitted("rename-commit")?.[0]).toEqual([file, "blurred.txt"]);

    const escapeWrapper = mountPane([row(file)], {
      renamingPath: file.entry.uri,
      renameDraft: "readme.txt",
    });
    await escapeWrapper.find("input.rename-input").trigger("keydown", { key: "Escape" });
    expect(escapeWrapper.emitted("rename-cancel")).toHaveLength(1);
    expect(escapeWrapper.emitted("rename-commit")).toBeUndefined();
  });

  it("applies selected and current classes from uris and paths", () => {
    const folder = makeNode("/var", "directory");
    const selected = makeNode("/var/selected.txt");
    const singular = makeNode("/var/singular.txt");
    const wrapper = mountPane([row(folder), row(selected, 1), row(singular, 1)], {
      currentPath: "/var",
      selectedPath: singular.entry.uri,
      selectedUris: new Set([selected.entry.uri]),
    });

    const rows = wrapper.findAll(".sftp-tree-row");
    expect(rows[0].classes()).toContain("is-current");
    expect(rows[1].classes()).toContain("is-selected");
    expect(rows[1].attributes("aria-selected")).toBe("true");
    expect(rows[2].classes()).toContain("is-selected");
    expect(rows[0].attributes("tabindex")).toBe("0");
    expect(rows[1].attributes("tabindex")).toBe("-1");
  });

  it("shows the default and custom empty text when there are no rows", () => {
    const fallback = mountPane([]);
    expect(fallback.find(".empty").text()).toBe("emptyFolder");

    const custom = mountPane([], { emptyText: t("sftpSearch.noMatch") });
    expect(custom.find(".empty").text()).toBe("sftpSearch.noMatch");
  });
});

describe("SftpTreePane details card", () => {
  it("opens after 250ms on icon hover and shows a loading row while the lazy load is pending", async () => {
    vi.useFakeTimers();
    const file = makeNode("/home/deploy/readme.txt", "file", {
      entry: {
        ...makeEntry("/home/deploy/readme.txt", "file"),
        size: 12,
        modifiedAt: 2_000,
        permissions: "rw-r--r--",
      },
    });
    let resolveDetails!: (value: SftpDetailInfo | null) => void;
    const loadDetails = vi.fn(
      () =>
        new Promise<SftpDetailInfo | null>((resolve) => {
          resolveDetails = resolve;
        }),
    );
    const wrapper = mountPane([row(file)], { loadDetails });

    try {
      const info = wrapper.find(".sftp-tree-info");
      expect(info.exists()).toBe(true);

      await info.trigger("mouseenter");
      await vi.advanceTimersByTimeAsync(200);
      expect(detailsCard()).toBeNull();
      expect(loadDetails).not.toHaveBeenCalled();

      await vi.advanceTimersByTimeAsync(50);
      await nextTick();
      expect(loadDetails).toHaveBeenCalledTimes(1);
      expect(detailsCard()?.textContent).toContain("sftpDetails.loading");

      resolveDetails({
        path: "/home/deploy/readme.txt",
        kind: "file",
        size: 4096,
        modifiedAt: 2_000,
        permissions: "rw-r--r--",
        owner: "deploy",
        group: "staff",
      });
      await flushMicrotasks();

      const text = detailsCard()?.textContent ?? "";
      expect(text).toContain("/home/deploy/readme.txt");
      expect(text).toContain("deploy");
      expect(text).toContain("staff");
      expect(text).toContain("4.0 KiB");
      expect(text).not.toContain("sftpDetails.loading");
    } finally {
      wrapper.unmount();
    }
  });

  it("hides the card on mouseleave, blur and scroll", async () => {
    vi.useFakeTimers();
    const file = makeNode("/readme.txt");
    const wrapper = mountPane([row(file)]);

    try {
      const info = wrapper.find(".sftp-tree-info");
      await info.trigger("mouseenter");
      await vi.advanceTimersByTimeAsync(250);
      await nextTick();
      expect(detailsCard()).not.toBeNull();

      await info.trigger("mouseleave");
      await nextTick();
      expect(detailsCard()).toBeNull();

      await info.trigger("focus");
      await vi.advanceTimersByTimeAsync(250);
      await nextTick();
      expect(detailsCard()).not.toBeNull();

      await info.trigger("blur");
      await nextTick();
      expect(detailsCard()).toBeNull();

      await info.trigger("mouseenter");
      await vi.advanceTimersByTimeAsync(250);
      await nextTick();
      expect(detailsCard()).not.toBeNull();

      await wrapper.find(".sftp-tree-pane").trigger("scroll");
      await nextTick();
      expect(detailsCard()).toBeNull();
    } finally {
      wrapper.unmount();
    }
  });

  it("does not open the card when the pointer leaves before the delay", async () => {
    vi.useFakeTimers();
    const file = makeNode("/readme.txt");
    const loadDetails = vi.fn().mockResolvedValue(null);
    const wrapper = mountPane([row(file)], { loadDetails });

    try {
      const info = wrapper.find(".sftp-tree-info");
      await info.trigger("mouseenter");
      await vi.advanceTimersByTimeAsync(100);
      await info.trigger("mouseleave");
      await vi.advanceTimersByTimeAsync(500);

      expect(detailsCard()).toBeNull();
      expect(loadDetails).not.toHaveBeenCalled();
    } finally {
      wrapper.unmount();
    }
  });

  it("calls loadDetails once per entry uri and reloads after modifiedAt changes", async () => {
    vi.useFakeTimers();
    const loadDetails = vi
      .fn()
      .mockResolvedValue({ path: "/a.txt", kind: "file", owner: "one", group: "g1" });
    const file = makeNode("/a.txt");
    const wrapper = mountPane([row(file)], { loadDetails });

    try {
      const info = wrapper.find(".sftp-tree-info");
      await info.trigger("mouseenter");
      await vi.advanceTimersByTimeAsync(250);
      await flushMicrotasks();
      expect(loadDetails).toHaveBeenCalledTimes(1);
      expect(detailsCard()?.textContent).toContain("one");

      await info.trigger("mouseleave");
      await info.trigger("mouseenter");
      await vi.advanceTimersByTimeAsync(250);
      await flushMicrotasks();
      expect(loadDetails).toHaveBeenCalledTimes(1);

      const updated = makeNode("/a.txt", "file", {
        entry: { ...makeEntry("/a.txt", "file"), modifiedAt: 500 },
      });
      await wrapper.setProps({ rows: [row(updated)] });
      await wrapper.find(".sftp-tree-info").trigger("mouseenter");
      await vi.advanceTimersByTimeAsync(250);
      await flushMicrotasks();
      expect(loadDetails).toHaveBeenCalledTimes(2);
    } finally {
      wrapper.unmount();
    }
  });

  it("reloads details when permissions change without an mtime change", async () => {
    vi.useFakeTimers();
    const loadDetails = vi.fn().mockResolvedValue({ path: "/a.txt", kind: "file" });
    const file = makeNode("/a.txt", "file", {
      entry: { ...makeEntry("/a.txt", "file"), modifiedAt: 100, permissions: "rw-r--r--" },
    });
    const wrapper = mountPane([row(file)], { loadDetails });

    try {
      await wrapper.find(".sftp-tree-info").trigger("mouseenter");
      await vi.advanceTimersByTimeAsync(250);
      await flushMicrotasks();
      expect(loadDetails).toHaveBeenCalledTimes(1);

      const updated = makeNode("/a.txt", "file", {
        entry: { ...makeEntry("/a.txt", "file"), modifiedAt: 100, permissions: "rw-------" },
      });
      await wrapper.setProps({ rows: [row(updated)] });
      await wrapper.find(".sftp-tree-info").trigger("mouseenter");
      await vi.advanceTimersByTimeAsync(250);
      await flushMicrotasks();
      expect(loadDetails).toHaveBeenCalledTimes(2);
    } finally {
      wrapper.unmount();
    }
  });

  it("still shows entry fields and an unavailable note when loadDetails resolves null", async () => {
    vi.useFakeTimers();
    const file = makeNode("/tmp/file.bin", "file", {
      entry: {
        ...makeEntry("/tmp/file.bin", "file"),
        size: 9,
        permissions: "rw-------",
        owner: "entry-owner",
        group: "entry-group",
      },
    });
    const loadDetails = vi.fn().mockResolvedValue(null);
    const wrapper = mountPane([row(file)], { loadDetails });

    try {
      await wrapper.find(".sftp-tree-info").trigger("mouseenter");
      await vi.advanceTimersByTimeAsync(250);
      await flushMicrotasks();

      const text = detailsCard()?.textContent ?? "";
      expect(text).toContain("/tmp/file.bin");
      expect(text).toContain("9 B");
      expect(text).toContain("rw-------");
      expect(text).toContain("entry-owner");
      expect(text).toContain("entry-group");
      expect(text).toContain("sftpDetails.unavailable");
    } finally {
      wrapper.unmount();
    }
  });

  it("does not select, toggle or open the row when the info icon is clicked", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);

    await wrapper.find(".sftp-tree-info").trigger("click");

    expect(wrapper.emitted("toggle")).toBeUndefined();
    expect(wrapper.emitted("select")).toBeUndefined();
    expect(wrapper.emitted("open")).toBeUndefined();
  });
});

describe("SftpTreePane directory follow scrolling", () => {
  it("pins a late row with the minimal bottom padding and drops it on manual scroll", async () => {
    const heightSpy = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(200);
    try {
      const rows = Array.from({ length: 20 }, (_, index) => row(makeNode(`/dir${index}`, "directory")));
      const wrapper = mountPane(rows, { followPadding: true });
      await nextTick();
      const scroller = wrapper.find(".sftp-tree-pane").element as HTMLElement;
      const api = wrapper.vm as unknown as { scrollPathToTop(path: string): Promise<boolean> };

      expect(await api.scrollPathToTop("/dir19")).toBe(true);
      // maxTop = 20*28 + 172 - 200 = 532 → 最后一行正好贴顶，余量只是补足
      // 视口高度差（不再恒为一整屏）。
      expect(scroller.scrollTop).toBe(532);
      const spacer = wrapper.find(".sftp-tree-follow-pad");
      expect(spacer.exists(), "follow padding spacer missing").toBe(true);
      expect((spacer.element as HTMLElement).style.height).toBe("172px");

      // 靠前的目标自然可达顶部：不补余量。
      expect(await api.scrollPathToTop("/dir1")).toBe(true);
      expect(scroller.scrollTop).toBe(28);
      expect(wrapper.find(".sftp-tree-follow-pad").exists()).toBe(false);

      // 根行本来就在顶部：短列表也不补余量。
      expect(await api.scrollPathToTop("/dir0")).toBe(true);
      expect(scroller.scrollTop).toBe(0);
      expect(wrapper.find(".sftp-tree-follow-pad").exists()).toBe(false);
      wrapper.unmount();
    } finally {
      heightSpy.mockRestore();
    }
  });

  it("pins with the measured row height instead of the nominal constant", async () => {
    const heightSpy = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(200);
    // 真实宿主里行高可能被主题/缩放改成 24px：虚拟窗口与置顶必须按实测值，
    // 否则按 28px 推算会累积成“顶掉一行”。
    const rowHeightSpy = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(24);
    try {
      const rows = Array.from({ length: 20 }, (_, index) => row(makeNode(`/dir${index}`, "directory")));
      const wrapper = mountPane(rows, { followPadding: true });
      await nextTick();
      const scroller = wrapper.find(".sftp-tree-pane").element as HTMLElement;
      const api = wrapper.vm as unknown as { scrollPathToTop(path: string): Promise<boolean> };

      expect(await api.scrollPathToTop("/dir19")).toBe(true);
      // 19*24 = 456；pad = 456 + 200 - 20*24 = 176。
      expect(scroller.scrollTop).toBe(456);
      expect((wrapper.find(".sftp-tree-follow-pad").element as HTMLElement).style.height).toBe("176px");
      wrapper.unmount();
    } finally {
      rowHeightSpy.mockRestore();
      heightSpy.mockRestore();
    }
  });

  it("pins a shallow row in a short list instead of leaving it mid-viewport", async () => {
    const heightSpy = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(200);
    try {
      const wrapper = mountPane(
        [row(makeNode("/a", "directory")), row(makeNode("/b", "directory")), row(makeNode("/c", "directory"))],
        { followPadding: true },
      );
      await nextTick();
      const scroller = wrapper.find(".sftp-tree-pane").element as HTMLElement;
      const api = wrapper.vm as unknown as { scrollPathToTop(path: string): Promise<boolean> };

      // pad = 56 + 200 - 84 = 172 → /c 精确贴顶。
      expect(await api.scrollPathToTop("/c")).toBe(true);
      expect(scroller.scrollTop).toBe(56);
      expect((wrapper.find(".sftp-tree-follow-pad").element as HTMLElement).style.height).toBe("172px");
      wrapper.unmount();
    } finally {
      heightSpy.mockRestore();
    }
  });

  it("releases the follow reserve once manual scrolling reaches the natural range", async () => {
    const heightSpy = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(200);
    try {
      const rows = Array.from({ length: 20 }, (_, index) => row(makeNode(`/dir${index}`, "directory")));
      const wrapper = mountPane(rows, { followPadding: true });
      await nextTick();
      const scroller = wrapper.find(".sftp-tree-pane").element as HTMLElement;
      const api = wrapper.vm as unknown as { scrollPathToTop(path: string): Promise<boolean> };

      expect(await api.scrollPathToTop("/dir19")).toBe(true);
      expect(wrapper.find(".sftp-tree-follow-pad").exists()).toBe(true);

      // 用户向上滚回自然可滚动范围（20*28-200=360）：余量撤掉，行位置不跳。
      scroller.scrollTop = 300;
      await wrapper.find(".sftp-tree-pane").trigger("scroll");
      await nextTick();

      expect(wrapper.find(".sftp-tree-follow-pad").exists()).toBe(false);
      expect(scroller.scrollTop).toBe(300);
      wrapper.unmount();
    } finally {
      heightSpy.mockRestore();
    }
  });

  it("does not reserve padding for an empty folder", async () => {
    const heightSpy = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(200);
    try {
      const wrapper = mountPane([], { followPadding: true });
      await nextTick();
      expect(wrapper.find(".sftp-tree-follow-pad").exists()).toBe(false);
      wrapper.unmount();
    } finally {
      heightSpy.mockRestore();
    }
  });

  it("scrolls the followed directory row to the top of the viewport", async () => {
    const wrapper = mountPane([row(makeNode("/a", "directory")), row(makeNode("/b", "directory")), row(makeNode("/c", "directory"))]);
    const scroller = wrapper.find(".sftp-tree-pane").element as HTMLElement;
    const api = wrapper.vm as unknown as { scrollPathToTop(path: string): Promise<boolean> };

    expect(await api.scrollPathToTop("/b")).toBe(true);
    expect(scroller.scrollTop).toBe(28);

    expect(await api.scrollPathToTop("/missing")).toBe(false);
    expect(scroller.scrollTop).toBe(28);
    wrapper.unmount();
  });
});

describe("SftpTreePane drag and drop targeting", () => {
  it("highlights a collapsed directory row for file drags", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const rowEl = wrapper.find(".sftp-tree-row");

    dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
    await nextTick();

    expect(rowEl.classes()).toContain("is-drop-target");
  });

  it("uses the host dragActive signal when DOM types are stripped", async () => {
    vi.useFakeTimers();
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)], { externalDragActive: true });
    const rowEl = wrapper.find(".sftp-tree-row");

    try {
      dispatchDrag(rowEl.element, "dragover", { types: [] });
      await nextTick();
      expect(rowEl.classes()).toContain("is-drop-target");
      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual(["/var"]);

      await wrapper.setProps({ externalDragActive: false });
      await nextTick();
      // 宿主 dragActive=false 可能先于 onDrop：路径保留给宿主落点读取，
      // 高亮本身已清除。
      expect(rowEl.classes()).not.toContain("is-drop-target");
      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual(["/var"]);

      await wrapper.setProps({ externalDragActive: true });
      dispatchDrag(rowEl.element, "dragover", { types: [] });
      await vi.advanceTimersByTimeAsync(2_000);
      expect(wrapper.emitted("toggle")).toHaveLength(1);
    } finally {
      wrapper.unmount();
    }
  });

  it("does not expose the full path as a native row tooltip", () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    expect(wrapper.find(".sftp-tree-row").attributes("title")).toBeUndefined();
  });

  it("does not highlight for non-file drags or non-directory rows", async () => {
    const folder = makeNode("/var", "directory");
    const file = makeNode("/readme.txt");
    const wrapper = mountPane([row(folder), row(file)]);
    const rows = wrapper.findAll(".sftp-tree-row");

    dispatchDrag(rows[1].element, "dragover", { types: ["Files"] });
    await nextTick();
    expect(rows[1].classes()).not.toContain("is-drop-target");

    dispatchDrag(rows[0].element, "dragover", { types: ["text/plain"] });
    await nextTick();
    expect(rows[0].classes()).not.toContain("is-drop-target");
  });

  it("keeps the highlight on child-to-child moves and clears it when leaving the row", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const rowEl = wrapper.find(".sftp-tree-row");
    const caret = rowEl.find(".sftp-tree-caret");

    dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
    await nextTick();
    expect(rowEl.classes()).toContain("is-drop-target");

    dispatchDrag(rowEl.element, "dragleave", { relatedTarget: caret.element });
    await nextTick();
    expect(rowEl.classes()).toContain("is-drop-target");

    dispatchDrag(rowEl.element, "dragleave", { relatedTarget: document.body });
    await nextTick();
    expect(rowEl.classes()).not.toContain("is-drop-target");
  });

  it("emits toggle once after hovering a collapsed folder for 2000ms", async () => {
    vi.useFakeTimers();
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const rowEl = wrapper.find(".sftp-tree-row");

    try {
      dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
      dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
      await nextTick();

      await vi.advanceTimersByTimeAsync(1_999);
      expect(wrapper.emitted("toggle")).toBeUndefined();

      await vi.advanceTimersByTimeAsync(1);
      expect(wrapper.emitted("toggle")).toHaveLength(1);
      expect(wrapper.emitted("toggle")?.[0]).toEqual([folder]);

      await vi.advanceTimersByTimeAsync(2_000);
      expect(wrapper.emitted("toggle")).toHaveLength(1);

      // 父级异步展开前原生 dragover 仍会持续触发：不得重复 toggle。
      dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
      await vi.advanceTimersByTimeAsync(2_000);
      expect(wrapper.emitted("toggle")).toHaveLength(1);
    } finally {
      wrapper.unmount();
    }
  });

  it("does not start the expand timer for expanded or loading folders", async () => {
    vi.useFakeTimers();
    const expanded = makeNode("/etc", "directory", { expanded: true });
    const loading = makeNode("/mnt", "directory", { loading: true });
    const wrapper = mountPane([row(expanded), row(loading)]);
    const rows = wrapper.findAll(".sftp-tree-row");

    try {
      dispatchDrag(rows[0].element, "dragover", { types: ["Files"] });
      await nextTick();
      expect(rows[0].classes()).toContain("is-drop-target");

      await vi.advanceTimersByTimeAsync(2_000);
      expect(wrapper.emitted("toggle")).toBeUndefined();

      dispatchDrag(rows[1].element, "dragover", { types: ["Files"] });
      await nextTick();
      expect(rows[1].classes()).toContain("is-drop-target");
      expect(rows[0].classes()).not.toContain("is-drop-target");

      await vi.advanceTimersByTimeAsync(2_000);
      expect(wrapper.emitted("toggle")).toBeUndefined();
    } finally {
      wrapper.unmount();
    }
  });

  it("cancels the expand timer when the drag leaves the row", async () => {
    vi.useFakeTimers();
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const rowEl = wrapper.find(".sftp-tree-row");

    try {
      dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
      await vi.advanceTimersByTimeAsync(1_000);
      dispatchDrag(rowEl.element, "dragleave", { relatedTarget: document.body });
      await nextTick();
      expect(rowEl.classes()).not.toContain("is-drop-target");

      await vi.advanceTimersByTimeAsync(2_000);
      expect(wrapper.emitted("toggle")).toBeUndefined();
    } finally {
      wrapper.unmount();
    }
  });

  it("clears the drag state on dragend", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const rowEl = wrapper.find(".sftp-tree-row");

    dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
    await nextTick();
    expect(rowEl.classes()).toContain("is-drop-target");

    dispatchDrag(rowEl.element, "dragend");
    await nextTick();
    expect(rowEl.classes()).not.toContain("is-drop-target");
  });

  it("emits drop-on with the node and original event without selecting or toggling", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const rowEl = wrapper.find(".sftp-tree-row");

    dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
    await nextTick();
    expect(rowEl.classes()).toContain("is-drop-target");

    const dropEvent = dispatchDrag(rowEl.element, "drop", { types: ["Files"] });
    await nextTick();

    const payload = wrapper.emitted("drop-on")?.[0];
    expect(payload?.[0]).toEqual(folder);
    expect(payload?.[1]).toBe(dropEvent);
    expect(payload?.[1]).toBeInstanceOf(Event);
    expect(dropEvent.defaultPrevented).toBe(true);
    expect(rowEl.classes()).not.toContain("is-drop-target");
    expect(wrapper.emitted("select")).toBeUndefined();
    expect(wrapper.emitted("toggle")).toBeUndefined();
  });

  it("ignores drops on non-directory rows", async () => {
    const file = makeNode("/readme.txt");
    const wrapper = mountPane([row(file)]);
    const rowEl = wrapper.find(".sftp-tree-row");

    dispatchDrag(rowEl.element, "drop", { types: ["Files"] });
    await nextTick();

    expect(wrapper.emitted("drop-on")).toBeUndefined();
  });

  it("keeps the pointer-hit target when stale :hover still reports the auto-expanded folder", async () => {
    // 真机复现：宿主原生拖拽期 CSS :hover 冻结在进入时的 A 行；A 自动展开后
    // 拖到 B，轮询仍拿到 A。事件命中的 B 不得被兜底覆盖，否则 drop 落回 A。
    vi.useFakeTimers();
    const folderA = makeNode("/a", "directory");
    const folderB = makeNode("/b", "directory");
    const wrapper = mountPane([row(folderA), row(folderB)], { externalDragActive: true });
    const rowEls = wrapper.findAll(".sftp-tree-row");
    const rowA = rowEls[0].element;
    const rowB = rowEls[1].element;
    const originalQuerySelector = document.querySelector.bind(document);
    const querySpy = vi.spyOn(document, "querySelector").mockImplementation((selector: string) => {
      if (selector === ".sftp-tree-row:hover") return rowA;
      return originalQuerySelector(selector);
    });

    try {
      dispatchDrag(rowA, "dragover", { types: [] });
      await nextTick();
      await vi.advanceTimersByTimeAsync(2_000);
      expect(wrapper.emitted("toggle")).toHaveLength(1);
      expect(wrapper.emitted("toggle")?.[0]).toEqual([folderA]);

      // 父级展开 A 后继续拖到 B：B 的行级 dragover 更新目标。
      folderA.expanded = true;
      dispatchDrag(rowA, "dragleave", { relatedTarget: rowB });
      dispatchDrag(rowB, "dragover", { types: [] });
      await nextTick();
      expect(rowEls[1].classes()).toContain("is-drop-target");
      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual(["/b"]);

      // 冻结的 :hover 轮询继续跑：目标必须保持 B，B 的 2s 展开计时不能被清掉。
      await vi.advanceTimersByTimeAsync(1_500);
      expect(rowEls[1].classes()).toContain("is-drop-target");
      expect(rowEls[0].classes()).not.toContain("is-drop-target");
      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual(["/b"]);

      await vi.advanceTimersByTimeAsync(500);
      expect(wrapper.emitted("toggle")?.at(-1)).toEqual([folderB]);
      expect(wrapper.emitted("toggle")).toHaveLength(2);
    } finally {
      querySpy.mockRestore();
      wrapper.unmount();
    }
  });

  it("lets a live :hover move to B after drag events stop", async () => {
    // 宿主可能在拖入时只派发一次 dragover，之后只更新 CSS :hover；事件锁存
    // 不能永久压住 live hover，否则 A 展开后仍然无法把目标切到 B。
    vi.useFakeTimers();
    const folderA = makeNode("/a", "directory");
    const folderB = makeNode("/b", "directory");
    const wrapper = mountPane([row(folderA), row(folderB)], { externalDragActive: true });
    const rowEls = wrapper.findAll(".sftp-tree-row");
    const rowA = rowEls[0].element;
    const rowB = rowEls[1].element;
    let hoverRow: Element = rowA;
    const originalQuerySelector = document.querySelector.bind(document);
    const querySpy = vi.spyOn(document, "querySelector").mockImplementation((selector: string) => {
      if (selector === ".sftp-tree-row:hover") return hoverRow;
      return originalQuerySelector(selector);
    });

    try {
      await vi.advanceTimersByTimeAsync(300);
      dispatchDrag(rowA, "dragover", { types: [] });
      await nextTick();
      await vi.advanceTimersByTimeAsync(2_000);
      expect(wrapper.emitted("toggle")?.[0]).toEqual([folderA]);

      folderA.expanded = true;
      hoverRow = rowB;
      await vi.advanceTimersByTimeAsync(2_200);

      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual(["/b"]);
      expect(wrapper.emitted("toggle")?.at(-1)).toEqual([folderB]);
      expect(wrapper.emitted("toggle")).toHaveLength(2);
    } finally {
      querySpy.mockRestore();
      wrapper.unmount();
    }
  });

  it("still uses the :hover fallback when no drag events reach the rows", async () => {
    vi.useFakeTimers();
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)], { externalDragActive: true });
    const rowEl = wrapper.find(".sftp-tree-row").element;
    const originalQuerySelector = document.querySelector.bind(document);
    const querySpy = vi.spyOn(document, "querySelector").mockImplementation((selector: string) => {
      if (selector === ".sftp-tree-row:hover") return rowEl;
      return originalQuerySelector(selector);
    });

    try {
      await vi.advanceTimersByTimeAsync(300);
      expect(wrapper.find(".sftp-tree-row").classes()).toContain("is-drop-target");
      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual(["/var"]);

      await vi.advanceTimersByTimeAsync(2_000);
      expect(wrapper.emitted("toggle")).toHaveLength(1);
    } finally {
      querySpy.mockRestore();
      wrapper.unmount();
    }
  });

  it("clears the pending expand timer on unmount", async () => {
    vi.useFakeTimers();
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const rowEl = wrapper.find(".sftp-tree-row");

    dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(2_000);

    expect(wrapper.emitted("toggle")).toBeUndefined();
  });
});
