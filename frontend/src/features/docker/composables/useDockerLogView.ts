// 容器日志的窗口化渲染引擎：1000 行流缓冲、200 行 DOM 窗口、100ms 合帧、
// 滚到窗口边缘自动换窗。与 Vue 组件解耦，组件只负责工具栏与流生命周期。
//
// 关键约束（2026-10-29 卡顿事故）：日志页的渲染成本必须由「可见窗口」而
// 不是「缓冲上限」决定。此前暂停跟随后 DOM 可涨到 10000 行（约 4 万节点），
// 每个分片都插入节点并触发整块 <pre> 布局——一次 2 万行注入实测 1.2s 长
// 任务、堆 +42MB，整个 DBX 跟着卡。现在 DOM 最多 200 行，缓冲最多 1000 行，
// 超出直接丢弃；窗口隐藏时只收行不动 DOM。
import { onScopeDispose, type Ref } from "vue";
import { FOLLOW_PAUSE_PX, FOLLOW_RESUME_PX, MAX_LOG_LINES } from "../lib/dockerLogBuffer";
import { documentHidden, onDocumentVisibilityChange } from "../../../shared/lib/documentVisibility";
import { escapeLogHtml, tokenizeOutputLine } from "../../../shared/lib/logTokens";

const FLUSH_INTERVAL_MS = 100;
/** DOM 只保留当前浏览窗口（含上下余量）。 */
const RENDER_WINDOW_ROWS = 200;
/** 滚到窗口边缘时一次换窗的行数（换窗后用 scrollIntoView 锚住原位置）。 */
const RENDER_SHIFT_ROWS = 100;
/** 距窗口边缘多近触发换窗。 */
const WINDOW_EDGE_PX = 8;

export interface DockerLogViewOptions {
  scroller: Ref<HTMLElement | undefined>;
  /** 跟随态由 UI 拥有；换窗/贴底逻辑会读写它。 */
  follow: Ref<boolean>;
}

export interface DockerLogView {
  /** 新完成的整行（流缓冲已按 1000 行裁剪）。 */
  appendLines(lines: readonly string[]): void;
  /** 清空视图与本地历史（清空按钮/tail 重启/切换容器）。 */
  reset(): void;
  /** 恢复跟随并跳到最新一屏。 */
  resume(): void;
  /** 暂停自动滚动（内容继续进 history）。 */
  pause(): void;
  /** 滚动事件入口（内部按帧合并）。 */
  handleScroll(): void;
  /** 页签重新可见时补一次置底（v-show 隐藏期间没有布局）。 */
  scrollToBottom(): void;
}

export function useDockerLogView(options: DockerLogViewOptions): DockerLogView {
  const scroller = options.scroller;
  const follow = options.follow;

  /** 已到行（上限 1000，供换窗/恢复使用）。 */
  let history: string[] = [];
  /** 当前窗口在 history 中的起始下标。 */
  let windowStart = 0;
  /** 已渲染行：span + 该行前面的换行分隔符（首行为 null）。 */
  let renderedRows: Array<{ span: HTMLSpanElement; sep: Text | null }> = [];
  let pendingLines: string[] = [];
  /** 本批未渲染行到达时，窗口是否正贴着 history 尾部。 */
  let pendingAtTail = false;
  /** 暂停窗口头部被 1000 行上限裁掉：下一拍重建到最旧的保留行。 */
  let windowResetPending = false;
  let flushTimer = 0;
  let flushQueued = false;
  let lastFlushAt = 0;
  /** 窗口隐藏期间只收行不动 DOM；复现时按 history 重建一屏。 */
  let renderSuspended = false;
  let scrollFrame = 0;

  function logRowHtml(line: string): string {
    let html = "";
    for (const token of tokenizeOutputLine(line)) {
      html += token.kind === "plain"
        ? escapeLogHtml(token.text)
        : `<span class="log-${token.kind}">${escapeLogHtml(token.text)}</span>`;
    }
    return html;
  }

  function createRow(line: string): HTMLSpanElement {
    const span = document.createElement("span");
    span.className = "docker-log-line";
    span.innerHTML = logRowHtml(line);
    return span;
  }

  function reset(): void {
    pendingLines = [];
    pendingAtTail = false;
    windowResetPending = false;
    history = [];
    windowStart = 0;
    renderedRows = [];
    if (flushTimer) {
      window.clearTimeout(flushTimer);
      flushTimer = 0;
    }
    flushQueued = false;
    const element = scroller.value;
    if (element) element.textContent = "";
  }

  /** 批量摘掉头部行：一次 splice 代替逐行 shift（后者每次搬移整个数组）。 */
  function dropHeadRows(count: number): void {
    if (count <= 0) return;
    const dropped = renderedRows.splice(0, Math.min(count, renderedRows.length));
    for (const row of dropped) {
      row.span.remove();
      row.sep?.remove();
    }
    const leading = renderedRows[0];
    if (leading?.sep) {
      leading.sep.remove();
      leading.sep = null;
    }
  }

  function appendRows(batch: readonly string[]): void {
    const element = scroller.value;
    if (!element || !batch.length) return;
    const fragment = document.createDocumentFragment();
    for (const line of batch) {
      let sep: Text | null = null;
      if (renderedRows.length > 0) {
        sep = document.createTextNode("\n");
        fragment.appendChild(sep);
      }
      const span = createRow(line);
      fragment.appendChild(span);
      renderedRows.push({ span, sep });
    }
    element.appendChild(fragment);
  }

  /** 尾部窗口起点（history 不足一屏时从 0 开始）。 */
  function tailWindowStart(): number {
    return Math.max(0, history.length - RENDER_WINDOW_ROWS);
  }

  /**
   * 只渲染 [start, start+200) 这一屏。`bottom` 用于跟随；`anchor` 用于换窗后
   * 把原来看到的那行放回原来的屏幕位置（scrollIntoView 走最近的滚动祖先）。
   */
  function renderWindow(
    start: number,
    scroll: "bottom" | "none" = "none",
    anchor?: { el: HTMLSpanElement; block: "start" | "end" },
  ): void {
    const element = scroller.value;
    if (!element) return;
    const from = Math.min(Math.max(0, start), tailWindowStart());
    windowStart = from;
    renderedRows = [];
    element.textContent = "";
    appendRows(history.slice(from, from + RENDER_WINDOW_ROWS));
    if (scroll === "bottom") {
      element.scrollTop = element.scrollHeight;
      return;
    }
    if (anchor && anchor.el.isConnected && typeof anchor.el.scrollIntoView === "function") {
      anchor.el.scrollIntoView({ block: anchor.block, inline: "nearest" });
    }
  }

  function appendLines(lines: readonly string[]): void {
    if (!lines.length) return;
    // 「贴尾」必须在入缓冲（可能触发裁头）之前判定：之后的 history.length
    // 已不再代表本批到达前的窗口位置。
    if (!pendingLines.length) pendingAtTail = windowStart + renderedRows.length === history.length;
    history.push(...lines);
    const overflow = history.length - MAX_LOG_LINES;
    if (overflow > 0) {
      history.splice(0, overflow);
      const oldStart = windowStart;
      windowStart = Math.max(0, oldStart - overflow);
      // 暂停时被裁掉的正是窗口头部：正在看的内容已被 1000 行上限淘汰，
      // 下一拍重建一屏最旧的保留行，避免 DOM 越删越少。
      if (!follow.value && overflow > oldStart) windowResetPending = true;
    }
    pendingLines.push(...lines);
    if (pendingLines.length > MAX_LOG_LINES) pendingLines.splice(0, pendingLines.length - MAX_LOG_LINES);
    scheduleFlush();
  }

  /** 渲染节拍与分片速率解耦：空闲后的首批立即上屏，连续输出最多 10 次/秒。 */
  function scheduleFlush(): void {
    if (flushQueued) return;
    flushQueued = true;
    const wait = Math.max(0, FLUSH_INTERVAL_MS - (performance.now() - lastFlushAt));
    if (wait === 0) {
      queueMicrotask(runFlush);
      return;
    }
    flushTimer = window.setTimeout(runFlush, wait);
  }

  function runFlush(): void {
    flushQueued = false;
    if (flushTimer) {
      window.clearTimeout(flushTimer);
      flushTimer = 0;
    }
    lastFlushAt = performance.now();
    flushPending();
  }

  function flushPending(): void {
    if (!pendingLines.length) return;
    // 隐藏窗口不做 DOM/布局工作；行仍进 history（上限 1000），复现时重建。
    if (renderSuspended || documentHidden()) {
      pendingLines = [];
      pendingAtTail = false;
      return;
    }
    const element = scroller.value;
    if (!element) {
      pendingLines = [];
      return;
    }
    const batch = pendingLines.splice(0);
    const atTail = pendingAtTail;
    pendingAtTail = false;
    if (follow.value) {
      // 尾部小批次追加后裁头；整屏以上的突发直接重建尾部窗口，
      // 避免先插 1000 行再删 800 行。
      if (atTail && batch.length < RENDER_WINDOW_ROWS) {
        appendRows(batch);
        const excess = renderedRows.length - RENDER_WINDOW_ROWS;
        if (excess > 0) dropHeadRows(excess);
        windowStart = Math.max(0, history.length - renderedRows.length);
        element.scrollTop = element.scrollHeight;
        return;
      }
      renderWindow(tailWindowStart(), "bottom");
      return;
    }
    // 暂停：只有窗口本就在尾部且还有空位时才追加；否则保持用户正在看的
    // 内容不动（新行留在 history，滚到窗口底部时再换窗）。
    if (atTail && renderedRows.length + batch.length <= RENDER_WINDOW_ROWS) {
      appendRows(batch);
    }
    if (windowResetPending) {
      windowResetPending = false;
      renderWindow(Math.min(windowStart, tailWindowStart()), "none");
      return;
    }
    if (!renderedRows.length) renderWindow(Math.min(windowStart, tailWindowStart()), "none");
  }

  function resume(): void {
    // 已在跟随时不再重建（滚动事件每次贴底都会走到这里；重建会触发新的
    // scroll 事件，形成「重建 → 滚到底 → 重建」的自激回路）。
    if (follow.value) return;
    follow.value = true;
    windowResetPending = false;
    renderWindow(tailWindowStart(), "bottom");
  }

  function pause(): void {
    if (follow.value) follow.value = false;
  }

  function evaluateScroll(): void {
    const element = scroller.value;
    if (!element) return;
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
    if (distance > FOLLOW_PAUSE_PX) pause();
    if (follow.value) return;
    // 暂停：接近窗口顶部/底部时换一屏（历史最多 1000 行，DOM 始终一屏）。
    if (element.scrollTop <= WINDOW_EDGE_PX && windowStart > 0) {
      const anchor = renderedRows[0]?.span;
      renderWindow(
        Math.max(0, windowStart - RENDER_SHIFT_ROWS),
        "none",
        anchor ? { el: anchor, block: "start" } : undefined,
      );
      return;
    }
    if (distance <= FOLLOW_RESUME_PX) {
      // 窗口后面还有新行就先换窗，已经在最新一屏才恢复跟随。
      if (windowStart + renderedRows.length < history.length) {
        const anchor = renderedRows.at(-1)?.span;
        renderWindow(
          Math.min(tailWindowStart(), windowStart + RENDER_SHIFT_ROWS),
          "none",
          anchor ? { el: anchor, block: "end" } : undefined,
        );
        if (windowStart + renderedRows.length >= history.length) resume();
        return;
      }
      resume();
    }
  }

  function handleScroll(): void {
    if (scrollFrame) return;
    if (typeof requestAnimationFrame !== "function") {
      evaluateScroll();
      return;
    }
    scrollFrame = requestAnimationFrame(() => {
      scrollFrame = 0;
      evaluateScroll();
    });
  }

  function scrollToBottom(): void {
    const element = scroller.value;
    if (element) element.scrollTop = element.scrollHeight;
  }

  renderSuspended = documentHidden();
  const stopVisibilityWatch = onDocumentVisibilityChange((hidden) => {
    renderSuspended = hidden;
    if (hidden) return;
    pendingLines = [];
    pendingAtTail = false;
    windowResetPending = false;
    renderWindow(
      follow.value ? tailWindowStart() : Math.min(windowStart, tailWindowStart()),
      follow.value ? "bottom" : "none",
    );
  });

  onScopeDispose(() => {
    if (scrollFrame) cancelAnimationFrame(scrollFrame);
    if (flushTimer) window.clearTimeout(flushTimer);
    stopVisibilityWatch();
  });

  return { appendLines, reset, resume, pause, handleScroll, scrollToBottom };
}
