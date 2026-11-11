// @vitest-environment happy-dom
// 选文件兜底路径（WKWebView/旧内核不派发 input 的 cancel）回归：focus 监听与
// 内层 800ms 定时器必须在结算/重开/卸载时成对摘除，且只能取消唤起它的那次
// 请求——否则残留监听会把之后某次新上传误判为取消、静默丢弃用户选的文件。
import { afterEach, describe, expect, it, vi } from "vitest";
import { defineComponent, ref } from "vue";
import { mount } from "@vue/test-utils";
import { useTrzsz } from "./useTrzsz";

function mountTrzsz() {
  const trzsz = useTrzsz({
    t: (key: string) => key,
    showNotice: vi.fn(),
    showError: vi.fn(),
    session: ref({ sessionId: "s1" }),
    canWrite: ref(true),
    zmodemBusy: ref(false),
    terminalMenuOpen: ref(false),
    terminal: () => ({ cols: 80, focus: vi.fn() }),
    writeTerminalOutput: vi.fn(),
    sendTerminalBytes: vi.fn(),
    probeLocalCapabilities: async () => undefined,
    saveHostFile: async () => undefined,
    loadDownloadUseDefaultDir: () => true,
    askDownloadTarget: async () => undefined,
    loadDownloadDir: () => "/downloads",
    resolveDownloadConflictFor: async () => "rename",
    applyChosenDirAsDefault: vi.fn(),
    revealTransferTarget: async () => undefined,
  });
  const input = document.createElement("input");
  input.click = vi.fn();
  trzsz.trzszInput.value = input;
  return { trzsz, input };
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("useTrzsz file picker fallback lifecycle", () => {
  it("does not cancel a newer pick when a stale focus fallback fires", async () => {
    vi.useFakeTimers();
    let api!: ReturnType<typeof mountTrzsz>;
    const wrapper = mount(defineComponent({
      setup() {
        api = mountTrzsz();
        return () => null;
      },
    }));

    const first = api.trzsz.pickTrzszUploadFiles(false);
    // 旧内核路径：窗口回焦先触发兜底监听（排下 800ms 取消定时器），change 迟迟未到。
    window.dispatchEvent(new Event("focus"));
    // 用户随即重新发起一次上传：上一次被按取消结算，新的 resolver 接棒。
    const second = api.trzsz.pickTrzszUploadFiles(false);
    await expect(first).resolves.toBeUndefined();

    // 旧代码此刻会执行残留定时器并调用全局 onTrzszPickCancel()，把第二次误取消。
    await vi.advanceTimersByTimeAsync(1000);

    const settled = await Promise.race([second.then(() => "settled"), Promise.resolve("pending")]);
    expect(settled).toBe("pending");

    api.trzsz.onTrzszPickCancel();
    await expect(second).resolves.toBeUndefined();
    wrapper.unmount();
  });

  it("removes the focus listener and inner timer on unmount", async () => {
    vi.useFakeTimers();
    const removeSpy = vi.spyOn(window, "removeEventListener");
    let api!: ReturnType<typeof mountTrzsz>;
    const wrapper = mount(defineComponent({
      setup() {
        api = mountTrzsz();
        return () => null;
      },
    }));

    const pending = api.trzsz.pickTrzszUploadFiles(false);
    window.dispatchEvent(new Event("focus"));
    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(1000);

    expect(removeSpy).toHaveBeenCalledWith("focus", expect.any(Function));
    await expect(pending).resolves.toBeUndefined();
  });
});
