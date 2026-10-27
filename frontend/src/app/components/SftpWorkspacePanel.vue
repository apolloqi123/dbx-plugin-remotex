<script setup lang="ts">
import { computed, type ComponentPublicInstance } from "vue";
import {
  Archive,
  Check,
  ChevronRight,
  Copy,
  Download,
  ExternalLink,
  Eye,
  EyeOff,
  FilePlus,
  FileText,
  FileUp,
  Folder,
  FolderPlus,
  FolderUp,
  Info,
  Link2,
  Lock,
  PackageOpen,
  Pencil,
  RefreshCw,
  Save,
  Scissors,
  Search,
  Trash2,
  TriangleAlert,
  X,
} from "@lucide/vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import WorkbenchGlyph from "../../shared/ui/icons/WorkbenchGlyph.vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../shared/ui/select/index";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "../../shared/ui/context-menu/index";
import SftpTreePane from "../../features/sftp/components/SftpTreePane.vue";
import type { SftpSanitizedEntry } from "../../features/sftp/lib/entries/sftpEntries";
import type { SftpDetailInfo } from "../../features/sftp/lib/actions/sftpDetails";
import type { SftpFileTreeNode, SftpTreeRow } from "../../features/sftp/lib/tree/sftpFileTree";
import type { SftpTypeFilter } from "../../features/sftp/lib/entries/sftpFileFilters";
import { SFTP_BOOKMARK_LABEL_MAX_LENGTH, type SftpBookmark } from "../../features/sftp/lib/paths/sftpBookmarks";
import type { CustomEditor, EditorConfig, KnownEditor } from "../../features/sftp/lib/editor/editorRules";
import type { SftpSoftError } from "../../features/sftp/composables/listing/useSftpNavigation";
import { batchProgressPercent, type BatchProgressState } from "../../features/transfers/lib/sftpBatchProgress";
import type { RemotePathSegment } from "../../features/sftp/lib/paths/remotePathInput";
import { formatBytes } from "../../shared/lib/format";
import { pathFromUri } from "../../features/sftp/lib/paths/sftpRemotePaths";

/** 右侧 SFTP 工作区面板：路径工具栏（返回/主目录/刷新/跟随/sudo/OTP/
 *  书签与快捷路径弹层）、面包屑导航、过滤/批量条、单栏文件树与右键菜单、底栏
 *  状态和磁盘用量。状态与 RPC 仍由 App 的 SFTP composables 持有，本组件只
 *  渲染并上抛动作；面板根 <section> 的焦点/拖放/粘贴监听仍留在 App（同节
 *  点还承载指标/容器两个非 SFTP 视图），因此根 ref 语义不变。 */
type DiskUsage = {
  filesystem: string;
  mount: string;
  totalBytes: number;
  usedBytes: number;
  availableBytes: number;
  percentUsed: number;
};
type SftpClipboardState = { mode: "copy" | "cut"; paths: string[]; connectionId: string };
type SftpFileMenu = { entry: SftpSanitizedEntry; selection: string[] };
type TreePaneApi = { scrollPathToTop(path: string): Promise<boolean>; markDropTargetAt(point: { x: number; y: number }): string; resetHostDragCalibration(): void };

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  locale: string;
  connected: boolean;
  loadingFiles: boolean;
  panelSurface: boolean;
  followDirectory: boolean;
  currentPath: string;
  pathBarEditing: boolean;
  pathCrumbs: RemotePathSegment[];
  bookmarkSaveOpen: boolean;
  bookmarkSaving: boolean;
  bookmarkLabelDraft: string;
  bookmarksOpen: boolean;
  sftpBookmarks: SftpBookmark[];
  quickPaths: readonly string[];
  canWrite: boolean;
  sftpClipboard: SftpClipboardState | undefined;
  pasteBusy: boolean;
  sudoMode: boolean;
  sftpSearchDraft: string;
  sftpTypeFilter: SftpTypeFilter;
  sftpShowHidden: boolean;
  sftpFiltersActive: boolean;
  selectedEntries: ReadonlyArray<SftpSanitizedEntry>;
  batchProgress: BatchProgressState | null;
  archiveBusy: boolean;
  batchDeleteSubmitting: boolean;
  sftpSoftError: SftpSoftError | null;
  fileMenu: SftpFileMenu | undefined;
  fileMenuProtected: boolean;
  sftpTreeRows: SftpTreeRow[];
  selectedUriSet: Set<string>;
  selectedPath: string;
  renamingPath: string;
  renameDraft: string;
  renameSubmitting: boolean;
  loadSftpDetails: (entry: SftpSanitizedEntry) => Promise<SftpDetailInfo | null>;
  dragActive: boolean;
  sftpDragDropTarget: string;
  sftpTreeEntries: ReadonlyArray<SftpSanitizedEntry>;
  sftpTreeTotalRows: ReadonlyArray<SftpTreeRow>;
  loadedTreeEntries: ReadonlyArray<SftpSanitizedEntry>;
  diskUsage: DiskUsage | undefined;
  folderUploadSupported: boolean;
  localCanSave: boolean;
  availableExternalEditors: ReadonlyArray<KnownEditor>;
  editorConfig: EditorConfig;
  isArchiveName: (name: string) => boolean;
  mappedEditorIdFor: (fileName: string) => string;
  /** 路径输入框/树实例以回调回填父级 ref，保持 beginPathBarEdit 聚焦与
   *  目录跟随 scrollPathToTop 的既有语义。 */
  setPathBarInputEl?: (el: HTMLInputElement | null) => void;
  onTreePane?: (api: TreePaneApi | null) => void;
}>();

const emit = defineEmits<{
  (e: "update:currentPath", value: string): void;
  (e: "update:pathBarEditing", value: boolean): void;
  (e: "update:bookmarkSaveOpen", value: boolean): void;
  (e: "update:bookmarkLabelDraft", value: string): void;
  (e: "update:bookmarksOpen", value: boolean): void;
  (e: "update:sftpSearchDraft", value: string): void;
  (e: "update:sftpTypeFilter", value: SftpTypeFilter): void;
  (e: "update:fileMenu", value: SftpFileMenu | undefined): void;
  (e: "update:batchDeleteOpen", value: boolean): void;
  (e: "update:deleteTarget", value: SftpSanitizedEntry): void;
  (e: "update:renamingPath", value: string): void;
  (e: "update:sftpDragDropTarget", value: string): void;
  (e: "update:otpPanelOpen", value: boolean): void;
  (e: "go-parent"): void;
  (e: "load-home"): void;
  (e: "load-directory"): void;
  (e: "set-directory-tracking", enabled: boolean): void;
  (e: "submit-path-input"): void;
  (e: "cancel-path-bar-edit"): void;
  (e: "begin-path-bar-edit"): void;
  (e: "go-to-path", path: string): void;
  (e: "toggle-bookmark-save"): void;
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

const currentPathModel = computed({
  get: () => props.currentPath,
  set: (value: string) => emit("update:currentPath", value),
});
const bookmarkLabelDraftModel = computed({
  get: () => props.bookmarkLabelDraft,
  set: (value: string) => emit("update:bookmarkLabelDraft", value),
});
const sftpSearchDraftModel = computed({
  get: () => props.sftpSearchDraft,
  set: (value: string) => emit("update:sftpSearchDraft", value),
});

function setPathBarInputEl(el: Element | ComponentPublicInstance | null) {
  props.setPathBarInputEl?.(el instanceof HTMLInputElement ? el : null);
}

function setTreePane(el: Element | ComponentPublicInstance | null) {
  props.onTreePane?.((el as unknown as TreePaneApi | null) ?? null);
}
</script>

<template>
  <div class="path-toolbar">
    <!-- 导航：上级目录 → 主目录 → 刷新 → 跟随终端目录（由左至右的浏览顺序）。 -->
    <button class="icon-button" :title="t('parentFolder')" :aria-label="t('parentFolder')" :disabled="currentPath === '/'" @click="emit('go-parent')"><WorkbenchGlyph name="parent" /></button>
    <button class="icon-button icon-amber" :title="t('home')" :aria-label="t('home')" :disabled="!connected" @click="emit('load-home')"><WorkbenchGlyph name="home" /></button>
    <button class="icon-button icon-cyan" :title="t('refresh')" :aria-label="t('refresh')" :disabled="!connected || loadingFiles" @click="emit('load-directory')"><WorkbenchGlyph name="refresh" :class="{ spinning: loadingFiles }" /></button>
    <!-- 跟随终端目录：开启时高亮图标，关闭时弱化；偏好与主终端 cwd 同步逻辑不变。 -->
    <button
      class="icon-button"
      :class="followDirectory ? 'icon-emerald is-active' : 'icon-neutral'"
      :title="t('followTerminal')"
      :aria-label="t('followTerminal')"
      :aria-pressed="followDirectory"
      :disabled="!connected || panelSurface"
      @click="emit('set-directory-tracking', !followDirectory)"
    >
      <WorkbenchGlyph :name="followDirectory ? 'follow' : 'follow-off'" />
    </button>
    <!-- #54 路径栏双态：非编辑态把当前路径渲染成可点击分段（末段为当前位置），
         点击分段直接回跳、点击分段外区域进入编辑；编辑态是原先的完整输入框，
         提交后回到分段展示。分段切分走 remotePathInput（有单测）。 -->
    <div class="path-bar">
      <input
        v-show="pathBarEditing"
        :ref="setPathBarInputEl"
        v-model="currentPathModel"
        spellcheck="false"
        @keydown.enter="emit('submit-path-input')"
        @keydown.esc="emit('cancel-path-bar-edit')"
        @blur="emit('update:pathBarEditing', false)"
      />
      <nav v-show="!pathBarEditing" class="path-crumbs" tabindex="0" @click="emit('begin-path-bar-edit')" @keydown.enter.self.prevent="emit('begin-path-bar-edit')">
        <template v-for="(crumb, index) in pathCrumbs" :key="crumb.path">
          <!-- 根段本身显示为 "/"，其后不再渲染分隔符，避免「// home」双斜杠 -->
          <span v-if="index && pathCrumbs[index - 1].name !== '/'" class="path-crumb-sep" aria-hidden="true">/</span>
          <button v-if="index < pathCrumbs.length - 1" class="path-crumb mono" :title="crumb.path" @click.stop="emit('go-to-path', crumb.path)">{{ crumb.name }}</button>
          <span v-else class="path-crumb current mono" :title="crumb.path" aria-current="location">{{ crumb.name }}</span>
        </template>
      </nav>
    </div>
    <!-- 书签：列表（含快捷路径）在左，收藏当前目录在右，符合「先看再存」的顺序。 -->
    <div>
      <Popover :open="bookmarksOpen" @update:open="(open) => { if (!open) emit('update:bookmarksOpen', false); }">
        <PopoverAnchor as-child>
          <button class="icon-button icon-amber" :title="t('sftpBookmark.title')" :aria-label="t('sftpBookmark.title')" :disabled="!connected" @pointerdown.stop @click.stop="emit('toggle-bookmarks-menu')"><WorkbenchGlyph name="bookmark-list" /></button>
        </PopoverAnchor>
        <PopoverContent class="popover bookmark-list-popover" align="end" :side-offset="5">
        <strong class="popover-section-title">{{ t("sftpBookmark.title") }}</strong>
        <template v-if="sftpBookmarks.length">
          <div v-for="bookmark in sftpBookmarks" :key="bookmark.id" class="bookmark-row">
            <button class="path-item mono" :title="`${bookmark.label} · ${bookmark.path}`" :aria-label="`${bookmark.label} · ${bookmark.path}`" @click="emit('go-to-path', bookmark.path)">{{ bookmark.label }}</button>
            <button class="bookmark-delete" :title="t('delete')" :aria-label="t('delete')" @click.stop="emit('remove-bookmark', bookmark)"><Trash2 /></button>
          </div>
        </template>
        <div v-else class="empty compact">{{ t("sftpBookmark.empty") }}</div>
        <strong class="popover-section-title">{{ t("sftpQuickPath.title") }}</strong>
        <button v-for="item in quickPaths" :key="item" class="path-item mono" :title="item" :aria-label="item" @click="emit('go-to-path', item)">{{ item }}</button>
        </PopoverContent>
      </Popover>
    </div>
    <div>
      <Popover :open="bookmarkSaveOpen" @update:open="(open) => { if (!open) emit('update:bookmarkSaveOpen', false); }">
        <PopoverAnchor as-child>
          <button class="icon-button icon-amber" :title="t('sftpBookmark.add')" :aria-label="t('sftpBookmark.add')" :disabled="!connected" @pointerdown.stop @click.stop="emit('toggle-bookmark-save')"><WorkbenchGlyph name="bookmark-add" /></button>
        </PopoverAnchor>
        <!-- 星标收藏弹层：label 默认取路径末段，可编辑后保存（前端先行校验 + 后端错误回显） -->
        <PopoverContent class="popover bookmark-save-popover" align="end" :side-offset="5">
        <strong class="popover-section-title">{{ t("sftpBookmark.add") }}</strong>
        <span class="bookmark-save-path mono" :title="currentPath">{{ currentPath }}</span>
        <input v-model="bookmarkLabelDraftModel" class="bookmark-label-input mono" :maxlength="SFTP_BOOKMARK_LABEL_MAX_LENGTH" spellcheck="false" :placeholder="t('sftpBookmark.namePlaceholder')" :disabled="bookmarkSaving" autofocus @keydown.enter="emit('confirm-bookmark-save')" />
        <div class="bookmark-save-actions">
          <button class="icon-button icon-emerald" :title="t('save')" :aria-label="t('save')" :disabled="bookmarkSaving" @click="emit('confirm-bookmark-save')"><Save /></button>
          <button class="icon-button" :title="t('cancel')" :aria-label="t('cancel')" :disabled="bookmarkSaving" @click="emit('update:bookmarkSaveOpen', false)"><X /></button>
        </div>
        </PopoverContent>
      </Popover>
    </div>
    <!-- 文件操作：粘贴 → OTP → sudo（sudo 是模式开关，固定在最右缘）。 -->
    <button class="icon-button" :title="t('sftpPaste.action')" :aria-label="t('sftpPaste.action')" :disabled="!connected || !canWrite || !sftpClipboard || pasteBusy" @click="emit('paste-clipboard')"><WorkbenchGlyph name="paste" /></button>
    <button class="icon-button icon-violet" :title="t('otpPanel.title')" :aria-label="t('otpPanel.title')" @click="emit('update:otpPanelOpen', true)"><WorkbenchGlyph name="otp-key" /></button>
    <button
      class="sudo-toggle"
      :class="{ 'is-on': sudoMode }"
      type="button"
      :aria-pressed="sudoMode"
      :title="!canWrite ? t('readOnly') : t('sudo.modeHint')"
      :disabled="!connected || !canWrite"
      @click="emit('toggle-sudo-mode')"
    >sudo</button>
  </div>
  <!-- SFTP 面板主体：单栏文件树（文件夹 + 文件同列），无侧栏。 -->
  <div class="sftp-body">
    <div class="sftp-main">
    <div class="sftp-filter-bar">
      <label class="sftp-search-input">
        <Search />
        <input v-model="sftpSearchDraftModel" type="search" :placeholder="t('sftpSearch.placeholder')" spellcheck="false" />
        <button v-if="sftpSearchDraft" class="sftp-search-clear" :title="t('cancel')" :aria-label="t('cancel')" @click.prevent="emit('clear-sftp-search')"><X /></button>
      </label>
      <Select :model-value="sftpTypeFilter" @update:model-value="(v) => emit('update:sftpTypeFilter', v as SftpTypeFilter)">
        <SelectTrigger size="xs" class="sftp-type-filter" :title="t('sftpFilter.all')">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="all">{{ t("sftpFilter.all") }}</SelectItem>
          <SelectItem value="directory">{{ t("sftpFilter.folders") }}</SelectItem>
          <SelectItem value="file">{{ t("sftpFilter.files") }}</SelectItem>
        </SelectContent>
      </Select>
      <button
        class="icon-button sftp-hidden-toggle"
        :class="{ 'is-active': sftpShowHidden }"
        :title="sftpShowHidden ? t('sftpFilter.hideHidden') : t('sftpFilter.showHidden')" :aria-label="sftpShowHidden ? t('sftpFilter.hideHidden') : t('sftpFilter.showHidden')"
        :aria-pressed="sftpShowHidden"
        @click="emit('toggle-sftp-show-hidden')"
      >
        <Eye v-if="sftpShowHidden" />
        <EyeOff v-else />
      </button>
    </div>
    <div v-if="selectedEntries.length > 1" class="sftp-batch-bar">
      <span>{{ t("sftpBatch.selected", { count: selectedEntries.length }) }}</span>
      <template v-if="batchProgress">
        <progress class="batch-progress-bar" :value="batchProgressPercent(batchProgress)" max="100" />
        <span class="batch-progress mono">{{ t("sftpBatch.progress", { done: batchProgress.done, total: batchProgress.total }) }}</span>
      </template>
      <button :disabled="!canWrite || archiveBusy || batchDeleteSubmitting" @click="emit('batch-archive')"><Archive />{{ t("sftpBatch.archive") }}</button>
      <button class="danger" :disabled="!canWrite || archiveBusy || batchDeleteSubmitting" @click="emit('update:batchDeleteOpen', true)"><Trash2 />{{ t("sftpBatch.delete") }}</button>
      <button @click="emit('clear-row-selection')"><X />{{ t("sftpBatch.clear") }}</button>
    </div>
    <!-- #209 目录跟随/连接后自动首拉失败的安静态：不弹全局错误横幅，
         在面板内给一条低调的软提示（原始错误进悬停 tooltip）+ 重试按钮。
         重试按手动导航语义执行（再失败才落全局横幅），成功或任一后续
         手动导航成功即清除；断开会话时一并清除。 -->
    <div v-if="sftpSoftError" class="sftp-soft-error" role="status" :title="sftpSoftError.detail">
      <TriangleAlert class="sftp-soft-error-icon" />
      <span class="sftp-soft-error-text">{{ sftpSoftError.message }}</span>
      <button class="sftp-soft-error-action" @click="sftpSoftError.action.run()">{{ sftpSoftError.action.label }}</button>
    </div>
    <!-- 行右键（文件操作）与空白处右键（新建/刷新）共用同一 ContextMenu 根：
         行事件由 SftpTreePane 上抛 showFileMenu 设置负载，空白区由
         onFileAreaContextMenu 兜底；定位/碰撞/Esc/外点关闭均由 reka 承担。
         树行 contextmenu 不 preventDefault，好让事件继续到本触发器。 -->
    <ContextMenu :open="!!fileMenu" @update:open="(open) => { if (!open) emit('update:fileMenu', undefined); }">
      <ContextMenuTrigger class="contents">
        <SftpTreePane
          :ref="setTreePane"
          :rows="sftpTreeRows"
          :follow-padding="followDirectory"
          :selected-uris="selectedUriSet"
          :selected-path="selectedPath"
          :current-path="currentPath"
          :renaming-path="renamingPath"
          :rename-draft="renameDraft"
          :rename-submitting="renameSubmitting"
          :t="t"
          :locale="locale"
          :load-details="loadSftpDetails"
          :external-drag-active="dragActive"
          :empty-text="sftpFiltersActive ? t('sftpSearch.noMatch') : ''"
          @contextmenu="(event: MouseEvent) => emit('file-area-context-menu', event)"
          @toggle="(node) => emit('toggle-sftp-tree-node', node)"
          @select="(entry, event) => emit('select-tree-entry', entry, event)"
          @open="(node) => emit('open-sftp-tree-node', node)"
          @drop-on="(node, event) => emit('sftp-tree-drop', node, event)"
          @drop-target-change="(path) => emit('update:sftpDragDropTarget', path)"
          @context="(node, event) => emit('show-file-menu', event, node.entry)"
          @rename-commit="(node, value) => emit('commit-tree-rename', node, value)"
          @rename-cancel="emit('update:renamingPath', '')"
          @row-keydown="(node, event) => emit('file-row-keydown', event, node.entry)"
        />
      </ContextMenuTrigger>
      <ContextMenuContent>
          <!-- 多选感知：右键时已多选（selection > 1）→ 菜单整体切换为批量区，单项动作隐藏 -->
          <template v-if="fileMenu && !fileMenuProtected && fileMenu.selection.length > 1">
            <ContextMenuItem @select="emit('batch-download')"><Download />{{ t("download") }}</ContextMenuItem>
            <ContextMenuItem :disabled="!canWrite || archiveBusy || batchDeleteSubmitting" @select="emit('batch-archive')"><Archive />{{ t("sftpBatch.archive") }}</ContextMenuItem>
            <ContextMenuItem variant="destructive" :disabled="!canWrite || archiveBusy || batchDeleteSubmitting" @select="emit('update:batchDeleteOpen', true)"><Trash2 />{{ t("sftpBatch.delete") }}</ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem @select="emit('copy-selected-paths')"><Copy />{{ t("sftpCopy.copySelected") }}</ContextMenuItem>
            <!-- 多选态右键命中的是文件夹时，仍提供该目录下的新建/上传。 -->
            <template v-if="fileMenu.entry.kind === 'directory'">
              <ContextMenuSeparator />
              <ContextMenuItem :disabled="!connected || !canWrite" @select="emit('folder-menu-action', 'mkdir', fileMenu.entry)"><FolderPlus />{{ t("newFolder") }}</ContextMenuItem>
              <ContextMenuItem :disabled="!connected || !canWrite" @select="emit('folder-menu-action', 'newFile', fileMenu.entry)"><FilePlus />{{ t("sftpNewFile.action") }}</ContextMenuItem>
              <ContextMenuItem :disabled="!connected || !canWrite" @select="emit('folder-menu-action', 'symlink', fileMenu.entry)"><Link2 />{{ t("symlink.createAction") }}</ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem :disabled="!connected || !canWrite" @select="emit('folder-menu-action', 'upload', fileMenu.entry)"><FileUp />{{ t("upload") }}</ContextMenuItem>
              <ContextMenuItem v-if="folderUploadSupported" :disabled="!connected || !canWrite" @select="emit('folder-menu-action', 'uploadFolder', fileMenu.entry)"><FolderUp />{{ t("folderUpload.action") }}</ContextMenuItem>
            </template>
          </template>
          <template v-else-if="fileMenu">
            <ContextMenuItem v-if="fileMenu.entry.kind === 'directory' || fileMenu.entry.kind === 'file'" @select="emit('open-entry', fileMenu.entry)"><Folder v-if="fileMenu.entry.kind === 'directory'" /><FileText v-else />{{ fileMenu.entry.kind === "directory" ? t("openFolder") : t("preview") }}</ContextMenuItem>
            <ContextMenuItem v-if="fileMenu.entry.kind === 'file' || fileMenu.entry.kind === 'directory'" @select="emit('download-entry', fileMenu.entry, sudoMode && canWrite && fileMenu.entry.kind === 'file')"><Download />{{ t("download") }}</ContextMenuItem>
            <ContextMenuItem v-if="fileMenu.entry.kind === 'directory'" :disabled="!connected || loadingFiles" @select="emit('refresh-sftp-folder', fileMenu.entry)"><RefreshCw />{{ t("refresh") }}</ContextMenuItem>
            <!-- 文件夹行右键：新建/上传落在被右键的目录内（不是面板当前目录）。 -->
            <template v-if="fileMenu.entry.kind === 'directory'">
              <ContextMenuSeparator />
              <ContextMenuItem :disabled="!connected || !canWrite" @select="emit('folder-menu-action', 'mkdir', fileMenu.entry)"><FolderPlus />{{ t("newFolder") }}</ContextMenuItem>
              <ContextMenuItem :disabled="!connected || !canWrite" @select="emit('folder-menu-action', 'newFile', fileMenu.entry)"><FilePlus />{{ t("sftpNewFile.action") }}</ContextMenuItem>
              <ContextMenuItem :disabled="!connected || !canWrite" @select="emit('folder-menu-action', 'symlink', fileMenu.entry)"><Link2 />{{ t("symlink.createAction") }}</ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem :disabled="!connected || !canWrite" @select="emit('folder-menu-action', 'upload', fileMenu.entry)"><FileUp />{{ t("upload") }}</ContextMenuItem>
              <ContextMenuItem v-if="folderUploadSupported" :disabled="!connected || !canWrite" @select="emit('folder-menu-action', 'uploadFolder', fileMenu.entry)"><FolderUp />{{ t("folderUpload.action") }}</ContextMenuItem>
              <ContextMenuSeparator />
            </template>
            <!-- 以 root 下载（M14-C DownloadSudo）：root 大文件二进制下载，读只读门禁
                 与 sudo 族一致（sidecar 对只读连接 fail closed，这里 canWrite 同步禁用）。
                 sudo 模式下默认「下载」已自动换 sudo 车道（UploadSudo/DownloadSudo
                 同一判定），该项隐藏避免重复入口。 -->
            <ContextMenuItem v-if="fileMenu.entry.kind === 'file' && !sudoMode" :disabled="!canWrite" @select="emit('download-entry', fileMenu.entry, true)"><Download />{{ t("sudoDownload.action") }}</ContextMenuItem>
            <!-- 外部编辑器打开方式（编辑器映射增强）：唯一入口——独立
                 「在外部编辑器中打开」已并入子菜单（首项系统默认即原
                 行为），避免双入口语义冲突。web/docker 的 sidecar 不在
                 本机，监听与回传都不可用，localCanSave 未探测到前保持
                 禁用；选中即开（一次性，不写关联），关联管理在设置弹窗
                 「外部编辑器」区。勾选标识当前映射解析命中，右对齐。 -->
            <ContextMenuSub v-if="fileMenu.entry.kind === 'file'" @update:open="(open: boolean) => { if (open) emit('ensure-external-editors'); }">
              <ContextMenuSubTrigger :disabled="!canWrite || !localCanSave" class="gap-2"><ExternalLink />{{ t("sftpEdit.openWith") }}<ChevronRight class="ml-auto opacity-60" /></ContextMenuSubTrigger>
              <ContextMenuSubContent class="min-w-52">
                <ContextMenuItem :disabled="!canWrite || !localCanSave" @select="emit('open-with-system-entry', fileMenu.entry)">{{ t("sftpEdit.systemDefault") }}<Check v-if="!mappedEditorIdFor(fileMenu.entry.name)" class="ml-auto" /></ContextMenuItem>
                <template v-if="availableExternalEditors.length">
                  <ContextMenuSeparator />
                  <ContextMenuItem v-for="editor in availableExternalEditors" :key="`known-${editor.id}`" :disabled="!canWrite || !localCanSave" @select="emit('open-with-known-entry', fileMenu.entry, editor)">{{ editor.name }}<Check v-if="mappedEditorIdFor(fileMenu.entry.name) === editor.id" class="ml-auto" /></ContextMenuItem>
                </template>
                <template v-if="editorConfig.customEditors.length">
                  <ContextMenuSeparator />
                  <ContextMenuItem v-for="editor in editorConfig.customEditors" :key="`custom-${editor.id}`" :disabled="!canWrite || !localCanSave" @select="emit('open-with-custom-entry', fileMenu.entry, editor)">{{ editor.name }}<Check v-if="mappedEditorIdFor(fileMenu.entry.name) === editor.id" class="ml-auto" /></ContextMenuItem>
                </template>
                <ContextMenuSeparator />
                <ContextMenuItem :disabled="!canWrite || !localCanSave" @select="emit('begin-custom-editor-command', fileMenu.entry)"><Pencil />{{ t("sftpEdit.customCommand") }}</ContextMenuItem>
              </ContextMenuSubContent>
            </ContextMenuSub>
            <!-- 符号链接改指向（P2-6）：读取现有 target 预填后 update。 -->
            <ContextMenuItem v-if="fileMenu.entry.kind === 'symlink'" :disabled="!canWrite" @select="emit('begin-symlink-edit', fileMenu.entry)"><Link2 />{{ t("symlink.editAction") }}</ContextMenuItem>
            <ContextMenuItem :disabled="!canWrite || fileMenuProtected" @select="emit('begin-rename', fileMenu.entry)"><Pencil />{{ t("rename") }}</ContextMenuItem>
            <ContextMenuItem @select="emit('copy-selected-entries', 'copy')"><Copy />{{ t("sftpCopy.copy") }}</ContextMenuItem>
            <ContextMenuItem :disabled="!canWrite || fileMenuProtected" @select="emit('copy-selected-entries', 'cut')"><Scissors />{{ t("sftpCopy.cut") }}</ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem @select="emit('copy-text-to-clipboard', pathFromUri(fileMenu.entry.uri), 'sftpCopy.copiedPath')"><Copy />{{ t("sftpCopy.copyPath") }}</ContextMenuItem>
            <ContextMenuItem @select="emit('copy-text-to-clipboard', fileMenu.entry.name, 'sftpCopy.copiedName')"><FileText />{{ t("sftpCopy.copyName") }}</ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem :disabled="!canWrite || fileMenuProtected" @select="emit('begin-chmod', fileMenu.entry)"><Lock />{{ t("permissionsEdit") }}</ContextMenuItem>
            <ContextMenuItem @select="emit('open-attributes', fileMenu.entry)"><Info />{{ t("sftpAttrs.action") }}</ContextMenuItem>
            <ContextMenuItem v-if="fileMenu.entry.kind === 'directory' || (fileMenu.entry.kind === 'file' && !isArchiveName(fileMenu.entry.name))" :disabled="!canWrite || archiveBusy || fileMenuProtected" @select="emit('archive-entry', fileMenu.entry)"><Archive />{{ t("archive.action") }}</ContextMenuItem>
            <ContextMenuItem v-if="fileMenu.entry.kind === 'file' && isArchiveName(fileMenu.entry.name)" :disabled="!canWrite || archiveBusy" @select="emit('extract-entry', fileMenu.entry)"><PackageOpen />{{ t("extract.action") }}</ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem variant="destructive" :disabled="!canWrite || fileMenuProtected" @select="emit('update:deleteTarget', fileMenu.entry)"><Trash2 />{{ t("delete") }}</ContextMenuItem>
          </template>
        </ContextMenuContent>
      </ContextMenu>
      <footer class="file-footer"><span>{{ sftpFiltersActive ? t("sftpSearch.footerMatch", { matched: sftpTreeEntries.length, total: sftpTreeTotalRows.length }) : t("items", { count: loadedTreeEntries.length }) }}</span><span v-if="diskUsage" :title="`${diskUsage.filesystem} → ${diskUsage.mount}`">{{ formatBytes(diskUsage.availableBytes) }} {{ t("diskFreeOf", { total: formatBytes(diskUsage.totalBytes) }) }}</span><span>{{ currentPath }}</span></footer>
    </div>
  </div>
  <div v-if="dragActive" class="drop-overlay">
    <FileUp />
    <strong>{{ t("upload") }}<template v-if="sftpDragDropTarget"> · {{ sftpDragDropTarget }}</template></strong>
  </div>
</template>

<style scoped>
/* SFTP 面板扩展（工作包 B）：搜索/类型过滤、批量条、路径历史、属性弹窗。 */
.sftp-filter-bar { display: flex; align-items: center; gap: 6px; border-bottom: 1px solid var(--border); padding: 5px 7px; }
.sftp-search-input { display: flex; flex: 1; min-width: 0; height: 26px; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: var(--radius); padding: 0 7px; background: var(--background); }
.sftp-search-input:focus-within { border-color: color-mix(in srgb, var(--primary) 70%, var(--border)); }
.sftp-search-input svg { width: 13px; height: 13px; flex: 0 0 13px; color: var(--muted-foreground); }
.sftp-search-input input { min-width: 0; flex: 1; border: 0; padding: 0; background: transparent; color: var(--foreground); font-size: 12px; outline: none; }
.sftp-search-clear { display: grid; width: 16px; height: 16px; flex: 0 0 16px; border: 0; border-radius: 50%; padding: 0; place-items: center; background: transparent; color: var(--muted-foreground); cursor: pointer; }
.sftp-search-clear:hover { background: var(--accent); color: var(--foreground); }
.sftp-search-clear svg { width: 11px; height: 11px; }
.sftp-type-filter { flex: 0 0 auto; }
.sftp-hidden-toggle { flex: 0 0 auto; width: 26px; height: 26px; border: 1px solid var(--border); border-radius: var(--radius); background: transparent; color: var(--muted-foreground); cursor: pointer; padding: 0; display: inline-grid; place-items: center; }
.sftp-hidden-toggle:hover { background: var(--accent); color: var(--foreground); }
.sftp-hidden-toggle.is-active { color: var(--primary); border-color: color-mix(in srgb, var(--primary) 55%, var(--border)); }
.sftp-hidden-toggle svg { width: 14px; height: 14px; }
.sftp-batch-bar { display: flex; align-items: center; gap: 6px; border-bottom: 1px solid var(--border); padding: 5px 8px; background: color-mix(in srgb, var(--primary) 8%, var(--background)); color: var(--muted-foreground); font-size: 11px; }
.sftp-batch-bar span { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sftp-batch-bar button { display: inline-flex; height: 24px; align-items: center; gap: 4px; border: 1px solid var(--border); border-radius: var(--radius); padding: 0 8px; background: var(--background); color: var(--foreground); font-size: 11px; cursor: pointer; }
.sftp-batch-bar button:hover:not(:disabled) { background: var(--accent); }
.sftp-batch-bar button.danger { color: var(--destructive); }
.sftp-batch-bar button svg { width: 12px; height: 12px; }
/* #209 面板内软错误条：目录跟随/自动首拉失败的安静提示（替代全局错误横幅）。
   低调灰字 + 警示图标贴条展示；重试按钮沿用 notice-action 的轻量描边样式。 */
.sftp-soft-error { display: flex; align-items: center; gap: 6px; border-bottom: 1px solid var(--border); padding: 4px 8px; background: color-mix(in srgb, var(--warning) 7%, var(--background)); color: var(--muted-foreground); font-size: 11px; }
.sftp-soft-error-icon { width: 12px; height: 12px; flex: 0 0 12px; color: var(--warning); }
.sftp-soft-error-text { min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sftp-soft-error-action { flex: 0 0 auto; display: inline-flex; height: 20px; align-items: center; border: 1px solid var(--border); border-radius: var(--radius); padding: 0 8px; background: var(--background); color: var(--primary); font-size: 11px; white-space: nowrap; cursor: pointer; }
.sftp-soft-error-action:hover:not(:disabled) { background: var(--accent); }
.sftp-batch-bar .batch-progress { flex: 0 0 auto; overflow: visible; font-variant-numeric: tabular-nums; }
.batch-progress-bar { flex: 0 1 140px; height: 6px; min-width: 80px; accent-color: var(--primary); }
/* 弹层根规则移到 style.css 全局：portal 后内容根不带本组件 scopeId。 */
.popover-section-title { margin: 4px; color: var(--muted-foreground); font-size: 10px; letter-spacing: .04em; text-transform: uppercase; }
.bookmark-list-popover .path-item { display: block; width: 100%; height: 26px; overflow: hidden; border: 0; border-radius: 4px; padding: 0 7px; background: transparent; color: var(--foreground); font-size: 11px; text-align: left; text-overflow: ellipsis; white-space: nowrap; cursor: pointer; }
.bookmark-list-popover .path-item:hover { background: var(--accent); }
/* 书签行：label 跳转 + 行尾悬浮删除（对齐 path-item 观感） */
.bookmark-row { display: flex; align-items: center; gap: 2px; }
.bookmark-row .path-item { flex: 1 1 auto; min-width: 0; }
.bookmark-row .bookmark-delete { display: flex; width: 22px; height: 22px; flex: 0 0 22px; align-items: center; justify-content: center; border: 0; border-radius: 4px; background: transparent; color: var(--muted-foreground); cursor: pointer; opacity: 0; }
.bookmark-row:hover .bookmark-delete, .bookmark-row .bookmark-delete:focus-visible { opacity: 1; }
.bookmark-row .bookmark-delete:hover { color: var(--destructive); background: var(--accent); }
.bookmark-row .bookmark-delete svg { width: 12px; height: 12px; }
/* 星标收藏弹层：路径预览 + 可编辑 label + 保存/取消（根规则见 style.css 全局） */
.bookmark-save-path { overflow: hidden; color: var(--muted-foreground); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.bookmark-label-input { width: 100%; border: 1px solid var(--border); border-radius: 4px; padding: 4px 7px; background: var(--background); color: var(--foreground); font-size: 11px; }
.bookmark-save-actions { display: flex; justify-content: flex-end; gap: 4px; }
</style>
