import { computed, ref, watch } from "vue";
import type { Terminal } from "@xterm/xterm";
import type { OutputGate } from "../../lib/emulator/terminalBackpressure";
import type { TerminalWriteThrottle } from "../../lib/emulator/terminalWriteThrottle";
import {
  computeGutterRows,
  getRenderCellHeight,
  sanitizeGutterSettings,
  trimTimestampMap,
  GUTTER_TIMESTAMP_RETENTION_ROWS,
  type GutterRow,
  type GutterSettings,
} from "../../lib/appearance/terminalGutter";

export interface TerminalGutterOptions {
  terminal: () => Terminal | undefined;
  outputGate: OutputGate;
  terminalWriteThrottle: TerminalWriteThrottle;
  scheduleFit: () => void;
  persistPrefs: (patch: Record<string, unknown>) => void;
}

/**
 * 主终端左侧行号/时间戳 gutter（P1-3，默认关闭）：行模型由纯函数
 * computeGutterRows 产出，这里只管挂子、rAF 重算与大输出期挂起；
 * 写入时刻由调用方在 write 完成回调里 stampGutterWrittenRows 采集。
 */
export function useTerminalGutter(options: TerminalGutterOptions) {
  const { terminal: terminalGet, outputGate, terminalWriteThrottle, scheduleFit, persistPrefs } = options;

// ---- 行号 / 时间戳 gutter（P1-3）----
const gutterSettings = ref<GutterSettings>(sanitizeGutterSettings(undefined));
const gutterRows = ref<GutterRow[]>([]);
const gutterCellHeight = ref<number | null>(null);
// 大输出写入期挂起重算（terminalWriteThrottle 积压 > 256KiB 即跳帧，落定后由
// 下一次 onRender/onScroll 事件跟上），避免 gutter 追帧放大 strained 场景开销。
const GUTTER_SUSPEND_PENDING_BYTES = 256 * 1024;
let gutterRafId = 0;
// 视口顶部像素偏移 = xterm 元素 padding-top。改外观（行高/字号/内边距）才会
// 变，但 runGutterRecompute 每个渲染帧都跑：每帧 getComputedStyle 会强制样式
// 求值。缓存并在 resize / 设置变化时失效，另加 2s 兜底重读。
let gutterScreenOffsetTop: number | null = null;
let gutterScreenOffsetAt = 0;
const GUTTER_OFFSET_TTL_MS = 2_000;
// 逻辑行首绝对行号 → 写入时刻。裁剪保留视口顶端前 3000 行。
const gutterTimestamps = new Map<number, number>();
let gutterLastStampedRow = -1;
let gutterRenderDisposable: { dispose(): void } | undefined;
let gutterScrollDisposable: { dispose(): void } | undefined;
let gutterResizeDisposable: { dispose(): void } | undefined;
let gutterEnterDisposable: { dispose(): void } | undefined;

const gutterPaneVisible = computed(() => gutterSettings.value.showLineNumbers || gutterSettings.value.showTimestamps);
// gutter 只占终端左 padding 环带：两种开关组合给固定宽度（行号 5 位 + 余量）。
const gutterWidth = computed(() => {
  if (!gutterPaneVisible.value) return 0;
  if (gutterSettings.value.showLineNumbers && gutterSettings.value.showTimestamps) return 132;
  return gutterSettings.value.showLineNumbers ? 56 : 88;
});
// 渲染尺寸读不到（渲染器未就绪/WebGL 恢复中）时整体隐藏降级，不报错。
const gutterVisible = computed(() => gutterPaneVisible.value && gutterCellHeight.value !== null && gutterRows.value.length > 0);
const gutterPaneStyle = computed(() => ({ "--dbx-gutter-width": `${gutterWidth.value}px` }));

function isGutterActive() {
  return gutterPaneVisible.value;
}

function attachGutterListeners() {
  const term = terminalGet();
  if (!term || gutterRenderDisposable || !isGutterActive()) return;
  gutterRenderDisposable = term.onRender(() => scheduleGutterRecompute());
  gutterScrollDisposable = term.onScroll(() => scheduleGutterRecompute());
  gutterResizeDisposable = term.onResize(() => {
    gutterScreenOffsetTop = null;
    scheduleGutterRecompute();
  });
  // 回车重盖光标逻辑行：独立 onData 挂子（routeTerminalData 输入路由不动）。
  gutterEnterDisposable = term.onData((data) => {
    if (!gutterSettings.value.showTimestamps) return;
    if (!data.includes("\r") && !data.includes("\n")) return;
    const active = terminalGet();
    if (!active) return;
    const buffer = active.buffer.active;
    stampLogicalLineContaining(buffer.baseY + buffer.cursorY, Date.now());
    scheduleGutterRecompute();
  });
  scheduleGutterRecompute();
}

function detachGutterListeners() {
  gutterRenderDisposable?.dispose();
  gutterRenderDisposable = undefined;
  gutterScrollDisposable?.dispose();
  gutterScrollDisposable = undefined;
  gutterResizeDisposable?.dispose();
  gutterResizeDisposable = undefined;
  gutterEnterDisposable?.dispose();
  gutterEnterDisposable = undefined;
  if (gutterRafId) {
    cancelAnimationFrame(gutterRafId);
    gutterRafId = 0;
  }
  gutterRows.value = [];
  gutterCellHeight.value = null;
  gutterScreenOffsetTop = null;
}

function scheduleGutterRecompute() {
  if (!isGutterActive()) return;
  // 大输出保护生效期挂起重算（与下方积压跳帧同一目标，口径更早介入）。
  if (outputGate.mode === "strained") return;
  if (gutterRafId) return;
  gutterRafId = window.requestAnimationFrame(runGutterRecompute);
}

/** 首视口行的像素起点 = xterm 元素的 padding-top（gutter 文本与画布行对齐）。 */
function resolveGutterScreenOffset(term: Terminal): number {
  const now = Date.now();
  if (gutterScreenOffsetTop === null || now - gutterScreenOffsetAt >= GUTTER_OFFSET_TTL_MS) {
    const paddingTop = term.element ? Number.parseFloat(window.getComputedStyle(term.element).paddingTop) : Number.NaN;
    gutterScreenOffsetTop = Number.isFinite(paddingTop) ? paddingTop : 0;
    gutterScreenOffsetAt = now;
  }
  return gutterScreenOffsetTop ?? 0;
}

function runGutterRecompute() {
  gutterRafId = 0;
  const term = terminalGet();
  if (!term || !isGutterActive()) return;
  if (terminalWriteThrottle.pendingBytes > GUTTER_SUSPEND_PENDING_BYTES) return;
  if (outputGate.mode === "strained") return;
  gutterCellHeight.value = getRenderCellHeight(term);
  const buffer = term.buffer.active;
  gutterRows.value = computeGutterRows({
    cellHeight: gutterCellHeight.value,
    screenOffsetTop: resolveGutterScreenOffset(term),
    scrollTop: buffer.viewportY,
    buffer: { type: buffer.type, length: buffer.length, getLine: (y) => buffer.getLine(y) },
    rows: term.rows,
    timestamps: gutterTimestamps,
    showLineNumbers: gutterSettings.value.showLineNumbers,
    showTimestamps: gutterSettings.value.showTimestamps,
    timestampFormat: gutterSettings.value.timestampFormat,
  });
  trimTimestampMap(gutterTimestamps, Math.max(0, buffer.viewportY - GUTTER_TIMESTAMP_RETENTION_ROWS));
}

// 时间戳采集挂点：写入节流 sink 的 terminal.write 完成回调（xterm 解析完这批
// 合并字节后触发）。对新写入的逻辑行首盖 Date.now()；原地重写（进度条）只重盖
// 底行；buffer 变短（clear/重连）时整表重置。
function stampGutterWrittenRows() {
  if (!gutterSettings.value.showTimestamps) return;
  const term = terminalGet();
  if (!term) return;
  const buffer = term.buffer.active;
  if (buffer.type === "alternate") return;
  const last = buffer.length - 1;
  if (last < 0) return;
  if (last < gutterLastStampedRow) resetGutterTimestamps();
  const now = Date.now();
  const from = Math.max(0, gutterLastStampedRow);
  for (let row = from; row <= last; row++) {
    if (buffer.getLine(row)?.isWrapped) continue;
    gutterTimestamps.set(row, now);
  }
  gutterLastStampedRow = last;
}

// 回车重盖：光标所在逻辑行（含 wrapped 向上回溯）整体盖为回车时刻。
function stampLogicalLineContaining(absoluteRow: number, atMs: number) {
  const term = terminalGet();
  if (!term) return;
  const buffer = term.buffer.active;
  if (buffer.type === "alternate") return;
  let row = Math.max(0, Math.min(absoluteRow, buffer.length - 1));
  let scanned = 0;
  while (row > 0 && scanned < 2048 && buffer.getLine(row)?.isWrapped) {
    row -= 1;
    scanned += 1;
  }
  gutterTimestamps.set(row, atMs);
}

function resetGutterTimestamps() {
  gutterTimestamps.clear();
  gutterLastStampedRow = -1;
}

// gutter 设置变化：挂/摘挂子 + 宽度变化后重算终端列宽（xterm 左 padding 随
// --dbx-gutter-width 变化，FitAddon 需要重新 fit）。
watch(gutterSettings, () => {
  // 开关会改左 padding 宽度；顶部偏移缓存一并失效，避免沿用旧对齐。
  gutterScreenOffsetTop = null;
  if (terminalGet() && isGutterActive()) attachGutterListeners();
  else detachGutterListeners();
  scheduleGutterRecompute();
  scheduleFit();
}, { deep: true });

function updateGutterSettings(patch: { showLineNumbers?: boolean; showTimestamps?: boolean; timestampFormat?: string }) {
  const next = sanitizeGutterSettings({ ...gutterSettings.value, ...patch });
  gutterSettings.value = next;
  void persistPrefs({
    terminal_show_line_numbers: next.showLineNumbers,
    terminal_show_timestamps: next.showTimestamps,
    terminal_timestamp_format: next.timestampFormat,
  });
}

  return {
    gutterSettings,
    gutterRows,
    gutterCellHeight,
    gutterPaneVisible,
    gutterWidth,
    gutterVisible,
    gutterPaneStyle,
    isGutterActive,
    attachGutterListeners,
    detachGutterListeners,
    scheduleGutterRecompute,
    resetGutterTimestamps,
    stampGutterWrittenRows,
    stampLogicalLineContaining,
    updateGutterSettings,
  };
}
