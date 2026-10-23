<script setup lang="ts">
/**
 * Secondary terminal pane inside one workbench.
 *
 * It is not a second SSH connection: the composable opens another PTY channel
 * on the main pane's already-authenticated transport, so bastion MFA is not
 * replayed. This component owns only the xterm renderer and the pane chrome;
 * session/frame/input ordering lives in useSplitTerminal.
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Terminal as XTerm, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { ArrowLeftRight, ArrowUpDown, Eraser, Loader2, Plus, Terminal as TerminalIcon, TextSelect, X } from "@lucide/vue";
import { readClipboardText, writeClipboardText, type ClipboardDeps } from "../../../shared/lib/clipboardBridge";
import { ContextMenu, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "../../../shared/ui/context-menu/index";
import TerminalContextMenu from "./TerminalContextMenu.vue";
import { useSplitTerminal } from "../composables/useSplitTerminal";
import { SPLIT_DEFAULT_COLS, SPLIT_DEFAULT_ROWS } from "../lib/terminalSplit";

interface Props {
  connectionId: string;
  workbenchId: string;
  mainSessionId: string;
  /** xterm ITheme from the workbench appearance. */
  theme: Record<string, unknown>;
  fontFamily: string;
  fontSize: number;
  t: (key: string, values?: Record<string, string | number>) => string;
  locale: string;
  /** false = 作为页签内容使用，不渲染窗格自己的标题栏（页签条已展示）。 */
  showHeader?: boolean;
  /** 页签是否可见；切回时重新 fit 并聚焦。 */
  active?: boolean;
  /** 工作台还能新建终端页签（连接可复用且未达上限）。 */
  canCreateTab?: boolean;
  /** 当前窗格可切换左右/上下分屏（仅主工作区的分屏页）。 */
  canSplit?: boolean;
  /** 关闭动作对象：分屏页关闭分屏，页签关闭页签。 */
  closeKind?: "split" | "tab";
}

const props = withDefaults(defineProps<Props>(), {
  showHeader: true,
  active: true,
  canCreateTab: false,
  canSplit: false,
  closeKind: "split",
});

const emit = defineEmits<{
  opened: [sessionId: string];
  closed: [];
  error: [message: string];
  "new-tab": [];
  split: [orientation: "row" | "column"];
}>();

const t = (key: string, values: Record<string, string | number> = {}) => props.t(key, values);
const closeLabel = computed(() => t(props.closeKind === "tab" ? "terminalTab.close" : "terminalSplit.close"));

const rootEl = ref<HTMLElement | null>(null);
const terminalHost = ref<HTMLElement | null>(null);
const menuOpen = ref(false);
const hasSelection = ref(false);

let terminal: XTerm | undefined;
let selectionDisposable: { dispose(): void } | undefined;
let fitAddon: FitAddon | undefined;
let unicode11: Unicode11Addon | undefined;
let webLinks: WebLinksAddon | undefined;
let dataDisposable: { dispose(): void } | undefined;
let resizeObserver: ResizeObserver | undefined;
let fitTimer = 0;
let disposed = false;
let appliedTheme: Record<string, unknown> | undefined;

const {
  status,
  errorMessage,
  open: openSplitSession,
  close: closeSplitSession,
  sendInput,
  resize: resizeSplitSession,
} = useSplitTerminal({
  connectionId: () => props.connectionId,
  workbenchId: () => props.workbenchId,
  mainSessionId: () => props.mainSessionId,
  t: (key, values) => props.t(key, values),
  onData: (data) => terminal?.write(data),
  onOpened: (sessionId) => emit("opened", sessionId),
  onError: (message) => emit("error", message),
});

function openTerminalLink(_event: MouseEvent, uri: string) {
  // Same hardening as the main terminal: never let the opened page navigate
  // its opener (reverse tabnabbing).
  const opened = window.open();
  if (!opened) return;
  try {
    opened.opener = null;
  } catch {
    // Some desktop webviews refuse opener writes; ignore like the built-in handler.
  }
  opened.location.href = uri;
}

function createTerminal() {
  if (!terminalHost.value || terminal) return;
  appliedTheme = { ...props.theme };
  terminal = new XTerm({
    convertEol: false,
    cursorBlink: true,
    fontFamily: props.fontFamily,
    fontSize: props.fontSize,
    scrollback: 5_000,
    allowProposedApi: true,
    theme: props.theme as ITheme,
  });
  fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);
  terminal.open(terminalHost.value);
  // Same width/emoji handling as the primary terminal: load after open().
  unicode11 = new Unicode11Addon();
  terminal.loadAddon(unicode11);
  terminal.unicode.activeVersion = "11";
  webLinks = new WebLinksAddon(openTerminalLink);
  terminal.loadAddon(webLinks);
  dataDisposable = terminal.onData((data) => sendInput(data));
  selectionDisposable = terminal.onSelectionChange(() => {
    hasSelection.value = terminal?.hasSelection() ?? false;
  });
}

/** Measured spawn size. Falls back to 80x24 until the pane is laid out. */
function measureDimensions(): { cols: number; rows: number } {
  const host = terminalHost.value;
  if (terminal && fitAddon && host && host.clientWidth > 0 && host.clientHeight > 0) {
    try {
      fitAddon.fit();
    } catch {
      // The iframe can be detached during a tab switch; keep xterm's size.
    }
  }
  const cols = terminal?.cols ?? 0;
  const rows = terminal?.rows ?? 0;
  return {
    cols: cols > 0 ? cols : SPLIT_DEFAULT_COLS,
    rows: rows > 0 ? rows : SPLIT_DEFAULT_ROWS,
  };
}

function scheduleFit() {
  if (disposed) return;
  window.clearTimeout(fitTimer);
  fitTimer = window.setTimeout(() => {
    if (disposed || !terminal || !fitAddon) return;
    const host = terminalHost.value;
    if (!host?.clientWidth || !host.clientHeight) return;
    try {
      fitAddon.fit();
    } catch {
      return;
    }
    resizeSplitSession(terminal.cols, terminal.rows);
  }, 20);
}

function observeSize() {
  if (!terminalHost.value || typeof ResizeObserver === "undefined") return;
  resizeObserver = new ResizeObserver(() => scheduleFit());
  resizeObserver.observe(terminalHost.value);
}

function focusTerminal() {
  terminal?.focus();
}

function clipboardDeps(): ClipboardDeps {
  return {
    bridge: window.dbxPlugin?.clipboard ?? null,
    nativeClipboard: typeof navigator !== "undefined" ? navigator.clipboard ?? null : null,
  };
}

function copySelection() {
  const text = terminal?.getSelection() ?? "";
  if (!text) return;
  void writeClipboardText(text, clipboardDeps()).catch(() => undefined);
  terminal?.focus();
}

async function pasteClipboard() {
  try {
    const text = await readClipboardText(clipboardDeps());
    if (!text) return;
    // xterm 的 paste 会按 bracketed-paste 模式包装远端需要的转义序列。
    terminal?.paste(text);
  } catch {
    // 沙箱拒绝读剪贴板时保持静默；键盘 Cmd/Ctrl+V 仍走浏览器原生 paste。
  } finally {
    terminal?.focus();
  }
}

function selectAllTerminal() {
  terminal?.selectAll();
  hasSelection.value = terminal?.hasSelection() ?? false;
  terminal?.focus();
}

function clearTerminal() {
  terminal?.clear();
  terminal?.focus();
}

function onCloseClick() {
  menuOpen.value = false;
  closeSplitSession();
  emit("closed");
}

function onNewTab() {
  emit("new-tab");
}

function onSplit(orientation: "row" | "column") {
  emit("split", orientation);
}

async function restartForMainSession(mainSessionId: string) {
  if (disposed || !mainSessionId) return;
  closeSplitSession();
  terminal?.reset();
  const dimensions = measureDimensions();
  await openSplitSession(dimensions.cols, dimensions.rows);
}

function sameTheme(next: Record<string, unknown>, previous: Record<string, unknown> | undefined): boolean {
  if (next === previous) return true;
  if (!previous) return false;
  const keys = Object.keys(next);
  if (keys.length !== Object.keys(previous).length) return false;
  return keys.every((key) => next[key] === previous[key]);
}

watch(
  () => props.theme,
  (theme) => {
    if (!terminal || sameTheme(theme, appliedTheme)) return;
    appliedTheme = { ...theme };
    terminal.options.theme = theme as ITheme;
    scheduleFit();
  },
  { deep: true },
);

watch(
  () => [props.fontFamily, props.fontSize] as const,
  ([fontFamily, fontSize]) => {
    if (!terminal) return;
    terminal.options.fontFamily = fontFamily;
    terminal.options.fontSize = fontSize;
    scheduleFit();
  },
);

// A new main session means the old shared transport is gone: close the split
// PTY (never leave it attached to a dead transport) and open a fresh channel
// on the new authenticated session.
watch(
  () => props.mainSessionId,
  (next, previous) => {
    if (next === previous) return;
    void restartForMainSession(next);
  },
);

// 页签切换：隐藏期间容器尺寸为 0，重新显示后必须 fit 一次并聚焦。
watch(
  () => props.active,
  async (active) => {
    if (!active || disposed) return;
    await nextTick();
    scheduleFit();
    terminal?.focus();
  },
);

onMounted(() => {
  createTerminal();
  observeSize();
  const dimensions = measureDimensions();
  void openSplitSession(dimensions.cols, dimensions.rows);
});

onBeforeUnmount(() => {
  disposed = true;
  window.clearTimeout(fitTimer);
  resizeObserver?.disconnect();
  resizeObserver = undefined;
  dataDisposable?.dispose();
  dataDisposable = undefined;
  selectionDisposable?.dispose();
  selectionDisposable = undefined;
  closeSplitSession();
  fitAddon?.dispose();
  fitAddon = undefined;
  unicode11?.dispose();
  unicode11 = undefined;
  webLinks?.dispose();
  webLinks = undefined;
  terminal?.dispose();
  terminal = undefined;
});

defineExpose({
  focus: focusTerminal,
});
</script>

<template>
  <ContextMenu @update:open="(open) => (menuOpen = open)">
    <ContextMenuTrigger as-child>
  <section
    ref="rootEl"
    class="split-terminal-pane"
    role="region"
    :aria-label="t('terminalSplit.title')"
    :aria-busy="status === 'connecting'"
  >
    <header v-if="showHeader" class="split-terminal-header">
      <span class="split-terminal-title">
        <TerminalIcon aria-hidden="true" />
        <span class="split-terminal-title-text">{{ t("terminalSplit.title") }}</span>
      </span>
      <span class="split-terminal-spacer" aria-hidden="true" />
      <span v-if="status === 'connecting'" class="split-terminal-status" role="status">
        <Loader2 class="spinning" aria-hidden="true" />
        <span>{{ t("sessionStatus.connecting") }}</span>
      </span>
      <span v-else-if="status === 'error'" class="split-terminal-status is-error" :title="errorMessage">
        {{ errorMessage }}
      </span>
      <button
        v-if="canCreateTab"
        type="button"
        class="split-terminal-add"
        :title="t('terminalTab.new')"
        :aria-label="t('terminalTab.new')"
        @click="onNewTab"
      >
        <Plus aria-hidden="true" />
      </button>
      <button
        type="button"
        class="split-terminal-close"
        :title="closeLabel"
        :aria-label="closeLabel"
        @click="onCloseClick"
      >
        <X aria-hidden="true" />
      </button>
    </header>
    <div class="split-terminal-body" @click="focusTerminal">
      <div ref="terminalHost" class="split-terminal-host terminal-host" />
      <div v-if="status === 'error'" class="split-terminal-error" role="alert">{{ errorMessage }}</div>
    </div>
  </section>
    </ContextMenuTrigger>
    <TerminalContextMenu
      :open="menuOpen"
      :has-selection="hasSelection"
      :can-paste="true"
      :engines="[]"
      :t="t"
      :close-label="closeLabel"
      @copy="copySelection"
      @paste="pasteClipboard"
      @close="onCloseClick"
    >
      <ContextMenuItem @select="selectAllTerminal"><TextSelect />{{ t("terminalSelectAll") }}</ContextMenuItem>
      <ContextMenuItem @select="clearTerminal"><Eraser />{{ t("terminalClear") }}</ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem :disabled="!canCreateTab" @select="onNewTab"><Plus />{{ t("terminalTab.new") }}</ContextMenuItem>
      <template v-if="canSplit">
        <ContextMenuItem @select="onSplit('row')"><ArrowLeftRight />{{ t("terminalSplit.leftRight") }}</ContextMenuItem>
        <ContextMenuItem @select="onSplit('column')"><ArrowUpDown />{{ t("terminalSplit.topBottom") }}</ContextMenuItem>
      </template>
    </TerminalContextMenu>
  </ContextMenu>
</template>

<style scoped>
/* The workspace itself (split-row/column) lives in the global terminal
   layout; this pane only fills its half and stacks a compact header over the
   xterm body. */
.split-terminal-pane {
  display: flex;
  min-width: 0;
  min-height: 0;
  height: 100%;
  flex-direction: column;
  overflow: hidden;
  background: var(--ssh-terminal-background);
  color: var(--foreground);
}

.split-terminal-header {
  display: flex;
  height: 24px;
  flex: 0 0 24px;
  align-items: center;
  gap: 6px;
  border-bottom: 1px solid var(--border);
  padding: 0 4px 0 8px;
  background: var(--background);
  color: var(--muted-foreground);
  font-size: 11px;
  line-height: 1;
  user-select: none;
}

.split-terminal-title {
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 5px;
  overflow: hidden;
  font-weight: 600;
  white-space: nowrap;
}

.split-terminal-title-text {
  overflow: hidden;
  text-overflow: ellipsis;
}

.split-terminal-title svg {
  width: 12px;
  height: 12px;
  flex: 0 0 auto;
  stroke-width: 1.8;
}

.split-terminal-spacer {
  flex: 1 1 auto;
}

.split-terminal-status {
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 4px;
  overflow: hidden;
  color: var(--muted-foreground);
  font-size: 10.5px;
  white-space: nowrap;
}

.split-terminal-status > span {
  overflow: hidden;
  text-overflow: ellipsis;
}

.split-terminal-status.is-error {
  color: var(--destructive);
}

.split-terminal-status svg {
  width: 11px;
  height: 11px;
  flex: 0 0 auto;
}

.split-terminal-add,
.split-terminal-close {
  display: inline-grid;
  width: 18px;
  height: 18px;
  flex: 0 0 18px;
  place-items: center;
  border: 0;
  border-radius: 4px;
  padding: 0;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
}

.split-terminal-add:hover,
.split-terminal-close:hover {
  background: var(--accent);
  color: var(--accent-foreground);
}

.split-terminal-add svg,
.split-terminal-close svg {
  width: 12px;
  height: 12px;
  stroke-width: 1.8;
}

.split-terminal-body {
  position: relative;
  min-height: 0;
  flex: 1 1 auto;
  overflow: hidden;
}

.split-terminal-host {
  position: absolute;
  inset: 0;
}

.split-terminal-error {
  position: absolute;
  z-index: 2;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: var(--ssh-terminal-background);
  color: var(--destructive);
  font-size: 12px;
  text-align: center;
}
</style>
