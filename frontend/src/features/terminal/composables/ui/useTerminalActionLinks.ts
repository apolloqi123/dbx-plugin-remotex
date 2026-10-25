import { computed, ref, watch, type Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import type { OutputGate } from "../../lib/emulator/terminalBackpressure";
import { shouldRebuildHighlightRow, toAbsoluteRowRange, matchesInLine, type CompiledHighlightRule } from "../../lib/decorations/keywordHighlight";
import { createActionLinkProvider } from "../../lib/decorations/actionLinksAddon";
import {
  matchActionLinks,
  sanitizeActionLinksSettings,
  type ActionLinkMatch,
  type ActionLinkMatcherToggles,
  type ActionLinksSettings,
} from "../../lib/decorations/actionLinksMatcher";
import { lineColumnTable, matchToColumns } from "../../lib/emulator/terminalLineColumns";

export interface TerminalActionLinksOptions {
  terminal: () => Terminal | undefined;
  terminalHost: Ref<HTMLElement | undefined>;
  compiledHighlightRules: Ref<CompiledHighlightRule[]>;
  outputGate: OutputGate;
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string) => void;
  session: Ref<{ sessionId: string } | null | undefined>;
  localSession: Ref<{ sessionId: string } | null | undefined>;
  serialSession: Ref<{ sessionId: string } | null | undefined>;
  telnetSession: Ref<{ sessionId: string } | null | undefined>;
  sendTerminalBytes: (data: Uint8Array) => void;
  replaceTerminalLineWith: (nextLine: string, pressEnter: boolean) => void;
  persistPrefs: (patch: Record<string, unknown>) => void;
}

/**
 * 主终端动作链接（P1-2，默认关闭）：link provider + 常驻虚线下划线 decoration +
 * 悬停/Alt+点击预览浮签。偏好权威态是设置 ref，经 update* 增量上抛由调用方持久化。
 */
export function useTerminalActionLinks(options: TerminalActionLinksOptions) {
  const {
    terminal: terminalGet,
    terminalHost,
    compiledHighlightRules,
    outputGate,
    t,
    showNotice,
    session,
    localSession,
    serialSession,
    telnetSession,
    sendTerminalBytes,
    replaceTerminalLineWith,
    persistPrefs,
  } = options;

// ---- 动作链接（P1-2）----
const actionLinksSettings = ref<ActionLinksSettings>(sanitizeActionLinksSettings(undefined));
const actionLinksEnabled = computed(() => actionLinksSettings.value.enabled);
// 与关键词高亮同帧率上限 / 同量级装饰总数护栏。
const ACTION_LINK_SCAN_MIN_INTERVAL_MS = 33;
const ACTION_LINK_DECORATION_LIMIT = 400;
let actionLinkProviderDisposable: { dispose(): void } | undefined;
let actionLinkRenderDisposable: { dispose(): void } | undefined;
let actionLinkScanScheduled = false;
let actionLinkLastScanAt = 0;
let actionLinkPendingRange: { start: number; end: number } | undefined;
// 每行一组（marker + 虚线 decorations + 登记文本）；去留条件复用关键词高亮的
// shouldRebuildHighlightRow——"本帧重绘且文本未变则整组保留"是防自激回路的
// 关键（xterm 在装饰注册/销毁后会再触发整幅重绘）。
const actionLinkDecorationsByRow = new Map<number, { text: string; dispose(): void }>();
let actionLinkDecorationCount = 0;
// 悬停 / Alt+点击的命令预览浮签（terminal-pane 内绝对定位，pointer-events 关）。
const actionLinkHint = ref<{ x: number; y: number; text: string } | null>(null);

function clearActionLinkDecorations() {
  for (const entry of actionLinkDecorationsByRow.values()) entry.dispose();
  actionLinkDecorationsByRow.clear();
  actionLinkDecorationCount = 0;
}

function attachActionLinks() {
  const term = terminalGet();
  if (!term || actionLinkProviderDisposable || !actionLinksEnabled.value) return;
  actionLinkProviderDisposable = term.registerLinkProvider(
    createActionLinkProvider(term, {
      matchers: actionLinksSettings.value.matchers,
      callbacks: {
        onActivate: handleActionLinkActivate,
        onHover: showActionLinkHintAt,
        onLeave: hideActionLinkHint,
      },
    }),
  );
  actionLinkRenderDisposable = term.onRender(({ start, end }) => scheduleActionLinkScan(start, end));
  scheduleActionLinkScan(0, term.rows - 1);
}

function detachActionLinks() {
  actionLinkProviderDisposable?.dispose();
  actionLinkProviderDisposable = undefined;
  actionLinkRenderDisposable?.dispose();
  actionLinkRenderDisposable = undefined;
  clearActionLinkDecorations();
  hideActionLinkHint();
}

// 点击 = 把建议命令送进现有 PTY 输入通路（不含换行：shell 输入行停在原地，
// 用户可补改后再回车执行）。Alt+点击 = 仅预览命令文本，不向 PTY 写入。
function handleActionLinkActivate(match: ActionLinkMatch, event: MouseEvent) {
  if (event.altKey) {
    showActionLinkHintAt(match, event);
    return;
  }
  const sessionId = localSession.value?.sessionId ?? session.value?.sessionId;
  if (!sessionId) return;
  sendTerminalBytes(new TextEncoder().encode(match.command));
  terminalGet()?.focus();
}

// ---- Docker 面板「在终端打开」（M3 遗留 6）----
// DockerPanel 挂在终端区右上浮层内，面板经 window 自定义事件把命令字符串直达
// 这里；走与建议浮层同款的「填入输入行不回车」
// 通道（replaceTerminalLineWith）：命令落在 shell 输入行原地，用户确认后自行回车。
// 无终端会话时 toast 提示先连接，不静默丢弃。
function handleDockerOpenInTerminal(event: Event) {
  const command = (event as CustomEvent<{ command?: string }>).detail?.command ?? "";
  if (!command) return;
  const term = terminalGet();
  const hasTerminalSession = Boolean(
    localSession.value ?? session.value ?? serialSession.value ?? telnetSession.value,
  );
  if (!term || !hasTerminalSession) {
    showNotice(t("docker.terminalNeedSession"));
    return;
  }
  replaceTerminalLineWith(command, false);
  term.focus();
}

function showActionLinkHintAt(match: ActionLinkMatch, event: MouseEvent) {
  const host = terminalHost.value;
  if (!host) return;
  const bounds = host.getBoundingClientRect();
  actionLinkHint.value = {
    x: Math.min(Math.max(event.clientX - bounds.left + 10, 4), Math.max(4, bounds.width - 280)),
    y: Math.max(4, event.clientY - bounds.top - 34),
    text: match.command,
  };
}

function hideActionLinkHint() {
  actionLinkHint.value = null;
}

function scheduleActionLinkScan(start: number, end: number) {
  if (!terminalGet() || !actionLinksEnabled.value) return;
  // 大输出保护生效期挂起扫描，恢复时由 onOutputGateRelease 补扫视口。
  if (outputGate.mode === "strained") return;
  actionLinkPendingRange = actionLinkPendingRange
    ? { start: Math.min(actionLinkPendingRange.start, start), end: Math.max(actionLinkPendingRange.end, end) }
    : { start, end };
  if (actionLinkScanScheduled) return;
  actionLinkScanScheduled = true;
  const wait = Math.max(0, ACTION_LINK_SCAN_MIN_INTERVAL_MS - (performance.now() - actionLinkLastScanAt));
  window.setTimeout(runActionLinkScan, wait);
}

function runActionLinkScan() {
  actionLinkScanScheduled = false;
  actionLinkLastScanAt = performance.now();
  const range = actionLinkPendingRange;
  actionLinkPendingRange = undefined;
  if (!range || !terminalGet() || !actionLinksEnabled.value) return;
  scanActionLinkRange(range.start, range.end);
}

function scanActionLinkRange(start: number, end: number) {
  const term = terminalGet();
  if (!term) return;
  const buffer = term.buffer.active;
  // onRender 的视口相对行号 → 缓冲绝对行号 + 视口判定，与关键词高亮同款换算。
  const dirty = toAbsoluteRowRange(start, end, buffer.viewportY, buffer.length);
  const vpFrom = Math.max(0, Math.min(buffer.viewportY, buffer.length - 1));
  const vpTo = Math.min(buffer.length - 1, vpFrom + term.rows - 1);
  for (const [row, entry] of actionLinkDecorationsByRow) {
    // 防自激：仅"滚出视口"或"本帧重绘且文本确实变化"才拆组重建。
    const dirtyRow = row >= dirty.from && row <= dirty.to;
    const currentText = dirtyRow ? buffer.getLine(row)?.translateToString(true) ?? "" : entry.text;
    if (!shouldRebuildHighlightRow({ row, viewportFrom: vpFrom, viewportTo: vpTo, dirty: dirtyRow, previousText: entry.text, currentText })) continue;
    entry.dispose();
    actionLinkDecorationsByRow.delete(row);
  }
  const matchers = actionLinksSettings.value.matchers;
  const keywordRules = compiledHighlightRules.value;
  const base = buffer.baseY + buffer.cursorY;
  for (let row = vpFrom; row <= vpTo; row++) {
    if (actionLinkDecorationsByRow.has(row)) continue;
    if (actionLinkDecorationCount >= ACTION_LINK_DECORATION_LIMIT) return;
    const bufferLine = buffer.getLine(row);
    const lineText = bufferLine?.translateToString(true) ?? "";
    if (!lineText) continue;
    const matches = matchActionLinks(lineText, matchers);
    if (!matches.length) continue;
    // 让位：与用户关键词高亮同段命中的范围跳过（高亮是用户显式配置的规则）。
    const keywordSpans = keywordRules.length ? matchesInLine(lineText, keywordRules) : [];
    const visible = keywordSpans.length
      ? matches.filter((match) => !keywordSpans.some((span) => match.start < span.end && match.end > span.start))
      : matches;
    if (!visible.length) continue;
    // 常驻虚线同样按单元格列号绘制（理由见关键词高亮处）；让位过滤仍在下标空间做，
    // 只比较命中区间的重叠关系，不需要换算。
    const columns = bufferLine ? lineColumnTable(bufferLine, lineText.length) : undefined;
    const marker = term.registerMarker(row - base);
    if (!marker) continue;
    const disposables: Array<{ dispose(): void }> = [marker];
    const entry = {
      text: lineText,
      decorations: 0,
      dispose() {
        for (const disposable of disposables.splice(0)) disposable.dispose();
        actionLinkDecorationCount -= entry.decorations;
        entry.decorations = 0;
      },
    };
    for (const match of visible) {
      if (actionLinkDecorationCount >= ACTION_LINK_DECORATION_LIMIT) break;
      const { x, width } = matchToColumns(columns, match.start, match.end);
      const decoration = term.registerDecoration({ marker, x, width });
      if (!decoration) continue;
      // 虚线下划线画在装饰元素下缘（装饰层在文字层上方，无填充不遮字形）。
      decoration.onRender((element) => {
        element.style.borderBottom = "1px dashed var(--primary)";
      });
      disposables.push(decoration);
      entry.decorations++;
      actionLinkDecorationCount++;
    }
    actionLinkDecorationsByRow.set(row, entry);
  }
}

// 设置变化（总开关或三类匹配器）：即时生效——provider 构造时快照 matchers，
// 任何变化都重建；关闭时零挂钩子（摘 provider + 清 decoration）。
watch(actionLinksSettings, (next) => {
  if (!terminalGet()) return;
  if (next.enabled) {
    actionLinkProviderDisposable?.dispose();
    actionLinkProviderDisposable = undefined;
    clearActionLinkDecorations();
    attachActionLinks();
  } else {
    detachActionLinks();
  }
}, { deep: true });

// 设置页增量上抛：归一化 → sidecar 持久化（watcher 即时挂/摘）。
function updateActionLinksSettings(patch: { enabled?: boolean; matchers?: Partial<ActionLinkMatcherToggles> }) {
  const next = sanitizeActionLinksSettings({
    enabled: patch.enabled ?? actionLinksSettings.value.enabled,
    matchers: { ...actionLinksSettings.value.matchers, ...(patch.matchers ?? {}) },
  });
  actionLinksSettings.value = next;
  void persistPrefs({
    action_links_enabled: next.enabled,
    action_links_matchers: { ipv4: next.matchers.ipv4, host_port: next.matchers.hostPort, archive: next.matchers.archive },
  });
}

  return {
    actionLinksSettings,
    actionLinksEnabled,
    actionLinkHint,
    attachActionLinks,
    detachActionLinks,
    handleActionLinkActivate,
    handleDockerOpenInTerminal,
    showActionLinkHintAt,
    hideActionLinkHint,
    scheduleActionLinkScan,
    updateActionLinksSettings,
  };
}
