import { computed, watch, type Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import type { OutputGate } from "../../lib/emulator/terminalBackpressure";
import {
  highlightFillStyle,
  highlightTextStyle,
  matchesInLine,
  shouldRebuildHighlightRow,
  toAbsoluteRowRange,
  type CompiledHighlightRule,
  type HighlightDisplayMode,
} from "../../lib/decorations/keywordHighlight";
// 命中是文本下标，装饰要的是单元格列号：宽字符行（CJK）不换算就会整段左移。
import { lineColumnTable, matchToColumns } from "../../lib/emulator/terminalLineColumns";

export interface TerminalHighlightOptions {
  terminal: () => Terminal | undefined;
  highlightEnabled: Ref<boolean>;
  compiledHighlightRules: Ref<CompiledHighlightRule[]>;
  highlightDisplayMode: Ref<HighlightDisplayMode>;
  outputGate: OutputGate;
}

/**
 * 主终端关键词高亮的 decoration 渲染引擎（规则表 CRUD 在 useHighlightRules）：
 * onRender 脏行合并 + 节流扫描 + 逐行 marker/decoration 组，显示方式（背景色块 /
 * 字体颜色）与规则表变化即时生效。终端实例由 getter 注入——终端会随会话重建。
 */
export function useTerminalHighlight(options: TerminalHighlightOptions) {
  const { terminal: terminalGet, highlightEnabled, compiledHighlightRules, highlightDisplayMode, outputGate } = options;

// decoration 引擎护栏：全局在档 decoration 上限（超限停止本帧注册）。
const HIGHLIGHT_DECORATION_LIMIT = 400;
// rAF 节流目标：≤30fps（约 33ms 一帧）。
const HIGHLIGHT_SCAN_MIN_INTERVAL_MS = 33;

// 「字体颜色」= 把规则色交给 decoration 的 foregroundColor，由渲染器写进该格字形：
// DOM 渲染器落到单元格 span 的 color（DomRendererRowFactory），WebGL 渲染器在
// CellColorResolver 里把 foregroundColorRGB 折进该格 fg 属性（glyph atlas 以 fg
// 为键、会自动重新光栅化）——两条渲染路径都生效，故无需回退 DOM、GPU 照常。
const keywordTextColorActive = computed(() => highlightDisplayMode.value === "text");

// ---- decoration 引擎 ----
// onRender({start,end}) 给的是"本帧实际重绘的行区间"（输入时常常只有光标一行），
// 不是整个视口：合并进 pending 区间，经 setTimeout 节流（≤30fps）后统一扫描；
// 扫描时装饰去留按 buffer.viewportY 的真实视口判定（见 scanHighlightRange）。
// alt buffer 与 normal buffer 走同一路径（buffer.active 直接扫描）。
let highlightRenderDisposable: { dispose(): void } | undefined;
// 每行一个组（marker + decorations + 该行登记时的文本）；行滚出视口整组 dispose。
// text 用于"文本未变则整组保留"：xterm 在装饰 dispose/注册后自身会再触发整幅
// 重绘（实测 30fps 持续循环），无脑拆建会让空闲终端陷入"重绘→扫描→拆建→重绘"
// 的自激回路，高亮层反复摘挂即是用户看到的闪烁。
const highlightDecorationsByRow = new Map<number, { text: string; dispose(): void }>();
let highlightDecorationCount = 0;
let highlightScanScheduled = false;
let highlightLastScanAt = 0;
let highlightPendingRange: { start: number; end: number } | undefined;

function clearHighlightDecorations() {
  for (const entry of highlightDecorationsByRow.values()) entry.dispose();
  highlightDecorationsByRow.clear();
  highlightDecorationCount = 0;
}

function attachHighlightRender() {
  const term = terminalGet();
  if (!term || highlightRenderDisposable || !highlightEnabled.value) return;
  highlightRenderDisposable = term.onRender(({ start, end }) => scheduleHighlightScan(start, end));
  rescanHighlightViewport();
}

function detachHighlightRender() {
  highlightRenderDisposable?.dispose();
  highlightRenderDisposable = undefined;
  clearHighlightDecorations();
}

function rescanHighlightViewport() {
  const term = terminalGet();
  if (!term || !highlightEnabled.value) return;
  scheduleHighlightScan(0, term.rows - 1);
}

function scheduleHighlightScan(start: number, end: number) {
  if (!terminalGet() || !highlightEnabled.value || !compiledHighlightRules.value.length) return;
  // 大输出保护生效期挂起扫描，恢复时由 onOutputGateRelease 补扫视口。
  if (outputGate.mode === "strained") return;
  highlightPendingRange = highlightPendingRange
    ? { start: Math.min(highlightPendingRange.start, start), end: Math.max(highlightPendingRange.end, end) }
    : { start, end };
  if (highlightScanScheduled) return;
  highlightScanScheduled = true;
  const wait = Math.max(0, HIGHLIGHT_SCAN_MIN_INTERVAL_MS - (performance.now() - highlightLastScanAt));
  window.setTimeout(runHighlightScan, wait);
}

function runHighlightScan() {
  highlightScanScheduled = false;
  highlightLastScanAt = performance.now();
  const range = highlightPendingRange;
  highlightPendingRange = undefined;
  if (!range || !terminalGet() || !highlightEnabled.value) return;
  scanHighlightRange(range.start, range.end);
}

function scanHighlightRange(start: number, end: number) {
  const term = terminalGet();
  if (!term) return;
  const buffer = term.buffer.active;
  // onRender 的 start/end 是视口相对行号，这里的 row / viewportY 是缓冲绝对行号
  // （换算与裁剪见 toAbsoluteRowRange：漏了这步，滚过一屏后脏行判定永不命中）。
  const dirty = toAbsoluteRowRange(start, end, buffer.viewportY, buffer.length);
  // onRender 给的是"本帧重绘的行"（输入时常常只有光标行），不是视口——装饰的
  // 去留必须按视口判定，否则每次击键都把整屏高亮 dispose 掉再异步补回（可见闪烁）。
  const vpFrom = Math.max(0, Math.min(buffer.viewportY, buffer.length - 1));
  const vpTo = Math.min(buffer.length - 1, vpFrom + term.rows - 1);
  for (const [row, entry] of highlightDecorationsByRow) {
    // 拆组条件（视口外 / 本帧重绘且文本变了）见 shouldRebuildHighlightRow；
    // "重绘但文本没变就保留"是掐断"拆建→重绘"自激回路的关键。
    const dirtyRow = row >= dirty.from && row <= dirty.to;
    const currentText = dirtyRow ? buffer.getLine(row)?.translateToString(true) ?? "" : entry.text;
    if (!shouldRebuildHighlightRow({ row, viewportFrom: vpFrom, viewportTo: vpTo, dirty: dirtyRow, previousText: entry.text, currentText })) continue;
    entry.dispose();
    highlightDecorationsByRow.delete(row);
  }
  const compiled = compiledHighlightRules.value;
  if (!compiled.length) return;
  // registerMarker 的 offset 相对光标绝对行（baseY + cursorY）；marker dispose 时
  // xterm 会连带 dispose 挂在其上的 decoration。
  const base = buffer.baseY + buffer.cursorY;
  for (let row = vpFrom; row <= vpTo; row++) {
    if (highlightDecorationsByRow.has(row)) continue;
    if (highlightDecorationCount >= HIGHLIGHT_DECORATION_LIMIT) return;
    const bufferLine = buffer.getLine(row);
    const lineText = bufferLine?.translateToString(true) ?? "";
    if (!lineText) continue;
    const matches = matchesInLine(lineText, compiled);
    if (!matches.length) continue;
    // 命中是文本下标，decoration 的 x/width 是单元格列号：宽字符（CJK、全角标点）在文本里
    // 占 1 个单元、在终端里占 2 列，直接用下标会让整段高亮左移——前面有几个宽字符就少几列
    // （中文行里越靠右的命中越歪，IP/时间戳会整段露白）。只有真有命中的行才补一次列号表，
    // 其余行继续走 translateToString 的内部缓存。
    const columns = bufferLine ? lineColumnTable(bufferLine, lineText.length) : undefined;
    const marker = term.registerMarker(row - base);
    if (!marker) continue;
    const disposables: Array<{ dispose(): void }> = [marker];
    const entry = {
      text: lineText,
      decorations: 0,
      dispose() {
        for (const disposable of disposables.splice(0)) disposable.dispose();
        // 计数只增不减会顶到全局上限、高亮逐渐不再出现（看起来像闪烁后消失）。
        highlightDecorationCount -= entry.decorations;
        entry.decorations = 0;
      },
    };
    for (const match of matches) {
      if (highlightDecorationCount >= HIGHLIGHT_DECORATION_LIMIT) break;
      // 「字体颜色」模式：把规则色交给 decoration 的 foregroundColor——无覆盖层、
      // 无需逐帧重放（渲染器每次重绘都会重新查询 decoration），也不与配色方案/主题
      // 的调色板打架。DOM 与 WebGL 两条渲染路径都消费该字段。
      const { x, width } = matchToColumns(columns, match.start, match.end);
      const decoration = term.registerDecoration(
        keywordTextColorActive.value
          ? { marker, x, width, foregroundColor: highlightTextStyle(match.color) }
          : { marker, x, width },
      );
      if (decoration) {
        if (!keywordTextColorActive.value) {
          // 背景色块模式保持原样：xterm 6 的 DOM renderer 仍不把 registerDecoration
          // 的 backgroundColor 选项应用到 cell（且 CM_RGB 路径会丢 alpha，实心色会
          // 盖字形），着色继续走 onRender 在 overlay 元素上半透明自绘。注意 overlay
          // 元素由 BufferDecorationRenderer 在渲染刷新帧里创建：窗口被遮挡/rAF 冻结
          // 期间注册的高亮要等下一次刷新帧才显色（可见后一帧内自愈，属显示时序而非
          // 逻辑缺陷——逐字符 span 是 cell 层 run 拆分，不代表 decoration 丢失）。
          decoration.onRender((element) => {
            element.style.backgroundColor = highlightFillStyle(match.color);
          });
        }
        disposables.push(decoration);
        entry.decorations++;
        highlightDecorationCount++;
      }
    }
    highlightDecorationsByRow.set(row, entry);
  }
}

// 规则表/显示方式变化：全量清理后重扫当前视口（即时生效语义）。显示方式切换必须
// 重建 decoration——背景色块画在 overlay 元素上、字体颜色是 registerDecoration 的
// foregroundColor 选项，两者无法就地改样式互转。
watch([compiledHighlightRules, highlightDisplayMode], () => {
  clearHighlightDecorations();
  rescanHighlightViewport();
});

  return {
    attachHighlightRender,
    detachHighlightRender,
    rescanHighlightViewport,
    scheduleHighlightScan,
  };
}
