<script setup lang="ts">
// SFTP 单栏文件树（文件夹 + 文件同列）：渲染层只负责展示与交互上抛，
// 节点状态、懒加载与过滤由父级通过 sftpFileTree.ts 维护。虚拟滚动沿用
// 文件列表的 computeWindow（28px 行高 + overscan），测试可关掉窗口化。
// Info 详情卡用 Teleport 挂在 body 上（fixed + pointer-events:none），不进入
// 虚拟列表布局；目录行同时是宿主文件拖放目标（高亮 + 悬停 1s 懒展开）。
// 虚拟滚动/详情卡/文件拖放分别下沉到 useTreeVirtualScroll / useTreeDetailsCard /
// useTreeFileDrop，单行 DOM 与行内样式在 SftpTreeRow.vue。
import { computed, ref, watch } from "vue";
import type { SftpSanitizedEntry } from "../lib/sftpEntries";
import type { SftpFileTreeNode, SftpTreeRow as TreeRow } from "../lib/sftpFileTree";
import type { SftpDetailInfo } from "../lib/sftpDetails";
import SftpTreeRow from "./SftpTreeRow.vue";
import { useTreeDetailsCard } from "../composables/useTreeDetailsCard";
import { useTreeFileDrop } from "../composables/useTreeFileDrop";
import { useTreeVirtualScroll } from "../composables/useTreeVirtualScroll";

const props = withDefaults(
  defineProps<{
    rows: TreeRow[];
    selectedUris: Set<string>;
    selectedPath: string;
    currentPath: string;
    renamingPath: string;
    renameDraft: string;
    renameSubmitting: boolean;
    t: (key: string, values?: Record<string, string | number>) => string;
    locale?: string;
    /** 测试可关闭窗口化，一次渲染全部行；生产默认开启。 */
    virtualize?: boolean;
    /** 空态文案（如搜索无结果时传 sftpSearch.noMatch）。 */
    emptyText?: string;
    /** 树列表默认 includeOwner:false，info 图标悬停时按需补齐 owner/group 等详情。 */
    loadDetails?: (entry: SftpSanitizedEntry) => Promise<SftpDetailInfo | null>;
    /** 宿主桥已判定文件拖拽进行中：部分宿主会剥离 dataTransfer.types，
     *  行级 dragover 不能只依赖 DOM types，否则高亮/悬停展开失效。 */
    externalDragActive?: boolean;
    /** 仅测试/兼容用：宿主拖拽坐标的平台 y 偏移；缺省按 iframe + macOS 推断。 */
    hostDragYOffset?: number;
    /** 目录跟随开启：列表底部留一屏滚动余量，让跟随目录可以真正贴顶显示
     *  （否则靠近末尾的目录会被 max scroll 钳在视口中部）。 */
    followPadding?: boolean;
  }>(),
  { virtualize: true, externalDragActive: false, followPadding: false },
);

const emit = defineEmits<{
  (event: "toggle", node: SftpFileTreeNode): void;
  (event: "select", entry: SftpSanitizedEntry, sourceEvent: MouseEvent | KeyboardEvent): void;
  (event: "open", node: SftpFileTreeNode): void;
  (event: "context", node: SftpFileTreeNode, mouseEvent: MouseEvent): void;
  (event: "rename-commit", node: SftpFileTreeNode, value: string): void;
  (event: "rename-cancel"): void;
  (event: "row-keydown", node: SftpFileTreeNode, keyEvent: KeyboardEvent): void;
  (event: "drop-on", node: SftpFileTreeNode, dragEvent: DragEvent): void;
  (event: "drop-target-change", path: string): void;
}>();

function displayName(node: SftpFileTreeNode): string {
  // 根节点在树里就显示为 "/"（与路径栏一致），不再用「根目录」占位词。
  return node.name === "/" ? "/" : node.name;
}

const {
  detailVisible,
  detailNode,
  detailLoading,
  detailUnavailable,
  detailCardEl,
  detailCardStyle,
  detailTitle,
  detailRows,
  hideDetails,
  onInfoEnter,
  onInfoFocus,
} = useTreeDetailsCard({
  rows: () => props.rows,
  t: () => props.t,
  locale: () => props.locale,
  loadDetails: () => props.loadDetails,
  displayName,
});

const {
  scrollEl,
  renderedRows,
  virtualWindow,
  followPadPx,
  onScroll,
  rowIndexOf,
  focusRowAt,
  scrollPathToTop,
} = useTreeVirtualScroll({
  rows: () => props.rows,
  virtualize: () => props.virtualize,
  followPadding: () => props.followPadding,
  onScrollHook: hideDetails,
});

const { dragOverUri, onTreeDragOver, onRowDragOver, onRowDragLeave, onRowDrop, markDropTargetAt, resetHostDragCalibration } = useTreeFileDrop({
  rows: () => props.rows,
  externalDragActive: () => props.externalDragActive,
  hostDragYOffset: () => {
    if (props.hostDragYOffset !== undefined) return props.hostDragYOffset;
    // macOS Tauri 的拖拽点比真实指针高约一个拖拽图标（tauri#10744）；
    // 插件在宿主 iframe 内运行时才需要补偿，构建/单测页面保持 0。
    if (typeof window === "undefined") return 0;
    return window.parent !== window && /Mac/i.test(navigator.platform) ? 28 : 0;
  },
  emit: {
    toggle: (node) => emit("toggle", node),
    dropOn: (node, dragEvent) => emit("drop-on", node, dragEvent),
    dropTargetChange: (path) => emit("drop-target-change", path),
  },
});

const resolvedEmptyText = computed(() => props.emptyText || props.t("emptyFolder"));

function caretName(node: SftpFileTreeNode): string {
  const name = displayName(node);
  return node.expanded
    ? props.t("sftpSide.collapseNode", { name })
    : props.t("sftpSide.expandNode", { name });
}

function isSelected(node: SftpFileTreeNode): boolean {
  return props.selectedPath === node.entry.uri || props.selectedUris.has(node.entry.uri);
}

// 改名草稿：父级持有 renameDraft，输入期间用本地副本保证受控且不写 prop。
const renameValue = ref("");
let renameSettled = false;

watch(
  () => props.renamingPath,
  (path) => {
    if (!path) return;
    renameValue.value = props.renameDraft;
    renameSettled = false;
  },
  { immediate: true },
);

function onRenameInput(event: Event) {
  renameValue.value = (event.target as HTMLInputElement).value;
}

function commitRename(node: SftpFileTreeNode) {
  if (renameSettled) return;
  renameSettled = true;
  if (props.renameSubmitting) return;
  emit("rename-commit", node, renameValue.value);
}

function cancelRename() {
  if (renameSettled) return;
  renameSettled = true;
  emit("rename-cancel");
}

// Roving tabindex：优先记住最后聚焦行，其次当前目录，最后首行兜底。
const activePath = ref("");

// 虚拟窗口内实际存在的行：roving tabindex 的兜底只能落在可见行上，
// 否则滚动/过滤后整个树会没有 Tab 停靠点。
const renderedPathSet = computed(() => new Set(renderedRows.value.map((row) => row.node.path)));
const firstRenderedIndex = computed(() => renderedRows.value[0]?.index ?? 0);

function tabIndexFor(node: SftpFileTreeNode, index: number): number {
  if (activePath.value && renderedPathSet.value.has(activePath.value)) {
    return node.path === activePath.value ? 0 : -1;
  }
  if (renderedPathSet.value.has(props.currentPath)) {
    return node.path === props.currentPath ? 0 : -1;
  }
  return index === firstRenderedIndex.value ? 0 : -1;
}

function onRowFocus(node: SftpFileTreeNode) {
  activePath.value = node.path;
}

function onRowSelect(entry: SftpSanitizedEntry, sourceEvent: MouseEvent) {
  emit("select", entry, sourceEvent);
}

function onRowToggle(node: SftpFileTreeNode) {
  emit("toggle", node);
}

function onRowOpen(node: SftpFileTreeNode) {
  emit("open", node);
}

function onRowContext(node: SftpFileTreeNode, mouseEvent: MouseEvent) {
  emit("context", node, mouseEvent);
}

defineExpose({ scrollPathToTop, markDropTargetAt, resetHostDragCalibration });

function moveRowFocus(node: SftpFileTreeNode, offset: number): boolean {
  const index = rowIndexOf(node);
  return index >= 0 ? focusRowAt(index + offset) : false;
}

function focusParentRow(node: SftpFileTreeNode): boolean {
  const index = rowIndexOf(node);
  if (index < 0) return false;
  const depth = props.rows[index].depth;
  for (let candidate = index - 1; candidate >= 0; candidate -= 1) {
    if (props.rows[candidate].depth < depth) return focusRowAt(candidate);
  }
  return false;
}

function onRowKeydown(node: SftpFileTreeNode, event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    emit("open", node);
    return;
  }
  if (event.key === " ") {
    event.preventDefault();
    if (node.kind === "directory") {
      emit("toggle", node);
      return;
    }
    // Space 选中文件（与主流文件管理器一致）；改用 select 而不是把未处理
    // 按键丢给 row-keydown 的 no-op 路径。
    emit("select", node.entry, event);
    return;
  }
  if (event.key === "ArrowDown") {
    if (moveRowFocus(node, 1)) {
      event.preventDefault();
      return;
    }
    emit("row-keydown", node, event);
    return;
  }
  if (event.key === "ArrowUp") {
    if (moveRowFocus(node, -1)) {
      event.preventDefault();
      return;
    }
    emit("row-keydown", node, event);
    return;
  }
  if (event.key === "ArrowLeft") {
    if (node.kind === "directory" && node.expanded) {
      event.preventDefault();
      emit("toggle", node);
      return;
    }
    if (focusParentRow(node)) {
      event.preventDefault();
      return;
    }
    emit("row-keydown", node, event);
    return;
  }
  if (event.key === "ArrowRight") {
    if (node.kind === "directory" && !node.expanded) {
      event.preventDefault();
      emit("toggle", node);
      return;
    }
    emit("row-keydown", node, event);
    return;
  }
  // F2/Delete 由父级开始改名/删除；其余未处理键也原样上抛。
  emit("row-keydown", node, event);
}
</script>

<template>
  <div
    ref="scrollEl"
    class="sftp-tree-pane"
    :lang="locale || undefined"
    @dragover="onTreeDragOver"
    @scroll.passive="onScroll"
  >
    <div class="sftp-tree-list" role="tree" :aria-label="t('sftpSide.tree')">
      <div
        v-if="virtualize && virtualWindow.padTop"
        class="sftp-tree-spacer"
        :style="{ height: `${virtualWindow.padTop}px` }"
        aria-hidden="true"
      ></div>
      <SftpTreeRow
        v-for="row in renderedRows"
        :key="row.node.entry.uri"
        :row="row"
        :tab-index="tabIndexFor(row.node, row.index)"
        :name="displayName(row.node)"
        :caret-label="caretName(row.node)"
        :current="row.node.path === currentPath"
        :selected="isSelected(row.node)"
        :drag-over="dragOverUri === row.node.entry.uri"
        :renaming="renamingPath === row.node.entry.uri"
        :rename-value="renameValue"
        :rename-submitting="renameSubmitting"
        :t="t"
        @select="onRowSelect"
        @toggle="onRowToggle"
        @open="onRowOpen"
        @context="onRowContext"
        @row-focus="onRowFocus"
        @keydown="onRowKeydown"
        @rename-input="onRenameInput"
        @rename-commit="commitRename"
        @rename-cancel="cancelRename"
        @info-enter="onInfoEnter"
        @info-focus="onInfoFocus"
        @info-hide="hideDetails"
        @row-drag-over="onRowDragOver"
        @row-drag-leave="onRowDragLeave"
        @row-drop="onRowDrop"
      />
      <div
        v-if="virtualize && virtualWindow.padBottom"
        class="sftp-tree-spacer"
        :style="{ height: `${virtualWindow.padBottom}px` }"
        aria-hidden="true"
      ></div>
      <!-- 跟随模式的底部余量：仅当靠后的目录无法自然滚到顶部时补最小高度。
           空目录不加，避免把「空文件夹」提示推到视口下方。 -->
      <div v-if="followPadding && rows.length && followPadPx > 0" class="sftp-tree-spacer sftp-tree-follow-pad" :style="{ height: `${followPadPx}px` }" aria-hidden="true"></div>
      <div v-if="!rows.length" class="empty compact">{{ resolvedEmptyText }}</div>
    </div>

    <Teleport to="body">
      <div
        v-if="detailVisible && detailNode"
        ref="detailCardEl"
        class="sftp-detail-card"
        role="tooltip"
        :aria-label="t('sftpDetails.title')"
        :style="detailCardStyle"
      >
        <div class="sftp-detail-head">{{ detailTitle }}</div>
        <div v-if="detailLoading" class="sftp-detail-loading">{{ t("sftpDetails.loading") }}</div>
        <div v-if="detailRows.length" class="sftp-detail-rows">
          <div v-for="item in detailRows" :key="item.key" class="sftp-detail-row">
            <span class="sftp-detail-label">{{ item.label }}</span>
            <span class="sftp-detail-value">{{ item.value }}</span>
          </div>
        </div>
        <div v-if="detailUnavailable && !detailLoading" class="sftp-detail-unavailable">
          {{ t("sftpDetails.unavailable") }}
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.sftp-tree-pane {
  position: relative;
  display: flex;
  min-height: 0;
  height: 100%;
  flex: 1 1 auto;
  flex-direction: column;
  overflow: auto;
  overflow-x: hidden;
  padding: 2px 3px;
}

.sftp-tree-list {
  min-width: 0;
}

/* 详情卡 Teleport 到 body：fixed 定位 + 不拦截指针，关闭由悬停/焦点/滚动驱动。
   卡片 vnode 在本组件渲染函数里创建，父级 scoped CSS 仍能命中 Teleport 内部。 */
.sftp-detail-card {
  position: fixed;
  z-index: 80;
  width: 276px;
  max-width: calc(100vw - 8px);
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--popover, var(--background));
  color: var(--popover-foreground, var(--foreground));
  box-shadow: 0 6px 18px rgb(0 0 0 / 0.28);
  font-size: 11px;
  line-height: 1.45;
  pointer-events: none;
}

.sftp-detail-head {
  margin-bottom: 4px;
  overflow: hidden;
  color: var(--foreground);
  font-size: 12px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sftp-detail-loading {
  margin-bottom: 4px;
  color: var(--muted-foreground);
}

.sftp-detail-row {
  display: grid;
  grid-template-columns: 76px minmax(0, 1fr);
  gap: 6px;
}

.sftp-detail-label {
  color: var(--muted-foreground);
  white-space: nowrap;
}

.sftp-detail-value {
  overflow-wrap: anywhere;
}

.sftp-detail-unavailable {
  margin-top: 4px;
  color: var(--muted-foreground);
  font-style: italic;
}

.sftp-tree-spacer {
  flex: none;
  width: 100%;
}
</style>
