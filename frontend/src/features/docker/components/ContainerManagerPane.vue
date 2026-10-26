<script setup lang="ts">
// 容器管理分屏外壳（Task 6 外壳 + Task 7 列表/详情编排）：终端列底部的独立
// 区域，默认占 33%、可拖拽/键盘调整。本组件只持有选中容器与活动页签；
// 列表（轮询/动作/引擎设置）与详情（页头/页签/日志与文件页）各自自治。
//
// props 契约固定：t / sessionId / connectionId / canWrite。sessionId 缺省时
// 列表回落本机 daemon（与容器面板既有语义一致）。
import { ref, watch } from "vue";
import { X } from "@lucide/vue";
import type { DockerContainer } from "../lib/dockerPanelModel";
import ContainerListPane from "./ContainerListPane.vue";
import ContainerDetailPane from "./ContainerDetailPane.vue";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  /** 工作台当前 SSH 会话；缺省表示本机 daemon 目标。 */
  sessionId?: string;
  connectionId?: string;
  /** 只读连接禁止上传/导入（下传给详情页签）。 */
  canWrite?: boolean;
}>();

const emit = defineEmits<{ (e: "close"): void }>();

const selected = ref<DockerContainer>();
const activeTab = ref<"logs" | "files">("logs");

/** 列表只在选中变化时 emit；选中消失时列表自行回退并重新 emit。 */
function select(container: DockerContainer): void {
  selected.value = container;
}

/** 行菜单「Logs」：选中并切到日志页（当前容器也可从文件页跳回）。 */
function showLogs(container: DockerContainer): void {
  selected.value = container;
  activeTab.value = "logs";
}

// 连接切换：旧选中项不属于新目标，先清空避免详情短暂显示别的机器。
watch([() => props.sessionId, () => props.connectionId], () => {
  selected.value = undefined;
  activeTab.value = "logs";
});
</script>

<template>
  <section class="container-manager-pane" :aria-label="t('docker.toolbarTitle')">
    <header class="container-manager-head">
      <strong class="container-manager-title">{{ t("docker.toolbarTitle") }}</strong>
      <span class="container-manager-spacer" />
      <button
        type="button"
        class="icon-button compact container-manager-close"
        :title="t('docker.close')"
        :aria-label="t('docker.close')"
        @click="emit('close')"
      >
        <X />
      </button>
    </header>
    <div class="container-manager-body">
      <div class="container-manager-list">
        <ContainerListPane
          :t="t"
          :session-id="sessionId"
          :connection-id="connectionId"
          @select="select"
          @logs="showLogs"
        />
      </div>
      <div v-if="selected" class="container-manager-detail">
        <ContainerDetailPane
          :t="t"
          :container="selected"
          :active-tab="activeTab"
          :session-id="sessionId"
          :connection-id="connectionId"
          :can-write="canWrite"
          @update:active-tab="activeTab = $event"
          @close="emit('close')"
        />
      </div>
      <div v-else class="container-manager-empty">
        <p>{{ t("docker.empty") }}</p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.container-manager-pane {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: var(--background);
}

.container-manager-head {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 0 0 auto;
  padding: 2px 8px;
  border-bottom: 1px solid var(--border);
}

.container-manager-title {
  font-size: 12px;
  white-space: nowrap;
}

.container-manager-spacer {
  flex: 1;
}

.container-manager-body {
  display: flex;
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
}

/* 列表列：38% 宽，240-420px 夹取；详情吃掉余量。 */
.container-manager-list {
  display: flex;
  flex: 0 0 38%;
  min-width: 240px;
  max-width: 420px;
  min-height: 0;
  border-right: 1px solid var(--border);
}

.container-manager-detail {
  display: flex;
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
}

.container-manager-empty {
  display: flex;
  flex: 1;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  opacity: 0.65;
}
</style>
