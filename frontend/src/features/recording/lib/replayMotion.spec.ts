// 回放指针/光标轨道的纯时间查找单测：播放头回退时必须重新取最近一次
// 采样，而不是残留未来状态。
import { describe, expect, it } from "vitest";
import { motionAtTime, type ReplayMotionEvent } from "./replayMotion";

const motion: ReplayMotionEvent[] = [
  { time: 0.5, data: { kind: "pointer", x: 0.1, y: 0.2, pressed: false, cols: 100, rows: 26 } },
  { time: 1.0, data: { kind: "cursor", col: 4, row: 9, cols: 100, rows: 26 } },
  { time: 1.5, data: { kind: "pointer", x: 0.8, y: 0.7, pressed: true, cols: 100, rows: 26 } },
  { time: 2.0, data: { kind: "cursor", col: 10, row: 2, cols: 100, rows: 26 } },
];

describe("motionAtTime", () => {
  it("returns null before the first sample", () => {
    expect(motionAtTime(motion, -1)).toEqual({ pointer: null, cursor: null });
    expect(motionAtTime(motion, 499)).toEqual({ pointer: null, cursor: null });
  });

  it("tracks the latest pointer and cursor independently at or before the playhead", () => {
    expect(motionAtTime(motion, 500)).toEqual({ pointer: motion[0]!.data, cursor: null });
    expect(motionAtTime(motion, 1000)).toEqual({ pointer: motion[0]!.data, cursor: motion[1]!.data });
    expect(motionAtTime(motion, 1750)).toEqual({ pointer: motion[2]!.data, cursor: motion[1]!.data });
    expect(motionAtTime(motion, 2500)).toEqual({ pointer: motion[2]!.data, cursor: motion[3]!.data });
  });

  it("ignores future samples when seeking backward", () => {
    expect(motionAtTime(motion, 1100)).toEqual({ pointer: motion[0]!.data, cursor: motion[1]!.data });
    expect(motionAtTime(motion, 900)).toEqual({ pointer: motion[0]!.data, cursor: null });
  });
});
