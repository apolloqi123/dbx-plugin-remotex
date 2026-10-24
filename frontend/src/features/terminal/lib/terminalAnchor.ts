import type { Terminal } from "@xterm/xterm";

// 终端光标锚点换算：把 xterm buffer 坐标语义集中到一个模块，避免 App.vue 里
// 视口/绝对行公式各自漂移。xterm typings 中：
// - cursorY 是光标在「视口内」的相对行（0..rows-1）；
// - viewportY 是视口顶在缓冲里的绝对行号；
// - baseY 是滚回贴底时视口顶的绝对行号（= buffer.length - rows）。
// 历史回归：曾用 `cursorY - viewportY` 求可见行——viewportY ≠ baseY（回滚区
// 存在后）时结果为负，ghost/建议浮层被画到画布外；曾用 `cursorY + viewportY`
// 采样「光标所在行」——用户上滚时采到的是滚回区旧行而非光标行。

/** 最小 buffer 形状：只取换算需要的三个行号（便于单测伪造）。 */
export interface CursorBufferLike {
  /** 光标在视口内的相对行（0..rows-1）。 */
  cursorY: number;
  /** 视口顶的缓冲绝对行号。 */
  viewportY: number;
  /** 滚回贴底时视口顶的缓冲绝对行号。 */
  baseY: number;
}

/** 光标所在视觉行的「视口内行号」（0..rows-1）：overlay 像素锚点用。 */
export function cursorViewportRow(buffer: CursorBufferLike): number {
  return buffer.cursorY;
}

/** 光标所在视觉行的「缓冲绝对行号」：buffer.getLine 采样用。 */
export function cursorAbsoluteRow(buffer: CursorBufferLike): number {
  return buffer.baseY + buffer.cursorY;
}

/**
 * DOM 兜底的单元格尺寸测量：xterm 私有 `_renderService.dimensions.css.cell`
 * 读不到时（渲染器未就绪/WebGL 恢复切换窗口期）用渲染 DOM 实测——DOM 渲染器
 * 给每个行元素写显式像素高，列宽按 .xterm-screen 实测宽 ÷ cols 推出。
 * 任一项测不出 → null（调用方维持既有降级，不造假锚点）。
 */
export function measureCellSizeFromDom(
  screen: { getBoundingClientRect(): { width: number } } | null,
  rows: { children: ArrayLike<{ getBoundingClientRect(): { height: number } }> } | null,
  cols: number,
): { width: number; height: number } | null {
  try {
    const firstRow = rows?.children?.[0];
    if (!firstRow || !(cols > 0)) return null;
    const rowHeight = firstRow.getBoundingClientRect().height;
    const screenWidth = screen?.getBoundingClientRect().width ?? 0;
    if (!(rowHeight > 0) || !(screenWidth > 0)) return null;
    return { width: screenWidth / cols, height: rowHeight };
  } catch {
    return null;
  }
}

/** 光标格换算的共享参数：.xterm-screen 原点 + 单元格尺寸 + buffer 坐标。 */
export interface TerminalCellFrame {
  originLeft: number;
  originTop: number;
  cellWidth: number;
  cellHeight: number;
  cursorX: number;
  visibleRow: number;
}

/**
 * 光标格锚点（主终端与分屏窗格共用）：以 .xterm-screen（渲染内容区，
 * 位于 .xterm 内边距内侧）为原点，加光标网格坐标。xterm 私有渲染尺寸
 * 读不到时用渲染 DOM 实测兜底；仍测不出返回 null，调用方降级贴底。
 */
export function readTerminalCellFrame(
  term: Terminal | undefined,
  host: HTMLElement | null | undefined,
): TerminalCellFrame | null {
  if (!term || !host) return null;
  try {
    const core = (term as unknown as { _core?: { _renderService?: { dimensions?: { css?: { cell?: { width?: number; height?: number } } } } } })._core;
    const cell = core?._renderService?.dimensions?.css?.cell;
    let cellWidth = cell?.width ?? 0;
    let cellHeight = cell?.height ?? 0;
    const screen = term.element?.querySelector(".xterm-screen");
    if (!(cellWidth > 0) || !(cellHeight > 0)) {
      const fromDom = measureCellSizeFromDom(
        screen ?? null,
        term.element?.querySelector(".xterm-rows") ?? null,
        term.cols,
      );
      if (!fromDom) return null;
      cellWidth = fromDom.width;
      cellHeight = fromDom.height;
    }
    const buffer = term.buffer.active;
    const hostRect = host.getBoundingClientRect();
    const origin = (screen ?? term.element)?.getBoundingClientRect();
    if (!origin) return null;
    return {
      originLeft: origin.left - hostRect.left,
      originTop: origin.top - hostRect.top,
      cellWidth,
      cellHeight,
      cursorX: buffer.cursorX,
      visibleRow: cursorViewportRow(buffer),
    };
  } catch {
    return null;
  }
}
