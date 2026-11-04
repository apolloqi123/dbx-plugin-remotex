import { describe, expect, it } from "vitest";
import {
  cpuSharePercent,
  cpuUsage,
  formatCores,
  MONITOR_CAPACITY,
  niceRateMax,
  rangePoints,
  sampleFromPayload,
  pushStatsSample,
  statsPoints,
  type ContainerStatsSample,
} from "./containerMonitor";

function payload(patch: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    available: true,
    running: true,
    cpuPercent: 3.47,
    memPercent: 1.25,
    memUsedBytes: 157_286_400,
    memLimitBytes: 8_160_437_862,
    netRxBytes: 1_200,
    netTxBytes: 3_400,
    blockReadBytes: 12_300_000,
    blockWriteBytes: 4_560_000,
    pids: 5,
    ...patch,
  };
}

function sample(at: number, patch: Partial<ContainerStatsSample> = {}): ContainerStatsSample {
  return {
    at,
    cpuPercent: 1,
    memPercent: 1,
    memUsedBytes: 1_000,
    memLimitBytes: 4_000,
    netRxBytes: 1_000,
    netTxBytes: 2_000,
    blockReadBytes: 0,
    blockWriteBytes: 0,
    pids: 3,
    ...patch,
  };
}

describe("sampleFromPayload", () => {
  it("normalizes a running snapshot", () => {
    expect(sampleFromPayload(payload(), 1_000)).toEqual({
      at: 1_000,
      cpuPercent: 3.47,
      memPercent: 1.25,
      memUsedBytes: 157_286_400,
      memLimitBytes: 8_160_437_862,
      netRxBytes: 1_200,
      netTxBytes: 3_400,
      blockReadBytes: 12_300_000,
      blockWriteBytes: 4_560_000,
      pids: 5,
    });
  });

  it("rejects stopped, denied and malformed payloads", () => {
    expect(sampleFromPayload(payload({ running: false }), 1)).toBeNull();
    expect(sampleFromPayload(payload({ available: false, needsSudo: true }), 1)).toBeNull();
    expect(sampleFromPayload(null, 1)).toBeNull();
    expect(sampleFromPayload("nope", 1)).toBeNull();
  });

  it("clamps missing and negative numbers to zero", () => {
    const parsed = sampleFromPayload(payload({ cpuPercent: -1, memUsedBytes: "x" }), 1);
    expect(parsed?.cpuPercent).toBe(0);
    expect(parsed?.memUsedBytes).toBe(0);
  });
});

describe("pushStatsSample", () => {
  it("keeps the newest samples up to capacity", () => {
    let ring: ContainerStatsSample[] = [];
    for (let index = 0; index < 5; index += 1) ring = pushStatsSample(ring, sample(index), 3);
    expect(ring.map((entry) => entry.at)).toEqual([2, 3, 4]);
    expect(pushStatsSample(ring, sample(9), 0)).toEqual([]);
  });

  it("uses the one-hour capacity by default", () => {
    expect(MONITOR_CAPACITY).toBe(1_200);
  });
});

describe("statsPoints", () => {
  it("turns cumulative counters into per-second rates", () => {
    const points = statsPoints([
      sample(0, { netRxBytes: 1_000, netTxBytes: 2_000 }),
      sample(3_000, { netRxBytes: 4_000, netTxBytes: 2_600 }),
    ]);
    expect(points[0].rxRate).toBe(0);
    expect(points[0].txRate).toBe(0);
    expect(points[1].rxRate).toBe(1_000);
    expect(points[1].txRate).toBe(200);
  });

  it("clamps counter resets and clock regressions to zero", () => {
    const points = statsPoints([
      sample(3_000, { netRxBytes: 10_000 }),
      sample(6_000, { netRxBytes: 500 }),
      sample(6_000, { netRxBytes: 900 }),
    ]);
    expect(points[1].rxRate).toBe(0);
    expect(points[2].rxRate).toBe(0);
  });
});

describe("rangePoints", () => {
  it("filters by the selected window", () => {
    const points = statsPoints([
      sample(0),
      sample(60_000),
      sample(120_000),
      sample(180_000),
    ]);
    expect(rangePoints(points, 60, 180_000).map((point) => point.at)).toEqual([120_000, 180_000]);
    expect(rangePoints(points, 300, 180_000)).toHaveLength(4);
    expect(rangePoints(points, 60, 600_000)).toEqual([]);
  });
});

describe("formatCores", () => {
  it("drops trailing zeros and keeps fractional limits", () => {
    expect(formatCores(16)).toBe("16");
    expect(formatCores(2)).toBe("2");
    expect(formatCores(2.5)).toBe("2.5");
    expect(formatCores(0.5)).toBe("0.5");
    expect(formatCores(1.25)).toBe("1.25");
    expect(formatCores(0)).toBe("0");
    expect(formatCores(Number.NaN)).toBe("0");
  });
});

describe("niceRateMax", () => {
  it("rounds a peak up to a readable binary-unit step", () => {
    expect(niceRateMax(300)).toBe(1024);
    expect(niceRateMax(1024)).toBe(1024);
    expect(niceRateMax(1500)).toBe(1536);
    expect(niceRateMax(2_500)).toBe(2560);
    expect(niceRateMax(1_700_000)).toBe(2 * 1024 ** 2);
    expect(niceRateMax(8.5 * 1024 ** 2)).toBe(10 * 1024 ** 2);
    // 800 KiB 之后要跳到下一个二进制单位，而不是 1024 KiB 的假档位。
    expect(niceRateMax(800 * 1024 + 1)).toBe(1024 ** 2);
    expect(niceRateMax(1024 ** 2)).toBe(1024 ** 2);
    expect(niceRateMax(1024 ** 3)).toBe(1024 ** 3);
  });

  it("keeps a 1 KiB/s floor for empty or invalid peaks", () => {
    expect(niceRateMax(0)).toBe(1024);
    expect(niceRateMax(Number.NaN)).toBe(1024);
    expect(niceRateMax(-10)).toBe(1024);
  });
});

describe("cpuSharePercent", () => {
  it("converts Docker's per-core percentage into a share of the core budget", () => {
    expect(cpuSharePercent(102.5, 7)).toBeCloseTo(14.64, 2);
    expect(cpuSharePercent(157.2, 2)).toBeCloseTo(78.6, 2);
    expect(cpuSharePercent(12.5, 16)).toBeCloseTo(0.78, 2);
    expect(cpuSharePercent(700, 7)).toBe(100);
  });

  it("keeps the raw Docker percentage when the total is unknown", () => {
    expect(cpuSharePercent(102.5, undefined)).toBe(102.5);
    expect(cpuSharePercent(102.5, 0)).toBe(102.5);
    expect(cpuSharePercent(102.5, Number.NaN)).toBe(102.5);
  });

  it("clamps invalid percentages to zero", () => {
    expect(cpuSharePercent(-5, 7)).toBe(0);
    expect(cpuSharePercent(Number.NaN, 7)).toBe(0);
  });
});

describe("cpuUsage", () => {
  it("uses millicores below one core and cores above", () => {
    expect(cpuUsage(12.5)).toEqual({ value: 125, millicores: true });
    expect(cpuUsage(99.9)).toEqual({ value: 999, millicores: true });
    expect(cpuUsage(100)).toEqual({ value: 1, millicores: false });
    expect(cpuUsage(157.2)).toEqual({ value: 1.57, millicores: false });
  });

  it("clamps invalid or negative percentages", () => {
    expect(cpuUsage(-5)).toEqual({ value: 0, millicores: true });
    expect(cpuUsage(Number.NaN)).toEqual({ value: 0, millicores: true });
  });
});
