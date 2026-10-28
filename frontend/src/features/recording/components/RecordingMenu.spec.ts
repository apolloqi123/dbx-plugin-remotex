// @vitest-environment happy-dom
// 统一录制入口：点击图标只展开菜单，不直接开始录制；菜单内再选择开始/停止或录制记录。
import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import RecordingMenu from "./RecordingMenu.vue";

const t = (key: string) => key;

const stubs = {
  Popover: { template: "<div><slot /></div>" },
  PopoverAnchor: { template: "<span><slot /></span>" },
  PopoverContent: { template: "<div><slot /></div>" },
};

function mountMenu(overrides: Partial<{ canRecord: boolean; recordingActive: boolean; recordCountdown: number | null }> = {}) {
  const actions = {
    toggleRecording: vi.fn(),
    toggleRecordings: vi.fn(),
    cancelRecordCountdown: vi.fn(),
  };
  const wrapper = mount(RecordingMenu, {
    props: {
      t,
      canRecord: overrides.canRecord ?? true,
      recordingActive: overrides.recordingActive ?? false,
      recordCountdown: overrides.recordCountdown ?? null,
      recordingElapsedSec: 0,
      recordingsCount: 1,
      recordingsOpen: false,
      actions,
    },
    global: { stubs },
  });
  return { wrapper, actions };
}

function menuItem(wrapper: ReturnType<typeof mountMenu>["wrapper"], text: string) {
  return wrapper.findAll(".recording-menu-item").find((item) => item.text().includes(text));
}

describe("RecordingMenu", () => {
  it("clicking the toolbar icon only opens the menu and never starts recording", async () => {
    const { wrapper, actions } = mountMenu();
    await wrapper.find(".recording-trigger").trigger("click");
    expect(actions.toggleRecording).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("recordingMenuStart");
    expect(wrapper.text()).toContain("recordingsTitle");
  });

  it("starts/stops recording and opens the recordings list from menu items", async () => {
    const { wrapper, actions } = mountMenu();
    await wrapper.find(".recording-trigger").trigger("click");
    await menuItem(wrapper, "recordingMenuStart")!.trigger("click");
    expect(actions.toggleRecording).toHaveBeenCalledTimes(1);

    await menuItem(wrapper, "recordingsTitle")!.trigger("click");
    expect(actions.toggleRecordings).toHaveBeenCalledTimes(1);
  });

  it("offers cancel while the countdown is running", async () => {
    const { wrapper, actions } = mountMenu({ recordCountdown: 3 });
    await wrapper.find(".recording-trigger").trigger("click");
    expect(wrapper.find(".recording-menu-item").text()).toContain("recordingMenuCancel");
    await wrapper.find(".recording-menu-item").trigger("click");
    expect(actions.cancelRecordCountdown).toHaveBeenCalledTimes(1);
    expect(actions.toggleRecording).not.toHaveBeenCalled();
  });

  it("disables starting when there is no recordable session", async () => {
    const { wrapper } = mountMenu({ canRecord: false });
    await wrapper.find(".recording-trigger").trigger("click");
    expect(menuItem(wrapper, "recordingMenuStart")!.attributes("disabled")).toBeDefined();
    expect(wrapper.text()).toContain("recordingMenuNoTarget");
  });
});
