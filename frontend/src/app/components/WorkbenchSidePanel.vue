<script setup lang="ts">
import { ref } from "vue";
import { RefreshCw, X } from "@lucide/vue";
import SftpWorkspacePanel from "./SftpWorkspacePanel.vue";
import ServerMetricsPanel, {
  type MetricsProcessRow,
  type MetricsProcessSortKey,
  type ServerMetricsView,
} from "../../features/metrics/components/ServerMetricsPanel.vue";
import type { SftpSanitizedEntry } from "../../features/sftp/lib/entries/sftpEntries";
import type { SftpTypeFilter } from "../../features/sftp/lib/entries/sftpFileFilters";
import type { SftpBookmark } from "../../features/sftp/lib/paths/sftpBookmarks";
import type { SftpFileTreeNode } from "../../features/sftp/lib/tree/sftpFileTree";
import type { CustomEditor, KnownEditor } from "../../features/sftp/lib/editor/editorRules";

/** 右侧栏外壳（T21 自 App.vue 模板迁入）：SFTP / 指标两视图互斥的
 *  `<section class="sftp-pane right-panel">` 整段，含面板根 ref（sftpPane 经
 *  defineExpose 回传 App，保持 focusSftpPaneOnPointerDown 的既有语义）、两视图
 *  切换、SFTP 工作区与指标视图包裹。可见性门与分隔条仍由 App 持有：
 *  分隔条的拖拽比例属于外层 pane 布局，不随本组件迁移。容器管理已迁到终端列
 *  底部的独立分屏（features/docker/components/ContainerManagerPane.vue）。
 *
 *  数据面全部显式下传：只读态走 props（SFTP 工作区按组件 props 形状整体传递），
 *  可写模型（路径/编辑态/弹层开关/搜索草稿/列过滤/右键菜单负载…）走 v-model
 *  链，动作经类型化 emit 上抛——组件内不 inject、不读全局，也不持有会话逻辑。 */

type SftpModelKey =
  | "currentPath"
  | "pathBarEditing"
  | "bookmarkLabelDraft"
  | "bookmarksOpen"
  | "sftpSearchDraft"
  | "sftpTypeFilter"
  | "fileMenu"
  // 树内重命名/拖放落点：子组件侧以 prop + update 事件成对出现，本组件按
  // v-model 模型接管，故从只读集合中剔除。
  | "renamingPath"
  | "sftpDragDropTarget";

/** SFTP 工作区的只读 props（模型与 t/locale 单独绑定），由子组件 props 形状
 *  派生，避免两处字段清单漂移。 */
type SftpReadonlyProps = Omit<
  InstanceType<typeof SftpWorkspacePanel>["$props"],
  SftpModelKey | "t" | "locale"
>;

type SidePanelShell = {
  /** 右侧栏整体开合（SFTP / 指标共用）。 */
  open: boolean;
  isRdpConnection: boolean;
  rightPanelView: "sftp" | "metrics";
  localUiMode: boolean;
  /** 终端拖入的高亮态（面板根 drag-active）。 */
  dragActive: boolean;
  /** SFTP 视图可见：面板根才接收粘贴/拖放（指标不应抢终端的落点）。 */
  sftpViewActive: boolean;
};

type SidePanelMetrics = {
  view: ServerMetricsView;
  distroBadge: { color: string; name: string; label: string } | null;
  hostname?: string | null;
  loading: boolean;
};

type SftpFileMenu = { entry: SftpSanitizedEntry; selection: string[] };

// 响应式 props 解构（Vue 3.5）：模板里沿用迁移前的绑定名（t/sftp/metrics…），
// 同时保持 prop 更新时的实时求值（编译器改写为 __props.xxx 访问）。
const { t, locale, shell, sftp, metrics } = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  locale: string;
  shell: SidePanelShell;
  sftp: SftpReadonlyProps;
  metrics: SidePanelMetrics;
}>();

const emit = defineEmits<{
  (e: "focus-pane", event: PointerEvent): void;
  (e: "sftp-paste", event: ClipboardEvent): void;
  (e: "sftp-drag-enter", event: DragEvent): void;
  (e: "sftp-drag-leave", event: DragEvent): void;
  (e: "sftp-drop", event: DragEvent): void;
  (e: "refresh-metrics"): void;
  (e: "toggle-processes"): void;
  (e: "refresh-processes"): void;
  (e: "kill-process", row: MetricsProcessRow, signal: 15 | 9): void;
  (e: "update:sort", key: MetricsProcessSortKey): void;
  (e: "toggle-disks-density"): void;
  (e: "toggle-nics-density"): void;
  (e: "close-panel"): void;
  (e: "load-home"): void;
  (e: "load-directory"): void;
  (e: "set-directory-tracking", enabled: boolean): void;
  (e: "submit-path-input"): void;
  (e: "cancel-path-bar-edit"): void;
  (e: "begin-path-bar-edit"): void;
  (e: "go-to-path", path: string): void;
  (e: "confirm-bookmark-save"): void;
  (e: "toggle-bookmarks-menu"): void;
  (e: "remove-bookmark", bookmark: SftpBookmark): void;
  (e: "paste-clipboard"): void;
  (e: "toggle-sudo-mode"): void;
  (e: "clear-sftp-search"): void;
  (e: "toggle-sftp-show-hidden"): void;
  (e: "batch-archive"): void;
  (e: "clear-row-selection"): void;
  (e: "batch-download"): void;
  (e: "copy-selected-paths"): void;
  (e: "file-area-context-menu", event: MouseEvent): void;
  (e: "toggle-sftp-tree-node", node: SftpFileTreeNode): void;
  (e: "select-tree-entry", entry: SftpSanitizedEntry, event: MouseEvent | KeyboardEvent): void;
  (e: "open-sftp-tree-node", node: SftpFileTreeNode): void;
  (e: "sftp-tree-drop", node: SftpFileTreeNode, event: DragEvent): void;
  (e: "show-file-menu", event: MouseEvent, entry: SftpSanitizedEntry): void;
  (e: "commit-tree-rename", node: SftpFileTreeNode, value: string): void;
  (e: "file-row-keydown", event: KeyboardEvent, entry: SftpSanitizedEntry): void;
  (e: "folder-menu-action", action: "mkdir" | "newFile" | "upload" | "uploadFolder" | "symlink", entry: SftpSanitizedEntry): void;
  (e: "open-entry", entry: SftpSanitizedEntry): void;
  (e: "download-entry", entry: SftpSanitizedEntry, useSudo: boolean): void;
  (e: "refresh-sftp-folder", entry: SftpSanitizedEntry): void;
  (e: "ensure-external-editors"): void;
  (e: "open-with-system-entry", entry: SftpSanitizedEntry): void;
  (e: "open-with-known-entry", entry: SftpSanitizedEntry, editor: KnownEditor): void;
  (e: "open-with-custom-entry", entry: SftpSanitizedEntry, editor: CustomEditor): void;
  (e: "begin-custom-editor-command", entry: SftpSanitizedEntry): void;
  (e: "begin-symlink-edit", entry: SftpSanitizedEntry): void;
  (e: "begin-rename", entry: SftpSanitizedEntry): void;
  (e: "copy-selected-entries", mode: "copy" | "cut"): void;
  (e: "copy-text-to-clipboard", value: string, noticeKey: string): void;
  (e: "begin-chmod", entry: SftpSanitizedEntry): void;
  (e: "open-attributes", entry: SftpSanitizedEntry): void;
  (e: "archive-entry", entry: SftpSanitizedEntry): void;
  (e: "extract-entry", entry: SftpSanitizedEntry): void;
}>();

const currentPath = defineModel<string>("currentPath", { required: true });
const pathBarEditing = defineModel<boolean>("pathBarEditing", { required: true });
const bookmarkLabelDraft = defineModel<string>("bookmarkLabelDraft", { required: true });
const bookmarksOpen = defineModel<boolean>("bookmarksOpen", { required: true });
const sftpSearchDraft = defineModel<string>("sftpSearchDraft", { required: true });
const sftpTypeFilter = defineModel<SftpTypeFilter>("sftpTypeFilter", { required: true });
const fileMenu = defineModel<SftpFileMenu | undefined>("fileMenu", { required: true });
const batchDeleteOpen = defineModel<boolean>("batchDeleteOpen", { required: true });
const deleteTarget = defineModel<SftpSanitizedEntry | undefined>("deleteTarget", { required: true });
const renamingPath = defineModel<string>("renamingPath", { required: true });
const sftpDragDropTarget = defineModel<string>("sftpDragDropTarget", { required: true });
const otpPanelOpen = defineModel<boolean>("otpPanelOpen", { required: true });

/** 面板根包装元素：App 的 focusSftpPaneOnPointerDown 经 defineExpose 回填使用。 */
const sftpPane = ref<HTMLElement>();

defineExpose({ sftpPane });
</script>

<template>
  <section
    v-if="shell.open && !shell.isRdpConnection && !shell.localUiMode"
    ref="sftpPane"
    class="sftp-pane right-panel"
    tabindex="-1"
    :class="{ 'drag-active': shell.dragActive && shell.sftpViewActive }"
    @pointerdown="emit('focus-pane', $event)"
    @paste.capture="shell.sftpViewActive && emit('sftp-paste', $event)"
    @dragenter.prevent="shell.sftpViewActive && emit('sftp-drag-enter', $event)"
    @dragover.prevent
    @dragleave="shell.sftpViewActive && emit('sftp-drag-leave', $event)"
    @drop.prevent="shell.sftpViewActive && emit('sftp-drop', $event)"
  >
    <SftpWorkspacePanel
      v-if="shell.rightPanelView === 'sftp'"
      v-bind="sftp"
      v-model:currentPath="currentPath"
      v-model:pathBarEditing="pathBarEditing"
      v-model:bookmarkLabelDraft="bookmarkLabelDraft"
      v-model:bookmarksOpen="bookmarksOpen"
      v-model:sftpSearchDraft="sftpSearchDraft"
      v-model:sftpTypeFilter="sftpTypeFilter"
      v-model:renamingPath="renamingPath"
      v-model:sftpDragDropTarget="sftpDragDropTarget"
      :t="t"
      :locale="locale"
      :file-menu="fileMenu"
      @update:fileMenu="fileMenu = $event"
      @update:batchDeleteOpen="batchDeleteOpen = true"
      @update:deleteTarget="deleteTarget = $event"
      @update:otpPanelOpen="otpPanelOpen = true"
      @load-home="emit('load-home')"
      @load-directory="emit('load-directory')"
      @set-directory-tracking="emit('set-directory-tracking', $event)"
      @submit-path-input="emit('submit-path-input')"
      @cancel-path-bar-edit="emit('cancel-path-bar-edit')"
      @begin-path-bar-edit="emit('begin-path-bar-edit')"
      @go-to-path="emit('go-to-path', $event)"
      @confirm-bookmark-save="emit('confirm-bookmark-save')"
      @toggle-bookmarks-menu="emit('toggle-bookmarks-menu')"
      @remove-bookmark="emit('remove-bookmark', $event)"
      @paste-clipboard="emit('paste-clipboard')"
      @toggle-sudo-mode="emit('toggle-sudo-mode')"
      @clear-sftp-search="emit('clear-sftp-search')"
      @toggle-sftp-show-hidden="emit('toggle-sftp-show-hidden')"
      @batch-archive="emit('batch-archive')"
      @clear-row-selection="emit('clear-row-selection')"
      @batch-download="emit('batch-download')"
      @copy-selected-paths="emit('copy-selected-paths')"
      @file-area-context-menu="emit('file-area-context-menu', $event)"
      @toggle-sftp-tree-node="emit('toggle-sftp-tree-node', $event)"
      @select-tree-entry="(entry, event) => emit('select-tree-entry', entry, event)"
      @open-sftp-tree-node="emit('open-sftp-tree-node', $event)"
      @sftp-tree-drop="(node, event) => emit('sftp-tree-drop', node, event)"
      @show-file-menu="(event, entry) => emit('show-file-menu', event, entry)"
      @commit-tree-rename="(node, value) => emit('commit-tree-rename', node, value)"
      @file-row-keydown="(event, entry) => emit('file-row-keydown', event, entry)"
      @folder-menu-action="(action, entry) => emit('folder-menu-action', action, entry)"
      @open-entry="emit('open-entry', $event)"
      @download-entry="(entry, useSudo) => emit('download-entry', entry, useSudo)"
      @refresh-sftp-folder="emit('refresh-sftp-folder', $event)"
      @ensure-external-editors="emit('ensure-external-editors')"
      @open-with-system-entry="emit('open-with-system-entry', $event)"
      @open-with-known-entry="(entry, editor) => emit('open-with-known-entry', entry, editor)"
      @open-with-custom-entry="(entry, editor) => emit('open-with-custom-entry', entry, editor)"
      @begin-custom-editor-command="emit('begin-custom-editor-command', $event)"
      @begin-symlink-edit="emit('begin-symlink-edit', $event)"
      @begin-rename="emit('begin-rename', $event)"
      @copy-selected-entries="emit('copy-selected-entries', $event)"
      @copy-text-to-clipboard="(value, noticeKey) => emit('copy-text-to-clipboard', value, noticeKey)"
      @begin-chmod="emit('begin-chmod', $event)"
      @open-attributes="emit('open-attributes', $event)"
      @archive-entry="emit('archive-entry', $event)"
      @extract-entry="emit('extract-entry', $event)"
    />
    <template v-else-if="shell.rightPanelView === 'metrics'">
      <header class="side-panel-head">
        <h2>{{ t("metrics") }}</h2>
        <span class="side-panel-head-actions">
          <button class="icon-button compact" :title="t('refresh')" :aria-label="t('refresh')" :disabled="metrics.loading" @click="emit('refresh-metrics')"><RefreshCw :class="{ spinning: metrics.loading }" /></button>
          <button class="icon-button compact" :title="t('close')" :aria-label="t('close')" @click="emit('close-panel')"><X /></button>
        </span>
      </header>
      <ServerMetricsPanel
        :view="metrics.view"
        :t="t"
        :locale="locale"
        @refresh="emit('refresh-metrics')"
        @toggle-processes="emit('toggle-processes')"
        @refresh-processes="emit('refresh-processes')"
        @kill-process="(row, signal) => emit('kill-process', row, signal)"
        @update:sort="(key) => emit('update:sort', key)"
        @toggle-disks-density="emit('toggle-disks-density')"
        @toggle-nics-density="emit('toggle-nics-density')"
      />
    </template>
  </section>
</template>
