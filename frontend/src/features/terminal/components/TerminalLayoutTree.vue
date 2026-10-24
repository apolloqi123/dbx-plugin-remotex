<script setup lang="ts">
/**
 * 递归终端布局：split 节点渲染两个子节点，叶子渲染自己的标签栏和终端表面。
 * 主叶子的第一页是 App 注入的主终端（#main 插槽），其余页与分屏页都是
 * 独立 PTY；每个窗格的 “+” 只作用于该窗格。
 *
 * 分屏页签的终端实例由根实例统一持有（Teleport 到稳定的宿主元素，再把宿主
 * 搬进当前叶子槽位）：再次分屏只改布局树、不卸载终端组件，PTY/滚动缓冲/
 * 历史保持原样。若把 SplitTerminalPane 直接挂在递归叶子里，叶子变成 split
 * 节点的瞬间整棵子树会被重建，已有分屏会被当成新会话重新登录。
 */
import { computed, nextTick, onMounted, onUpdated, ref, watch } from "vue";
import SplitTerminalPane from "./SplitTerminalPane.vue";
import TerminalPaneTabs from "./TerminalPaneTabs.vue";
import type { CtxSearchEngine } from "./TerminalContextMenu.vue";
import {
  TERMINAL_PANE_TAB_LIMIT,
  type TerminalLayoutNode,
  type TerminalLayoutOrientation,
  type TerminalPaneLeaf,
  type TerminalPaneTab,
} from "../lib/terminalLayout";

interface Props {
  node: TerminalLayoutNode;
  connectionId: string;
  workbenchId: string;
  mainSessionId: string;
  theme: Record<string, unknown>;
  fontFamily: string;
  fontSize: number;
  t: (key: string, values?: Record<string, string | number>) => string;
  locale: string;
  canSplit: boolean;
  canCreateTab: boolean;
  engines: CtxSearchEngine[];
  connected: boolean;
  canWrite: boolean;
  transferBusy: boolean;
  /** 工作台通知：分屏页签里的 AI 结果/失败提示与主终端共用同一 toast。 */
  notify?: (message: string) => void;
  /** 0 = 布局树根实例（唯一持有分屏终端实例与宿主）。 */
  depth?: number;
}

const props = withDefaults(defineProps<Props>(), { depth: 0 });

const emit = defineEmits<{
  opened: [tabId: string, sessionId: string];
  activateTab: [paneId: string, tabId: string];
  closeTab: [paneId: string, tabId: string];
  newTab: [paneId: string];
  closePane: [paneId: string];
  split: [paneId: string, orientation: TerminalLayoutOrientation];
  searchOnline: [engine: CtxSearchEngine, text: string];
  /** 某个页签终端上报的 OSC 7 目录（tabId 用于区分同窗格/跨窗格页签）。 */
  cwd: [tabId: string, path: string];
  /** 用户在任何窗格内按下指针/聚焦：记录最后交互窗格，作为重新打开
   *  SFTP 面板时的目录跟随初始目标（不因聚焦本身触发跟随）。 */
  activatePane: [paneId: string];
}>();

interface ShellPaneEntry {
  tab: TerminalPaneTab;
  pane: TerminalPaneLeaf;
}

const isRoot = computed(() => props.depth === 0);
const splitClass = computed(() => (props.node.kind === "split" ? (props.node.orientation === "row" ? "split-row" : "split-column") : ""));
const shellTabs = computed(() =>
  props.node.kind === "pane" ? props.node.tabs.filter((tab) => tab.kind === "shell") : [],
);
const canAddTab = computed(
  () => props.canCreateTab && props.node.kind === "pane" && props.node.tabs.length < TERMINAL_PANE_TAB_LIMIT,
);
const leafPaneId = computed(() => (props.node.kind === "pane" ? props.node.id : ""));

// —— 根实例：分屏终端实例的稳定宿主 ——————————————————————————————
const rootEl = ref<HTMLElement | null>(null);
const layoutEpoch = ref(0);
const placedTabIds = ref<Set<string>>(new Set());
const hosts = new Map<string, HTMLElement>();

const shellPaneEntries = computed<ShellPaneEntry[]>(() => {
  const entries: ShellPaneEntry[] = [];
  const walk = (node: TerminalLayoutNode): void => {
    if (node.kind === "pane") {
      for (const tab of node.tabs) {
        if (tab.kind === "shell") entries.push({ tab, pane: node });
      }
      return;
    }
    walk(node.first);
    walk(node.second);
  };
  walk(props.node);
  return entries;
});

function ensureHost(tabId: string): HTMLElement {
  let host = hosts.get(tabId);
  if (!host) {
    host = document.createElement("div");
    host.className = "terminal-pane-host";
    host.dataset.terminalHost = tabId;
    hosts.set(tabId, host);
  }
  return host;
}

/**
 * 把每个分屏终端宿主搬进当前布局里对应的叶子槽位。槽位在递归叶子里，
 * 每次分屏/关闭都会重建；宿主元素本身保持同一个对象，Teleport 内容
 * （终端组件实例）因此不会被卸载。
 */
function placeHosts(): void {
  if (!isRoot.value) return;
  const root = rootEl.value;
  if (!root) return;
  const slots = new Map<string, HTMLElement>();
  root.querySelectorAll<HTMLElement>("[data-tab-slot]").forEach((el) => {
    const tabId = el.dataset.tabSlot;
    if (tabId) slots.set(tabId, el);
  });
  const nextPlaced = new Set<string>();
  let moved = false;
  for (const entry of shellPaneEntries.value) {
    const slot = slots.get(entry.tab.id);
    if (!slot) continue;
    const host = ensureHost(entry.tab.id);
    if (host.parentElement !== slot) {
      slot.appendChild(host);
      moved = true;
    }
    nextPlaced.add(entry.tab.id);
  }
  if (moved) layoutEpoch.value += 1;
  const same =
    nextPlaced.size === placedTabIds.value.size &&
    [...nextPlaced].every((id) => placedTabIds.value.has(id));
  if (!same) placedTabIds.value = nextPlaced;
}

onMounted(() => {
  if (isRoot.value) placeHosts();
});

onUpdated(() => {
  if (isRoot.value) placeHosts();
});

watch(shellPaneEntries, (entries) => {
  if (!isRoot.value) return;
  const live = new Set(entries.map((entry) => entry.tab.id));
  for (const [tabId, host] of hosts) {
    if (live.has(tabId)) continue;
    host.remove();
    hosts.delete(tabId);
  }
  void nextTick(placeHosts);
});
</script>

<template>
  <div v-if="node.kind === 'split'" ref="rootEl" class="terminal-layout-split" :class="splitClass">
    <TerminalLayoutTree
      v-bind="$props"
      :depth="depth + 1"
      :node="node.first"
      @opened="(tabId, sessionId) => emit('opened', tabId, sessionId)"
      @activate-tab="(paneId, tabId) => emit('activateTab', paneId, tabId)"
      @close-tab="(paneId, tabId) => emit('closeTab', paneId, tabId)"
      @new-tab="(paneId) => emit('newTab', paneId)"
      @close-pane="(paneId) => emit('closePane', paneId)"
      @split="(paneId, orientation) => emit('split', paneId, orientation)"
      @search-online="(engine, text) => emit('searchOnline', engine, text)"
      @cwd="(tabId, path) => emit('cwd', tabId, path)"
      @activate-pane="(paneId) => emit('activatePane', paneId)"
    >
      <template #main><slot name="main" /></template>
    </TerminalLayoutTree>
    <TerminalLayoutTree
      v-bind="$props"
      :depth="depth + 1"
      :node="node.second"
      @opened="(tabId, sessionId) => emit('opened', tabId, sessionId)"
      @activate-tab="(paneId, tabId) => emit('activateTab', paneId, tabId)"
      @close-tab="(paneId, tabId) => emit('closeTab', paneId, tabId)"
      @new-tab="(paneId) => emit('newTab', paneId)"
      @close-pane="(paneId) => emit('closePane', paneId)"
      @split="(paneId, orientation) => emit('split', paneId, orientation)"
      @search-online="(engine, text) => emit('searchOnline', engine, text)"
      @cwd="(tabId, path) => emit('cwd', tabId, path)"
      @activate-pane="(paneId) => emit('activatePane', paneId)"
    >
      <template #main><slot name="main" /></template>
    </TerminalLayoutTree>
  </div>
  <div
    v-else
    ref="rootEl"
    class="terminal-layout-pane"
    :class="{ 'is-main': node.paneKind === 'main' }"
    @pointerdown.capture="emit('activatePane', leafPaneId)"
    @focusin.capture="emit('activatePane', leafPaneId)"
  >
    <TerminalPaneTabs
      :tabs="node.tabs"
      :active-id="node.activeTabId"
      :t="t"
      :can-add="canAddTab"
      :can-close-pane="node.paneKind === 'split'"
      @activate="(tabId) => emit('activateTab', leafPaneId, tabId)"
      @close-tab="(tabId) => emit('closeTab', leafPaneId, tabId)"
      @new-tab="emit('newTab', leafPaneId)"
      @close-pane="emit('closePane', leafPaneId)"
    />
    <div class="terminal-pane-stack">
      <div
        v-if="node.paneKind === 'main'"
        v-show="node.activeTabId === 'main'"
        class="terminal-pane-surface"
      >
        <slot name="main" />
      </div>
      <div
        v-for="tab in shellTabs"
        :key="tab.id"
        v-show="node.activeTabId === tab.id"
        class="terminal-pane-surface"
        :data-tab-slot="tab.id"
      ></div>
    </div>
  </div>

  <!-- 根实例统一挂载所有分屏终端：宿主就位后才挂载，保证以真实尺寸启动。 -->
  <template v-if="depth === 0">
    <Teleport
      v-for="entry in shellPaneEntries"
      :key="`pane-${entry.tab.id}`"
      :to="ensureHost(entry.tab.id)"
    >
      <SplitTerminalPane
        v-if="placedTabIds.has(entry.tab.id)"
        :connection-id="connectionId"
        :workbench-id="workbenchId"
        :main-session-id="mainSessionId"
        :theme="theme"
        :font-family="fontFamily"
        :font-size="fontSize"
        :t="t"
        :locale="locale"
        :show-header="false"
        :active="entry.pane.activeTabId === entry.tab.id"
        :can-create-tab="canCreateTab"
        :can-split="canSplit"
        :engines="engines"
        :connected="connected"
        :can-write="canWrite"
        :transfer-busy="transferBusy"
        :notify="notify"
        :layout-epoch="layoutEpoch"
        close-kind="tab"
        @opened="(sessionId) => emit('opened', entry.tab.id, sessionId)"
        @closed="emit('closeTab', entry.pane.id, entry.tab.id)"
        @new-tab="emit('newTab', entry.pane.id)"
        @split="(orientation) => emit('split', entry.pane.id, orientation)"
        @search-online="(engine, text) => emit('searchOnline', engine, text)"
        @cwd="(path) => emit('cwd', entry.tab.id, path)"
      />
    </Teleport>
  </template>
</template>
