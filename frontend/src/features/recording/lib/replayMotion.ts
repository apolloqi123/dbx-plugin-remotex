// 回放指针 / 光标轨道的纯数据模型与时间查找。零 Vue / 零 DOM 依赖；
// 只消费自定义 asciicast `x` 事件，绝不把元数据当终端输出。

export interface PointerMotion {
  kind: "pointer";
  /** Host-relative normalized x in 0..1. */
  x: number;
  /** Host-relative normalized y in 0..1. */
  y: number;
  pressed: boolean;
  cols: number;
  rows: number;
}

export interface CursorMotion {
  kind: "cursor";
  /** 0-based xterm caret cell. */
  col: number;
  row: number;
  cols: number;
  rows: number;
}

export type RecordingMotionData = PointerMotion | CursorMotion;

export interface ReplayMotionEvent {
  /** Seconds since recording start. */
  time: number;
  data: RecordingMotionData;
}

function finiteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function validGeometry(cols: unknown, rows: unknown): cols is number {
  return (
    finiteNumber(cols) &&
    Number.isInteger(cols) &&
    cols >= 1 &&
    cols <= 1000 &&
    finiteNumber(rows) &&
    Number.isInteger(rows) &&
    rows >= 1 &&
    rows <= 1000
  );
}

/** Defensive guard for replay: a malformed `x` object is ignored rather than
 * rendered or written to xterm. */
export function isRecordingMotionData(value: unknown): value is RecordingMotionData {
  if (!value || typeof value !== "object") return false;
  const data = value as Record<string, unknown>;
  if (!validGeometry(data.cols, data.rows)) return false;
  const cols = data.cols as number;
  const rows = data.rows as number;
  if (data.kind === "pointer") {
    return (
      finiteNumber(data.x) &&
      data.x >= 0 &&
      data.x <= 1 &&
      finiteNumber(data.y) &&
      data.y >= 0 &&
      data.y <= 1 &&
      typeof data.pressed === "boolean"
    );
  }
  if (data.kind === "cursor") {
    return (
      finiteNumber(data.col) &&
      Number.isInteger(data.col) &&
      data.col >= 0 &&
      data.col < cols &&
      finiteNumber(data.row) &&
      Number.isInteger(data.row) &&
      data.row >= 0 &&
      data.row < rows
    );
  }
  return false;
}

/** One linear scan: the latest independent pointer/cursor sample at or before
 * `timeMs / 1000`. Negative or non-finite playheads fall back to the stable
 * lower bound (before the first sample), never a stale future state. */
export function motionAtTime(
  motion: readonly ReplayMotionEvent[],
  timeMs: number,
): { pointer: PointerMotion | null; cursor: CursorMotion | null } {
  const seconds = finiteNumber(timeMs) ? timeMs / 1000 : -Infinity;
  let pointer: PointerMotion | null = null;
  let cursor: CursorMotion | null = null;
  for (const event of motion) {
    if (!finiteNumber(event.time) || event.time > seconds) continue;
    if (event.data.kind === "pointer") pointer = event.data;
    else if (event.data.kind === "cursor") cursor = event.data;
  }
  return { pointer, cursor };
}
