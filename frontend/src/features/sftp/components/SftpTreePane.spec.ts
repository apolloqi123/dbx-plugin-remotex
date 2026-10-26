// @vitest-environment happy-dom
// SftpTreePane 组件测试：行交互、键盘、改名、选中态与目录跟随滚动。
import { afterEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { t, makeEntry, makeNode, row, mountPane } from "./sftpTreePaneFixture";

afterEach(() => {
  document.body.innerHTML = "";
  vi.useRealTimers();
});

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

  it("selects a folder row and toggles it on a plain click", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    await wrapper.find(".sftp-tree-row").trigger("click");
    const payload = wrapper.emitted("select")?.[0];
    expect(payload?.[0]).toEqual(folder.entry);
    expect(payload?.[1]).toBeInstanceOf(MouseEvent);
    expect(wrapper.emitted("toggle")?.[0]).toEqual([folder]);
  });

  it("keeps folder selection semantics for Ctrl/Cmd/Shift clicks", async () => {
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

  it("selects and toggles once on a double click, then opens", async () => {
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const el = wrapper.find(".sftp-tree-row");

    await el.trigger("click", { detail: 1 });
    await el.trigger("click", { detail: 2 });
    await el.trigger("dblclick");

    // 首击选中并 toggle；detail > 1 的重复 click 被忽略，随后 dblclick 打开。
    expect(wrapper.emitted("select")).toHaveLength(1);
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

