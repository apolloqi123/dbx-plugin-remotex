<script setup lang="ts">
// 终端/编辑区/右侧栏工作区（T21b 自 App.vue 模板迁出，模板逐行保持原样；
// 绑定经 `bindings` 传入，模板 ref 仍由 state 工厂持有，直接 :ref 回填）。
import BatchBar from "./BatchBar.vue";
import EditorPane from "../../features/sftp/components/EditorPane.vue";
import TerminalContextMenu from "../../features/terminal/components/TerminalContextMenu.vue";
import TerminalLayoutTree from "../../features/terminal/components/TerminalLayoutTree.vue";
import WorkbenchSidePanel from "./WorkbenchSidePanel.vue";
import { ArrowLeftRight, ArrowUpDown, Eraser, FileUp, History, Search, ShieldCheck, TextSelect, X } from "@lucide/vue";
import { ContextMenu, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "../../shared/ui/context-menu/index";
import WorkbenchTerminalStage from "./WorkbenchTerminalStage.vue";
import type { WorkbenchApi } from "../workbench/useWorkbench";
import { onBeforeUnmount, type ComponentPublicInstance } from "vue";
import { trackExposedRef } from "../workbench/lib/trackExposedRef";

const props = defineProps<{ bindings: WorkbenchApi }>();

const {
  activateEditorTab, activatePaneTab, activeEditorId, appearance, archiveEntry,
  batchArchive, batchBarOpen, batchDeleteOpen, batchDownload, batchDraft,
  batchError, batchLoading, batchQuickOpen, batchQuickPickName, batchQuickSearch,
  batchSaveMode, batchSaveName, batchSaving, batchSelected, batchSending,
  batchSessionLabel, batchSummary, batchTargets, batchTargetsOpen, beginChmod,
  beginCustomEditorCommand, beginPathBarEdit, beginRename, beginSymlinkEdit, bookmarkLabelDraft,
  bookmarkSaveOpen, broadcastBatchBarState, browseBatchHistoryDown, browseBatchHistoryUp, canOpenShellTab,
  canSplitTerminal, canWrite, cancelBatchBarSave, cancelPathBarEdit, chooseTrzszUpload,
  chooseZmodem, clearRowSelection, clearSftpSearch, clearTerminal, closeEditorTab,
  closePane, closePaneTab, closeRightPanel, closeTerminalSplit, commitTreeRename,
  confirmBatchBarSave, confirmBookmarkSave, connected, connectionId, copySelectedEntries,
  copySelectedPaths, copyTerminalSelection, copyTextToClipboard, createPaneTab, ctxSearchEngines,
  currentPath, deleteTarget, dismissBatchResult, dividerResizeHint, downloadEntry,
  editorPaneRef, editorSplitRatio, editorTabs, ensureExternalEditors, extractEntry,
  fileMenu, filteredBatchQuickCommands, focusSftpPaneOnPointerDown, folderMenuAction, goParent,
  goToPath, groupedBatchQuickCommands, hasEditorTabs, importPanelOpen, isLocalMode,
  isRdpConnection, killProcessRow, loadDirectory, loadHome, localRecentCommands,
  localUiMode, locale, nudgeSplitDivider, onDividerKeydown, onDrop,
  onEditorDividerKeydown, onFileAreaContextMenu, onFileRowKeydown, onPaneActivated, onSftpClipboardPaste,
  onSftpDragEnter, onSftpDragLeave, onSftpTreeDrop, onShellTabCwd, onShellTabOpened,
  onTerminalDragEnter, onTerminalDragLeave, onTerminalDrop, openAttributes, openBatchBarSave,
  openEntry, openSftpTreeNode, openTerminalSearch, openTerminalSplit, openWithCustomEntry,
  openWithKnownEntry, openWithSystemEntry, orderedPaneClass, otpPanelOpen, paneContainer,
  paneLayout, paneSplitStyle, pasteClipboard, pasteTerminal, pathBarEditing,
  pathHistoryOpen, pickBatchQuick, pickBatchTargets, quickCollapsedGroups, quickCommandGroupSummary,
  quickCommands, quickGroupName, quickHasRealGroups, refreshBatchTargets, refreshMetrics,
  refreshProcessList, refreshSftpFolder, removeBookmark, renamingPath, rerunLocalCommand,
  rightPanelView, saveEditorTab, searchSelectionOnline, searchTextOnline, selectAllTerminal,
  selectTreeEntry, sendBatchCommand, sendSudoRefresh, session, setDirectoryTracking,
  setMetricsProcessSort, sftpDragDropTarget, sftpPaneOpen, sftpSearchDraft, sftpTypeFilter,
  showFileMenu, showNotice, sidePanelDocker, sidePanelMetrics, sidePanelSftp,
  sidePanelShell, splitTerminalFont, splitTerminalPane, splitTerminalTheme, startDividerDrag,
  startEditorDividerDrag, submitPathInput, t, terminal, terminalBasis,
  terminalHasSplit, terminalLayout, terminalMenuOpen, terminalStackRef, terminalTransferBusy,
  toggleBatchQuickPopover, toggleBatchTargetId, toggleBatchTargetsPopover, toggleBookmarkSave, togglePathHistoryMenu,
  toggleProcessPanel, toggleQuickGroupCollapse, toggleSftpShowHidden, toggleSftpTreeNode, toggleSudoMode,
  updateEditorText, workbenchId,
} = props.bindings;

/** 侧栏根元素经 defineExpose 回传；模板 ref 跨组件不会自动绑定到父级 ref，
 *  且子组件根节点的 `v-if` 切换不会重新触发函数 ref，必须持续跟踪暴露值。 */
let boundSidePanel: InstanceType<typeof WorkbenchSidePanel> | null = null;
let stopSidePanelSync: (() => void) | undefined;

function bindSidePanel(el: Element | ComponentPublicInstance | null) {
  const panel = el as InstanceType<typeof WorkbenchSidePanel> | null;
  // 函数 ref 在父组件每次 patch 时都会重入；实例未变时保持现有 watcher。
  if (panel === boundSidePanel) return;
  boundSidePanel = panel;
  stopSidePanelSync?.();
  stopSidePanelSync = undefined;
  if (!panel) {
    props.bindings.sftpPane.value = undefined;
    return;
  }
  stopSidePanelSync = trackExposedRef(() => panel.sftpPane, props.bindings.sftpPane);
}

onBeforeUnmount(() => {
  stopSidePanelSync?.();
});
</script>

<template>
    <section ref="paneContainer" :class="orderedPaneClass" :style="paneSplitStyle">
      <!-- 终端栈：编辑器页签在上、终端在下。SFTP 分栏比（terminalBasis）施加
           在栈容器上，内层终端 pane 只负责填满栈内剩余高度；无页签时编辑器
           与把手不渲染，终端回到整栈高度。 -->
      <section ref="terminalStackRef" class="terminal-stack" :style="terminalBasis">
      <EditorPane
        v-if="hasEditorTabs"
        ref="editorPaneRef"
        :style="{ flexBasis: `${editorSplitRatio}%` }"
        :tabs="editorTabs"
        :active-id="activeEditorId"
        :t="t"
        :locale="locale"
        :appearance="appearance"
        @activate="activateEditorTab"
        @close="closeEditorTab"
        @save="saveEditorTab"
        @update:text="({ id, text }) => updateEditorText(id, text)"
      />
      <div
        v-if="hasEditorTabs"
        class="editor-divider"
        role="separator"
        aria-orientation="horizontal"
        :aria-valuenow="Math.round(editorSplitRatio)"
        aria-valuemin="25"
        aria-valuemax="75"
        tabindex="0"
        @pointerdown="startEditorDividerDrag"
        @keydown="onEditorDividerKeydown"
      />
      <div
        class="terminal-workspace"
        @dragenter.prevent="onTerminalDragEnter"
        @dragover.prevent
        @dragleave="onTerminalDragLeave"
        @drop.prevent="onTerminalDrop($event)"
      >
      <TerminalLayoutTree
        :node="terminalLayout"
        :connection-id="connectionId || ''"
        :workbench-id="workbenchId"
        :main-session-id="session?.sessionId || ''"
        :theme="splitTerminalTheme"
        :font-family="splitTerminalFont.fontFamily"
        :font-size="splitTerminalFont.fontSize"
        :t="t"
        :locale="locale"
        :can-split="canSplitTerminal"
        :can-create-tab="canOpenShellTab"
        :engines="ctxSearchEngines"
        :connected="connected"
        :can-write="canWrite"
        :transfer-busy="terminalTransferBusy"
        :notify="showNotice"
        @opened="onShellTabOpened"
        @activate-tab="activatePaneTab"
        @close-tab="closePaneTab"
        @new-tab="createPaneTab"
        @close-pane="closePane"
        @split="splitTerminalPane"
        @search-online="searchTextOnline"
        @cwd="onShellTabCwd"
        @activate-pane="onPaneActivated"
      >
        <template #main>
      <ContextMenu :open="terminalMenuOpen" @update:open="(open) => { if (!open) terminalMenuOpen = false; }">
        <ContextMenuTrigger as-child>
    <WorkbenchTerminalStage :bindings="bindings" />
        </ContextMenuTrigger>
        <!-- P2-8：Copy/Paste/Search online/Close 由 TerminalContextMenu 承载；
             插件自有菜单项经默认插槽保持在原有位置。 -->
        <TerminalContextMenu
          :open="terminalMenuOpen"
          :has-selection="terminal?.hasSelection() ?? false"
          :can-paste="connected && !terminalTransferBusy"
          :engines="ctxSearchEngines"
          :t="t"
          @copy="copyTerminalSelection"
          @paste="pasteTerminal"
          @search="searchSelectionOnline"
          @close="terminalMenuOpen = false"
        >
          <!-- 本地终端最近命令（VS Code Run Recent Command 简化版）：
               依赖 shell integration 注入的 633;E 命令行。 -->
          <template v-if="isLocalMode && localRecentCommands.length">
            <ContextMenuItem @select="rerunLocalCommand(localRecentCommands[0])"><History />{{ t("localTerminal.rerunLast") }}</ContextMenuItem>
            <ContextMenuItem v-for="command in localRecentCommands.slice(0, 5)" :key="command" @select="rerunLocalCommand(command)"><span class="mono local-rerun-command">{{ command }}</span></ContextMenuItem>
            <ContextMenuSeparator />
          </template>
          <ContextMenuItem @select="selectAllTerminal"><TextSelect />{{ t("terminalSelectAll") }}</ContextMenuItem>
          <ContextMenuItem @select="openTerminalSearch"><Search />{{ t("terminalSearch.open") }}</ContextMenuItem>
          <ContextMenuItem @select="clearTerminal"><Eraser />{{ t("terminalClear") }}</ContextMenuItem>
          <ContextMenuItem :disabled="!connected" @select="sendSudoRefresh"><ShieldCheck />{{ t("sudoRefresh.title") }}</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem :disabled="!canSplitTerminal" @select="openTerminalSplit('row')"><ArrowLeftRight />{{ t("terminalSplit.leftRight") }}</ContextMenuItem>
          <ContextMenuItem :disabled="!canSplitTerminal" @select="openTerminalSplit('column')"><ArrowUpDown />{{ t("terminalSplit.topBottom") }}</ContextMenuItem>
          <ContextMenuItem v-if="terminalHasSplit" @select="closeTerminalSplit"><X />{{ t("terminalSplit.close") }}</ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem :disabled="!connected || terminalTransferBusy || !canWrite" @select="chooseZmodem"><FileUp />{{ t("zmodemUpload") }}</ContextMenuItem>
          <ContextMenuItem :disabled="!connected || terminalTransferBusy || !canWrite" @select="chooseTrzszUpload"><FileUp />{{ t("trzszUpload") }}</ContextMenuItem>
        </TerminalContextMenu>
      </ContextMenu>
        </template>
      </TerminalLayoutTree>
        <!-- 批量发送结果浮条：显示在命令条上方，可手动关闭。 -->
        <div v-if="connected && batchBarOpen && !isRdpConnection && (batchSummary || batchError)" class="batch-bar-status" role="status">
          <template v-if="batchSummary">
            <p :class="batchSummary.failed ? 'task-error' : 'muted'">
              {{ batchSummary.failed ? t("batchSendPartial", { sent: batchSummary.sent, failed: batchSummary.failed }) : t("batchSendSent", { count: batchSummary.sent }) }}
            </p>
            <div v-if="batchSummary.failed" class="batch-result-list">
              <span v-for="row in batchSummary.rows.filter((item) => !item.success)" :key="row.sessionId" class="batch-result-row mono">
                {{ batchSessionLabel(row.sessionId) }} · {{ row.error || t("batchSendFailed") }}
              </span>
            </div>
          </template>
          <p v-else class="task-error">{{ batchError }}</p>
          <button class="icon-button" :title="t('close')" :aria-label="t('close')" @click="dismissBatchResult"><X /></button>
        </div>
        <!-- 批量发送命令条（Electerm quick-command bar）：贴终端底部，回车即发送；
             目标选择/快速命令切换/保存为快速命令均在条上完成。 -->
        <BatchBar
          v-if="connected && batchBarOpen && !isRdpConnection"
          v-model:batchTargetsOpen="batchTargetsOpen"
          v-model:batchQuickOpen="batchQuickOpen"
          v-model:batchQuickSearch="batchQuickSearch"
          v-model:batchSaveName="batchSaveName"
          v-model:batchDraft="batchDraft"
          :batch-loading="batchLoading"
          :batch-sending="batchSending"
          :batch-targets="batchTargets"
          :batch-selected="batchSelected"
          :filtered-batch-quick-commands="filteredBatchQuickCommands"
          :grouped-batch-quick-commands="groupedBatchQuickCommands"
          :batch-quick-pick-name="batchQuickPickName"
          :batch-save-mode="batchSaveMode"
          :batch-saving="batchSaving"
          :quick-commands="quickCommands"
          :quick-has-real-groups="quickHasRealGroups"
          :quick-collapsed-groups="quickCollapsedGroups"
          :quick-command-group-summary="quickCommandGroupSummary"
          :quick-group-name="quickGroupName"
          :session-id="session?.sessionId || ''"
          :t="t"
          @toggle-targets="toggleBatchTargetsPopover"
          @pick-targets="pickBatchTargets"
          @refresh-targets="refreshBatchTargets"
          @toggle-target="toggleBatchTargetId"
          @toggle-quick="toggleBatchQuickPopover"
          @pick-quick="pickBatchQuick"
          @toggle-group="toggleQuickGroupCollapse"
          @broadcast="broadcastBatchBarState"
          @browse-history="(direction) => direction === 'up' ? browseBatchHistoryUp() : browseBatchHistoryDown()"
          @send="sendBatchCommand"
          @open-save="openBatchBarSave"
          @confirm-save="confirmBatchBarSave"
          @cancel-save="cancelBatchBarSave"
        />
      </div>
      </section>

      <div
        v-if="sftpPaneOpen && !isRdpConnection && (rightPanelView === 'docker' || !localUiMode)"
        class="divider"
        role="separator"
        :aria-orientation="paneLayout.direction === 'column' ? 'horizontal' : 'vertical'"
        :aria-valuenow="Math.round(paneLayout.ratio)"
        aria-valuemin="35"
        aria-valuemax="80"
        :title="dividerResizeHint"
        tabindex="0"
        @pointerdown="startDividerDrag"
        @keydown="onDividerKeydown($event, nudgeSplitDivider)"
      />

      <!-- 右侧栏三视图外壳（T21 迁入 components/WorkbenchSidePanel.vue）：
           面板根焦点/粘贴/拖放、SFTP 工作区、指标/容器视图整体下传；分隔条与
           分栏比例仍属外层 pane 布局，留在本模板。 -->
      <WorkbenchSidePanel
        :ref="bindSidePanel"
        v-model:currentPath="currentPath"
        v-model:pathBarEditing="pathBarEditing"
        v-model:bookmarkSaveOpen="bookmarkSaveOpen"
        v-model:bookmarkLabelDraft="bookmarkLabelDraft"
        v-model:pathHistoryOpen="pathHistoryOpen"
        v-model:sftpSearchDraft="sftpSearchDraft"
        v-model:sftpTypeFilter="sftpTypeFilter"
        v-model:fileMenu="fileMenu"
        v-model:batchDeleteOpen="batchDeleteOpen"
        v-model:deleteTarget="deleteTarget"
        v-model:renamingPath="renamingPath"
        v-model:sftpDragDropTarget="sftpDragDropTarget"
        v-model:otpPanelOpen="otpPanelOpen"
        v-model:importPanelOpen="importPanelOpen"
        :t="t"
        :locale="locale"
        :shell="sidePanelShell"
        :sftp="sidePanelSftp"
        :metrics="sidePanelMetrics"
        :docker="sidePanelDocker"
        @focus-pane="focusSftpPaneOnPointerDown"
        @sftp-paste="onSftpClipboardPaste"
        @sftp-drag-enter="onSftpDragEnter"
        @sftp-drag-leave="onSftpDragLeave"
        @sftp-drop="onDrop"
        @refresh-metrics="refreshMetrics"
        @toggle-processes="toggleProcessPanel"
        @refresh-processes="refreshProcessList"
        @kill-process="(row, signal) => killProcessRow(row, signal)"
        @update:sort="setMetricsProcessSort"
        @close-panel="closeRightPanel"
        @go-parent="goParent"
        @load-home="loadHome"
        @load-directory="loadDirectory()"
        @set-directory-tracking="setDirectoryTracking"
        @submit-path-input="submitPathInput"
        @cancel-path-bar-edit="cancelPathBarEdit"
        @begin-path-bar-edit="beginPathBarEdit"
        @go-to-path="goToPath"
        @toggle-bookmark-save="toggleBookmarkSave"
        @confirm-bookmark-save="confirmBookmarkSave"
        @toggle-path-history-menu="togglePathHistoryMenu"
        @remove-bookmark="removeBookmark"
        @paste-clipboard="pasteClipboard"
        @toggle-sudo-mode="toggleSudoMode"
        @clear-sftp-search="clearSftpSearch"
        @toggle-sftp-show-hidden="toggleSftpShowHidden"
        @batch-archive="batchArchive"
        @clear-row-selection="clearRowSelection"
        @batch-download="batchDownload"
        @copy-selected-paths="copySelectedPaths"
        @file-area-context-menu="onFileAreaContextMenu"
        @toggle-sftp-tree-node="toggleSftpTreeNode"
        @select-tree-entry="selectTreeEntry"
        @open-sftp-tree-node="openSftpTreeNode"
        @sftp-tree-drop="onSftpTreeDrop"
        @show-file-menu="showFileMenu"
        @commit-tree-rename="commitTreeRename"
        @file-row-keydown="onFileRowKeydown"
        @folder-menu-action="folderMenuAction"
        @open-entry="openEntry"
        @download-entry="downloadEntry"
        @refresh-sftp-folder="refreshSftpFolder"
        @ensure-external-editors="ensureExternalEditors"
        @open-with-system-entry="openWithSystemEntry"
        @open-with-known-entry="openWithKnownEntry"
        @open-with-custom-entry="openWithCustomEntry"
        @begin-custom-editor-command="beginCustomEditorCommand"
        @begin-symlink-edit="beginSymlinkEdit"
        @begin-rename="beginRename"
        @copy-selected-entries="copySelectedEntries"
        @copy-text-to-clipboard="copyTextToClipboard"
        @begin-chmod="beginChmod"
        @open-attributes="openAttributes"
        @archive-entry="archiveEntry"
        @extract-entry="extractEntry"
      />
    </section>
</template>
