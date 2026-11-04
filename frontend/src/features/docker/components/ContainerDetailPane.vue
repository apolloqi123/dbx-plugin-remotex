<script setup lang="ts">
// 选中容器详情外壳（Task 7）：页头（名称/状态 + 日志/文件/终端页签 + 关闭）。
// 页签状态由 ContainerManagerPane 持有（activeTab prop + update:activeTab）。
//
// 三个页签用 v-show 常驻挂载（Task 9）：日志流由 ContainerLogsTab 自己持有，
// 切到文件/终端页若卸载组件会停流、丢掉历史；因此只在容器切换/关闭/卸载时收流。
// Wave 2 在页内填充实现，props 契约不变。
import { Activity, Box, FolderOpen, ScrollText, Terminal, X } from "@lucide/vue";
import type { ContainerTerminalStateEvent } from "../composables/useContainerTerminal";
import { dockerStateClass, type DockerContainer } from "../lib/dockerPanelModel";
import ContainerLogsTab from "./ContainerLogsTab.vue";
import ContainerFilesTab from "./ContainerFilesTab.vue";
import ContainerTerminalTab from "./ContainerTerminalTab.vue";
import ContainerMonitorTab from "./ContainerMonitorTab.vue";

type DetailTab = "logs" | "files" | "terminal" | "monitor";

defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  container: DockerContainer;
  activeTab: DetailTab;
  sessionId?: string;
  connectionId?: string;
  canWrite?: boolean;
  /** 工作台终端主题/字体（可选，容器终端缺省用内置回退）。 */
  theme?: Record<string, unknown>;
  fontFamily?: string;
  fontSize?: number;
  terminalStateEvents?: (handler: (event: ContainerTerminalStateEvent) => void) => () => void;
}>();

const emit = defineEmits<{
  (e: "update:activeTab", tab: DetailTab): void;
  /** 页头 X：只表示“收起详情”；分屏整体关闭由 ContainerManagerPane 头部负责。 */
  (e: "close"): void;
}>();
</script>

<template>
  <section class="container-detail-pane">
    <header class="container-detail-head">
      <span class="ui-icon-tile ui-tone-emerald container-detail-logo"><Box /></span>
      <strong class="container-detail-name mono" :title="container.id">{{ container.name || container.id.slice(0, 12) }}</strong>
      <span class="docker-badge" :class="dockerStateClass(container.state)">{{ container.state }}</span>
      <div class="container-detail-tabs" role="tablist">
        <button
          type="button"
          role="tab"
          class="container-detail-tab"
          data-tab="logs"
          :aria-selected="activeTab === 'logs'"
          :title="t('docker.logsTab')"
          :aria-label="t('docker.logsTab')"
          @click="emit('update:activeTab', 'logs')"
        >
          <ScrollText />
        </button>
        <button
          type="button"
          role="tab"
          class="container-detail-tab"
          data-tab="files"
          :aria-selected="activeTab === 'files'"
          :title="t('docker.filesTab')"
          :aria-label="t('docker.filesTab')"
          @click="emit('update:activeTab', 'files')"
        >
          <FolderOpen />
        </button>
        <button
          type="button"
          role="tab"
          class="container-detail-tab"
          data-tab="terminal"
          :aria-selected="activeTab === 'terminal'"
          :title="t('docker.terminalTab')"
          :aria-label="t('docker.terminalTab')"
          @click="emit('update:activeTab', 'terminal')"
        >
          <Terminal />
        </button>
        <button
          type="button"
          role="tab"
          class="container-detail-tab"
          data-tab="monitor"
          :aria-selected="activeTab === 'monitor'"
          :title="t('docker.monitorTab')"
          :aria-label="t('docker.monitorTab')"
          @click="emit('update:activeTab', 'monitor')"
        >
          <Activity />
        </button>
      </div>
      <span class="container-detail-spacer" />
      <button
        type="button"
        class="icon-button compact container-detail-close"
        :title="t('docker.close')"
        :aria-label="t('docker.close')"
        @click="emit('close')"
      >
        <X />
      </button>
    </header>
    <ContainerLogsTab
      v-show="activeTab === 'logs'"
      :t="t"
      :container="container"
      :session-id="sessionId"
      :connection-id="connectionId"
      :can-write="canWrite"
      :visible="activeTab === 'logs'"
    />
    <ContainerFilesTab
      v-show="activeTab === 'files'"
      :t="t"
      :container="container"
      :session-id="sessionId"
      :connection-id="connectionId"
      :can-write="canWrite"
      :visible="activeTab === 'files'"
    />
    <ContainerTerminalTab
      v-show="activeTab === 'terminal'"
      :t="t"
      :container="container"
      :session-id="sessionId"
      :connection-id="connectionId"
      :can-write="canWrite"
      :visible="activeTab === 'terminal'"
      :theme="theme"
      :font-family="fontFamily"
      :font-size="fontSize"
      :terminal-state-events="terminalStateEvents"
    />
    <ContainerMonitorTab
      v-show="activeTab === 'monitor'"
      :t="t"
      :container="container"
      :session-id="sessionId"
      :connection-id="connectionId"
      :visible="activeTab === 'monitor'"
    />
  </section>
</template>

<style scoped>
.container-detail-pane {
  display: flex;
  flex: 1 1 auto;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
}

.container-detail-head {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  min-height: 34px;
  padding: 4px 6px 4px 10px;
  border-bottom: 1px solid var(--border);
}

.container-detail-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
}

.container-detail-spacer {
  flex: 1;
}

.docker-badge {
  display: inline-block;
  flex: 0 0 auto;
  padding: 0 6px;
  border-radius: 8px;
  font-size: 11px;
  line-height: 16px;
  background: rgba(128, 128, 128, 0.18);
}

.docker-state-running {
  background: rgba(34, 160, 94, 0.22);
}

.docker-state-paused,
.docker-state-restarting {
  background: rgba(217, 154, 35, 0.25);
}

.docker-state-dead {
  background: rgba(214, 69, 69, 0.25);
}

.container-detail-tabs {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 2px;
}

.container-detail-tab {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: none;
  color: inherit;
  padding: 3px 8px;
  cursor: pointer;
  border-radius: 8px;
  opacity: 0.7;
}

.container-detail-tab svg {
  width: 14px;
  height: 14px;
}

.container-detail-tab[aria-selected="true"] {
  background: color-mix(in srgb, var(--primary) 16%, transparent);
  opacity: 1;
}
</style>
