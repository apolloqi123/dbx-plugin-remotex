// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import TransferRow from "./TransferRow.vue";
import type { TransferRowView } from "../lib/transferPanelRows";

const t = (key: string) => key;

function row(overrides: Partial<TransferRowView> = {}): TransferRowView {
  return {
    key: "live:1",
    taskId: "1",
    source: "live",
    fileName: "report-2026.pdf",
    direction: "upload",
    status: "running",
    percent: 50,
    sizeLabel: "8.0 MiB",
    detailLabel: "4.0 MiB / 8.0 MiB",
    speedLabel: "1.2 MiB/s",
    timeLabel: "09-30 09:52",
    localPath: "/Users/demo/report-2026.pdf",
    isFolder: false,
    paused: false,
    canPause: true,
    canCancel: true,
    canResume: false,
    canReveal: true,
    canOpenFile: true,
    ...overrides,
  };
}

describe("TransferRow", () => {
  it("renders icon, name, badges, status and meta", () => {
    const wrapper = mount(TransferRow, { props: { row: row(), t, expanded: false } });
    expect(wrapper.text()).toContain("report-2026.pdf");
    expect(wrapper.text()).toContain("transferDirection.upload");
    expect(wrapper.text()).toContain("SFTP");
    expect(wrapper.text()).toContain("transferStatus.running");
    expect(wrapper.text()).toContain("4.0 MiB / 8.0 MiB");
    expect(wrapper.text()).toContain("09-30 09:52");
    expect(wrapper.find(".transfer-row").classes()).toContain("is-running");
    expect(wrapper.find(".transfer-row-line").attributes("style")).toContain("50%");
    // PDF maps to the document tile.
    expect(wrapper.find(".transfer-file-icon").attributes("data-tone")).toBe("blue");
  });

  it("emits row intents from the toggle and action buttons", async () => {
    const wrapper = mount(TransferRow, { props: { row: row(), t, expanded: false } });
    await wrapper.find(".transfer-row-toggle").trigger("click");
    expect(wrapper.emitted("toggle")).toHaveLength(1);

    for (const [title, event] of [
      ["openDownloadedFile", "open-file"],
      ["revealInFolder", "reveal"],
      ["transferPause", "pause"],
      ["cancel", "cancel"],
    ] as const) {
      const button = wrapper.findAll("button").find((candidate) => candidate.attributes("title") === title);
      expect(button, title).toBeTruthy();
      await button!.trigger("click");
      expect(wrapper.emitted(event), event).toHaveLength(1);
    }
  });

  it("shows paused/resumable actions and hides absent ones", async () => {
    const paused = row({ paused: true });
    const wrapper = mount(TransferRow, { props: { row: paused, t, expanded: false } });
    const titles = wrapper.findAll("button").map((button) => button.attributes("title"));
    expect(titles).toContain("transferResume");
    expect(titles).not.toContain("transferPause");
    await wrapper.findAll("button").find((button) => button.attributes("title") === "transferResume")!.trigger("click");
    expect(wrapper.emitted("pause")).toHaveLength(1);

    const resumable = mount(TransferRow, {
      props: {
        row: row({ status: "resumable", canResume: true, canPause: false, canCancel: false, canReveal: false, canOpenFile: false }),
        t,
        expanded: false,
      },
    });
    const resumeButton = resumable.findAll("button").find((button) => button.attributes("title") === "resumableResume");
    expect(resumeButton).toBeTruthy();
    await resumeButton!.trigger("click");
    expect(resumable.emitted("resume")).toHaveLength(1);
  });

  it("expands into a detail card with the local path and error", () => {
    const wrapper = mount(TransferRow, {
      props: { row: row({ status: "failed", error: "disk quota exceeded" }), t, expanded: true },
    });
    expect(wrapper.find(".transfer-detail").exists()).toBe(true);
    expect(wrapper.text()).toContain("transferLocalPath");
    expect(wrapper.text()).toContain("/Users/demo/report-2026.pdf");
    expect(wrapper.text()).toContain("disk quota exceeded");
    expect(wrapper.find(".transfer-detail-state").text()).toContain("transferStatus.failed");
  });

  it("uses the folder tile for directory transfers and hides completed status text", () => {
    const wrapper = mount(TransferRow, {
      props: {
        row: row({ fileName: "Nessus", isFolder: true, status: "completed", canPause: false, canCancel: false, percent: 100 }),
        t,
        expanded: false,
      },
    });
    expect(wrapper.find(".transfer-file-icon").attributes("data-tone")).toBe("amber");
    expect(wrapper.text()).not.toContain("transferStatus.completed");
    expect(wrapper.find(".transfer-row-line").classes()).toContain("is-completed");
  });
});
