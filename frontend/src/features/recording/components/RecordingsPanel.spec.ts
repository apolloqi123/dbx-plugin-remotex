// @vitest-environment happy-dom
// 录制面板改版：时间脊线、常驻可达动作、刷新/停止/取消倒计时与键盘交互。
import { afterEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import RecordingsPanel from "./RecordingsPanel.vue";
import type { RecordingSummary } from "../lib/replayScheduler";

const t = (key: string) => key;
const item: RecordingSummary = {
  recordingId: "rec-1",
  host: "server.demo.internal",
  startedAt: 1_700_000_000,
  durationSecs: 24,
  bytes: 4096,
};

const stubs = {
  Disc: { template: "<span />" },
  FileText: { template: "<span />" },
  FolderOpen: { template: "<span />" },
  ImagePlay: { template: "<span />" },
  Loader2: { template: "<span />" },
  Play: { template: "<span />" },
  Search: { template: "<span />" },
  Trash2: { template: "<span />" },
  X: { template: "<span />" },
};

function panel(overrides: Record<string, unknown> = {}) {
  return mount(RecordingsPanel, {
    attachTo: document.body,
    props: {
      recordings: [item],
      filteredRecordings: [item],
      loading: false,
      query: "health",
      hits: { "rec-1": ["health check passed"] },
      searchBusy: false,
      recordingActive: false,
      recordingElapsedSec: 0,
      recordCountdown: null,
      replayExporting: false,
      recordingExportingId: null,
      localCanSave: true,
      canRecord: true,
      clearAllBusy: false,
      t,
      formatRecordedAt: () => "Nov 15 14:02",
      formatDuration: (secs: number) => `${secs}s`,
      formatBytes: () => "4.0 KiB",
      ...overrides,
    },
    global: { stubs },
  });
}

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("RecordingsPanel", () => {
  it("shows Stop while recording, Cancel start during countdown, and a Refresh header action", () => {
    const active = panel({ recordingActive: true });
    expect(active.text()).toContain("recordingsStop");
    expect(active.find('[aria-label="recordingsRefresh"]').exists()).toBe(true);
    const countdown = panel({ recordCountdown: 3 });
    expect(countdown.text()).toContain("recordingMenuCancel");
  });

  it("opens replay from a focused row with Enter and closes with Esc", async () => {
    const wrapper = panel();
    const row = wrapper.find(".recording-row");
    expect(row.attributes("tabindex")).toBe("0");
    await row.trigger("keydown.enter");
    expect(wrapper.emitted("open-replay")).toBeTruthy();
    // Enter on a row action must not bubble into "open replay" as well.
    await wrapper.find('[aria-label="recordingDelete"]').trigger("keydown.enter");
    expect(wrapper.emitted("open-replay")).toHaveLength(1);
    await wrapper.trigger("keydown.esc");
    expect(wrapper.emitted("close")).toBeTruthy();
  });

  it("keeps every row action labeled and emits refresh / clear-search", async () => {
    const wrapper = panel();
    for (const button of wrapper.findAll(".recording-action")) {
      expect(button.attributes("aria-label")).toBeTruthy();
    }
    await wrapper.find('[aria-label="recordingsRefresh"]').trigger("click");
    expect(wrapper.emitted("refresh")).toBeTruthy();
    await wrapper.find(".recordings-search-clear").trigger("click");
    expect(wrapper.emitted("clear-search")).toBeTruthy();
  });
});
