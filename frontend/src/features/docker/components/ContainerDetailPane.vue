<script setup lang="ts">
// 选中容器详情外壳（Task 7）：页头（名称/状态/关闭）+ 日志/文件页签。
// 页签状态由 ContainerManagerPane 持有（activeTab prop + update:activeTab）。
//
// 两个页签用 v-show 常驻挂载（Task 9）：日志流由 ContainerLogsTab 自己持有，
// 切到文件页若卸载组件会停流、丢掉历史；因此只在容器切换/关闭/卸载时收流。
// Wave 2 在页内填充实现，props 契约不变。
import { X } from "@lucide/vue";
import { dockerStateClass, type DockerContainer } from "../lib/dockerPanelModel";
import ContainerLogsTab from "./ContainerLogsTab.vue";
import ContainerFilesTab from "./ContainerFilesTab.vue";

defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  container: DockerContainer;
  activeTab: "logs" | "files";
  sessionId?: string;
  connectionId?: string;
  canWrite?: boolean;
}>();

const emit = defineEmits<{
  (e: "update:activeTab", tab: "logs" | "files"): void;
  /** 页头 X：只表示“收起详情”；分屏整体关闭由 ContainerManagerPane 头部负责。 */
  (e: "close"): void;
}>();
</script>

<template>
  <section class="container-detail-pane">
    <header class="container-detail-head">
      <strong class="container-detail-name mono" :title="container.id">{{ container.name || container.id.slice(0, 12) }}</strong>
      <span class="docker-badge" :class="dockerStateClass(container.state)">{{ container.state }}</span>
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
    <div class="container-detail-tabs" role="tablist">
      <button
        type="button"
        role="tab"
        class="container-detail-tab"
        data-tab="logs"
        :aria-selected="activeTab === 'logs'"
        @click="emit('update:activeTab', 'logs')"
      >
        {{ t("docker.logsTab") }}
      </button>
      <button
        type="button"
        role="tab"
        class="container-detail-tab"
        data-tab="files"
        :aria-selected="activeTab === 'files'"
        @click="emit('update:activeTab', 'files')"
      >
        {{ t("docker.filesTab") }}
      </button>
    </div>
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
  padding: 2px 6px 2px 10px;
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
  gap: 2px;
  padding: 0 6px;
  border-bottom: 1px solid var(--border);
}

.container-detail-tab {
  border: none;
  background: none;
  color: inherit;
  padding: 4px 8px;
  font-size: 12px;
  cursor: pointer;
  border-bottom: 2px solid transparent;
  opacity: 0.7;
}

.container-detail-tab[aria-selected="true"] {
  border-bottom-color: var(--primary);
  opacity: 1;
}
</style>
