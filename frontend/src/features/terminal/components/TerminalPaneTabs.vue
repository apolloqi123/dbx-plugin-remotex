<script setup lang="ts">
/**
 * 单个终端窗格的标签栏：标签 + 最后一个标签右侧的「+」。
 * 分屏窗格额外提供关闭窗格按钮；标签的 PTY 生命周期由父级按 tab id 管理。
 */
import { Plus, Terminal as TerminalIcon, X } from "@lucide/vue";
import type { TerminalPaneTab } from "../lib/appearance/terminalLayout";

interface Props {
  tabs: TerminalPaneTab[];
  activeId: string;
  t: (key: string, values?: Record<string, string | number>) => string;
  canAdd: boolean;
  canClosePane: boolean;
}

const props = defineProps<Props>();

const emit = defineEmits<{
  activate: [tabId: string];
  closeTab: [tabId: string];
  newTab: [];
  closePane: [];
}>();

function label(tab: TerminalPaneTab): string {
  return tab.kind === "main" ? props.t("terminalTab.main") : props.t("terminalTab.index", { index: tab.labelIndex });
}
</script>

<template>
  <div class="terminal-pane-tabs" role="tablist" :aria-label="t('terminalTab.tabs')">
    <div
      v-for="tab in tabs"
      :key="tab.id"
      class="terminal-pane-tab"
      :class="{ 'is-active': tab.id === activeId }"
    >
      <button
        type="button"
        class="terminal-pane-tab-label"
        role="tab"
        :aria-selected="tab.id === activeId"
        @click="emit('activate', tab.id)"
      >
        <TerminalIcon aria-hidden="true" />
        <span>{{ label(tab) }}</span>
      </button>
      <button
        v-if="tab.kind === 'shell'"
        type="button"
        class="terminal-pane-tab-close"
        :title="t('terminalTab.close')"
        :aria-label="t('terminalTab.close')"
        @click.stop="emit('closeTab', tab.id)"
      ><X aria-hidden="true" /></button>
    </div>
    <button
      type="button"
      class="terminal-pane-tab-add"
      :title="canAdd ? t('terminalTab.new') : t('terminalTab.limit', { count: tabs.length })"
      :aria-label="t('terminalTab.new')"
      :disabled="!canAdd"
      @click="emit('newTab')"
    ><Plus aria-hidden="true" /></button>
    <span class="terminal-pane-tabs-spacer" aria-hidden="true" />
    <button
      v-if="canClosePane"
      type="button"
      class="terminal-pane-close"
      :title="t('terminalSplit.close')"
      :aria-label="t('terminalSplit.close')"
      @click="emit('closePane')"
    ><X aria-hidden="true" /></button>
  </div>
</template>

<style scoped>
.terminal-pane-tabs {
  display: flex;
  height: 29px;
  flex: 0 0 29px;
  align-items: center;
  gap: 3px;
  overflow-x: auto;
  overflow-y: hidden;
  border-bottom: 1px solid var(--border);
  padding: 0 6px;
  background: var(--background);
  scrollbar-width: none;
}

.terminal-pane-tabs::-webkit-scrollbar {
  display: none;
}

.terminal-pane-tab {
  display: inline-flex;
  height: 22px;
  flex: 0 0 auto;
  align-items: center;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: color-mix(in srgb, var(--muted) 22%, transparent);
}

.terminal-pane-tab.is-active {
  border-color: color-mix(in srgb, var(--primary) 65%, var(--border));
  background: color-mix(in srgb, var(--primary) 13%, var(--background));
  color: var(--primary);
}

.terminal-pane-tab-label {
  display: inline-flex;
  height: 100%;
  align-items: center;
  gap: 4px;
  border: 0;
  border-radius: 3px;
  padding: 0 6px;
  background: transparent;
  color: inherit;
  font-size: 11px;
  line-height: 1;
  white-space: nowrap;
  cursor: pointer;
}

.terminal-pane-tab-label svg {
  width: 11px;
  height: 11px;
  flex: 0 0 auto;
  stroke-width: 1.8;
}

.terminal-pane-tab-close {
  display: inline-grid;
  width: 16px;
  height: 16px;
  flex: 0 0 16px;
  place-items: center;
  border: 0;
  border-radius: 3px;
  margin-right: 2px;
  padding: 0;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
}

.terminal-pane-tab-close:hover {
  background: var(--accent);
  color: var(--accent-foreground);
}

.terminal-pane-tab-close svg {
  width: 10px;
  height: 10px;
  stroke-width: 1.8;
}

.terminal-pane-tab-add,
.terminal-pane-close {
  display: inline-grid;
  width: 22px;
  height: 22px;
  flex: 0 0 22px;
  place-items: center;
  border: 0;
  border-radius: 4px;
  padding: 0;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
}

.terminal-pane-tab-add:hover:not(:disabled),
.terminal-pane-close:hover {
  background: var(--accent);
  color: var(--accent-foreground);
}

.terminal-pane-tab-add:disabled {
  cursor: default;
  opacity: 0.45;
}

.terminal-pane-tab-add svg,
.terminal-pane-close svg {
  width: 12px;
  height: 12px;
  stroke-width: 1.8;
}

.terminal-pane-tabs-spacer {
  flex: 1 1 auto;
}
</style>
