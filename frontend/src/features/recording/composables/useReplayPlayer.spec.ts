// 回放叠层状态的纯逻辑：拖动进度条回退时指针/光标必须重新取播放头之前
// 的最近采样；旧录制没有元数据轨道时静默降级为不显示。
import { describe, expect, it } from "vitest";
import { replayStateWithMotion, type ReplayState } from "./useReplayPlayer";
import type { RecordingSummary } from "../lib/replayScheduler";
import type { ReplayMotionEvent } from "../lib/replayMotion";

const summary: RecordingSummary = { recordingId: "rec-1" };
const motion: ReplayMotionEvent[] = [
  { time: 0.5, data: { kind: "pointer", x: 0.1, y: 0.2, pressed: false, cols: 100, rows: 26 } },
  { time: 1.0, data: { kind: "cursor", col: 4, row: 9, cols: 100, rows: 26 } },
  { time: 1.5, data: { kind: "pointer", x: 0.8, y: 0.7, pressed: true, cols: 100, rows: 26 } },
  { time: 2.0, data: { kind: "cursor", col: 10, row: 2, cols: 100, rows: 26 } },
];

const baseState: ReplayState = {
  summary,
  events: [],
  motion,
  pointer: null,
  cursor: null,
};

describe("replayStateWithMotion", () => {
  it("recomputes both overlay positions when seeking backward", () => {
    const forward = replayStateWithMotion(baseState, 2500);
    expect(forward.pointer?.x).toBe(0.8);
    expect(forward.cursor?.col).toBe(10);
    const backward = replayStateWithMotion(forward, 1100);
    expect(backward.pointer?.x).toBe(0.1);
    expect(backward.cursor?.col).toBe(4);
  });

  it("returns null overlays for empty or legacy recordings", () => {
    const legacy: ReplayState = { ...baseState, motion: [] };
    const state = replayStateWithMotion(legacy, 1000);
    expect(state.pointer).toBeNull();
    expect(state.cursor).toBeNull();
  });
});
