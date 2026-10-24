<script setup lang="ts">
/**
 * Secondary terminal pane inside one workbench.
 *
 * It is not a second SSH connection: the composable opens another PTY channel
 * on the main pane's already-authenticated transport, so bastion MFA is not
 * replayed. The xterm renderer lives in useSplitTerminalRenderer, clipboard
 * menu actions in useSplitClipboard, and session/frame/input ordering lives in
 * useSplitTerminal; this component keeps the pane chrome, AI/search state and
 * the disposal interleaving.
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ArrowLeftRight, ArrowUpDown, Eraser, FileUp, Loader2, Plus, Search, ShieldCheck, TextSelect } from "@lucide/vue";
import { ContextMenu, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "../../../shared/ui/context-menu/index";
import SplitTerminalHeader from "./SplitTerminalHeader.vue";
import TerminalContextMenu, { type CtxSearchEngine } from "./TerminalContextMenu.vue";
import TerminalSearchPanel from "./TerminalSearchPanel.vue";
import { useSplitClipboard } from "../composables/useSplitClipboard";
import { useSplitSearch } from "../composables/useSplitSearch";
import { useSplitTerminal } from "../composables/useSplitTerminal";
import { useSplitTerminalRenderer } from "../composables/useSplitTerminalRenderer";
import { useTerminalAiSearch } from "../composables/useTerminalAiSearch";
import { Osc7DirectoryParser } from "../lib/terminalDirectoryTracking";
import { Osc633CommandParser } from "../lib/terminalCommandMarkers";
import { applyLineEditControlChar } from "../lib/terminalLineModel";
import { PromptNewlineTransformer } from "../lib/terminalPromptNewline";
import { aiBridgeAvailable } from "../../agent/lib/aiBridge";
import { loadAiSettings } from "../../agent/lib/aiSettings";

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
  /** 在线搜索引擎表（与主终端共用设置）。 */
  engines?: CtxSearchEngine[];
  /** 会话可写/连接态/传输占用：用于对齐主终端菜单的禁用态。 */
  connected?: boolean;
  canWrite?: boolean;
  transferBusy?: boolean;
  /** 布局变化计数：分屏/关闭让宿主搬到新槽位后重新 fit。 */
  layoutEpoch?: number;
  /** 工作台通知（AI 结果回填/失败提示），与主终端共用同一 toast 通道。 */
  notify?: (message: string) => void;
}

const props = withDefaults(defineProps<Props>(), {
  showHeader: true,
  active: true,
  canCreateTab: false,
  canSplit: false,
  closeKind: "split",
  engines: () => [],
  connected: false,
  canWrite: false,
  transferBusy: false,
  layoutEpoch: 0,
});

const emit = defineEmits<{
  opened: [sessionId: string];
  closed: [];
  error: [message: string];
  "new-tab": [];
  split: [orientation: "row" | "column"];
  "search-online": [engine: CtxSearchEngine, text: string];
  /** Shell 提示钩子上报的 OSC 7 目录；真实目录变化时驱动 SFTP 跟随。 */
  cwd: [path: string];
}>();

const t = (key: string, values: Record<string, string | number> = {}) => props.t(key, values);
const closeLabel = computed(() => t(props.closeKind === "tab" ? "terminalTab.close" : "terminalSplit.close"));

const rootEl = ref<HTMLElement | null>(null);
const terminalHost = ref<HTMLElement | null>(null);
const menuOpen = ref(false);

// xterm 实例、附加组件、主题/字体应用与 fit 链路由渲染器组合式持有；
// 本组件只通过 getter 读写，dispose 时由本组件决定与 PTY 关闭的交错顺序。
const {
  hasSelection,
  getTerminal,
  getSearchAddon,
  createTerminal,
  measureDimensions,
  scheduleFit,
  observeSize,
  focus: focusTerminal,
  resetTerminal,
  syncSelection,
  isDisposed,
  dispose: disposeRenderer,
} = useSplitTerminalRenderer({
  getHost: () => terminalHost.value,
  getTheme: () => props.theme,
  getFontFamily: () => props.fontFamily,
  getFontSize: () => props.fontSize,
  getActive: () => props.active,
  getLayoutEpoch: () => props.layoutEpoch,
  onData: (data) => routeTerminalData(data),
  onOpenSearch: () => openSplitSearch(),
  onSearchResults: (change) => handleSplitSearchResults(change),
  onResize: (cols, rows) => resizeSplitSession(cols, rows),
});

// 分屏自己的搜索面板：状态与 SearchAddon 接线由 useSplitSearch 持有，
// addon 实例仍由渲染器创建/销毁，保持原有 dispose 顺序。
const {
  open: searchOpen,
  seedQuery: searchSeedQuery,
  seedOptions: searchSeedOptions,
  matchState: searchMatchState,
  resultIndex: searchResultIndex,
  resultCount: searchResultCount,
  openSearch: openSplitSearch,
  closeSearch: closeSplitSearch,
  clearSearch: clearSplitSearch,
  run: runSplitSearch,
  handleResultsChange: handleSplitSearchResults,
} = useSplitSearch({
  getTerminal,
  getSearchAddon,
  closeMenu: () => (menuOpen.value = false),
});

// `#` AI 命令搜索：状态机/锚点/直连生成与主终端共用同一 composable。
// 本地行缓冲只在“空行键入 #”时激活，不依赖远端回显。
let lineInput = "";
let currentCwd = "";
let commandRunning = false;
const commandParser = new Osc633CommandParser();

const aiSearch = useTerminalAiSearch({
  terminal: getTerminal,
  getHost: () => terminalHost.value,
  t,
  getLine: () => lineInput,
  // 会话未连上时不截留 #（sendInput 会丢弃，卡片却已出现）。
  isEnabled: () => status.value === "connected" && aiBridgeAvailable(window.dbxPlugin) && loadAiSettings().search,
  gates: () => ({
    alternateActive: getTerminal()?.buffer.active.type === "alternate",
    commandRunning,
    transferBusy: props.transferBusy,
    suggestionOpen: false,
    completionOpen: false,
    historyPanelOpen: false,
    quickSelectOpen: false,
    searchOpen: searchOpen.value,
  }),
  replaceLine: (command) => {
    // 与主终端一致：Ctrl+U 清掉 shell 当前输入行，再写入命令但不回车。
    sendInput("\u0015" + command);
    lineInput = command;
    getTerminal()?.focus();
  },
  notify: (message) => props.notify?.(message),
  context: () => ({
    locale: props.locale,
    ...(props.connectionId ? { connectionId: props.connectionId } : {}),
    ...(currentCwd ? { cwd: currentCwd } : {}),
  }),
});

const {
  state: aiSearchState,
  pending: aiSearchPending,
  pendingQuery: aiSearchPendingQuery,
  anchor: aiSearchAnchor,
  barEl: aiSearchBarEl,
  barStyle: aiSearchBarStyle,
} = aiSearch;

/** 本地行模型（与主终端同口径）：回车/Ctrl+C 清空、退格回删、Ctrl+U/W 行编辑；
 *  ESC 序列会整行改写，模型不追（与 App 的 trackPendingInput 一致）。 */
function trackLineInput(data: string) {
  if (data.includes("\u001b")) return;
  for (const character of data) {
    if (character === "\r" || character === "\n" || character === "\u0003") lineInput = "";
    else if (character === "\u007f") lineInput = lineInput.slice(0, -1);
    else if (character === "\t") continue;
    else {
      const edited = applyLineEditControlChar(lineInput, character);
      if (edited !== null) lineInput = edited;
      else if (character >= " ") lineInput += character;
    }
  }
}

function routeTerminalData(data: string) {
  if (aiSearch.handleData(data)) return;
  trackLineInput(data);
  sendInput(data);
}

// 分屏会话同样挂着 per-prompt 目录钩子；OSC 7 在输出流里，按窗格解析后上报，
// 由 App 在目录真实变化时驱动 SFTP 跟随。
const directoryParser = new Osc7DirectoryParser();
// 与主终端同款提示符换行修复：OSC 7 提示帧到达时若光标不在行首，先补 CRLF。
// 目录钩子安装时 sidecar 会丢弃安装行回显并补清行，但远端 dotfile 自带 OSC 7
// 的一次提示符双发射、以及行尾无换行的输出，仍需要在前端兜一层。
const promptNewline = new PromptNewlineTransformer();

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
  onData: (data) => {
    // 全屏应用（vim/top 绝对定位重绘）不能插入换行；解析链仍吃原始字节。
    const term = getTerminal();
    const suppressInsert = term?.buffer?.active?.type === "alternate";
    term?.write(promptNewline.push(data, { suppressInsert }));
    for (const path of directoryParser.push(data)) {
      currentCwd = path;
      emit("cwd", path);
    }
    // shell integration 的 OSC 633 命令生命周期：# 在命令运行中/REPL 里不截留。
    const marker = commandParser.push(data);
    if (marker.commandActive !== undefined) commandRunning = marker.commandActive;
  },
  onOpened: (sessionId) => {
    emit("opened", sessionId);
    // open 在途时窗格可能已重排（或字体刚加载）：连上后补一次 fit；尺寸与
    // spawn 相同会被 useSplitTerminal 的同尺寸去重挡掉，不会重绘提示符。
    scheduleFit();
  },
  onError: (message) => emit("error", message),
});

// 右键菜单的剪贴板/选区动作：只读写渲染器持有的 xterm，动作后恢复焦点。
const { copySelection, pasteClipboard, selectAllTerminal, clearTerminal, sendSudoRefresh } = useSplitClipboard({
  getTerminal,
  sendInput,
  closeMenu: () => (menuOpen.value = false),
  isConnected: () => props.connected,
  isTransferBusy: () => props.transferBusy,
  syncSelection,
});

function searchSelectionOnline(engine: CtxSearchEngine) {
  menuOpen.value = false;
  emit("search-online", engine, getTerminal()?.getSelection() || "");
  getTerminal()?.focus();
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
  if (isDisposed() || !mainSessionId) return;
  closeSplitSession();
  resetTerminal();
  directoryParser.reset();
  promptNewline.reset();
  aiSearch.reset();
  commandParser.reset();
  commandRunning = false;
  lineInput = "";
  currentCwd = "";
  const dimensions = measureDimensions();
  await openSplitSession(dimensions.cols, dimensions.rows);
}

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

onMounted(async () => {
  createTerminal();
  observeSize();
  // 布局刚插入时字体/弹性尺寸可能还没稳定：等 Vue patch 完成并尽量等 webfont
  // 就绪（上限 300ms）再量，避免以 80×24 默认值或 fallback 字体度量 spawn 后
  // 又 fit 一次大尺寸 resize（远端 shell 收到 SIGWINCH 重绘提示符，表现为
  // 一行两个 `[user@host ~]#`）。
  try {
    await Promise.race([
      document.fonts?.ready ?? Promise.resolve(),
      new Promise((resolve) => window.setTimeout(resolve, 300)),
    ]);
  } catch {
    // 字体服务不可用：连续采样/去重仍然兜底。
  }
  await nextTick();
  if (isDisposed()) return;
  const dimensions = measureDimensions();
  void openSplitSession(dimensions.cols, dimensions.rows);
});

onBeforeUnmount(() => {
  aiSearch.reset();
  // 渲染器按原顺序停止计时器/监听，回调里先关 PTY，再销毁 addon 与 xterm。
  disposeRenderer(() => closeSplitSession());
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
    <SplitTerminalHeader
      v-if="showHeader"
      :status="status"
      :error-message="errorMessage"
      :close-label="closeLabel"
      :t="t"
      @close="onCloseClick"
    />
    <div class="split-terminal-body" @click="focusTerminal">
      <div ref="terminalHost" class="split-terminal-host terminal-host" />
      <!-- `#` AI 命令搜索：与本窗格光标行同位，输入只进本地状态机不落 PTY。 -->
      <div
        v-if="(aiSearchState.active || aiSearchPending) && aiSearchAnchor"
        ref="aiSearchBarEl"
        class="terminal-ai-search mono"
        :class="{ pending: aiSearchPending }"
        :style="aiSearchBarStyle"
        role="status"
      >
        <span class="terminal-ai-search-badge" aria-hidden="true">#</span>
        <template v-if="aiSearchPending">
          <span class="terminal-ai-search-query">{{ aiSearchPendingQuery }}</span>
          <span class="terminal-ai-search-keys terminal-ai-search-pending"><Loader2 class="spinning" aria-hidden="true" /> {{ t("aiSearch.pendingHint") }}<span aria-hidden="true"> · </span><kbd>Esc</kbd> {{ t("aiSearch.pendingEscHint") }}</span>
        </template>
        <template v-else>
          <span class="terminal-ai-search-query" :class="{ placeholder: !aiSearchState.query }">{{ aiSearchState.query || t("aiSearch.hintPlaceholder") }}</span>
          <span class="terminal-ai-search-keys"><kbd>Enter</kbd> {{ t("aiSearch.hintSubmit") }}<span aria-hidden="true"> · </span><kbd>Esc</kbd> {{ t("aiSearch.hintCancel") }}</span>
        </template>
      </div>
      <TerminalSearchPanel
        v-if="searchOpen"
        :locale="locale"
        :initial-query="searchSeedQuery"
        :initial-options="searchSeedOptions"
        :match-state="searchMatchState"
        :result-index="searchResultIndex"
        :result-count="searchResultCount"
        @find-next="(query, options) => runSplitSearch(query, options, 'next')"
        @find-previous="(query, options) => runSplitSearch(query, options, 'prev')"
        @clear="clearSplitSearch"
        @close="closeSplitSearch"
      />
      <div v-if="status === 'error'" class="split-terminal-error" role="alert">{{ errorMessage }}</div>
    </div>
  </section>
    </ContextMenuTrigger>
    <TerminalContextMenu
      :open="menuOpen"
      :has-selection="hasSelection"
      :can-paste="connected && !transferBusy"
      :engines="engines"
      :t="t"
      :close-label="closeLabel"
      @copy="copySelection"
      @paste="pasteClipboard"
      @search="searchSelectionOnline"
      @close="onCloseClick"
    >
      <ContextMenuItem @select="selectAllTerminal"><TextSelect />{{ t("terminalSelectAll") }}</ContextMenuItem>
      <ContextMenuItem @select="openSplitSearch"><Search />{{ t("terminalSearch.open") }}</ContextMenuItem>
      <ContextMenuItem @select="clearTerminal"><Eraser />{{ t("terminalClear") }}</ContextMenuItem>
      <ContextMenuItem :disabled="!connected || transferBusy" @select="sendSudoRefresh"><ShieldCheck />{{ t("sudoRefresh.title") }}</ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem :disabled="!canCreateTab" @select="onNewTab"><Plus />{{ t("terminalTab.new") }}</ContextMenuItem>
      <template v-if="canSplit">
        <ContextMenuItem @select="onSplit('row')"><ArrowLeftRight />{{ t("terminalSplit.leftRight") }}</ContextMenuItem>
        <ContextMenuItem @select="onSplit('column')"><ArrowUpDown />{{ t("terminalSplit.topBottom") }}</ContextMenuItem>
      </template>
      <ContextMenuSeparator />
      <ContextMenuItem disabled :title="t('terminalTab.transferMainOnly')"><FileUp />{{ t("zmodemUpload") }}</ContextMenuItem>
      <ContextMenuItem disabled :title="t('terminalTab.transferMainOnly')"><FileUp />{{ t("trzszUpload") }}</ContextMenuItem>
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
