import { describe, expect, it } from "vitest";
import {
  buildTransferRows,
  countTransferFilters,
  filterTransferRows,
  formatTransferStamp,
  type BuildTransferRowsInput,
} from "./transferPanelRows";
import type { TransferTask } from "./transferQueue";

const formatBytes = (value = 0) => `${value} B`;

const liveTask = (overrides: Partial<TransferTask> = {}): TransferTask => ({
  taskId: "live-1",
  direction: "upload",
  fileName: "report.pdf",
  size: 1000,
  transferred: 500,
  status: "running",
  joinedAt: new Date(2026, 8, 30, 9, 52).getTime(),
  ...overrides,
});

function input(overrides: Partial<BuildTransferRowsInput> = {}): BuildTransferRowsInput {
  return {
    transferList: [],
    history: [],
    resumableTasks: [],
    pausedTaskIds: new Set<string>(),
    speeds: {},
    formatBytes,
    transferPercent: (task) => (task.size > 0 ? (task.transferred / task.size) * 100 : 0),
    transferShownBytes: (task) => task.transferred,
    transferPausable: (status) => status === "running" || status === "queued",
    ...overrides,
  };
}

describe("transfer panel rows", () => {
  it("formats compact timestamps and rejects missing values", () => {
    expect(formatTransferStamp(new Date(2026, 8, 30, 9, 52).getTime())).toBe("09-30 09:52");
    expect(formatTransferStamp(undefined)).toBe("");
    expect(formatTransferStamp(Number.NaN)).toBe("");
  });

  it("merges live, history and resumable rows, deduping live ids", () => {
    const rows = buildTransferRows(
      input({
        transferList: [liveTask(), liveTask({ taskId: "live-2", direction: "download", fileName: "server.log", status: "queued", transferred: 0 })],
        history: [
          // Same task id as the live row: the live row wins.
          { taskId: "live-1", direction: "upload", fileName: "report.pdf", size: 1000, transferred: 1000, status: "completed" },
          { taskId: "hist-old", direction: "download", fileName: "old.bin", size: 200, transferred: 200, status: "completed", finishedAt: 1000 },
          { taskId: "hist-new", direction: "download", fileName: "new.bin", size: 300, transferred: 300, status: "completed", finishedAt: 3000 },
        ],
        resumableTasks: [{ taskId: "resume-1", remotePath: "/tmp/big.iso", fileName: "big.iso", size: 400, resumableBytes: 100 }],
      }),
    );

    expect(rows.map((row) => row.key)).toEqual([
      "live:live-1",
      "live:live-2",
      "history:hist-new",
      "history:hist-old",
      "resumable:resume-1",
    ]);
    expect(rows[0].percent).toBe(50);
    expect(rows[0].paused).toBe(false);
    expect(rows[1].status).toBe("queued");
    expect(rows[2].timeLabel).toBe(formatTransferStamp(3000));
    expect(rows[4].status).toBe("resumable");
    expect(rows[4].percent).toBe(25);
    expect(rows[4].canResume).toBe(true);
    expect(rows[4].direction).toBe("upload");
  });

  it("orders running tasks before queued regardless of ledger order", () => {
    const rows = buildTransferRows(
      input({
        transferList: [
          liveTask({ taskId: "queued", status: "queued", joinedAt: 2000 }),
          liveTask({ taskId: "running", status: "running", joinedAt: 1000 }),
        ],
      }),
    );
    expect(rows.map((row) => row.taskId)).toEqual(["running", "queued"]);
  });

  it("marks paused rows and action affordances", () => {
    const rows = buildTransferRows(
      input({
        transferList: [liveTask()],
        pausedTaskIds: new Set(["live-1"]),
        speeds: { "live-1": 2048 },
      }),
    );
    expect(rows[0].paused).toBe(true);
    expect(rows[0].canPause).toBe(true);
    expect(rows[0].canCancel).toBe(true);
    expect(rows[0].speedLabel).toBe("2048 B/s");
    expect(rows[0].canReveal).toBe(false);
  });

  it("counts and filters by direction, pause and completion", () => {
    const rows = buildTransferRows(
      input({
        transferList: [
          liveTask(),
          liveTask({ taskId: "live-2", direction: "download", fileName: "dl.bin", status: "running" }),
        ],
        pausedTaskIds: new Set(["live-2"]),
        history: [{ taskId: "hist-1", direction: "upload", fileName: "done.zip", size: 10, transferred: 10, status: "completed" }],
        resumableTasks: [{ taskId: "resume-1", remotePath: "/tmp/big.iso", fileName: "big.iso", size: 100, resumableBytes: 10 }],
      }),
    );

    expect(countTransferFilters(rows)).toEqual({ all: 4, upload: 3, download: 1, paused: 1, completed: 1 });
    expect(filterTransferRows(rows, "upload").map((row) => row.taskId)).toEqual(["live-1", "hist-1", "resume-1"]);
    expect(filterTransferRows(rows, "download").map((row) => row.taskId)).toEqual(["live-2"]);
    expect(filterTransferRows(rows, "paused").map((row) => row.taskId)).toEqual(["live-2"]);
    expect(filterTransferRows(rows, "completed").map((row) => row.taskId)).toEqual(["hist-1"]);
    expect(filterTransferRows(rows, "all")).toHaveLength(4);
  });
});
