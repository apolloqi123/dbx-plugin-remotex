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

/**
 * 详情被手动收起后置 true，直到用户显式选择容器才恢复。
 *
 * 规则（最简单且无惊喜）：列表的自动选中（首载/轮询/刷新，`explicit` 缺省）
 * 只能更新已有选中项的快照，不能重建被收起的详情；只有用户显式动作
 * （行点击、行菜单 Logs，`explicit === true`）才清除该标记并重新打开详情。
 * 列表清空（`null`）只清选中项，不重置标记——「空 → 重新出现」同样属于
 * 列表自动变化，不该替用户把详情再弹出来。
 */
const dismissed = ref(false);

/** 列表只在选中签名变化或清空时 emit（清空传 null；显式选择带 explicit）。 */
function select(container: DockerContainer | null, explicit = false): void {
  if (explicit) dismissed.value = false;
  if (!container) {
    selected.value = undefined;
    return;
  }
  // 已收起且当前没有选中项：自动选中不重建详情，只保留列表高亮。
  if (dismissed.value && !selected.value) return;
  selected.value = container;
}

/** 详情页头 X：只收起详情，不关闭整个分屏（分屏头部的 X 才 emit close）。 */
function clearSelection(): void {
  selected.value = undefined;
  dismissed.value = true;
}

/** 行菜单「Logs」：选中并切到日志页（当前容器也可从文件页跳回）。 */
function showLogs(container: DockerContainer): void {
  dismissed.value = false;
  selected.value = container;
  activeTab.value = "logs";
}

// 连接切换：旧选中项不属于新目标，先清空避免详情短暂显示别的机器；新连接
// 按初始规则重新自动选中，所以收起标记一并复位。
watch([() => props.sessionId, () => props.connectionId], () => {
  selected.value = undefined;
  dismissed.value = false;
  activeTab.value = "logs";
});
</script>

<template>
  <section class="container-manager-pane" :aria-label="t('docker.toolbarTitle')">
    <header class="container-manager-head">
      <strong class="container-manager-title">{{ t("docker.managerTitle") }}</strong>
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
          @close="clearSelection"
        />
      </div>
      <div v-else class="container-manager-empty">
        <p>{{ t("docker.selectContainer") }}</p>
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
