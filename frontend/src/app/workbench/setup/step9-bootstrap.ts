/**
 * Host drag bridge, workbench bootstrap/mount, persistence watchers, prompt hints, toolbar actions and side panel bindings.
 */
import { computed, onBeforeUnmount, onMounted, watch } from "vue";
import { shouldLazyLoadSftpListing } from "../../lib/workbenchLayout";
import { type WorkbenchToolbarActions } from "../../lib/workbenchToolbarActions";
import { useHostDragBridge } from "../composables/useHostDragBridge";
import { usePromptHints } from "../../../features/terminal/composables/ui/usePromptHints";
import { useWorkbenchBootstrap } from "../composables/useWorkbenchBootstrap";
import type { SetupState } from "./setupState";
import type { InstallWorkbenchCoreResult } from "./step1-core";
import type { InstallWorkbenchTerminalResult } from "./step2-terminal";
import type { InstallWorkbenchPrefsResult } from "./step3-prefs";
import type { InstallWorkbenchSessionIoResult } from "./step4-sessions";
import type { InstallWorkbenchAppearanceResult } from "./step5-appearance";
import type { InstallWorkbenchConnectionsResult } from "./step6-connections";
import type { InstallWorkbenchSftpResult } from "./step7-sftp";
import type { InstallWorkbenchShellResult } from "./step8-shell";
type Deps = InstallWorkbenchCoreResult & InstallWorkbenchTerminalResult & InstallWorkbenchPrefsResult & InstallWorkbenchSessionIoResult & InstallWorkbenchAppearanceResult & InstallWorkbenchConnectionsResult & InstallWorkbenchSftpResult & InstallWorkbenchShellResult & { state: SetupState };

export function installWorkbenchBootstrap(deps: Deps) {
  const {
  SFTP_QUICK_PATHS, addLocalShellEnvRow, adjustTerminalZoom, aiBridgeOk, aiBridgeReady, aiSearchState,
  applePlatform, applyAgentMode, applyAppearance, applyRdpCustomResolution, applyRdpResolution, applyTerminalPaddingVars,
  archiveBusy, attachSession, availableExternalEditors, batchDeleteSubmitting, batchProgress, beginResumeUpload,
  bindSftpPathBarInput, bindSftpWorkspaceTreePane, bookmarkSaving, canWrite, cancelRecordCountdown, cancelReconnect, cancelTransfer,
  closeRdpSession, commandHistoryBackup, commandHistoryIndex, commandRunning, completionOpen, connected,
  connectionId, createTerminal, detachActionLinks, detachGutterListeners, detachHighlightRender,
  detachTerminalHost, diskUsage, disposeOutputPipeline, disposeTerminalInstance, downloadChunkWaiters, dragActive,
  editorConfig, fileMenuProtected, folderUploadSupported, followDirectory, handleBinary, handleDockerOpenInTerminal,
  handleHostFileDrop, handlePluginEvent, hideTooltip, historyPanelOpen, historyScope, hostContext,
  hostFontObserver, hostKeyPrompt, hydrateHighlightRules, hydratePrefs, hydrateQuickCommands, initialState,
  initializeSftpTree, isArchiveName, isLocalMode, isRdpConnection, isRdpResolutionActive, loadSftpDetails,
  loadedTreeEntries, loadingFiles, localCanSave, localSession, localShellRestored, localState,
  localUiMode, locale, mappedEditorIdFor, measureLatency, metrics, metricsDistroBadge,
  metricsLoading, metricsView, onDocumentClickCapture, onDocumentClickCloseMenus, onDocumentKeydown, onDocumentMouseDownCapture,
  onResumeFilePicked, onTooltipFocusIn, onTooltipFocusOut, onTooltipOut, onTooltipOver,
  openAuditLog, openCommandSessionTab, openConnectionSurface, openCopiedSessionTab, openLocalShellSurface,
  openLocalShellSurfaceMenu, openLocalTerminalTab, openNewSessionTab, openSession, openSessionShellPick, openSessionShellPrefs,
  openSettings, openTransferTarget, paneDirection, paneOrder, panelSurface, pasteBusy,
  pasteQuickCommand, pathCrumbs, persistState, persistThrottle, pickLocalShell, pickPanelScheme,
  createQuickCommandGroup, deleteQuickCommand, dissolveQuickCommandGroupById, importQuickCommands,
  quickSelectOpen, rdpResolutionLabel, rdpSession, rdpState, rdpTypeAsKeystrokes,
  renameQuickCommandGroupById, saveQuickCommand,
  rdpWindowProportionPreset, rdpWriteRemoteClipboard, readTerminalSuggestionAnchor, reattachLocalSession, reattachVncSession, reconnectNow,
  refreshTransferPanel, removeLocalShellEnvRow, renameDraft, renameSubmitting, requestHostReopenConnection, requestLocalTerminal,
  resetDragHighlightOnWindowLeave, resetTransportReuseState, resolvePasteConfirm, resolvePendingConfirmDialog, restartLocalTerminal, restoreUiState,
  restored, revealTransferTarget, rightPanelView, scheduleFit, searchOpen, selectedEntries,
  selectedPath, selectedUriSet, sendQuickCommand, sendSudoRefresh, serialBinaryInput, serialLastSequence,
  serialPendingFrames, serialSession, serialState, session, setLocalShellCwd, setLocalShellEnv,
  setLocalShellIntegrationPref, setLocalShellPref, sftpBookmarks, sftpClipboard, sftpDragDropTarget, sftpFiltersActive,
  sftpListingLoaded, sftpPaneOpen, sftpShowHidden, sftpSoftError, sftpTreeEntries, sftpTreePane,
  sftpTreeRoot, sftpTreeRows, sftpTreeTotalRows, sftpViewActive, showError, showNotice,
  showTransferHistoryMenu, sort, splitRatio, startLocalTerminal, stopAgentPromptTimer, stopCommandMarkerTick,
  sudoMode, suggestionOpen, t, telnetLastSequence, telnetPendingFrames, telnetSession,
  telnetState, terminal, terminalError, terminalHost, terminalHotkeys, terminalState,
  terminalTransferBusy, toggleAgentModeMenu, toggleBatchBar, toggleConnectionInfo, toggleDockerPane, toggleForwardsMenu,
  toggleLocalTerminal, toggleMetricsPanel, togglePaneOrder, togglePanelPaletteMenu, togglePanelToolbar,
  toggleQuickMenu, toggleRdpPasteMenu, toggleRdpResolutionMenu, toggleRecording, toggleRecordings,
  toggleSessionMenu, toggleSftpPane, toggleTransferPanel, toggleTransferPause,
  trackStableFocus, uploadAckWaiters, visibleColumns, workbenchId, writeWorkbenchState,
  } = deps;
  const M = deps.state;

  const hostDrag = useHostDragBridge({ dragActive, sftpDragDropTarget, sftpTreePane });

  // 工作台启动桥：宿主握手 → UI 水合 → 会话接回/新开 → 事件订阅的启动顺序，
  // 后端事件分派入口、MCP 设置镜像与挂载/卸载的监听器/资源收口整体收口在
  // workbench/composables/useWorkbenchBootstrap；mount/unmount 由下方生命周期钩子调用。
  const { mount, unmount } = useWorkbenchBootstrap({
    t,
    hostContext, locale, terminalState, terminalError, panelSurface, connectionId, workbenchId, restored,
    sftpDragDropTarget, sftpTreePane,
    applyHostDragState: hostDrag.applyHostDragState,
    onHostDragMessage: hostDrag.onHostDragMessage,
    openSession, attachSession, requestHostReopenConnection, resetTransportReuseState,
    localSession, localState, localShellRestored, startLocalTerminal, reattachLocalSession,
    telnetSession, telnetState, telnetLastSequence, telnetPendingFrames,
    reattachVncSession, rdpSession, rdpState,
    serialSession, serialState, serialLastSequence, serialPendingFrames, serialBinaryInput,
    terminalHost, createTerminal, scheduleFit, detachTerminalHost, disposeTerminalInstance,
    handleBinary, disposeOutputPipeline,
    initialState, restoreUiState, writeWorkbenchState, persistThrottle, hydratePrefs,
    applyTerminalPaddingVars, applyAppearance, hostFontObserver,
    hydrateQuickCommands, hydrateHighlightRules,
    showNotice, showError,
    hideTooltip, onTooltipOver, onTooltipOut, onTooltipFocusIn, onTooltipFocusOut,
    onDocumentClickCapture, onDocumentMouseDownCapture, onDocumentKeydown, trackStableFocus,
    onDocumentClickCloseMenus, resolvePasteConfirm, resolvePendingConfirmDialog,
    handleDockerOpenInTerminal, detachHighlightRender, detachActionLinks, detachGutterListeners,
    stopCommandMarkerTick, stopAgentPromptTimer, cancelReconnect,
    // usePromptHints 在 setup 尾部才声明：卸载时才求值，getter 避开 TDZ。
    disposePromptHints: () => disposePromptHints(),
    handleHostFileDrop, uploadAckWaiters, downloadChunkWaiters,
    handlePluginEvent,
    aiBridgeReady, aiBridgeOk, folderUploadSupported, resetDragHighlightOnWindowLeave,
    // 卸载标志与终端诊断计时器是 App 的 `let` 本体，经回调写穿。
    markDisposed: () => { M.disposed = true; },
    clearTerminalDiagTimer: () => window.clearInterval(M.terminalDiagTimer),
  });

  watch([splitRatio, paneOrder, paneDirection, sftpPaneOpen, followDirectory, sudoMode, visibleColumns, sftpShowHidden, sort], persistState, { deep: true });

  // SFTP 自动连接关闭时的惰性加载：面板从收起到打开的瞬间才补发首次列目录
  // 并初始化单栏树（连接时已打开的面板由 afterSessionConnected 触发；本会话
  // 已加载/树已展开时不重复，见 shouldLazyLoadSftpListing 与 initializeSftpTree）。
  watch(sftpPaneOpen, (open) => {
    if (!open) return;
    // 跟随开启时即使树已加载也要重走一遍：面板重开会让 SftpTreePane 重新挂载，
    // scrollTop 归零，需要重新揭示并把活动终端的目录滚到顶部。
    if (shouldLazyLoadSftpListing({ paneOpen: open, sessionLive: !!session.value, listingLoaded: sftpListingLoaded.value }) || !sftpTreeRoot.value.loaded || followDirectory.value) {
      void initializeSftpTree();
    }
  });

  // ---------------------------------------------------------------------------
  // MCP 设置镜像（存储迁移批 1，IMPL_PLAN_STORAGE_SYNC）：随工作台启动桥迁入
  // workbench/composables/useWorkbenchBootstrap.syncMcpSettingsMirror（启动时
  // 播种/回推，保存侧双写仍在 SettingsDialog.saveMcpSettings）。
  // ---------------------------------------------------------------------------

  onMounted(() => {
    mount();
  });

  onBeforeUnmount(() => {
    unmount();
  });


  // 空提示符快捷键引导条（TerminalPromptHints）：状态/显隐门/消散收口在
  // composables/usePromptHints；声明位置收在 setup 尾部（依赖链上有后段才
  // 声明的 const，放前面必踩 TDZ）。
  // 提升的薄包装：早期配置对象（aiSearch onActivate / 补全配置）直接引用
  // dismissPromptHints 函数名，实体在下方 composable 调用后接线，保持既有
  // 引用时机（调用都发生在 setup 结束之后）。

  function dismissPromptHints() {
    M.dismissPromptHintsImpl?.();
  }
  const {
    promptHintsEnabled,
    promptHintsLineEmpty,
    promptHintsAnchor,
    markTerminalOutputActive,
    promptHintsHistoryKey,
    promptHintsCompletionsKey,
    promptHintsVisible,
    setPromptHintsEnabled,
    dismissPromptHints: dismissPromptHintsNow,
    interactivePromptGatePending,
    resetPromptHintsQuiet,
    disposePromptHints,
  } = usePromptHints({
    terminal: () => terminal.value,
    isLocalMode, session, commandRunning, terminalTransferBusy,
    suggestionOpen, completionOpen, historyPanelOpen, quickSelectOpen, searchOpen,
    aiSearchActive: () => aiSearchState.value.active,
    hostKeyPrompt, terminalHotkeys, applePlatform,
    readTerminalSuggestionAnchor,
  });
  M.dismissPromptHintsImpl = dismissPromptHintsNow;

  // 命令历史作用域切换（连接切换 / SSH↔本地 / 本地↔串口）：命令弹窗的历史
  // 浏览态随环换桶作废，避免 ↑↓ 指向另一作用域的下标/草稿。放 setup 尾部：
  // watch 创建即求值 historyScope，引用的 connectionId/serialSession 在前段。
  watch(historyScope, () => {
    commandHistoryIndex.value = -1;
    commandHistoryBackup.value = "";
  });

  // 工具条组件的动作面（T19 工具栏拆分）：模板里原先直接绑定的函数在 setup 尾部
  // 一次性收敛成类型化对象，经 :actions 下发到 WorkbenchToolbar 及其子组件。
  // 状态权威仍在本文件（弹层开合走 v-model 链），组件不 inject、不持有会话逻辑。
  const toolbarActions: WorkbenchToolbarActions = {
    togglePanelToolbar,
    toggleSessionMenu,
    toggleConnectionInfo,
    measureLatency,
    togglePaneOrder,
    toggleSftpPane,
    adjustTerminalZoom,
    reconnectNow,
    sendSudoRefresh,
    toggleDockerPane,
    toggleBatchBar,
    toggleAgentModeMenu,
    toggleForwardsMenu,
    applyAgentMode,
    toggleMetricsPanel,
    toggleRecording,
    toggleRecordings,
    cancelRecordCountdown,
    openSettings,
    openAuditLog,
    toggleTransferPanel,
    showTransferHistoryMenu,
    toggleTransferPause,
    cancelTransfer,
    revealTransferTarget,
    openTransferTarget,
    refreshTransferPanel,
    beginResumeUpload,
    onResumeFilePicked,
    openNewSessionTab,
    openCopiedSessionTab,
    openCommandSessionTab,
    toggleLocalTerminal,
    openSessionShellPick,
    openSessionShellPrefs,
    pickLocalShell,
    setLocalShellPref,
    setLocalShellIntegrationPref,
    setLocalShellCwd,
    setLocalShellEnv,
    addLocalShellEnvRow,
    removeLocalShellEnvRow,
    openLocalTerminalTab,
    openLocalShellSurfaceMenu,
    openLocalShellSurface,
    openConnectionSurface,
    restartLocalTerminal,
    requestLocalTerminal,
    toggleQuickMenu,
    sendQuickCommand,
    pasteQuickCommand,
    saveQuickCommand,
    deleteQuickCommand,
    importQuickCommands,
    createQuickCommandGroup,
    renameQuickCommandGroupById,
    dissolveQuickCommandGroupById,
    showNotice,
    showError,
    toggleRdpResolutionMenu,
    applyRdpResolution,
    rdpResolutionLabel,
    isRdpResolutionActive,
    rdpWindowProportionPreset,
    applyRdpCustomResolution,
    toggleRdpPasteMenu,
    rdpWriteRemoteClipboard,
    rdpTypeAsKeystrokes,
    closeRdpSession,
    togglePanelPaletteMenu,
    pickPanelScheme,
  };

  // 右侧栏组件的数据面（T21 侧栏拆分）：模板里原先直接读取的 SFTP / 指标
  // 绑定在 setup 尾部一次性收敛成类型化对象，经 :shell / :sftp / :metrics
  // 下发到 WorkbenchSidePanel；可写模型（路径/编辑态/弹层开关/右键菜单负载…）仍走
  // v-model 链，动作经类型化 emit 上抛。状态权威仍在本文件，组件不 inject、不持有
  // 会话逻辑。容器管理已改为终端列底部独立分屏（ContainerManagerPane），不再下传。
  const sidePanelShell = computed(() => ({
    open: sftpPaneOpen.value,
    isRdpConnection: isRdpConnection.value,
    rightPanelView: rightPanelView.value,
    localUiMode: localUiMode.value,
    dragActive: dragActive.value,
    sftpViewActive: sftpViewActive.value,
  }));

  const sidePanelMetrics = computed(() => ({
    view: metricsView.value,
    distroBadge: metricsDistroBadge.value,
    hostname: metrics.value?.hostname,
    loading: metricsLoading.value,
  }));

  const sidePanelSftp = computed(() => ({
    connected: connected.value,
    loadingFiles: loadingFiles.value,
    panelSurface: panelSurface.value,
    followDirectory: followDirectory.value,
    pathCrumbs: pathCrumbs.value,
    bookmarkSaving: bookmarkSaving.value,
    sftpBookmarks: sftpBookmarks.value,
    quickPaths: SFTP_QUICK_PATHS,
    canWrite: canWrite.value,
    sftpClipboard: sftpClipboard.value,
    pasteBusy: pasteBusy.value,
    sudoMode: sudoMode.value,
    sftpShowHidden: sftpShowHidden.value,
    sftpFiltersActive: sftpFiltersActive.value,
    selectedEntries: selectedEntries.value,
    batchProgress: batchProgress.value,
    archiveBusy: archiveBusy.value,
    batchDeleteSubmitting: batchDeleteSubmitting.value,
    sftpSoftError: sftpSoftError.value,
    fileMenuProtected: fileMenuProtected.value,
    sftpTreeRows: sftpTreeRows.value,
    selectedUriSet: selectedUriSet.value,
    selectedPath: selectedPath.value,
    renameDraft: renameDraft.value,
    renameSubmitting: renameSubmitting.value,
    loadSftpDetails,
    dragActive: dragActive.value,
    sftpTreeEntries: sftpTreeEntries.value,
    sftpTreeTotalRows: sftpTreeTotalRows.value,
    loadedTreeEntries: loadedTreeEntries.value,
    diskUsage: diskUsage.value,
    folderUploadSupported: folderUploadSupported.value,
    localCanSave: localCanSave.value,
    availableExternalEditors: availableExternalEditors.value,
    editorConfig: editorConfig.value,
    isArchiveName,
    mappedEditorIdFor,
    setPathBarInputEl: bindSftpPathBarInput,
    onTreePane: bindSftpWorkspaceTreePane,
  }));

  return {
    hostDrag, mount, unmount, dismissPromptHints, promptHintsEnabled, promptHintsLineEmpty, promptHintsAnchor, markTerminalOutputActive,
    promptHintsHistoryKey, promptHintsCompletionsKey, promptHintsVisible, setPromptHintsEnabled, dismissPromptHintsNow, interactivePromptGatePending, resetPromptHintsQuiet, disposePromptHints,
    toolbarActions, sidePanelShell, sidePanelMetrics, sidePanelSftp,
  };
}

export type InstallWorkbenchBootstrapResult = ReturnType<typeof installWorkbenchBootstrap>;
