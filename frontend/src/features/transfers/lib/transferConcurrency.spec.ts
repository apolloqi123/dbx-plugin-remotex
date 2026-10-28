import { describe, expect, it } from "vitest";
import { downloadBatchLimit, runLimited } from "./transferConcurrency";

describe("runLimited", () => {
  it("keeps at most `limit` items in flight and runs everything once", async () => {
    let active = 0;
    let peak = 0;
    const started: number[] = [];
    await runLimited([1, 2, 3, 4, 5], 2, async (item) => {
      started.push(item);
      active += 1;
      peak = Math.max(peak, active);
      await new Promise((resolve) => setTimeout(resolve, 1));
      active -= 1;
    });
    expect(started.sort((a, b) => a - b)).toEqual([1, 2, 3, 4, 5]);
    expect(peak).toBe(2);
  });

  it("clamps the width to the item count and tolerates non-positive limits", async () => {
    const seen: string[] = [];
    await runLimited(["a", "b"], 0, async (item) => { seen.push(item); });
    expect(seen).toEqual(["a", "b"]);
    await runLimited([], 4, async () => { throw new Error("must not run"); });
  });

  it("propagates a runner failure after stopping new work", async () => {
    const seen: number[] = [];
    await expect(
      runLimited([1, 2, 3, 4], 1, async (item) => {
        seen.push(item);
        if (item === 2) throw new Error("boom");
      }),
    ).rejects.toThrow("boom");
    expect(seen).toEqual([1, 2]);
  });

  it("stops sibling workers from pulling new items after a failure", async () => {
    const seen: number[] = [];
    await expect(
      runLimited([1, 2, 3, 4], 2, async (item) => {
        seen.push(item);
        if (item === 1) await Promise.resolve();
        if (item === 2) throw new Error("boom");
      }),
    ).rejects.toThrow("boom");
    expect(seen.sort((a, b) => a - b)).toEqual([1, 2]);
  });
});

describe("downloadBatchLimit", () => {
  const base = { compat: false, askDirectory: false, concurrency: 4, maxActive: 4, active: 0 };

  it("uses the batch concurrency when the session is idle", () => {
    expect(downloadBatchLimit(base)).toBe(4);
  });

  it("serializes compat mode and the per-file save dialog", () => {
    expect(downloadBatchLimit({ ...base, compat: true })).toBe(1);
    expect(downloadBatchLimit({ ...base, askDirectory: true })).toBe(1);
  });

  it("leaves room for transfers that already occupy the session", () => {
    expect(downloadBatchLimit({ ...base, active: 2 })).toBe(2);
    expect(downloadBatchLimit({ ...base, active: 4 })).toBe(1);
    expect(downloadBatchLimit({ ...base, active: 9 })).toBe(1);
  });
});
