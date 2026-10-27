<script setup lang="ts">
// 工作台外壳（T32）：/main 壳模板与模板专用绑定收口于此；状态与接线仍在
// workbench/useWorkbench，App.vue 只保留根组件与生命周期挂载。
import WorkbenchWorkspace from "./WorkbenchWorkspace.vue";
import FolderPickerDialog from "../../features/sftp/components/FolderPickerDialog.vue";
import PortForwardDialog from "../../features/forwarding/components/PortForwardDialog.vue";
import RdpConnectDialog from "../../features/remote-desktop/components/RdpConnectDialog.vue";
import SerialConnectDialog from "../../features/serial/components/SerialConnectDialog.vue";
import SerialUploadDialog from "../../features/serial/components/SerialUploadDialog.vue";
import SettingsDialog from "../../features/settings/components/SettingsDialog.vue";
import TelnetConnectDialog from "../../features/telnet/components/TelnetConnectDialog.vue";
import VncConnectDialog from "../../features/remote-desktop/components/VncConnectDialog.vue";
import WorkbenchDialogs from "./WorkbenchDialogs.vue";
import WorkbenchSessionDialogs from "./WorkbenchSessionDialogs.vue";
import WorkbenchSftpDialogs from "./WorkbenchSftpDialogs.vue";
import WorkbenchToolbar from "./WorkbenchToolbar.vue";
import { ChevronDown, X } from "@lucide/vue";
import { PERM_COLUMNS, PERM_ROLES, permBit } from "../../features/sftp/composables/actions/useSftpChmod";
import { ToastAction, ToastClose, ToastProvider, ToastRoot, ToastViewport } from "../../shared/ui/toast/index";
import { batchProgressPercent } from "../../features/transfers/lib/sftpBatchProgress";
import { batchUploadPercent } from "../../features/transfers/lib/transferQueue";
import { folderUploadPercent } from "../../features/sftp/lib/transfer/folderUpload";
import { formatBytes, formatRate } from "../../shared/lib/format";
import { transferPausable } from "../../features/transfers/lib/transferResume";
import { workbenchBindings, type Workbench } from "../workbench/useWorkbench";

const props = defineProps<{ wb: Workbench }>();

// 绑定从聚合 API 的扁平视图按名取出：模板标识符与迁出前逐一对应。
const bindings = workbenchBindings(props.wb);

const {
  tooltipBubble, settingsDialog, uploadInput, folderUploadInput, zmodemInput, trzszInput,
  RDP_RESOLUTION_PRESETS, actionLinksSettings, activeAppearanceThemeId, activeTransfers, addImportedSchemes,
  agentMode, agentModeBusy, agentModeHint, agentModeOpen, agentPromptCommand,
  agentPromptHead, agentPromptRemaining, agentPromptRemember, aiBridgeReady, aiSettingsState,
  alertTriageBusy, alertTriageError, alertTriageOpen, alertTriagePayload, alertTriageResult,
  appearance, applePlatform, applyCommandHistoryLimit, applyTerminalAppearanceTheme, applyTerminalFontSettings,
  attrsInfo, attrsLoading, attrsMode, attrsOwnerDisplay, attrsSubmitting,
  attrsTarget, auditClearOpen, auditClearSubmitting, auditEntries, auditKindFilter,
  auditLoadFailed, auditLoading, auditOpen, auditRowKindClass, auditTime,
  auditTruncated, batchBarOpen, batchDeleteOpen, batchDeleteSubmitting, batchProgress,
  beginPreviewEdit, bindDropUploadPathInputEl, canOpenLocalTab, canWrite,
  cancelPreviewEdit, chmodDraft, chmodSubmitting, chmodTarget,
  clearWallpaperImage, closeAttributes, closePreview,
  commitCustomEditor, commitSymlink, confirmAuditClear,
  confirmBatchDelete, confirmChmod, confirmCommandSessionTab, confirmDelete, confirmDropUpload,
  confirmLocalTerminal, confirmRecordingClearAll, confirmRecordingDelete, confirmTransferHistoryClear, connectViaRdpDialog,
  connected, connection, connectionAuthMethodLabel, connectionId, connectionIdentity,
  connectionInfoOpen, connectionLatency, connectionLatencyBusy, connectionLatencyFailed, connectionReadOnly,
  copySuggestions, createDirectory, createNewFile, ctxSearchEnginesText, customEditorCommand,
  customEditorName, customEditorOpen, customEditorPattern, customEditorRemember, deleteHighlightRule,
  deleteQuickCommand, deleteSubmitting, deleteTarget, deleteTerminalAppearanceTheme, dismissWatchModified,
  dissolveQuickCommandGroupById, dockConnections, dockerPaneOpen, downloadConflictPrompt, downloadPrefsAdapter,
  downloadPrompt, dropCwdTarget, dropUploadPathInput, dropUploadPrompt, dropUploadTarget,
  editorPrefsAdapter, externalEditBusy, fileBatchProgress, filteredQuickCommands, folderPickerTarget,
  folderUploadProgress, folderUploadSupported, formatDuration, formatModified, formatRecordedAt,
  forwardsOpen, ghostTabAccept, groupedQuickCommands, gutterSettings,
  highlightDisplayMode, highlightRules, highlightSaving, hostKeyPrompt, hostTerminalFontFamily,
  hostTerminalTheme, importPanelOpen, importQuickCommands, isLocalMode, isRdpConnection,
  isRdpMode, isSerialMode, isTelnetMode, isVncMode, jsonPreviewState,
  loadAuditEntries, localCanSave, localDownloadDir, localOpenConfirmOpen, localShellCwdPref,
  localShellEnvPref, localShellIntegrationPref, localShellPref, localShellRestored, localShellSurfaceOpen,
  localShells, localShellsLoading, localState, localUiMode, locale,
  metricsDistroBadge, newFileDialog, newFileDraft, newFileSubmitting, notice,
  noticeActions, noticeKey, noticeOpen, onFolderPicked, onFolderUploadInput,
  onNoticeOpenChange, onSftpErrorOpenChange, onTrzszPickCancel, onTrzszPickInput, onUploadInput,
  onZmodemInput, operationDialog, operationDraft, otpPanelOpen, paneDirection,
  paneOrder, panelActiveSchemeId, panelCpuPercent, panelMemPercent, panelMetricsErrorTitle,
  panelMetricsFailed, panelMetricsVisible, panelNetRates, panelPaletteOpen, panelSchemeChoices,
  panelSurface, panelToolbarHidden, pasteConfirm, pausedTaskIds, pendingConfirmDialog,
  previewDirty, previewDraft, previewEditable, previewEditableAllowed, previewImageUrl,
  previewImageZoomed, previewLoading, previewMode, previewOpen, previewSaving,
  previewSize, previewText, previewTitle, previewTruncated, profilesOpen,
  promptHintsEnabled, quickCollapsedGroups, quickCommandGroupSummary, quickCommands, quickExpandedId,
  quickHasRealGroups, quickImportBusy, quickMenuOpen, quickSaving, quickSearch,
  rdpCertPrompt, rdpCertRemaining, rdpCertRemember, rdpCustomHeight, rdpCustomWidth,
  rdpDesktopSize, rdpDialogOpen, rdpPasteOpen, rdpPasteText, rdpResizing,
  rdpResolutionOpen, rdpScaleMode, rdpTypingBusy, rdpTypingSent, rdpTypingTotal,
  reconnectCountdown, reconnectPending, recordingActive, recordingClearAllOpen, recordingClearAllSubmitting,
  recordingDeleteSubmitting, recordingDeleteTarget, recordingTarget, recordings, recordingsOpen,
  rememberHostKey, removeImportedScheme, renameQuickCommandGroupById, resetAiFixConsent,
  resolveAgentPrompt, resolveDownloadConflict, resolveDownloadPrompt, resolveDropUpload, resolveHostKey,
  resolvePasteConfirm, resolvePendingConfirmDialog, resolveRdpCertificate, resolveUploadDuplicate, resumableLoading,
  resumableTasks, rightPanelView, runAlertTriage, saveAttributesPermissions,
  saveHighlightRule, savePreview, saveQuickCommand, saveTerminalAppearanceTheme, selectedEntries,
  selectedShellInjectable, sendSuggestionToTerminal, serialDialogOpen, serialUploadBusy, serialUploadDialogOpen,
  session, sessionMenuOpen, sessionMenuShellOpen, sessionMenuShellPickOpen, sessionPillText,
  sessionStatus, setGhostEnabled, setGhostTabAccept, setHighlightDisplayMode, setHistoryPanelEnabled,
  setLocalShellPref, setPaneDirection, setPromptHintsEnabled, setWallpaperImage, setWebglEnabled,
  settingsOpen, sftpError, sftpErrorKey, sftpErrorOpen,
  sftpErrorRetry, sftpPaneOpen, sftpViewActive, showError,
  showNotice, spawnSessionCommand, spawnSessionDialogOpen, startSerialSession, startSerialUpload,
  startTelnetSession, startVncSession, suggestionPrefsAdapter, symlinkDialog, symlinkDraft,
  symlinkSubmitting, symlinkTargetDraft, t, telnetDialogOpen, terminal,
  terminalAppearance, terminalAppearanceState, terminalBehavior, terminalFontSize, terminalHotkeys,
  terminalState, toggleAttrsPerm, toggleChmodPerm, toggleHighlightRule, togglePanelToolbar,
  toolbarActions, toolbarStyle, tooltip,
  transferHistory, transferHistoryClearOpen, transferHistoryFailed, transferHistoryLoading, transferHistoryMenu,
  transferList, transferPanelOpen, transferPercent, transferPrefsAdapter, transferShownBytes,
  transferSpeeds, updateActionLinksSettings, updateAiSettings, updateCtxSearchEngines, updateGutterSettings,
  updateTerminalAppearance, updateTerminalBehavior, updateTerminalHotkeys, updateWallpaperEnabled, updateWallpaperOpacity,
  uploadDuplicateApplyAll, uploadDuplicatePrompt, uploadWatchedFileAlways, uploadWatchedFileOnce, visibleAuditEntries,
  vncDialogOpen, wallpaperEnabled, wallpaperOpacity, wallpaperSessionOnly, watchModifiedPrompt,
  watchModifiedQueue, webglEnabled,
} = bindings;
</script>

<template>
  <main class="workbench" :class="{ 'panel-surface': panelSurface }">
    <!-- dock 工具条隐藏后：右上角低透明度微钮还原（偏好落 pluginStore）。 -->
    <button v-if="panelSurface && panelToolbarHidden" type="button" class="panel-toolbar-reveal" :title="t('panel.showToolbar')" :aria-label="t('panel.showToolbar')" @click="togglePanelToolbar"><ChevronDown /></button>
    <!-- 工作台工具条：外壳/身份区/主操作位/panel 变体 + 会话、快速命令、
         RDP 工具组、配色快切子组件（T19 自本模板迁出）。状态与动作仍由
         本文件持有，经显式 props/v-model/actions 下发。 -->
    <WorkbenchToolbar
      v-model:connection-info-open="connectionInfoOpen"
      v-model:agent-mode-open="agentModeOpen"
      v-model:importPanelOpen="importPanelOpen"
      v-model:transfer-panel-open="transferPanelOpen"
      v-model:transfer-history-clear-open="transferHistoryClearOpen"
      v-model:serial-upload-dialog-open="serialUploadDialogOpen"
      v-model:forwards-open="forwardsOpen"
      v-model:session-menu-open="sessionMenuOpen"
      v-model:session-menu-shell-open="sessionMenuShellOpen"
      v-model:session-menu-shell-pick-open="sessionMenuShellPickOpen"
      v-model:local-shell-surface-open="localShellSurfaceOpen"
      v-model:quick-menu-open="quickMenuOpen"
      v-model:quick-search="quickSearch"
      v-model:panel-palette-open="panelPaletteOpen"
      v-model:rdp-scale-mode="rdpScaleMode"
      v-model:rdp-resolution-open="rdpResolutionOpen"
      v-model:rdp-custom-width="rdpCustomWidth"
      v-model:rdp-custom-height="rdpCustomHeight"
      v-model:rdp-paste-open="rdpPasteOpen"
      v-model:rdp-paste-text="rdpPasteText"
      :t="t"
      :panel-surface="panelSurface"
      :panel-toolbar-hidden="panelToolbarHidden"
      :toolbar-style="toolbarStyle"
      :connection="connection"
      :connection-identity="connectionIdentity"
      :connection-read-only="connectionReadOnly"
      :session-status="sessionStatus"
      :session-pill-text="sessionPillText"
      :reconnect-countdown="reconnectCountdown"
      :metrics-distro-badge="metricsDistroBadge"
      :connection-auth-method-label="connectionAuthMethodLabel"
      :connection-latency="connectionLatency"
      :connection-latency-busy="connectionLatencyBusy"
      :connection-latency-failed="connectionLatencyFailed"
      :connected="connected"
      :is-rdp-connection="isRdpConnection"
      :is-rdp-mode="isRdpMode"
      :is-serial-mode="isSerialMode"
      :is-vnc-mode="isVncMode"
      :is-telnet-mode="isTelnetMode"
      :local-ui-mode="localUiMode"
      :is-local-mode="isLocalMode"
      :local-shell-restored="localShellRestored"
      :pane-order="paneOrder"
      :sftp-pane-open="sftpPaneOpen"
      :sftp-view-active="sftpViewActive"
      :right-panel-view="rightPanelView"
      :terminal-state="terminalState"
      :reconnect-pending="reconnectPending"
      :serial-upload-busy="serialUploadBusy"
      :connection-id="connectionId"
      :docker-pane-open="dockerPaneOpen"
      :batch-send-bar-open="batchBarOpen"
      :agent-mode="agentMode"
      :agent-mode-busy="agentModeBusy"
      :agent-mode-hint="agentModeHint"
      :recording-active="recordingActive"
      :recording-target="recordingTarget"
      :recordings-open="recordingsOpen"
      :active-transfers="activeTransfers"
      :panel-metrics-visible="panelMetricsVisible"
      :panel-metrics-failed="panelMetricsFailed"
      :panel-cpu-percent="panelCpuPercent"
      :panel-mem-percent="panelMemPercent"
      :panel-net-rates="panelNetRates"
      :panel-metrics-error-title="panelMetricsErrorTitle"
      :format-rate="formatRate"
      :panel-scheme-choices="panelSchemeChoices"
      :panel-active-scheme-id="panelActiveSchemeId"
      :local-shells="localShells"
      :local-shells-loading="localShellsLoading"
      :local-shell-pref="localShellPref"
      :local-shell-integration-pref="localShellIntegrationPref"
      :selected-shell-injectable="selectedShellInjectable"
      :local-shell-cwd-pref="localShellCwdPref"
      :local-shell-env-pref="localShellEnvPref"
      :local-state="localState"
      :can-open-local-tab="canOpenLocalTab"
      :dock-connections="dockConnections"
      :quick-commands="quickCommands"
      :filtered-quick-commands="filteredQuickCommands"
      :grouped-quick-commands="groupedQuickCommands"
      :quick-has-real-groups="quickHasRealGroups"
      :quick-collapsed-groups="quickCollapsedGroups"
      :quick-expanded-id="quickExpandedId"
      :quick-command-group-summary="quickCommandGroupSummary"
      :resolution-presets="RDP_RESOLUTION_PRESETS"
      :rdp-resizing="rdpResizing"
      :rdp-desktop-size="rdpDesktopSize"
      :rdp-typing-busy="rdpTypingBusy"
      :rdp-typing-sent="rdpTypingSent"
      :rdp-typing-total="rdpTypingTotal"
      :transfer-list="transferList"
      :transfer-history="transferHistory"
      :resumable-tasks="resumableTasks"
      :folder-upload-progress="folderUploadProgress"
      :file-batch-progress="fileBatchProgress"
      :paused-task-ids="pausedTaskIds"
      :transfer-speeds="transferSpeeds"
      :transfer-history-menu="transferHistoryMenu"
      :transfer-history-loading="transferHistoryLoading"
      :transfer-history-failed="transferHistoryFailed"
      :resumable-loading="resumableLoading"
      :folder-upload-percent="folderUploadPercent"
      :batch-upload-percent="batchUploadPercent"
      :format-bytes="formatBytes"
      :transfer-percent="transferPercent"
      :transfer-shown-bytes="transferShownBytes"
      :transfer-pausable="transferPausable"
      :actions="toolbarActions"
      @close-history-menu="transferHistoryMenu = undefined"
    />

    <div v-if="tooltip" ref="tooltipBubble" class="app-tooltip" :class="{ 'app-tooltip-above': tooltip.above }" :style="{ left: `${tooltip.x}px`, top: `${tooltip.y}px`, '--arrow-offset': `${tooltip.arrowOffset}px` }" role="tooltip">{{ tooltip.text }}</div>
    <ToastProvider :label="t('notification')">
      <ToastRoot v-if="notice" :key="noticeKey" :open="noticeOpen" :duration="noticeActions.length ? 8000 : 3500" class="notice" @update:open="onNoticeOpenChange">
        <span>{{ notice }}</span>
        <ToastAction v-for="action in noticeActions" :key="action.label" :alt-text="action.label" class="notice-action" @click="action.run()">{{ action.label }}</ToastAction>
      </ToastRoot>
      <ToastViewport class="notice-viewport" />
    </ToastProvider>
    <ToastProvider :label="t('notification')">
      <ToastRoot v-if="sftpError" :key="sftpErrorKey" :open="sftpErrorOpen" :duration="8000" class="error-banner" @update:open="onSftpErrorOpenChange">
        <span>{{ sftpError }}</span>
        <ToastAction v-if="sftpErrorRetry" :alt-text="t('retry')" class="notice-action" @click="sftpErrorRetry()">{{ t("retry") }}</ToastAction>
        <ToastClose :title="t('close')"><X /></ToastClose>
      </ToastRoot>
      <ToastViewport class="error-viewport" />
    </ToastProvider>

    <WorkbenchWorkspace :bindings="bindings" />

    <!-- 内联弹窗宿主（T21 迁入 components/WorkbenchDialogs.vue）：OTP/会话导入、
         文件预览（含编辑态与 JSON 面板）、录制删除/一键清空、传输历史清空、审计
         日志查看/清空、终端拖入上传落点询问。弹窗壳仍走 reka Dialog（portal/遮罩/
         焦点陷阱/外点关闭由 reka 承担，Esc 由 onDocumentKeydown 分层链独占）。 -->
    <WorkbenchDialogs
      v-model:otpPanelOpen="otpPanelOpen"
      v-model:importPanelOpen="importPanelOpen"
      v-model:previewOpen="previewOpen"
      v-model:previewImageZoomed="previewImageZoomed"
      v-model:previewDraft="previewDraft"
      v-model:recordingDeleteTarget="recordingDeleteTarget"
      v-model:recordingClearAllOpen="recordingClearAllOpen"
      v-model:transferHistoryClearOpen="transferHistoryClearOpen"
      v-model:auditOpen="auditOpen"
      v-model:auditClearOpen="auditClearOpen"
      v-model:auditKindFilter="auditKindFilter"
      v-model:dropUploadTarget="dropUploadTarget"
      v-model:dropUploadPathInput="dropUploadPathInput"
      :t="t"
      :locale="locale"
      :appearance="appearance"
      :preview-title="previewTitle"
      :preview-text="previewText"
      :preview-loading="previewLoading"
      :preview-size="previewSize"
      :preview-editable="previewEditable"
      :preview-editable-allowed="previewEditableAllowed"
      :preview-saving="previewSaving"
      :preview-mode="previewMode"
      :preview-image-url="previewImageUrl"
      :preview-truncated="previewTruncated"
      :preview-dirty="previewDirty"
      :json-preview-state="jsonPreviewState"
      :recording-delete-submitting="recordingDeleteSubmitting"
      :recordings-count="recordings.length"
      :recording-clear-all-submitting="recordingClearAllSubmitting"
      :audit-entries="auditEntries"
      :audit-loading="auditLoading"
      :audit-load-failed="auditLoadFailed"
      :audit-truncated="auditTruncated"
      :visible-audit-entries="visibleAuditEntries"
      :audit-clear-submitting="auditClearSubmitting"
      :audit-time="auditTime"
      :audit-row-kind-class="auditRowKindClass"
      :drop-upload-prompt="dropUploadPrompt"
      :drop-cwd-target="dropCwdTarget"
      :set-drop-upload-path-input-el="bindDropUploadPathInputEl"
      :format-recorded-at="formatRecordedAt"
      :format-duration="formatDuration"
      @close-preview="closePreview"
      @begin-preview-edit="beginPreviewEdit"
      @cancel-preview-edit="cancelPreviewEdit"
      @save-preview="savePreview"
      @confirm-recording-delete="confirmRecordingDelete"
      @confirm-recording-clear="confirmRecordingClearAll"
      @confirm-transfer-history-clear="confirmTransferHistoryClear"
      @load-audit-entries="loadAuditEntries"
      @confirm-audit-clear="confirmAuditClear"
      @confirm-drop-upload="confirmDropUpload"
      @cancel-drop-upload="resolveDropUpload('cancel')"
    />

    <!-- 端口映射管理（-L/-R/-D）：与连接右键弹框复用预设、列表和操作。
         以 connectionId 创建独立映射，关闭终端不停止端口转发。 -->
    <PortForwardDialog
      :locale="locale"
      :open="forwardsOpen"
      :connection-id="connectionId"
      :session-id="null"
      independent
      @update:open="forwardsOpen = $event"
      @error="showError($event, 'terminal')"
    />

    <WorkbenchSftpDialogs
      v-model:operationDialog="operationDialog"
      v-model:operationDraft="operationDraft"
      v-model:symlinkDialog="symlinkDialog"
      v-model:symlinkDraft="symlinkDraft"
      v-model:symlinkTargetDraft="symlinkTargetDraft"
      :symlink-submitting="symlinkSubmitting"
      :watch-modified-queue="watchModifiedQueue"
      :watch-modified-prompt="watchModifiedPrompt"
      :external-edit-busy="externalEditBusy"
      v-model:customEditorOpen="customEditorOpen"
      v-model:customEditorName="customEditorName"
      v-model:customEditorCommand="customEditorCommand"
      v-model:customEditorRemember="customEditorRemember"
      :custom-editor-pattern="customEditorPattern"
      v-model:chmodTarget="chmodTarget"
      v-model:chmodDraft="chmodDraft"
      :chmod-submitting="chmodSubmitting"
      v-model:deleteTarget="deleteTarget"
      :delete-submitting="deleteSubmitting"
      v-model:batchDeleteOpen="batchDeleteOpen"
      :selected-entries="selectedEntries"
      :batch-progress="batchProgress"
      :batch-delete-submitting="batchDeleteSubmitting"
      v-model:downloadPrompt="downloadPrompt"
      :local-can-save="localCanSave"
      :download-conflict-prompt="downloadConflictPrompt"
      :upload-duplicate-prompt="uploadDuplicatePrompt"
      v-model:uploadDuplicateApplyAll="uploadDuplicateApplyAll"
      v-model:newFileDialog="newFileDialog"
      v-model:newFileDraft="newFileDraft"
      :new-file-submitting="newFileSubmitting"
      :attrs-target="attrsTarget"
      :attrs-loading="attrsLoading"
      :attrs-info="attrsInfo"
      v-model:attrsMode="attrsMode"
      :attrs-submitting="attrsSubmitting"
      :attrs-owner-display="attrsOwnerDisplay"
      :can-write="canWrite"
      :perm-roles="PERM_ROLES"
      :perm-columns="PERM_COLUMNS"
      :perm-bit="permBit"
      :batch-progress-percent="batchProgressPercent"
      :format-modified="formatModified"
      :t="t"
      @create-directory="createDirectory"
      @commit-symlink="commitSymlink"
      @dismiss-watch-modified="dismissWatchModified"
      @upload-watched-always="uploadWatchedFileAlways"
      @upload-watched-single="uploadWatchedFileOnce"
      @commit-custom-editor="commitCustomEditor"
      @confirm-chmod="confirmChmod"
      @confirm-delete="confirmDelete"
      @confirm-batch-delete="confirmBatchDelete"
      @resolve-download-prompt="resolveDownloadPrompt"
      @resolve-download-conflict="resolveDownloadConflict"
      @resolve-upload-duplicate="resolveUploadDuplicate"
      @open-folder-picker="folderPickerTarget = 'prompt'"
      @create-new-file="createNewFile"
      @close-attributes="closeAttributes"
      @toggle-chmod-perm="toggleChmodPerm"
      @toggle-attrs-perm="toggleAttrsPerm"
      @save-attributes-permissions="saveAttributesPermissions"
    />




    <!-- 设置弹窗 + quick sudo 配置档管理弹窗：独立组件（设置域 UI/状态集中处；Esc 链与终端偏好仍留在 App）。 -->
    <SettingsDialog
      ref="settingsDialog"
      v-model:open="settingsOpen"
      v-model:profilesOpen="profilesOpen"
      :session-id="session?.sessionId"
      :connection-id="session?.connectionId"
      :terminal-font-size="terminalFontSize"
      :host-font-size="appearance.terminal.fontSize"
      :host-font-family="hostTerminalFontFamily(appearance)"
      :local-download-dir="localDownloadDir"
      :local-can-save="localCanSave"
      :webgl-enabled="webglEnabled"
      :local-shell="localShellPref"
      @set-local-shell="setLocalShellPref"
      :terminal-behavior="terminalBehavior"
      :pane-direction="paneDirection"
      @update:pane-direction="setPaneDirection"
      :terminal-hotkeys="terminalHotkeys"
      :apple-platform="applePlatform"
      :action-links="actionLinksSettings"
      :gutter="gutterSettings"
      :ctx-search-engines="ctxSearchEnginesText"
      :wallpaper-enabled="wallpaperEnabled"
      :wallpaper-opacity="wallpaperOpacity"
      :wallpaper-session-only="wallpaperSessionOnly"
      :appearance="terminalAppearanceState"
      :custom-themes="terminalAppearance.customThemes"
      :active-theme-id="activeAppearanceThemeId"
      :host-theme="hostTerminalTheme()"
      :host-color-scheme="appearance.colorScheme"
      :download-prefs="downloadPrefsAdapter"
      :transfer-prefs="transferPrefsAdapter"
      :editor-prefs="editorPrefsAdapter"
      :suggestion-prefs="suggestionPrefsAdapter"
      :highlight-rules="highlightRules"
      :highlight-saving="highlightSaving"
      :highlight-display-mode="highlightDisplayMode"
      :quick-commands="quickCommands"
      :quick-saving="quickSaving"
      :quick-importing="quickImportBusy"
      :t="t"
      @notice="showNotice"
      @error="showError"
      @save-highlight-rule="saveHighlightRule"
      @delete-highlight-rule="deleteHighlightRule"
      @toggle-highlight-rule="toggleHighlightRule"
      @update:highlight-display-mode="setHighlightDisplayMode"
      @save-quick-command="saveQuickCommand"
      @delete-quick-command="deleteQuickCommand"
      @import-quick-commands="importQuickCommands"
      @rename-quick-command-group="renameQuickCommandGroupById"
      @dissolve-quick-command-group="dissolveQuickCommandGroupById"
      @browse-download-dir="folderPickerTarget = 'settings'"
      @update:webgl="setWebglEnabled"
      @update-behavior="updateTerminalBehavior"
      @update-hotkeys="updateTerminalHotkeys"
      @update:action-links="updateActionLinksSettings"
      @update:gutter="updateGutterSettings"
      @update:ctx-search-engines="updateCtxSearchEngines"
      @update:ghost-suggest="setGhostEnabled"
      @update:history-panel-enabled="setHistoryPanelEnabled"
      @update:command-history-limit="applyCommandHistoryLimit"
      :ghost-tab-accept="ghostTabAccept"
      @update:ghost-tab-accept="setGhostTabAccept"
      :ai-settings="aiSettingsState"
      :ai-bridge-ready="aiBridgeReady"
      @update-ai-settings="updateAiSettings"
      @reset-ai-consent="resetAiFixConsent"
      :prompt-hints-enabled="promptHintsEnabled"
      @update:prompt-hints-enabled="setPromptHintsEnabled"
      @update:wallpaper-enabled="updateWallpaperEnabled"
      @update:wallpaper-opacity="updateWallpaperOpacity"
      @set-wallpaper-image="setWallpaperImage"
      @clear-wallpaper="clearWallpaperImage"
      @apply-font="(payload) => applyTerminalFontSettings(payload.family, payload.size)"
      @update-appearance="updateTerminalAppearance"
      @apply-theme="applyTerminalAppearanceTheme"
      @save-theme="saveTerminalAppearanceTheme"
      @delete-theme="deleteTerminalAppearanceTheme"
      @add-schemes="addImportedSchemes"
      @remove-scheme="removeImportedScheme"
    />

    <WorkbenchSessionDialogs
      v-model:spawnSessionDialogOpen="spawnSessionDialogOpen"
      v-model:spawnSessionCommand="spawnSessionCommand"
      :host-key-prompt="hostKeyPrompt"
      v-model:rememberHostKey="rememberHostKey"
      :rdp-cert-prompt="rdpCertPrompt"
      v-model:rdpCertRemember="rdpCertRemember"
      :rdp-cert-remaining="rdpCertRemaining"
      :agent-prompt-head="agentPromptHead"
      v-model:agentPromptCommand="agentPromptCommand"
      v-model:agentPromptRemember="agentPromptRemember"
      :agent-prompt-remaining="agentPromptRemaining"
      v-model:alertTriageOpen="alertTriageOpen"
      v-model:alert-triage-payload="alertTriagePayload"
      :alert-triage-busy="alertTriageBusy"
      :alert-triage-error="alertTriageError"
      :alert-triage-result="alertTriageResult"
      :has-session="!!session"
      :paste-confirm="pasteConfirm"
      :pending-confirm-dialog="pendingConfirmDialog"
      v-model:localOpenConfirmOpen="localOpenConfirmOpen"
      :t="t"
      @confirm-command-session-tab="confirmCommandSessionTab"
      @resolve-host-key="resolveHostKey"
      @resolve-rdp-certificate="resolveRdpCertificate"
      @resolve-agent-prompt="resolveAgentPrompt"
      @run-alert-triage="runAlertTriage"
      @send-suggestion="sendSuggestionToTerminal"
      @copy-suggestions="copySuggestions"
      @resolve-paste-confirm="resolvePasteConfirm"
      @resolve-pending-confirm="resolvePendingConfirmDialog"
      @confirm-local-terminal="confirmLocalTerminal"
    />

    <!-- 目录选择器：DOM 末尾渲染，保证叠在设置弹窗/下载询问弹窗之上。
         不传 initialPath：默认从「此电脑」盘符页开始（macOS/Linux 无盘符概念，
         回退到默认下载目录），与 kimi-code-desktop 的选择器行为对齐。 -->
    <FolderPickerDialog
      v-if="folderPickerTarget"
      :locale="locale"
      @select="onFolderPicked"
      @close="folderPickerTarget = null"
    />

    <!-- Telnet 连接表单（P2-3）：host/port/回退格/回车 + Expect 自动应答。 -->
    <TelnetConnectDialog :locale="locale" :open="telnetDialogOpen" @update:open="(open) => (telnetDialogOpen = open)" @connect="startTelnetSession" />

    <!-- 串口连接表单（P3）：端口发现/波特率/数据位/校验/停止位/退格。 -->
    <SerialConnectDialog :locale="locale" :open="serialDialogOpen" @update:open="(open) => (serialDialogOpen = open)" @connect="startSerialSession" />
    <SerialUploadDialog :locale="locale" :open="serialUploadDialogOpen" :busy="serialUploadBusy" @update:open="(open) => (serialUploadDialogOpen = open)" @start="startSerialUpload" />

    <VncConnectDialog :locale="locale" :open="vncDialogOpen" @update:open="(open) => (vncDialogOpen = open)" @connect="startVncSession" />

    <!-- RDP 连接表单（P3-4）：host/port/NLA 凭据/分辨率/证书策略。 -->
    <RdpConnectDialog :locale="locale" :open="rdpDialogOpen" @update:open="(open) => (rdpDialogOpen = open)" @connect="connectViaRdpDialog" />

    <!-- RDP 确认/VNC 确认/Telnet 确认/串口确认（M32-A 移除）：工具条协议直开
         入口下线后，表单兜底从连接路由失败路径直接弹 ConnectDialog，不再有
         "占用终态先确认替换" 的流程。 -->


    <input ref="uploadInput" class="hidden" type="file" multiple @change="onUploadInput" />
    <!-- webkitdirectory 会自动携带多选语义；目录选择以相对路径（webkitRelativePath）回传。 -->
    <input v-if="folderUploadSupported" ref="folderUploadInput" class="hidden" type="file" multiple webkitdirectory @change="onFolderUploadInput" />
    <input ref="zmodemInput" class="hidden" type="file" multiple @change="onZmodemInput" />
    <input ref="trzszInput" class="hidden" type="file" multiple @change="onTrzszPickInput" @cancel="onTrzszPickCancel" />
  </main>
</template>
