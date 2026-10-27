<script setup lang="ts">
// 单行文件树（SftpTreePane 的呈现层）：只做行内 DOM 与交互事件的语义化上抛
// （选中/展开/打开/右键/改名/键盘/详情卡/拖放），节点状态与虚拟窗口由父级
// 维护。样式必须与行同处一个 scoped 块：父级 scoped CSS 到不了子组件内部。
import {
  ChevronDown,
  ChevronRight,
  File as FileIcon,
  FileText,
  FolderClosed,
  FolderOpen,
  Info,
  Link2,
  TriangleAlert,
} from "@lucide/vue";
import type { SftpSanitizedEntry } from "../lib/entries/sftpEntries";
import type { SftpFileTreeNode } from "../lib/tree/sftpFileTree";

const props = defineProps<{
  row: { node: SftpFileTreeNode; depth: number; index: number };
  tabIndex: number;
  /** 行的展示名（根节点统一显示为 "/"，由父级决定）。 */
  name: string;
  caretLabel: string;
  current: boolean;
  selected: boolean;
  dragOver: boolean;
  renaming: boolean;
  renameValue: string;
  renameSubmitting: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (event: "select", entry: SftpSanitizedEntry, sourceEvent: MouseEvent): void;
  (event: "toggle", node: SftpFileTreeNode): void;
  (event: "open", node: SftpFileTreeNode): void;
  (event: "context", node: SftpFileTreeNode, mouseEvent: MouseEvent): void;
  (event: "row-focus", node: SftpFileTreeNode): void;
  (event: "keydown", node: SftpFileTreeNode, keyEvent: KeyboardEvent): void;
  (event: "rename-input", inputEvent: Event): void;
  (event: "rename-commit", node: SftpFileTreeNode): void;
  (event: "rename-cancel"): void;
  (event: "info-enter", node: SftpFileTreeNode, mouseEvent: MouseEvent): void;
  (event: "info-focus", node: SftpFileTreeNode, focusEvent: FocusEvent): void;
  (event: "info-hide"): void;
  (event: "row-drag-over", node: SftpFileTreeNode, dragEvent: DragEvent): void;
  (event: "row-drag-leave", node: SftpFileTreeNode, dragEvent: DragEvent): void;
  (event: "row-drop", node: SftpFileTreeNode, dragEvent: DragEvent): void;
}>();

function onRowClick(event: MouseEvent) {
  // 双击的第二次 click（detail > 1）忽略：首击已选中/展开，dblclick 只
  // 负责 open，避免同一行连发两次动作。
  if (event.detail > 1) return;
  const node = props.row.node;
  // 文件与文件夹统一单击选中并高亮；文件夹普通单击同时展开/收起（带修饰
  // 键时只做多选，交给 Ctrl/Cmd 切换与 Shift 范围）。
  emit("select", node.entry, event);
  if (node.kind === "directory" && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
    emit("toggle", node);
  }
}

function onCaretClick(event: MouseEvent) {
  // caret 双击是「展开 + 打开」手势的组成部分：第二次 click 不再 toggle，
  // 否则会先展开再收起（与行双击的 detail 守卫一致）。
  if (event.detail > 1) return;
  emit("toggle", props.row.node);
}

function onInfoEnter(event: MouseEvent) {
  emit("info-enter", props.row.node, event);
}

function onInfoFocus(event: FocusEvent) {
  emit("info-focus", props.row.node, event);
}

function onDragOver(event: DragEvent) {
  emit("row-drag-over", props.row.node, event);
}

function onDragLeave(event: DragEvent) {
  emit("row-drag-leave", props.row.node, event);
}

function onDrop(event: DragEvent) {
  emit("row-drop", props.row.node, event);
}
</script>

<template>
  <div
    class="sftp-tree-row"
    :class="{
      'is-current': current,
      'is-selected': selected,
      'is-file': row.node.kind !== 'directory',
      'is-drop-target': dragOver,
    }"
    role="treeitem"
    :aria-level="row.depth + 1"
    :aria-expanded="row.node.kind === 'directory' ? (row.node.expanded ? 'true' : 'false') : undefined"
    :aria-selected="selected ? 'true' : 'false'"
    :data-tree-index="row.index"
    :tabindex="tabIndex"
    :style="{ paddingLeft: `${8 + row.depth * 14}px` }"
    @click="onRowClick($event)"
    @dblclick="emit('open', row.node)"
    @focus="emit('row-focus', row.node)"
    @keydown="emit('keydown', row.node, $event)"
    @contextmenu="emit('context', row.node, $event)"
    @dragover="onDragOver"
    @dragleave="onDragLeave"
    @drop="onDrop"
  >
    <button
      v-if="row.node.kind === 'directory'"
      type="button"
      class="sftp-tree-caret"
      :aria-label="caretLabel"
      @click.stop="onCaretClick($event)"
      @dblclick.stop
      @keydown.stop
    >
      <span v-if="row.node.loading" class="sftp-tree-spinner" />
      <ChevronDown v-else-if="row.node.expanded" />
      <ChevronRight v-else />
    </button>
    <span v-else class="sftp-tree-caret-spacer" aria-hidden="true"></span>

    <FolderOpen v-if="row.node.kind === 'directory' && row.node.expanded" class="sftp-tree-dir-icon" />
    <FolderClosed v-else-if="row.node.kind === 'directory'" class="sftp-tree-dir-icon" />
    <Link2 v-else-if="row.node.kind === 'symlink'" class="sftp-tree-file-icon is-link" />
    <FileText v-else-if="row.node.kind === 'other'" class="sftp-tree-file-icon is-other" />
    <FileIcon v-else class="sftp-tree-file-icon" />

    <input
      v-if="renaming"
      :value="renameValue"
      class="rename-input"
      :disabled="renameSubmitting"
      spellcheck="false"
      autofocus
      @input="emit('rename-input', $event)"
      @click.stop
      @dblclick.stop
      @keydown.stop
      @keydown.enter.stop.prevent="emit('rename-commit', row.node)"
      @keydown.escape.stop.prevent="emit('rename-cancel')"
      @blur="emit('rename-commit', row.node)"
    />
    <span v-else class="sftp-tree-name">{{ name }}</span>
    <TriangleAlert
      v-if="row.node.entry.lossy"
      class="sftp-lossy-icon"
      :title="t('sftpName.lossyTitle')"
    />
    <button
      type="button"
      class="sftp-tree-info"
      :aria-label="t('sftpDetails.title')"
      @click.stop
      @dblclick.stop
      @keydown.stop
      @mouseenter="onInfoEnter($event)"
      @mouseleave="emit('info-hide')"
      @focus="onInfoFocus($event)"
      @blur="emit('info-hide')"
    >
      <Info />
    </button>
  </div>
</template>

<style scoped>
.sftp-tree-row {
  height: 28px;
  min-height: 28px;
  flex: 0 0 28px;
  outline: none;
}

.sftp-tree-row:focus-visible {
  outline: 1px solid var(--primary);
  outline-offset: -1px;
}

.sftp-tree-row.is-selected,
.sftp-tree-row.is-selected:hover {
  background: var(--accent);
  color: var(--accent-foreground);
}

.sftp-tree-row.is-selected .sftp-tree-name {
  color: var(--primary);
  font-weight: 600;
}

.sftp-tree-caret-spacer {
  flex: 0 0 14px;
  width: 14px;
  height: 14px;
}

/* 宿主同款 Lucide 线性文件夹/文件：与宿主树一致的 14px 尺寸（无彩色底块），
   琥珀色文件夹 + 蓝色文件 + 青色符号链接；描边也沿用 Lucide 默认 2。 */
.sftp-tree-row > svg.sftp-tree-dir-icon,
.sftp-tree-row > svg.sftp-tree-file-icon {
  width: 14px;
  height: 14px;
  flex: 0 0 14px;
  padding: 0;
  stroke-width: 2;
}

.sftp-tree-row > svg.sftp-tree-dir-icon { color: var(--tone-amber); }
.sftp-tree-row > svg.sftp-tree-file-icon { color: var(--tone-blue); }
.sftp-tree-row > svg.sftp-tree-file-icon.is-link { color: var(--tone-cyan); }
.sftp-tree-row > svg.sftp-tree-file-icon.is-other { color: var(--muted-foreground); }

.sftp-tree-name {
  min-width: 0;
  flex: 1 1 auto;
}

/* 行尾 Info：默认隐藏，只对「当前目录行 / 鼠标悬停行 / 键盘焦点行」显示，
   避免每行都挂一个常驻图标。 */
.sftp-tree-info {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 16px;
  width: 16px;
  height: 16px;
  margin-left: auto;
  padding: 0;
  border: 0;
  border-radius: 3px;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
  opacity: 0;
  pointer-events: none;
}

.sftp-tree-info svg {
  width: 12px;
  height: 12px;
}

.sftp-tree-row:hover .sftp-tree-info,
.sftp-tree-row.is-current .sftp-tree-info,
.sftp-tree-row:focus-within .sftp-tree-info,
.sftp-tree-info:focus-visible {
  opacity: 1;
  pointer-events: auto;
}

.sftp-tree-info:hover {
  background: color-mix(in srgb, var(--muted-foreground) 20%, transparent);
  color: var(--foreground);
}

/* 拖拽目标行必须压过 App 层 .drop-overlay（z=12）：否则整屏拖拽提示的
   半透明底色会盖住行高亮，看起来像“没有高亮”。 */
.sftp-tree-row.is-drop-target {
  position: relative;
  z-index: 13;
  background: color-mix(in srgb, var(--primary) 24%, var(--background));
  box-shadow: inset 0 0 0 2px var(--primary);
}

.rename-input {
  min-width: 0;
  flex: 1 1 auto;
  height: 22px;
}
</style>
