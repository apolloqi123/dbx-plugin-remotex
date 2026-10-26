<script setup lang="ts">
// 容器管理分屏外壳（Task 6 外壳 + Task 7 列表/详情编排）：终端列底部的独立
// 区域，默认只显示列表。用户从行菜单「Logs」/「Files」显式打开详情后才切成
// 左右分栏；分栏线可拖拽或方向键调整。本组件只持有选中容器、活动页签与
// 分栏宽度；列表（轮询/动作/引擎设置）与详情（页头/页签/日志、文件与终端页）
// 各自自治。
//
// props 契约固定：t / sessionId / connectionId / canWrite。sessionId 缺省时
// 列表回落本机 daemon（与容器面板既有语义一致）。
import { computed, onUnmounted, ref, watch } from "vue";
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

const DEFAULT_LIST_WIDTH = 40;
const MIN_LIST_WIDTH = 25;
const MAX_LIST_WIDTH = 70;

const selected = ref<DockerContainer>();
type DetailTab = "logs" | "files" | "terminal";
const activeTab = ref<DetailTab>("logs");

/** 详情打开时才占分栏宽度；关闭后列表恢复整宽（内联 flex-basis 100%）。 */
const detailOpen = computed(() => selected.value !== undefined);

/** 列表占 body 的百分比；拖拽/方向键都会经过同一夹取。 */
const listWidth = ref(DEFAULT_LIST_WIDTH);
const bodyRef = ref<HTMLElement>();

function clampWidth(value: number): number {
  return Math.min(MAX_LIST_WIDTH, Math.max(MIN_LIST_WIDTH, value));
}

function setListWidth(value: number): void {
  listWidth.value = clampWidth(Math.round(value * 10) / 10);
}

/**
 * 列表选中只同步已打开的详情快照：自动选中/轮询/行点击都不会创建详情。
 * 行点击只让列表自己高亮；详情已打开时切换目标容器，`null`（清空）收起详情。
 */
function select(container: DockerContainer | null): void {
  if (!container) {
    selected.value = undefined;
    return;
  }
  if (!detailOpen.value) return;
  selected.value = container;
}

/** 行菜单 Logs/Files：打开（或切换）详情并定位页签。 */
function openDetail(container: DockerContainer, tab: "logs" | "files"): void {
  selected.value = container;
  activeTab.value = tab;
}

/** 详情页头 X：只收起详情，不关闭整个分屏（分屏头部的 X 才 emit close）。 */
function closeDetail(): void {
  selected.value = undefined;
}

// —— 分隔条拖拽：window 级监听，pointerup/cancel/卸载时摘除 ——————————————
let dragStartX = 0;
let dragStartWidth = DEFAULT_LIST_WIDTH;
let dragging = false;

function onPointerMove(event: PointerEvent): void {
  const rect = bodyRef.value?.getBoundingClientRect();
  if (!dragging || !rect || !rect.width) return;
  setListWidth(dragStartWidth + ((event.clientX - dragStartX) / rect.width) * 100);
}

function stopDrag(): void {
  if (!dragging) return;
  dragging = false;
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", stopDrag);
  window.removeEventListener("pointercancel", stopDrag);
}

function startDrag(event: PointerEvent): void {
  if (event.button > 0) return;
  event.preventDefault();
  dragging = true;
  dragStartX = event.clientX;
  dragStartWidth = listWidth.value;
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", stopDrag);
  window.addEventListener("pointercancel", stopDrag);
}

/** 方向键 ±4%，Shift ×3；两端由 setListWidth 夹到 25–70。 */
function onDividerKeydown(event: KeyboardEvent): void {
  const step = event.shiftKey ? 12 : 4;
  if (event.key === "ArrowLeft") setListWidth(listWidth.value - step);
  else if (event.key === "ArrowRight") setListWidth(listWidth.value + step);
  else return;
  event.preventDefault();
}

onUnmounted(stopDrag);

// 连接切换：旧选中项不属于新目标，收起详情；列表按新目标重新轮询。
watch([() => props.sessionId, () => props.connectionId], () => {
  selected.value = undefined;
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
    <div ref="bodyRef" class="container-manager-body">
      <div class="container-manager-list" :style="{ flexBasis: detailOpen ? `${listWidth}%` : '100%' }">
        <ContainerListPane
          :t="t"
          :session-id="sessionId"
          :connection-id="connectionId"
          @select="select"
          @logs="openDetail($event, 'logs')"
          @files="openDetail($event, 'files')"
        />
      </div>
      <template v-if="selected">
        <div
          class="container-manager-divider"
          role="separator"
          aria-orientation="vertical"
          :aria-valuenow="listWidth"
          :aria-valuemin="MIN_LIST_WIDTH"
          :aria-valuemax="MAX_LIST_WIDTH"
          tabindex="0"
          @pointerdown="startDrag"
          @keydown="onDividerKeydown"
        />
        <div class="container-manager-detail">
          <ContainerDetailPane
            :t="t"
            :container="selected"
            :active-tab="activeTab"
            :session-id="sessionId"
            :connection-id="connectionId"
            :can-write="canWrite"
            @update:active-tab="activeTab = $event"
            @close="closeDetail"
          />
        </div>
      </template>
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

/* 列表列：详情关闭时内联 flex-basis 100%；打开后由拖拽宽度决定（25–70%）。 */
.container-manager-list {
  display: flex;
  flex: 0 0 auto;
  min-width: 0;
  min-height: 0;
}

/* 分隔条：1px 视觉线 + 左右加宽的命中区，hover/拖拽时高亮。 */
.container-manager-divider {
  flex: 0 0 5px;
  margin: 0 -2px;
  cursor: col-resize;
  position: relative;
  z-index: 1;
  outline: none;
  touch-action: none;
}

.container-manager-divider::after {
  content: "";
  position: absolute;
  inset: 0 2px;
  background: var(--border);
}

.container-manager-divider:hover::after,
.container-manager-divider:focus-visible::after {
  background: var(--primary);
}

.container-manager-detail {
  display: flex;
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
}
</style>
