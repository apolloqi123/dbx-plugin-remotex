import { describe, expect, it } from "vitest";
import {
  cursorInWindow,
  dragPointSpread,
  estimateHostScale,
  mapHostDragPoint,
  medianDragPoint,
  offsetFromSample,
  sanitizeDragScale,
} from "./hostDragMapping";

// 与 DBX 0.6.x 宿主实现保持一致：宿主点 = (真实点 + 偏移) / scale − 偏移。
function hostPointFor(point: { x: number; y: number }, offset: { x: number; y: number }, scale: number) {
  return {
    x: (point.x + offset.x) / scale - offset.x,
    y: (point.y + offset.y) / scale - offset.y,
  };
}

describe("cursorInWindow", () => {
  it("subtracts the window origin from the screen cursor", () => {
    expect(cursorInWindow({ cursor: { x: 900, y: 400 }, window: { x: 100, y: 50, width: 1400, height: 900 } })).toEqual({ x: 800, y: 350 });
  });

  it("rejects a cursor that is not over the window", () => {
    expect(cursorInWindow({ cursor: { x: 10, y: 400 }, window: { x: 100, y: 50, width: 1400, height: 900 } })).toBeNull();
    expect(cursorInWindow({ cursor: { x: 900, y: 2000 }, window: { x: 100, y: 50, width: 1400, height: 900 } })).toBeNull();
  });
});

describe("offsetFromSample", () => {
  it("recovers the iframe offset from a Retina host sample", () => {
    const offset = { x: 240, y: 40 };
    const point = { x: 500, y: 300 };
    const cursorWindow = { x: point.x + offset.x, y: point.y + offset.y };
    const host = hostPointFor(point, offset, 2);
    expect(offsetFromSample({ cursorWindow, host, scale: 2 })).toEqual(offset);
  });

  it("is a no-op offset when the host already forwards iframe CSS pixels", () => {
    expect(offsetFromSample({ cursorWindow: { x: 600, y: 300 }, host: { x: 600, y: 300 }, scale: 1 })).toEqual({ x: 0, y: 0 });
  });

  it("rejects impossible offsets instead of calibrating against a stale window", () => {
    expect(offsetFromSample({ cursorWindow: { x: 10, y: 10 }, host: { x: 900, y: 600 }, scale: 2 })).toBeNull();
    expect(offsetFromSample({ cursorWindow: { x: 900, y: 600 }, host: { x: 0, y: 0 }, scale: 2, window: { x: 0, y: 0, width: 300, height: 200 } })).toBeNull();
    expect(offsetFromSample({ cursorWindow: { x: Number.NaN, y: 10 }, host: { x: 1, y: 2 }, scale: 2 })).toBeNull();
  });
});

describe("mapHostDragPoint", () => {
  it("undoes the extra devicePixelRatio division", () => {
    const offset = { x: 240, y: 40 };
    const point = { x: 500, y: 300 };
    expect(mapHostDragPoint({ host: hostPointFor(point, offset, 2), offset, scale: 2 })).toEqual(point);
  });

  it("leaves already-correct host coordinates untouched", () => {
    expect(mapHostDragPoint({ host: { x: 500, y: 300 }, offset: { x: 0, y: 0 }, scale: 1 })).toEqual({ x: 500, y: 300 });
  });

  it("tracks a folder row while the pointer moves across the tree", () => {
    const offset = { x: 240, y: 40 };
    for (const y of [120, 148, 176, 204]) {
      expect(mapHostDragPoint({ host: hostPointFor({ x: 800, y }, offset, 2), offset, scale: 2 })).toEqual({ x: 800, y });
    }
  });
});

describe("medianDragPoint", () => {
  it("smooths pairing jitter from the drag-event / RPC race", () => {
    expect(medianDragPoint([
      { x: 240, y: 40 },
      { x: 242, y: 41 },
      { x: 239, y: 39 },
      { x: 300, y: 80 },
    ])).toEqual({ x: 241, y: 40.5 });
  });

  it("returns null without usable samples", () => {
    expect(medianDragPoint([])).toBeNull();
    expect(medianDragPoint([{ x: Number.NaN, y: 1 }])).toBeNull();
  });
});

describe("dragPointSpread", () => {
  it("reports the worst axis range so a still pointer can stop sampling", () => {
    expect(dragPointSpread([{ x: 240, y: 40 }, { x: 243, y: 39 }, { x: 241, y: 41 }])).toBe(3);
    expect(dragPointSpread([{ x: 240, y: 40 }])).toBe(Number.POSITIVE_INFINITY);
  });
});

describe("sanitizeDragScale", () => {
  it("falls back to 1 for non-finite or non-positive ratios", () => {
    expect(sanitizeDragScale(2)).toBe(2);
    expect(sanitizeDragScale(0)).toBe(1);
    expect(sanitizeDragScale(Number.NaN)).toBe(1);
  });
});

describe("estimateHostScale", () => {
  const offset = { x: 240, y: 40 };
  const pairsFor = (scale: number, points: number[]) => points.map((y) => {
    const point = { x: 600, y };
    return { cursorWindow: { x: point.x + offset.x, y: point.y + offset.y }, host: hostPointFor(point, offset, scale) };
  });

  it("detects a host that already forwards iframe CSS pixels", () => {
    expect(estimateHostScale(pairsFor(1, [100, 220, 360, 480]), 2)).toBe(1);
  });

  it("keeps the display scale while the host still divides by devicePixelRatio", () => {
    expect(estimateHostScale(pairsFor(2, [100, 220, 360, 480]), 2)).toBe(2);
  });

  it("waits for enough movement before changing the scale", () => {
    expect(estimateHostScale(pairsFor(1, [100, 104, 108]), 2)).toBe(2);
    expect(estimateHostScale(pairsFor(1, [100]), 2)).toBe(2);
  });
});
