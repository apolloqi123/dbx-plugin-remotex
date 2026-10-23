<script setup lang="ts">
/**
 * 递归终端布局：split 节点渲染两个子节点，叶子渲染自己的标签栏和终端表面。
 * 主叶子的第一页是 App 注入的主终端（#main 插槽），其余页与分屏页都是
 * 独立 PTY；每个窗格的 “+” 只作用于该窗格。
 */
import { computed } from "vue";
import SplitTerminalPane from "./SplitTerminalPane.vue";
import TerminalPaneTabs from "./TerminalPaneTabs.vue";
import type { CtxSearchEngine } from "./TerminalContextMenu.vue";
import {
  TERMINAL_PANE_TAB_LIMIT,
  type TerminalLayoutNode,
  type TerminalLayoutOrientation,
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
}

const props = defineProps<Props>();

const emit = defineEmits<{
  opened: [tabId: string, sessionId: string];
  activateTab: [paneId: string, tabId: string];
  closeTab: [paneId: string, tabId: string];
  newTab: [paneId: string];
  closePane: [paneId: string];
  split: [paneId: string, orientation: TerminalLayoutOrientation];
  searchOnline: [engine: CtxSearchEngine, text: string];
}>();

const splitClass = computed(() => (props.node.kind === "split" ? (props.node.orientation === "row" ? "split-row" : "split-column") : ""));
const shellTabs = computed(() =>
  props.node.kind === "pane" ? props.node.tabs.filter((tab) => tab.kind === "shell") : [],
);
const canAddTab = computed(
  () => props.canCreateTab && props.node.kind === "pane" && props.node.tabs.length < TERMINAL_PANE_TAB_LIMIT,
);
const leafPaneId = computed(() => (props.node.kind === "pane" ? props.node.id : ""));
</script>

<template>
  <div v-if="node.kind === 'split'" class="terminal-layout-split" :class="splitClass">
    <TerminalLayoutTree
      v-bind="$props"
      :node="node.first"
      @opened="(tabId, sessionId) => emit('opened', tabId, sessionId)"
      @activate-tab="(paneId, tabId) => emit('activateTab', paneId, tabId)"
      @close-tab="(paneId, tabId) => emit('closeTab', paneId, tabId)"
      @new-tab="(paneId) => emit('newTab', paneId)"
      @close-pane="(paneId) => emit('closePane', paneId)"
      @split="(paneId, orientation) => emit('split', paneId, orientation)"
      @search-online="(engine, text) => emit('searchOnline', engine, text)"
    >
      <template #main><slot name="main" /></template>
    </TerminalLayoutTree>
    <TerminalLayoutTree
      v-bind="$props"
      :node="node.second"
      @opened="(tabId, sessionId) => emit('opened', tabId, sessionId)"
      @activate-tab="(paneId, tabId) => emit('activateTab', paneId, tabId)"
      @close-tab="(paneId, tabId) => emit('closeTab', paneId, tabId)"
      @new-tab="(paneId) => emit('newTab', paneId)"
      @close-pane="(paneId) => emit('closePane', paneId)"
      @split="(paneId, orientation) => emit('split', paneId, orientation)"
      @search-online="(engine, text) => emit('searchOnline', engine, text)"
    >
      <template #main><slot name="main" /></template>
    </TerminalLayoutTree>
  </div>
  <div v-else class="terminal-layout-pane" :class="{ 'is-main': node.paneKind === 'main' }">
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
      >
        <SplitTerminalPane
          :connection-id="connectionId"
          :workbench-id="workbenchId"
          :main-session-id="mainSessionId"
          :theme="theme"
          :font-family="fontFamily"
          :font-size="fontSize"
          :t="t"
          :locale="locale"
          :show-header="false"
          :active="node.activeTabId === tab.id"
          :can-create-tab="canCreateTab"
          :can-split="canSplit"
          :engines="engines"
          :connected="connected"
          :can-write="canWrite"
          :transfer-busy="transferBusy"
          close-kind="tab"
          @opened="(sessionId) => emit('opened', tab.id, sessionId)"
          @closed="emit('closeTab', leafPaneId, tab.id)"
          @new-tab="emit('newTab', leafPaneId)"
          @split="(orientation) => emit('split', leafPaneId, orientation)"
          @search-online="(engine, text) => emit('searchOnline', engine, text)"
        />
      </div>
    </div>
  </div>
</template>
