/** 把宿主转发的拖拽点换算回 iframe CSS 坐标。
 *
 *  DBX 0.6.x（macOS）计算：`宿主点 = (真实点 + iframe 偏移) / scale − iframe 偏移`，
 *  其中 scale 是 `window.devicePixelRatio`。反解：
 *  `iframe 偏移 = 光标在窗口内的坐标 / scale − 宿主点`，
 *  `真实点 = scale × 宿主点 + (scale − 1) × iframe 偏移`。
 *
 *  一条 (光标, 宿主点) 样本只能反解出偏移；同一次拖拽内偏移是常量，多采
 *  几次取中位数即可消除 RPC 与拖拽事件之间的时间差抖动。
 *
 *  这里假设宿主页面从窗口左上角开始（DBX 用 Overlay 标题栏，webview 铺满
 *  窗口）。如果宿主给 webview 加内边距，偏移会恒定多出 inset/scale，换算
 *  结果会整体偏 inset/2 —— 真出现时应在 hostPoint 上先扣掉该内边距。 */

export interface DragMappingPoint {
  x: number;
  y: number;
}

export interface DragMappingRect extends DragMappingPoint {
  width: number;
  height: number;
}

export interface DragWindowSample {
  cursor: DragMappingPoint;
  window: DragMappingRect;
}

export interface DragPair {
  /** 真实光标在宿主窗口内的坐标。 */
  cursorWindow: DragMappingPoint;
  /** 同一时刻宿主转发的点。 */
  host: DragMappingPoint;
}

/** iframe 偏移必须是窗口内一个真实位置；留一点负值容忍测量抖动。 */
const MIN_OFFSET = -64;
const MAX_OFFSET = 8192;

export function sanitizeDragScale(scale: number): number {
  return Number.isFinite(scale) && scale > 0 ? scale : 1;
}

/** 光标屏幕坐标 → 宿主窗口内坐标；光标不在窗口内时返回 null。 */
export function cursorInWindow(sample: {
  cursor: DragMappingPoint;
  window: DragMappingRect;
}): DragMappingPoint | null {
  const { cursor, window } = sample;
  if (
    !Number.isFinite(cursor.x) ||
    !Number.isFinite(cursor.y) ||
    !Number.isFinite(window.x) ||
    !Number.isFinite(window.y) ||
    !Number.isFinite(window.width) ||
    !Number.isFinite(window.height)
  ) {
    return null;
  }
  const point = { x: cursor.x - window.x, y: cursor.y - window.y };
  if (point.x < -1 || point.y < -1 || point.x > window.width + 1 || point.y > window.height + 1) {
    return null;
  }
  return point;
}

export function offsetFromSample(sample: {
  cursorWindow: DragMappingPoint;
  host: DragMappingPoint;
  scale: number;
  window?: DragMappingRect | null;
}): DragMappingPoint | null {
  const scale = sanitizeDragScale(sample.scale);
  if (!Number.isFinite(sample.cursorWindow.x) || !Number.isFinite(sample.cursorWindow.y)) return null;
  if (!Number.isFinite(sample.host.x) || !Number.isFinite(sample.host.y)) return null;
  const offset = {
    x: sample.cursorWindow.x / scale - sample.host.x,
    y: sample.cursorWindow.y / scale - sample.host.y,
  };
  if (!Number.isFinite(offset.x) || !Number.isFinite(offset.y)) return null;
  if (offset.x < MIN_OFFSET || offset.y < MIN_OFFSET || offset.x > MAX_OFFSET || offset.y > MAX_OFFSET) {
    return null;
  }
  if (sample.window && (offset.x > sample.window.width + 64 || offset.y > sample.window.height + 64)) {
    return null;
  }
  return offset;
}

/** 估计宿主把逻辑点缩小的倍数：DBX 0.6.x 多除了一次 dpr（约等于 scale），
 *  已经修好的宿主则是 1。只有 3 组以上、位移足够大的样本才敢改判，避免
 *  事件时间差带来的比值抖动把已校准的映射带偏。 */
export function estimateHostScale(pairs: DragPair[], displayScale: number, current: number = displayScale): number {
  const ratios: number[] = [];
  for (let i = 0; i < pairs.length; i += 1) {
    for (let j = i + 1; j < pairs.length; j += 1) {
      const dxCursor = pairs[i].cursorWindow.x - pairs[j].cursorWindow.x;
      const dyCursor = pairs[i].cursorWindow.y - pairs[j].cursorWindow.y;
      const dxHost = pairs[i].host.x - pairs[j].host.x;
      const dyHost = pairs[i].host.y - pairs[j].host.y;
      const useX = Math.abs(dxCursor) >= Math.abs(dyCursor);
      const movedCursor = useX ? dxCursor : dyCursor;
      const movedHost = useX ? dxHost : dyHost;
      if (Math.abs(movedCursor) < 32 || Math.abs(movedHost) < 12) continue;
      const ratio = movedCursor / movedHost;
      if (Number.isFinite(ratio) && ratio > 0.5 && ratio < 4) ratios.push(ratio);
    }
  }
  if (ratios.length < 3) return sanitizeDragScale(current);
  const estimated = median(ratios);
  // 明显是 1 才改判；明显偏离 1 就回到宿主默认缩放；过渡值保持现状，
  // 避免事件时间差造成的比值抖动让换算来回切换。
  if (estimated <= 1.35) return 1;
  if (estimated >= 1.6) return sanitizeDragScale(displayScale);
  return sanitizeDragScale(current);
}

export function mapHostDragPoint(sample: {
  host: DragMappingPoint;
  offset: DragMappingPoint;
  scale: number;
}): DragMappingPoint {
  const scale = sanitizeDragScale(sample.scale);
  return {
    x: scale * sample.host.x + (scale - 1) * sample.offset.x,
    y: scale * sample.host.y + (scale - 1) * sample.offset.y,
  };
}

function median(values: number[]): number {
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[middle - 1] + sorted[middle]) / 2 : sorted[middle];
}

export function medianDragPoint(points: DragMappingPoint[]): DragMappingPoint | null {
  const usable = points.filter((point) => Number.isFinite(point.x) && Number.isFinite(point.y));
  if (!usable.length) return null;
  return {
    x: median(usable.map((point) => point.x)),
    y: median(usable.map((point) => point.y)),
  };
}

/** 最近几次样本的极差（px）；用来判断光标停住后偏移是否已收敛。 */
export function dragPointSpread(points: DragMappingPoint[]): number {
  const usable = points.filter((point) => Number.isFinite(point.x) && Number.isFinite(point.y));
  if (usable.length < 2) return Number.POSITIVE_INFINITY;
  const xs = usable.map((point) => point.x);
  const ys = usable.map((point) => point.y);
  return Math.max(Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys));
}
