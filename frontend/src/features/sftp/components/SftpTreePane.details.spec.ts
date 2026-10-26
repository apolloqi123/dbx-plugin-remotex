// @vitest-environment happy-dom
// SftpTreePane 组件测试：Info 详情卡与拖放目标高亮/1s 展开。
import { afterEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { t, makeEntry, makeNode, row, mountPane, flushMicrotasks, dispatchDrag, detailsCard } from "./sftpTreePaneFixture";

afterEach(() => {
  document.body.innerHTML = "";
  vi.useRealTimers();
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
      await vi.advanceTimersByTimeAsync(1_000);
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

  it("emits toggle once after hovering a collapsed folder for 1000ms", async () => {
    vi.useFakeTimers();
    const folder = makeNode("/var", "directory");
    const wrapper = mountPane([row(folder)]);
    const rowEl = wrapper.find(".sftp-tree-row");

    try {
      dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
      dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
      await nextTick();

      await vi.advanceTimersByTimeAsync(999);
      expect(wrapper.emitted("toggle")).toBeUndefined();

      await vi.advanceTimersByTimeAsync(1);
      expect(wrapper.emitted("toggle")).toHaveLength(1);
      expect(wrapper.emitted("toggle")?.[0]).toEqual([folder]);

      await vi.advanceTimersByTimeAsync(1_000);
      expect(wrapper.emitted("toggle")).toHaveLength(1);

      // 父级异步展开前原生 dragover 仍会持续触发：不得重复 toggle。
      dispatchDrag(rowEl.element, "dragover", { types: ["Files"] });
      await vi.advanceTimersByTimeAsync(1_000);
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

      await vi.advanceTimersByTimeAsync(1_000);
      expect(wrapper.emitted("toggle")).toBeUndefined();

      dispatchDrag(rows[1].element, "dragover", { types: ["Files"] });
      await nextTick();
      expect(rows[1].classes()).toContain("is-drop-target");
      expect(rows[0].classes()).not.toContain("is-drop-target");

      await vi.advanceTimersByTimeAsync(1_000);
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
      await vi.advanceTimersByTimeAsync(900);
      dispatchDrag(rowEl.element, "dragleave", { relatedTarget: document.body });
      await nextTick();
      expect(rowEl.classes()).not.toContain("is-drop-target");

      await vi.advanceTimersByTimeAsync(1_000);
      expect(wrapper.emitted("toggle")).toBeUndefined();
    } finally {
      wrapper.unmount();
    }
  });

  it("expands the next folder after 1s when the drag moves on", async () => {
    vi.useFakeTimers();
    const folderA = makeNode("/a", "directory");
    const folderB = makeNode("/b", "directory");
    const wrapper = mountPane([row(folderA), row(folderB)], { externalDragActive: true });
    const rowEls = wrapper.findAll(".sftp-tree-row");
    const rowA = rowEls[0].element;
    const rowB = rowEls[1].element;
    const pointSpy = vi.spyOn(document, "elementFromPoint").mockReturnValue(rowA);

    try {
      const api = wrapper.vm as unknown as { markDropTargetAt(point: { x: number; y: number }): string | undefined };
      api.markDropTargetAt({ x: 20, y: 10 });
      await vi.advanceTimersByTimeAsync(999);
      expect(wrapper.emitted("toggle")).toBeUndefined();
      await vi.advanceTimersByTimeAsync(1);
      expect(wrapper.emitted("toggle")?.[0]).toEqual([folderA]);

      // 不放手，移动到另一个折叠目录：重新计时并在 1s 后展开它。
      folderA.expanded = true;
      pointSpy.mockReturnValue(rowB);
      api.markDropTargetAt({ x: 20, y: 50 });
      await vi.advanceTimersByTimeAsync(999);
      expect(wrapper.emitted("toggle")).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(1);
      expect(wrapper.emitted("toggle")?.[1]).toEqual([folderB]);
      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual(["/b"]);
    } finally {
      pointSpy.mockRestore();
      wrapper.unmount();
    }
  });

  it("falls back to row geometry when elementFromPoint is unavailable", async () => {
    // WKWebView 原生拖拽期 elementFromPoint 可能返回 null（或拖拽遮罩）：
    // 宿主坐标仍然有效，按渲染行的 getBoundingClientRect 命中。
    vi.useFakeTimers();
    const folderA = makeNode("/a", "directory");
    const folderB = makeNode("/b", "directory");
    const wrapper = mountPane([row(folderA), row(folderB)], { externalDragActive: true });
    const rowEls = wrapper.findAll(".sftp-tree-row");
    const rowA = rowEls[0].element;
    const rowB = rowEls[1].element;
    const pointSpy = vi.spyOn(document, "elementFromPoint").mockReturnValue(null);
    const rect = (top: number) => ({
      left: 0,
      top,
      right: 200,
      bottom: top + 28,
      width: 200,
      height: 28,
      x: 0,
      y: top,
      toJSON: () => ({}),
    }) as DOMRect;
    const rectASpy = vi.spyOn(rowA, "getBoundingClientRect").mockReturnValue(rect(0));
    const rectBSpy = vi.spyOn(rowB, "getBoundingClientRect").mockReturnValue(rect(28));

    try {
      const api = wrapper.vm as unknown as { markDropTargetAt(point: { x: number; y: number }): string | undefined };
      expect(api.markDropTargetAt({ x: 30, y: 40 })).toBe("/b");
      await nextTick();
      expect(rowEls[1].classes()).toContain("is-drop-target");
      expect(rowEls[0].classes()).not.toContain("is-drop-target");

      expect(api.markDropTargetAt({ x: 30, y: 10 })).toBe("/a");
      await nextTick();
      expect(rowEls[0].classes()).toContain("is-drop-target");
    } finally {
      rectBSpy.mockRestore();
      rectASpy.mockRestore();
      pointSpy.mockRestore();
      wrapper.unmount();
    }
  });

  // DBX 0.6.x 宿主：宿主点 = (真实点 + iframe 偏移) / devicePixelRatio − 偏移。
  // 直接使用会让落点缩到文件树左侧；sidecar 探针给出真实光标后可反解偏移。
  it("recovers the iframe offset from the real cursor and follows folder rows", async () => {
    vi.useFakeTimers();
    const originalDpr = window.devicePixelRatio;
    Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
    const folderA = makeNode("/a", "directory");
    const folderB = makeNode("/b", "directory");
    const folderC = makeNode("/c", "directory");
    const offset = { x: 240, y: 40 };
    let truePoint = { x: 20, y: 40 };
    const hostPoint = (point: { x: number; y: number }) => ({
      x: (point.x + offset.x) / 2 - offset.x,
      y: (point.y + offset.y) / 2 - offset.y,
    });
    const sampler = vi.fn(async (): Promise<HostPointerSample> => ({
      platform: "macos",
      cursor: { x: offset.x + truePoint.x, y: offset.y + truePoint.y },
      window: { x: 0, y: 0, width: 1000, height: 800 },
    }));
    const wrapper = mountPane([row(folderA), row(folderB), row(folderC)], { externalDragActive: true, pointerSampler: sampler });
    const rowEls = wrapper.findAll(".sftp-tree-row");
    const pointSpy = vi.spyOn(document, "elementFromPoint").mockImplementation((x: number, y: number) => {
      if (x < 0) return null;
      if (y < 28) return rowEls[0].element;
      return y < 56 ? rowEls[1].element : rowEls[2].element;
    });

    try {
      const api = wrapper.vm as unknown as { markDropTargetAt(point: { x: number; y: number }): string | undefined };
      // 宿主点同比缩小并左移，直接命中会落到树外（x < 0）。
      expect(api.markDropTargetAt(hostPoint(truePoint))).toBe("");
      await flushMicrotasks();
      expect(sampler).toHaveBeenCalledTimes(1);
      await nextTick();
      // 反解出偏移后，真实光标所在的 B 立即成为目标。
      expect(rowEls[1].classes()).toContain("is-drop-target");

      // 移动到 C：同样被缩放的宿主点换算后仍跟随真实光标。
      truePoint = { x: 20, y: 70 };
      expect(api.markDropTargetAt(hostPoint(truePoint))).toBe("/c");
      await nextTick();
      expect(rowEls[2].classes()).toContain("is-drop-target");
      expect(rowEls[1].classes()).not.toContain("is-drop-target");

      // 悬停 1s 展开当前真实目标。
      await vi.advanceTimersByTimeAsync(1_000);
      expect(wrapper.emitted("toggle")?.[0]).toEqual([folderC]);
    } finally {
      pointSpy.mockRestore();
      wrapper.unmount();
      Object.defineProperty(window, "devicePixelRatio", { value: originalDpr, configurable: true });
    }
  });

  it("clears the folder target when the calibrated pointer lands on a file row", async () => {
    vi.useFakeTimers();
    const originalDpr = window.devicePixelRatio;
    Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
    const folderA = makeNode("/a", "directory");
    const plainFile = makeNode("/f", "file");
    const folderC = makeNode("/c", "directory");
    const offset = { x: 240, y: 40 };
    let truePoint = { x: 20, y: 10 };
    const hostPoint = (point: { x: number; y: number }) => ({
      x: (point.x + offset.x) / 2 - offset.x,
      y: (point.y + offset.y) / 2 - offset.y,
    });
    const sampler = vi.fn(async (): Promise<HostPointerSample> => ({
      platform: "macos",
      cursor: { x: offset.x + truePoint.x, y: offset.y + truePoint.y },
      window: { x: 0, y: 0, width: 1000, height: 800 },
    }));
    const wrapper = mountPane([row(folderA), row(plainFile), row(folderC)], { externalDragActive: true, pointerSampler: sampler });
    const rowEls = wrapper.findAll(".sftp-tree-row");
    const pointSpy = vi.spyOn(document, "elementFromPoint").mockImplementation((x: number, y: number) => {
      if (x < 0) return null;
      if (y < 28) return rowEls[0].element;
      return y < 56 ? rowEls[1].element : rowEls[2].element;
    });

    try {
      const api = wrapper.vm as unknown as { markDropTargetAt(point: { x: number; y: number }): string | undefined };
      expect(api.markDropTargetAt(hostPoint(truePoint))).toBe("");
      await flushMicrotasks();
      await nextTick();
      expect(rowEls[0].classes()).toContain("is-drop-target");

      // 真实光标落在文件行：已校准的坐标不再“猜”相邻目录，目标必须清空。
      truePoint = { x: 20, y: 40 };
      expect(api.markDropTargetAt(hostPoint(truePoint))).toBe("");
      await nextTick();
      expect(rowEls[0].classes()).not.toContain("is-drop-target");
      expect(rowEls[1].classes()).not.toContain("is-drop-target");
      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual([""]);

      // 再移到目录：目标恢复。
      truePoint = { x: 20, y: 70 };
      expect(api.markDropTargetAt(hostPoint(truePoint))).toBe("/c");
      await nextTick();
      expect(rowEls[2].classes()).toContain("is-drop-target");
    } finally {
      pointSpy.mockRestore();
      wrapper.unmount();
      Object.defineProperty(window, "devicePixelRatio", { value: originalDpr, configurable: true });
    }
  });

  it("keeps raw host coordinates when the pointer probe returns nothing", async () => {
    // 旧宿主/探针被禁用（非 macOS、sidecar 忙碌）：不校准，仍按宿主点命中。
    vi.useFakeTimers();
    const folderA = makeNode("/a", "directory");
    const folderB = makeNode("/b", "directory");
    const sampler = vi.fn(async () => null);
    const wrapper = mountPane([row(folderA), row(folderB)], { externalDragActive: true, pointerSampler: sampler });
    const rowEls = wrapper.findAll(".sftp-tree-row");
    const pointSpy = vi.spyOn(document, "elementFromPoint").mockImplementation((_x: number, y: number) => (y < 28 ? rowEls[0].element : rowEls[1].element));

    try {
      const api = wrapper.vm as unknown as { markDropTargetAt(point: { x: number; y: number }): string | undefined };
      expect(api.markDropTargetAt({ x: 30, y: 40 })).toBe("/b");
      await flushMicrotasks();
      await nextTick();
      expect(rowEls[1].classes()).toContain("is-drop-target");
      expect(rowEls[0].classes()).not.toContain("is-drop-target");
    } finally {
      pointSpy.mockRestore();
      wrapper.unmount();
    }
  });

  it("uses forwarded host drag positions even when :hover is frozen", async () => {
    // DBX 宿主现在把原生拖拽坐标转发进 iframe：坐标命中优先于 DOM 事件与
    // :hover 兜底，A 展开后移动到 B 必须立刻换目标。
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
    const pointSpy = vi.spyOn(document, "elementFromPoint").mockReturnValue(rowB);

    try {
      const api = wrapper.vm as unknown as { markDropTargetAt(point: { x: number; y: number }): string | undefined };
      expect(api.markDropTargetAt({ x: 20, y: 40 })).toBe("/b");
      await nextTick();
      expect(rowEls[1].classes()).toContain("is-drop-target");
      expect(rowEls[0].classes()).not.toContain("is-drop-target");

      // 冻结的 :hover 仍报 A：宿主机坐标已接管，目标必须保持 B。
      await vi.advanceTimersByTimeAsync(300);
      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual(["/b"]);
      expect(rowEls[1].classes()).toContain("is-drop-target");
      expect(rowEls[0].classes()).not.toContain("is-drop-target");

      pointSpy.mockReturnValue(rowA);
      expect(api.markDropTargetAt({ x: 20, y: 10 })).toBe("/a");
      await nextTick();
      expect(rowEls[0].classes()).toContain("is-drop-target");
      expect(rowEls[1].classes()).not.toContain("is-drop-target");
    } finally {
      pointSpy.mockRestore();
      querySpy.mockRestore();
      wrapper.unmount();
    }
  });

  it("keeps the host target when dragActive arrives after the first position", async () => {
    // App 先收到宿主坐标、再让 externalDragActive prop 变 true：后到的
    // active watch 不能重启 :hover/pointer 兜底覆盖宿主目标。
    vi.useFakeTimers();
    const folderA = makeNode("/a", "directory");
    const folderB = makeNode("/b", "directory");
    const wrapper = mountPane([row(folderA), row(folderB)], { externalDragActive: false });
    const rowEls = wrapper.findAll(".sftp-tree-row");
    const rowA = rowEls[0].element;
    const rowB = rowEls[1].element;
    const originalQuerySelector = document.querySelector.bind(document);
    const querySpy = vi.spyOn(document, "querySelector").mockImplementation((selector: string) => {
      if (selector === ".sftp-tree-row:hover") return rowA;
      return originalQuerySelector(selector);
    });
    const pointSpy = vi.spyOn(document, "elementFromPoint").mockReturnValue(rowB);

    try {
      const api = wrapper.vm as unknown as { markDropTargetAt(point: { x: number; y: number }): string | undefined };
      const activate = wrapper.setProps({ externalDragActive: true });
      expect(api.markDropTargetAt({ x: 20, y: 40 })).toBe("/b");
      await activate;
      await nextTick();
      await vi.advanceTimersByTimeAsync(300);

      expect(rowEls[1].classes()).toContain("is-drop-target");
      expect(rowEls[0].classes()).not.toContain("is-drop-target");
      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual(["/b"]);
    } finally {
      pointSpy.mockRestore();
      querySpy.mockRestore();
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
      await vi.advanceTimersByTimeAsync(1_000);
      expect(wrapper.emitted("toggle")).toHaveLength(1);
      expect(wrapper.emitted("toggle")?.[0]).toEqual([folderA]);

      // 父级展开 A 后继续拖到 B：B 的行级 dragover 更新目标。
      folderA.expanded = true;
      dispatchDrag(rowA, "dragleave", { relatedTarget: rowB });
      dispatchDrag(rowB, "dragover", { types: [] });
      await nextTick();
      expect(rowEls[1].classes()).toContain("is-drop-target");
      expect(wrapper.emitted("drop-target-change")?.at(-1)).toEqual(["/b"]);

      // 冻结的 :hover 轮询继续跑：目标必须保持 B，B 的 1s 展开计时不能被清掉。
      await vi.advanceTimersByTimeAsync(500);
      expect(wrapper.emitted("toggle")).toHaveLength(1);
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
      await vi.advanceTimersByTimeAsync(1_000);
      expect(wrapper.emitted("toggle")?.[0]).toEqual([folderA]);

      folderA.expanded = true;
      hoverRow = rowB;
      await vi.advanceTimersByTimeAsync(1_100);

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

      await vi.advanceTimersByTimeAsync(1_000);
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
    await vi.advanceTimersByTimeAsync(1_000);

    expect(wrapper.emitted("toggle")).toBeUndefined();
  });
});
