/**
 * Terminal instance, input routing, SFTP dialogs/workspace, transfer queue and terminal menus.
 */
import { joinRemote } from "../../../features/sftp/lib/paths/sftpRemotePaths";
import { ref } from "vue";
import { useCommandHistory } from "../../../features/terminal/composables/ui/useCommandHistory";
import { useQuickCommandPanel } from "../../../features/quick-commands/composables/useQuickCommandPanel";
import { useQuickCommands } from "../../../features/quick-commands/composables/useQuickCommands";
import { useSftpDialogs } from "../../../features/sftp/composables/actions/useSftpDialogs";
import { useSftpWorkspace } from "../../../features/sftp/composables/listing/useSftpWorkspace";
import { useTerminalInput } from "../../../features/terminal/composables/io/useTerminalInput";
import { useTerminalInstance } from "../../../features/terminal/composables/session/useTerminalInstance";
import { useTerminalMenus } from "../../../features/terminal/composables/ui/useTerminalMenus";
import { useTransferQueue } from "../../../features/transfers/composables/useTransferQueue";
import type { SftpEntry } from "../types";
import type { SetupState } from "./setupState";
import type { InstallWorkbenchCoreResult } from "./step1-core";
type Deps = InstallWorkbenchCoreResult & { late: Record<string, any>; state: SetupState };

export function installWorkbenchTerminal(deps: Deps) {
  const {
  aiSearch, appearance, canWrite, columnsOpen, connected, connectionId,
  directoryTrackingSupported, followDirectory, handleSearchResults, isRdpConnection, isRdpMode, noticeSftpErrorKey,
  panelSurface, persistDirectoryFollowPref, session, sessionMenuOpen, sessionMenuShellOpen, sessionMenuShellPickOpen,
  sftpError, sftpErrorOpen, sftpPaneOpen, sftpSearch, sftpShowHidden, sftpTreePane,
  sftpTypeFilter, showError, showNotice, sudoMode, termSelectCopy, terminal,
  terminalAppearance, terminalBehavior, terminalCopyCache, terminalCwd, terminalDiag, terminalDragActive,
  terminalFontOverride, terminalFontSize, terminalHost, terminalState, visibleColumns,
  } = deps;
  const { late } = deps;
  const M = deps.state;

  const {
    sendTerminalBytes,
    routeTerminalData,
    trackPendingInput,
    readEchoTextBeforeCursor,
    terminalInputQueue,
    serialInputQueue,
  } = useTerminalInput({
    session: () => session.value,
    localSession: () => late.localSession.value,
    telnetSession: () => late.telnetSession.value,
    serialSession: () => late.serialSession.value,
    serialBinaryInput: () => late.serialBinaryInput.value,
    setSerialBinaryInput: (enabled) => { late.serialBinaryInput.value = enabled; },
    serialUploadBusy: () => late.serialUploadBusy.value,
    terminalState: () => terminalState.value,
    zmodemBusy: () => late.zmodemBusy.value,
    trzszBusy: () => late.trzszBusy.value,
    terminalTransferBusy: () => late.terminalTransferBusy.value,
    handleAiSearchData: (data) => aiSearch.handleData(data),
    feedTrzszTerminalInput: (data) => late.feedTrzszTerminalInput(data),
    diag: () => terminalDiag,
    getPendingTerminalInput: () => M.pendingTerminalInput,
    setPendingTerminalInput: (value) => { M.pendingTerminalInput = value; },
    setPromptHintsLineEmpty: (empty) => { late.promptHintsLineEmpty.value = empty; },
    refreshSuggestionsAfterInput: (data, lineBeforeInput) => late.refreshSuggestionsAfterInput(data, lineBeforeInput),
    refreshGhostAfterInput: (data) => late.refreshGhostAfterInput(data),
    notifyCompletionLineChanged: () => late.completionController.lineChanged(),
    terminal: () => terminal.value,
    commandMarkerActive: () => late.commandMarker.active,
    pushTerminalCommandHistory: (command) => pushTerminalCommandHistory(command),
    showError,
    isLocalMode: () => late.isLocalMode.value,
    isTelnetMode: () => late.isTelnetMode.value,
    markLocalExited: (code) => late.markLocalExited(code),
    reportTransportUnavailable: (sessionId) => late.terminalTransportWatch.reportUnavailable(sessionId),
    isReconnectPending: () => late.reconnectPending.value,
    scheduleSessionReconnect: () => late.scheduleReconnect(),
  });

  const {
    searchAddon,
    webglRenderer,
    createTerminal,
    detachTerminalHost,
    scheduleFit,
    fitTerminalDimensions,
    settledTerminalDimensions,
    terminalCursorPosition,
    setWebglEnabled,
    webglRecoveryOptions,
    disposeTerminalInstance,
  } = useTerminalInstance({
    terminalHost,
    terminalCwd,
    terminalCopyCache,
    t: (key) => late.t(key),
    showNotice,
    clipboardDeps: () => late.clipboardDeps(),
    getHostFont: () => ({
      fontFamily: late.hostTerminalFontFamily(appearance.value),
      fontSize: appearance.value.terminal.fontSize,
    }),
    getFontOverride: () => terminalFontOverride.value,
    setFontSize: (size) => { terminalFontSize.value = size; },
    getTerminalAppearance: () => terminalAppearance.value,
    getTerminalBehavior: () => terminalBehavior.value,
    getTerminalTheme: () => late.terminalTheme(),
    registerOscColorQueryHandlers: () => late.registerOscColorQueryHandlers(),
    setUserVarCwdEvent: (event) => { M.userVarCwdEvent = event; },
    getKeyHandler: () => late.handleTerminalKey,
    getSearchResultsHandler: () => handleSearchResults,
    getOpenTerminalLink: () => late.openTerminalLink,
    getPasteHandler: () => late.interceptTerminalPaste,
    getWheelHandler: () => late.handleTerminalWheel,
    getMouseDownHandler: () => late.handleTerminalMouseDown,
    getMouseUpHandler: () => late.handleTerminalMouseUp,
    getBellHandler: () => late.handleTerminalBell,
    routeTerminalData,
    isTerminalMenuOpen: () => terminalMenuOpen.value,
    isSelectCopyEnabled: () => termSelectCopy.value,
    getSshSessionId: () => M.activeTerminalSessionId || session.value?.sessionId,
    getTelnetSessionId: () => late.telnetSession.value?.sessionId,
    getLocalSessionId: () => late.localSession.value?.sessionId,
    setTrzszTerminalColumns: (cols) => late.setTrzszTerminalColumns(cols),
    syncSuggestionAnchorsOnSettle: () => late.syncSuggestionAnchorsOnSettle(),
    syncGhostAnchorOnSettle: () => late.syncGhostAnchorOnSettle(),
    isWallpaperActive: () => late.wallpaperActive.value,
    onReady: () => {
      if (late.highlightEnabled.value) late.attachHighlightRender();
      if (late.actionLinksEnabled.value) late.attachActionLinks();
      if (late.isGutterActive()) late.attachGutterListeners();
    },
    onTerminalCreated: (created) => { terminal.value = created; },
  });
  // —— SFTP 面板对话框态：删除/批量删除、新建目录/文件落点、面板内剪贴板与
  // 粘贴忙碌、错误横幅 key 收口在 composables/actions/useSftpDialogs（新建文件
  // 动作逐字迁移）。删除/新建目录的动作本体仍由下方 useSftpDelete /
  // useSftpCreateDirectory 接线，本段只持有状态。——
  const {
    deleteTarget,
    deleteSubmitting,
    batchDeleteOpen,
    batchDeleteSubmitting,
    mkdirTargetDir,
    newFileDialog,
    newFileDraft,
    newFileSubmitting,
    sftpClipboard,
    pasteBusy,
    sftpErrorKey,
    openNewFileDialog,
    createNewFile,
  } = useSftpDialogs({
    t: (key, values) => late.t(key, values),
    showNotice, showError,
    session,
    sftpErrorKey: noticeSftpErrorKey,
    getConnected: () => connected.value,
    getCanWrite: () => canWrite.value,
    getSudoMode: () => sudoMode.value,
    getCurrentPath: () => currentPath.value,
    joinRemote,
    refreshAfterUpload: (targetDir) => late.refreshSftpAfterUpload(targetDir),
  });

  // —— SFTP 工作区：当前目录列表（entries/currentPath 唯一出口）、单栏树、行选中
  // 与行内重命名收口在 composables/listing/useSftpWorkspace。树懒加载/目录跟随/
  // 导航枢纽（loadDirectory）仍由既有 useSftpTreeWorkspace / useSftpNavigation
  // 持有，工作区只做组合接线；晚于本段声明的权威态经惰性 getter/setter 注入。——
  const {
    entries,
    currentPath,
    selectedPath,
    loadingFiles,
    sftpListingLoaded,
    sftpHomePath,
    sftpTreeRoot,
    sftpTreeRows,
    sftpTreeEntries,
    sftpTreeTotalRows,
    loadedTreeEntries,
    selectedEntry,
    selectedEntries,
    selectedUris,
    selectedUriSet,
    lastClickedUri,
    remoteBasename,
    isProtectedDirectory,
    isLoadedDirectoryUri,
    onSftpSelection: selectTreeEntry,
    onFileRowKeydown,
    loadHome,
    refreshSftpHomePath,
    loadDirectory,
    sftpSoftError,
    toggleSudoMode,
    enableSudoModeAndReload,
    goParent,
    setDirectoryTracking,
    goToPath,
    pathBarEditing,
    pathBarInputEl,
    pathBarDraft,
    pathCrumbs,
    beginPathBarEdit,
    cancelPathBarEdit,
    submitPathInput,
    listSftpDirectory,
    loadSftpDetails,
    clearSftpDetailCache,
    revealSftpPath,
    refreshExpandedSftpTree,
    refreshSftpAfterMutation,
    initializeSftpTree,
    followSftpPath,
    toggleSftpTreeNode,
    openSftpTreeNode,
    commitTreeRename,
    resetSftpTree,
    renamingPath,
    renameDraft,
    renameSubmitting,
    beginRename,
  } = useSftpWorkspace({
    t: (key, values) => late.t(key, values),
    showNotice, showError,
    session,
    sftpError, sftpErrorOpen, sftpErrorKey,
    terminalCwd, sftpPaneOpen, visibleColumns, directoryTrackingSupported,
    directoryParser: { reset: () => late.directoryParser.reset() },
    sftpTreePane,
    deleteTarget,
    joinRemote,
    getSudoMode: () => sudoMode.value,
    setSudoMode: (value) => { sudoMode.value = value; },
    getConnected: () => connected.value,
    getCanWrite: () => canWrite.value,
    getIsRdpConnection: () => isRdpConnection.value,
    getLocalUiMode: () => late.localUiMode.value,
    getIsLocalMode: () => late.isLocalMode.value,
    getActivePaneCwd: () => late.activePaneCwd.value,
    getFollowDirectory: () => followDirectory.value,
    setFollowDirectory: (value) => { followDirectory.value = value; },
    getSftpShowHidden: () => sftpShowHidden.value,
    getSftpSearch: () => sftpSearch.value,
    getSftpFiltersActive: () => late.sftpFiltersActive.value,
    getSftpTypeFilter: () => sftpTypeFilter.value,
    setSftpTypeFilter: (value) => { sftpTypeFilter.value = value; },
    getPendingTerminalInput: () => M.pendingTerminalInput,
    persistState: () => late.persistState(),
    refreshDiskUsage: () => late.refreshDiskUsage(),
    persistDirectoryFollowPref: (value) => persistDirectoryFollowPref(value),
    resetLinkTargets: () => { late.linkTargets.value = {}; },
    hydrateLinkTargets: (list, isCurrent) => late.hydrateLinkTargets(list, isCurrent),
    rememberPathHistory: (path) => late.rememberPathHistory(path),
    closePathHistoryMenu: () => { late.pathHistoryOpen.value = false; },
    clearSftpSearch: () => late.clearSftpSearch(),
    clearRowSelection: () => late.clearRowSelection(),
    selectFile: (entry, event) => late.selectFile(entry, event),
    openEditorTab: (entry) => late.openEditorTab(entry),
    handleEditorRename: (oldPath, newPath) => late.handleEditorRename(oldPath, newPath),
  });

  // syncPrefs 实体在下方 useWorkbenchPrefs 解构后才接线；useTransferQueue 在
  // setup 早期按引用捕获它，用提升的薄包装保持既有接线时机（调用都在 setup 后）。

  function syncPrefs() {
    return M.syncPrefsImpl?.() ?? Promise.resolve();
  }
  // 传输队列核心：任务账本/进度/暂停取消/偏好收口在 composables/useTransferQueue。
  const {
    TRANSFER_CONCURRENCY_KEY,
    TRANSFER_MAX_ACTIVE_KEY,
    TRANSFER_DOWNLOAD_LIMIT_KEY,
    TRANSFER_DUPLICATE_KEY,
    transferTasks,
    pausedTaskIds,
    transferPanelOpen,
    transferSpeeds,
    transferHistoryMenu,
    transferConcurrencyState,
    transferMaxActiveState,
    transferDownloadLimitState,
    transferDuplicateState,
    cancelledTransferTasks,
    uploadAckWaiters,
    downloadChunkWaiters,
    updateTransfer,
    transferList,
    activeTransfers,
    restoreTransfers,
    reconcileActiveTransfers,
    refreshTransferPanel,
    loadTransferConcurrency,
    persistTransferConcurrency,
    loadTransferDuplicatePolicy,
    persistTransferDuplicatePolicy,
    loadTransferMaxActive,
    persistTransferMaxActive,
    loadTransferDownloadLimit,
    persistTransferDownloadLimit,
    uploadSource,
    waitWhilePaused,
    toggleTransferPause,
    cancelTransfer,
    transferPercent,
    transferBarValue,
    transferShownBytes,
    transferBytesTitle,
  } = useTransferQueue({
    t: (key, values) => late.t(key, values),
    showError,
    session, currentPath, panelSurface,
    resolveUploadDuplicateName: (name, targetDir) => late.resolveUploadDuplicateName(name, targetDir),
    joinRemote, writeU64: (...args: any[]) => late.writeU64(...args), syncPrefs,
    refreshTransferHistory: () => late.refreshTransferHistory(),
    refreshResumableUploads: () => late.refreshResumableUploads(),
    // UploadSudo：sudo 模式 + 可写连接时上传走 sudo 车道（sudo/upload/*）。
    // 只读连接不切——sudo 族对只读 fail closed，普通下载/上传可读目标不受影响。
    sudoUpload: () => sudoMode.value && canWrite.value,
  });

  // 窗口级兜底（issue #197）：拖拽离开 webview 窗口或被 Esc 取消时，最后一次
  // dragleave 的 relatedTarget 为空——无论高亮来自哪条链路（面板 DOM 事件或宿主
  // 桥 onDragState）都复位，SFTP 面板不再停留在「上传」悬浮层。子元素间移动的
  // leave relatedTarget 非空，不会误触发。
  function resetDragHighlightOnWindowLeave(event: DragEvent) {
    if (event.relatedTarget) return;
    late.dragActive.value = false;
    terminalDragActive.value = false;
  }
  // 右键菜单/工具条弹层的受控 open 与负载、打开动作收口在
  // composables/useTerminalMenus；「关闭全部兄弟弹层」的族清单仍在下方
  // closeToolbarPopovers（结构守卫按该函数体扫描收口覆盖）。后续才声明的
  // ref（快速命令/会话/连接信息/RDP/配色快切/批量）一律经 getter/setter 惰性接入。
  const {
    terminalMenuOpen,
    fileMenu,
    fileMenuProtected,
    menuSelectionSnapshot,
    showTerminalMenu,
    showFileMenu,
    showTransferHistoryMenu,
    onDocumentClickCloseMenus,
    copySelectedPaths,
    toggleQuickMenu,
    toggleSessionMenu,
    toggleConnectionInfo,
    toggleColumnsMenu,
    toggleRdpResolutionMenu,
    toggleRdpPasteMenu,
    togglePanelPaletteMenu,
    toggleBatchQuickPopover,
    toggleTransferPanel,
  } = useTerminalMenus<SftpEntry>({
    terminal: () => terminal.value,
    terminalBehavior: () => terminalBehavior.value,
    isRdp: () => isRdpMode.value || isRdpConnection.value,
    pasteTerminal: () => { void late.pasteTerminal(); },
    copyTerminalSelection: () => { void late.copyTerminalSelection(); },
    transferHistoryMenu,
    closeToolbarPopovers: () => late.closeToolbarPopovers(),
    currentPath,
    selectedPath,
    getSelectedUris: () => selectedUris.value,
    setSelectedUris: (uris) => { selectedUris.value = uris; },
    setLastClickedUri: (uri) => { lastClickedUri.value = uri; },
    getSelectedEntryUris: () => selectedEntries.value.map((item) => item.uri),
    isLoadedDirectoryUri: (uri) => isLoadedDirectoryUri(uri),
    isProtectedDirectory: (entry) => isProtectedDirectory(entry),
    copyTextToClipboard: (value, noticeKey, values) => late.copyTextToClipboard(value, noticeKey, values),
    quickMenu: {
      isOpen: () => quickMenuOpen.value,
      setOpen: (open) => { quickMenuOpen.value = open; },
    },
    resetQuickMenu: () => {
      quickSearch.value = "";
      quickExpandedId.value = null;
    },
    sessionMenu: {
      isOpen: () => sessionMenuOpen.value,
      setOpen: (open) => { sessionMenuOpen.value = open; },
    },
    closeSessionShellViews: () => {
      sessionMenuShellOpen.value = false;
      sessionMenuShellPickOpen.value = false;
    },
    connectionInfo: {
      isOpen: () => late.connectionInfoOpen.value,
      setOpen: (open) => { late.connectionInfoOpen.value = open; },
    },
    refreshConnectionInfo: () => {
      void late.measureLatency();
      void late.refreshConnectionAuthMethod();
      // 发行版徽标数据源是 metrics 快照：未拉过时补拉一次（一次 exec，约 0.4s），
      // 否则从未开过指标浮层的会话在连接信息里永远看不到徽标。
      if (!late.metrics.value) void late.refreshMetrics();
    },
    columnsOpen,
    rdpResolution: {
      isOpen: () => late.rdpResolutionOpen.value,
      setOpen: (open) => { late.rdpResolutionOpen.value = open; },
    },
    rdpPaste: {
      isOpen: () => late.rdpPasteOpen.value,
      setOpen: (open) => { late.rdpPasteOpen.value = open; },
    },
    panelPalette: {
      isOpen: () => late.panelPaletteOpen.value,
      setOpen: (open) => { late.panelPaletteOpen.value = open; },
    },
    batchQuick: {
      isOpen: () => late.batchQuickOpen.value,
      setOpen: (open) => { late.batchQuickOpen.value = open; },
    },
    transferPanelOpen,
    closeMetrics: () => late.closeMetrics(),
  });
  const commandRunning = ref(false);
  // 命令历史（按连接作用域分桶）：采集/持久化/远端 shell 历史导入收口在
  // composables/useCommandHistory；watch(historyScope) 仍在 setup 尾部（见下）。
  const {
    historyScope,
    commandHistoryLimit,
    applyCommandHistoryLimit,
    commandHistory,
    commandHistoryTimes,
    commandHistoryMeta,
    commandHistoryIndex,
    commandHistoryBackup,
    persistCommandHistory,
    persistCommandHistoryTimes,
    pushTerminalCommandHistory,
    recordCommandExecutionMeta,
    loadRemoteShellHistory,
  } = useCommandHistory({
    getConnectionId: () => connectionId.value,
    getSerialSessionId: () => late.serialSession.value?.sessionId,
  });
  // 快速命令：用户自定义片段（≤20 条），全局存储在插件数据目录（sidecar），
  // 所有连接/工作台共享；状态与 PTY 写入语义收口在 composables/useQuickCommands。
  const {
    quickCommands,
    quickSaving,
    quickImportBusy,
    hydrateQuickCommands,
    deleteQuickCommand,
    sendQuickCommand,
    pasteQuickCommand,
    sendSudoRefresh,
    saveQuickCommand,
    renameQuickCommandGroupById,
    dissolveQuickCommandGroupById,
    importQuickCommands,
  } = useQuickCommands({
    t: (key, values) => late.t(key, values),
    showNotice,
    showError,
    session,
    commandRunning,
    terminalTransferBusy: () => late.terminalTransferBusy.value,
    terminal: () => terminal.value,
    trackPendingInput,
    sendTerminalBytes,
    // 惰性求值：hideGhostSuggestion 由下方 useGhostSuggest 解构（issue #196，
    // 快捷指令程序化写入前消除行内 ghost，防残影重锚到底部光标行）。
    hideGhostSuggestion: () => late.hideGhostSuggestion(),
  });
  // 快速命令弹层视图态（搜索/展开/分组折叠）：收口在 composables/useQuickCommandPanel。
  const {
    quickMenuOpen,
    quickSearch,
    quickExpandedId,
    filteredQuickCommands,
    groupedQuickCommands,
    quickHasRealGroups,
    quickCollapsedGroups,
    quickCommandGroupSummary,
    toggleQuickGroupCollapse,
    quickGroupName,
    toggleQuickExpand,
  } = useQuickCommandPanel({
    quickCommands,
    t: (key, values) => late.t(key, values),
  });
  // 命令建议 + 结构化补全（FIG wave-1）：浮层/引擎/调度中枢收口在
  // composables/useCommandSuggestions。

  return {
    sendTerminalBytes, routeTerminalData, trackPendingInput, readEchoTextBeforeCursor, terminalInputQueue, serialInputQueue, searchAddon, webglRenderer,
    createTerminal, detachTerminalHost, scheduleFit, fitTerminalDimensions, settledTerminalDimensions, terminalCursorPosition, setWebglEnabled, webglRecoveryOptions,
    disposeTerminalInstance, deleteTarget, deleteSubmitting, batchDeleteOpen, batchDeleteSubmitting, mkdirTargetDir, newFileDialog, newFileDraft,
    newFileSubmitting, sftpClipboard, pasteBusy, sftpErrorKey, openNewFileDialog, createNewFile, entries, currentPath,
    selectedPath, loadingFiles, sftpListingLoaded, sftpHomePath, sftpTreeRoot, sftpTreeRows, sftpTreeEntries, sftpTreeTotalRows,
    loadedTreeEntries, selectedEntry, selectedEntries, selectedUris, selectedUriSet, lastClickedUri, remoteBasename, isProtectedDirectory,
    isLoadedDirectoryUri, selectTreeEntry, onFileRowKeydown, loadHome, refreshSftpHomePath, loadDirectory, sftpSoftError, toggleSudoMode,
    enableSudoModeAndReload, goParent, setDirectoryTracking, goToPath, pathBarEditing, pathBarInputEl, pathBarDraft, pathCrumbs,
    beginPathBarEdit, cancelPathBarEdit, submitPathInput, listSftpDirectory, loadSftpDetails, clearSftpDetailCache, revealSftpPath, refreshExpandedSftpTree,
    refreshSftpAfterMutation, initializeSftpTree, followSftpPath, toggleSftpTreeNode, openSftpTreeNode, commitTreeRename, resetSftpTree, renamingPath,
    renameDraft, renameSubmitting, beginRename, syncPrefs, TRANSFER_CONCURRENCY_KEY, TRANSFER_MAX_ACTIVE_KEY, TRANSFER_DOWNLOAD_LIMIT_KEY, TRANSFER_DUPLICATE_KEY,
    transferTasks, pausedTaskIds, transferPanelOpen, transferSpeeds, transferHistoryMenu, transferConcurrencyState, transferMaxActiveState, transferDownloadLimitState,
    transferDuplicateState, cancelledTransferTasks, uploadAckWaiters, downloadChunkWaiters, updateTransfer, transferList, activeTransfers, restoreTransfers,
    reconcileActiveTransfers, refreshTransferPanel, loadTransferConcurrency, persistTransferConcurrency, loadTransferDuplicatePolicy, persistTransferDuplicatePolicy, loadTransferMaxActive, persistTransferMaxActive,
    loadTransferDownloadLimit, persistTransferDownloadLimit, uploadSource, waitWhilePaused, toggleTransferPause, cancelTransfer, transferPercent, transferBarValue,
    transferShownBytes, transferBytesTitle, resetDragHighlightOnWindowLeave, terminalMenuOpen, fileMenu, fileMenuProtected, menuSelectionSnapshot, showTerminalMenu,
    showFileMenu, showTransferHistoryMenu, onDocumentClickCloseMenus, copySelectedPaths, toggleQuickMenu, toggleSessionMenu, toggleConnectionInfo, toggleColumnsMenu,
    toggleRdpResolutionMenu, toggleRdpPasteMenu, togglePanelPaletteMenu, toggleBatchQuickPopover, toggleTransferPanel, commandRunning, historyScope, commandHistoryLimit,
    applyCommandHistoryLimit, commandHistory, commandHistoryTimes, commandHistoryMeta, commandHistoryIndex, commandHistoryBackup, persistCommandHistory, persistCommandHistoryTimes,
    pushTerminalCommandHistory, recordCommandExecutionMeta, loadRemoteShellHistory, quickCommands, quickSaving, quickImportBusy, hydrateQuickCommands, deleteQuickCommand,
    sendQuickCommand, pasteQuickCommand, sendSudoRefresh, saveQuickCommand, renameQuickCommandGroupById, dissolveQuickCommandGroupById, importQuickCommands, quickMenuOpen,
    quickSearch, quickExpandedId, filteredQuickCommands, groupedQuickCommands, quickHasRealGroups, quickCollapsedGroups, quickCommandGroupSummary, toggleQuickGroupCollapse,
    quickGroupName, toggleQuickExpand,
  };
}

export type InstallWorkbenchTerminalResult = ReturnType<typeof installWorkbenchTerminal>;
