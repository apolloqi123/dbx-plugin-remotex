/**
 * Audit log, SFTP folder actions, workbench panels, editors, archives and SFTP actions.
 */
import { joinRemote, pathFromUri } from "../../../features/sftp/lib/paths/sftpRemotePaths";
import { type EditorConfig } from "../../../features/sftp/lib/editor/editorRules";
import { useEditorOpenWith } from "../../../features/sftp/composables/editor/useEditorOpenWith";
import { useEditorSplitPane } from "../../../features/sftp/composables/editor/useEditorSplitPane";
import { useEditorTabs } from "../../../features/sftp/composables/editor/useEditorTabs";
import { useExternalEdits } from "../../../features/sftp/composables/editor/useExternalEdits";
import { useOnlineSearch } from "../../../features/terminal/composables/ui/useOnlineSearch";
import { useSftpArchive } from "../../../features/sftp/composables/transfer/useSftpArchive";
import { useSftpAttributes } from "../../../features/sftp/composables/actions/useSftpAttributes";
import { useSftpBookmarks } from "../../../features/sftp/composables/actions/useSftpBookmarks";
import { useSftpCreateDirectory } from "../../../features/sftp/composables/actions/useSftpCreateDirectory";
import { useSftpDelete } from "../../../features/sftp/composables/actions/useSftpDelete";
import { useSftpDownload } from "../../../features/sftp/composables/transfer/useSftpDownload";
import { setDockerTransferBridge } from "../../../features/docker/lib/dockerFileTransfer";
import { useSftpDropUpload } from "../../../features/sftp/composables/transfer/useSftpDropUpload";
import { useSftpFolderActions } from "../../../features/sftp/composables/actions/useSftpFolderActions";
import { useFilePreview } from "../../../features/sftp/composables/editor/useFilePreview";
import { useSftpSelectionClipboard } from "../../../features/sftp/composables/actions/useSftpSelectionClipboard";
import { useSplitDivider } from "../composables/useSplitDivider";
import { useSymlink } from "../../../features/sftp/composables/actions/useSymlink";
import { useWorkbenchPanels } from "../composables/useWorkbenchPanels";
import type { SftpEntry } from "../types";
import type { InstallWorkbenchCoreResult } from "./step1-core";
import type { InstallWorkbenchTerminalResult } from "./step2-terminal";
import type { InstallWorkbenchPrefsResult } from "./step3-prefs";
import type { InstallWorkbenchSessionIoResult } from "./step4-sessions";
import type { InstallWorkbenchAppearanceResult } from "./step5-appearance";
import type { InstallWorkbenchConnectionsResult } from "./step6-connections";
type Deps = InstallWorkbenchCoreResult & InstallWorkbenchTerminalResult & InstallWorkbenchPrefsResult & InstallWorkbenchSessionIoResult & InstallWorkbenchAppearanceResult & InstallWorkbenchConnectionsResult & { late: Record<string, any> };

export function installWorkbenchSftp(deps: Deps) {
  const {
  applyChosenDirAsDefault, askDownloadTarget, batchDeleteOpen, batchDeleteSubmitting, batchProgress, canWrite,
  cancelledTransferTasks, commandHistory, commandHistoryBackup, commandHistoryIndex, commandHistoryTimes, commandRunning,
  connected, connectionId, copyTextToClipboard, currentPath, deleteSubmitting, deleteTarget,
  dockerPaneOpen, dockerSplitRatio, downloadChunkWaiters, entries, fileMenu, isRdpConnection,
  lastClickedUri, listSftpDirectory, loadDirectory, loadDownloadDir, loadDownloadUseDefaultDir, loadSftpCompatMode, loadTransferConcurrency,
  loadTransferDuplicatePolicy, loadTransferMaxActive, localCanSave, localDownloadDir, localUiMode, mkdirTargetDir,
  openNewFileDialog, paneContainer, workbenchSplitRef, paneLayout, paneOrder, pasteBusy, persistCommandHistory,
  persistCommandHistoryTimes, persistState, persistTerminalFeaturePrefs, pushTerminalCommandHistory, refreshExpandedSftpTree, refreshSftpAfterMutation,
  remoteBasename, resetDockerSplitRatio, resolveDownloadConflictFor, revealSftpPath, rightPanelView, scheduleFit, selectedEntries,
  selectedPath, selectedUriSet, selectedUris, session, sftpClipboard, sftpHomePath,
  sftpNameEncodingState, sftpPane, sftpPaneOpen, sftpShowHidden, sftpTreeEntries, sftpTreeRoot,
  sftpViewActive, sftpViewVisible, showError, showNotice, splitRatio, sudoMode,
  t, terminal, terminalCwd, terminalDragActive, terminalMenuOpen, terminalTransferBusy,
  transferDuplicateState, transferTasks, uploadSource, waitWhilePaused, zmodemInput,
  } = deps;
  const { late } = deps;

  const {
    onFileAreaContextMenu,
    folderMenuAction,
    refreshSftpFolder,
    refreshSftpAfterUpload,
  } = useSftpFolderActions({
    terminalMenuOpen,
    fileMenu,
    session: () => session.value,
    currentPath,
    sftpTreeRoot,
    sftpShowHidden,
    mkdirTargetDir,
    openMkdirDialog: (targetDir) => {
      mkdirTargetDir.value = targetDir;
      operationDraft.value = "";
      operationDialog.value = "mkdir";
    },
    beginSymlinkCreate: (targetDir) => beginSymlinkCreate(targetDir),
    chooseUpload: (targetDir) => { void chooseUpload(targetDir); },
    chooseFolderUpload: (targetDir) => chooseFolderUpload(targetDir),
    openNewFileDialog: (targetDir) => openNewFileDialog(targetDir),
    loadDirectory: () => loadDirectory(),
    revealSftpPath: (path, opts) => revealSftpPath(path, opts),
    listSftpDirectory: (path) => listSftpDirectory(path),
    refreshExpandedSftpTree: () => refreshExpandedSftpTree(),
    showError: (cause) => showError(cause),
  });

  // R3-P1-3：目录列表加载的单调请求序号。慢链路下"先发 A 后发 B、A 晚到"
  // 会把列表/路径/历史整体回跳；只有最新请求允许落地，过期响应整体丢弃。
  // SFTP 目录导航（loadDirectory 枢纽/sudo/路径跳转）随工作区收口到
  // composables/listing/useSftpWorkspace，解构点见 setup 早期。

  // 右侧栏两视图（SFTP/指标）与终端列底部容器分屏开关：收口在
  // composables/useWorkbenchPanels。指标视图的 open/close 由后段
  // useMetricsPanel 提供，按 getter 惰性求值；容器分屏比例重置由布局域提供。
  const {
    togglePaneOrder,
    openRightPanel,
    closeRightPanel,
    toggleSftpPane,
    toggleMetricsPanel,
    toggleDockerPane,
    closeDockerPane,
  } = useWorkbenchPanels({
    paneOrder, sftpPaneOpen, rightPanelView, dockerPaneOpen, localUiMode, isRdpConnection, connected,
    metricsOpen: () => late.metricsOpen.value,
    openMetrics: () => late.openMetrics(),
    closeMetrics: () => late.closeMetrics(),
    persistState, scheduleFit,
    resetDockerSplitRatio: () => resetDockerSplitRatio(),
  });

  // 下载/传输/命令建议偏好与水合、下载目标/冲突询问链已收口在
  // composables/useWorkbenchPrefs（见上方早期声明点）。视图/终端行为偏好
  // （SFTP 默认/自动连接、目录跟随、终端行为与快捷键）已收口在
  // composables/useWorkbenchViewPrefs（见早期声明点）。

  // 文件预览与保存：状态机收口在 composables/editor/useFilePreview。
  const {
    previewOpen, previewTitle, previewText, previewLoading, previewPath, previewSize,
    previewEditable, previewDraft, previewSaving, previewMode, previewImageUrl,
    previewImageZoomed, previewTruncated, previewDirty, previewEditableAllowed, jsonPreviewState,
    openEntry, openImagePreview, closePreview, beginPreviewEdit, cancelPreviewEdit, savePreview,
    MAX_INLINE_PREVIEW_BYTES,
  } = useFilePreview({
    t, showNotice, showError, session, sudoMode, canWrite,
    loadDirectory: (path?: string) => loadDirectory(path),
    downloadEntry: (entry, forceSudo?: boolean) => downloadEntry(entry, forceSudo),
    pathFromUri,
  });

  // ---------------------------------------------------------------------------
  // 终端区编辑器页签（树内双击/Enter 打开）：页签状态/远端读写收口在
  // composables/useEditorTabs；预览弹窗与右键「预览」保持既有 useFilePreview。
  // ---------------------------------------------------------------------------
  const {
    tabs: editorTabs,
    activeId: activeEditorId,
    hasTabs: hasEditorTabs,
    openEntry: openEditorTab,
    activate: activateEditorTab,
    updateText: updateEditorText,
    saveTab: saveEditorTab,
    saveActive,
    closeTab: closeEditorTab,
    handleRename: handleEditorRename,
    closeTabsUnder: closeEditorTabsUnder,
  } = useEditorTabs({
    t, showNotice, showError, session, sudoMode, canWrite, pathFromUri,
    // 保存成功后刷新当前列表、所有已展开树节点并重新揭示当前路径；失败由
    // useEditorTabs 统一转全局错误条，保存本身已成功不回滚脏标记。
    onSaved: refreshSftpAfterMutation,
  });
  // 编辑器分栏：比例/拖拽/键盘调整/焦点判定收口在 composables/useEditorSplitPane。
  const {
    editorPaneRef,
    terminalStackRef,
    editorSplitRatio,
    startEditorDividerDrag,
    onEditorDividerKeydown,
    editorPaneContainsFocus,
  } = useEditorSplitPane({ hasEditorTabs, scheduleFit });

  // 分栏把手的拖拽/键盘微调收口在 composables/useSplitDivider；
  // splitRatio / dockerSplitRatio 仍是 App 的权威态，经 getter/setter 注入。
  // 容器分屏把手以工作台外层列为坐标系（容器页是全宽底部行）。
  const {
    startDividerDrag,
    onDividerKeydown,
    nudgeSplitDivider,
    startDockerDividerDrag,
    onDockerDividerKeydown,
    nudgeDockerDivider,
  } = useSplitDivider({
    paneContainer,
    workbenchSplitRef,
    paneLayout,
    paneOrder,
    splitRatio: { get: () => splitRatio.value, set: (value) => { splitRatio.value = value; } },
    dockerSplitRatio: { get: () => dockerSplitRatio.value, set: (value) => { dockerSplitRatio.value = value; } },
    scheduleFit,
    persistState,
  });

  // 压缩/解压：行内归档与解包收口在 composables/useSftpArchive。
  const {
    archiveBusy,
    isArchiveName,
    archiveDirectoryName,
    archiveEntry,
    extractEntry,
  } = useSftpArchive({
    t, showNotice, showError, session, currentPath, pathFromUri, joinRemote, loadDirectory,
    closeFileMenu: () => { fileMenu.value = undefined; },
  });

  // —— 打开方式子菜单与自定义编辑器命令（编辑器映射增强）：状态/动作收口在
  // composables/useEditorOpenWith（声明点见 useExternalEdits 之后，编辑器配置
  // ref 由该 composable 提供）——

  /** 设置弹窗的外部编辑器适配器：权威态在本域（pluginStore 单键 + 探测缓存）。 */
  const editorPrefsAdapter = {
    loadConfig: () => editorConfig.value,
    persistConfig: (value: EditorConfig) => updateEditorConfig(value),
    loadEditors: () => externalEditors.value,
    reloadEditors: () => ensureExternalEditors(),
  };

  // 行内重命名（提交/短路语义）随工作区收口到 composables/listing/useSftpWorkspace，
  // 解构点见 setup 早期的 useSftpWorkspace（beginRename/commitRename/renameDraft…）。

  // 新建目录对话框收口在 composables/useSftpCreateDirectory。
  const {
    operationDialog,
    operationDraft,
    createDirectory,
  } = useSftpCreateDirectory({
    showError, session, sudoMode, currentPath,
    targetDir: mkdirTargetDir,
    joinRemote, loadDirectory,
    refreshAfterCreate: refreshSftpAfterUpload,
  });


  // ---------------------------------------------------------------------------
  // SFTP 面板：搜索/多选/批量/新建文件/属性/书签/复制粘贴
  // ---------------------------------------------------------------------------

  // SFTP 路径书签：存储迁移/收藏/删除与书签弹层开合收口在
  // composables/useSftpBookmarks。
  const {
    sftpBookmarks,
    bookmarksOpen,
    toggleBookmarksMenu,
    bookmarkSaveOpen,
    bookmarkSaving,
    bookmarkLabelDraft,
    refreshBookmarks,
    toggleBookmarkSave,
    confirmBookmarkSave,
    removeBookmark,
  } = useSftpBookmarks({
    t, showNotice, showError, connected, currentPath, closeToolbarPopovers: () => late.closeToolbarPopovers(),
  });


  // 删除与批量操作：收口在 composables/useSftpDelete。
  const {
    confirmDelete,
    confirmBatchDelete,
    batchArchive,
  } = useSftpDelete({
    t, showNotice, showError, session, sudoMode, currentPath,
    deleteTarget, deleteSubmitting, batchDeleteOpen, batchDeleteSubmitting, batchProgress,
    selectedEntries, pathFromUri, joinRemote, loadDirectory,
    clearRowSelection: () => clearRowSelection(),
    archiveBusy,
    onDeleted: (paths) => {
      for (const path of paths) closeEditorTabsUnder(path);
    },
    onMutated: refreshSftpAfterMutation,
  });

  // 新建文件对话框与提交动作随对话框态收口到 composables/actions/useSftpDialogs
  // （openNewFileDialog/createNewFile 逐字迁移）。

  // 文件属性对话框：stat/chmod 状态与动作收口在 composables/useSftpAttributes。
  const {
    attrsTarget,
    attrsInfo,
    attrsOwnerDisplay,
    attrsLoading,
    attrsMode,
    attrsSubmitting,
    openAttributes,
    closeAttributes,
    saveAttributesPermissions,
  } = useSftpAttributes({
    t, showNotice, showError, session, sudoMode, pathFromUri, loadDirectory,
    closeFileMenu: () => { fileMenu.value = undefined; },
  });

  // 选中与剪贴板粘贴：收口在 composables/useSftpSelectionClipboard。
  const {
    clearRowSelection,
    selectFile,
    copySelectedEntries,
    pasteClipboard,
  } = useSftpSelectionClipboard({
    t, showNotice, showError, session, canWrite, connectionId, currentPath,
    entries, visibleEntries: sftpTreeEntries, selectedPath, selectedUris, selectedUriSet, lastClickedUri,
    sftpClipboard, pasteBusy,
    fileMenu,
    closeFileMenu: () => { fileMenu.value = undefined; },
    joinRemote, remoteBasename, pathFromUri, loadDirectory,
  });




  // 外部编辑回传：watch 确认状态机/上传串行链收口在 composables/useExternalEdits。
  const {
    externalEditBusy,
    activeExternalWatches,
    watchModifiedQueue,
    watchModifiedPrompt,
    alwaysUploadWatches,
    dismissWatchModified,
    uploadWatchedFileOnce,
    uploadWatchedFileAlways,
    handleWatchModified,
    joinLocalPath,
    openInExternalEditor,
    editorConfig,
    updateEditorConfig,
    externalEditors,
    ensureExternalEditors,
  } = useExternalEdits({
    t, showNotice, showError, loadDirectory,
    session,
    openTransferPanel: () => late.openTransferPanel(), transferTasks, pathFromUri, waitWhilePaused, cancelledTransferTasks, downloadChunkWaiters,
    waitForDownloadReady: (taskId, compression) => waitForDownloadReady(taskId, compression),
    probeLocalCapabilities: () => probeLocalCapabilities(),
    loadDownloadDir, copyTextToClipboard,
    closeFileMenu: () => { fileMenu.value = undefined; },
  });

  // 打开方式子菜单/自定义编辑器命令：收口在 composables/useEditorOpenWith。
  const {
    availableExternalEditors,
    mappedEditorIdFor,
    openWithSystemEntry,
    openWithKnownEntry,
    openWithCustomEntry,
    customEditorOpen,
    customEditorName,
    customEditorCommand,
    customEditorRemember,
    customEditorPattern,
    beginCustomEditorCommand,
    commitCustomEditor,
  } = useEditorOpenWith<SftpEntry>({
    editorConfig, externalEditors, updateEditorConfig, openInExternalEditor,
  });


  // 符号链接（P2-6）：对话框/提交/tooltip 缓存收口在 composables/useSymlink。
  const {
    symlinkDialog,
    symlinkDraft,
    symlinkTargetDraft,
    symlinkSubmitting,
    linkTargets,
    beginSymlinkCreate,
    beginSymlinkEdit,
    commitSymlink,
    linkTargetTitle,
    hydrateLinkTargets,
  } = useSymlink({
    t, showError, session, currentPath, pathFromUri, joinRemote, loadDirectory,
    refreshAfterCreate: refreshSftpAfterUpload,
  });

  // 下载落盘链：收口在 composables/useSftpDownload。
  const {
    probeLocalCapabilities,
    downloadEntry,
    downloadRemoteFile,
    waitForDownloadChunk,
    waitForDownloadReady,
    batchDownload,
    saveHostFile,
    revealTransferTarget,
    openTransferTarget,
  } = useSftpDownload({
    t, showNotice, showError, session, entries, fileMenu,
    // 跨目录/被过滤折叠的选中项都按已加载树解析，批量下载不缩水。
    selectedEntries,
    localDownloadDir, localCanSave,
    transferTasks, cancelledTransferTasks, downloadChunkWaiters, waitWhilePaused,
    loadDownloadDir, loadDownloadUseDefaultDir, askDownloadTarget,
    applyChosenDirAsDefault, resolveDownloadConflictFor,
    openTransferPanel: () => late.openTransferPanel(),
    pathFromUri,
    // sudo 模式 + 可写连接时批量下载对普通文件走 sudo 车道（与右键单项一致）。
    sudoDownload: () => sudoMode.value && canWrite.value,
    // 下载文件级并发：与上传队列同口径（批次并发与会话深度取小）。
    loadTransferConcurrency,
    loadTransferMaxActive,
    loadSftpCompatMode,
  });

  // 容器文件页的传输桥：容器页 props 固定（t/container/sessionId/connectionId/
  // canWrite），拿不到本域的 SFTP 下载链与上传队列；setup 期登记一次，供
  // features/docker 的 useContainerFileTransfer 取用（工作台生命周期内稳定）。
  // 本机 saveLocal 的撞名策略与 SFTP 下载共用同一偏好/询问。
  setDockerTransferBridge({
    downloadRemoteFile,
    uploadSource,
    resolveDownloadConflict: (name) => resolveDownloadConflictFor("", name),
  });

  // —— 断点续传：暂停/恢复 + 可续传上传 ———


  // —— 上传重复目标策略（P1-5）：上传前 sftp/exists 预检，按 transfer_duplicate_policy
  // 决定重命名 / 覆盖 / 询问。询问弹窗支持「应用到全部」（批次内生效）。——

  /** 批次级「应用到全部」决策：undefined = 尚未决定（逐个询问）。 */



  /**
 * Focus the SFTP workbench itself when the user clicks its blank area. This
 * gives Ctrl/Cmd+V a stable native paste target without stealing focus from
 * path/search inputs or toolbar controls.
 */
  function focusSftpPaneOnPointerDown(event: PointerEvent) {
    const target = event.target;
    if (target instanceof Element && target.closest("button, input, select, textarea, a, [contenteditable='true']")) return;
    sftpPane.value?.focus({ preventScroll: true });
  }

  // 上传链：五路上传入口/重复策略/拖拽落点收口在 composables/useUploadChain；
  // 面板拖拽高亮/宿主拖放落点/树行 drop 由 composables/transfer/useSftpDropUpload
  // 接管（本机文件拖入的上传编排不复制实现）。
  const {
    dragActive,
    sftpDragDropTarget,
    onSftpTreeDrop,
    uploadInput,
    folderUploadInput,
    folderUploadSupported,
    folderUploadProgress,
    fileBatchProgress,
    dropUploadPrompt,
    dropUploadTarget,
    dropCwdTarget,
    dropCwdTargetWire,
    dropUploadPathInput,
    dropUploadPathInputEl,
    uploadDuplicatePrompt,
    uploadDuplicateApplyAll,
    resolveUploadDuplicateName,
    resolveUploadDuplicate,
    onUploadInput,
    chooseFolderUpload,
    onFolderUploadInput,
    onSftpClipboardPaste,
    handleHostFileDrop,
    onDrop,
    onSftpDragEnter,
    onSftpDragLeave,
    onTerminalDragEnter,
    onTerminalDragLeave,
    onTerminalDrop,
    chooseUpload,
    confirmDropUpload,
    resolveDropUpload,
  } = useSftpDropUpload({
    t, showNotice, showError, session, connected, canWrite, currentPath, sftpHomePath,
    terminalCwd, sftpNameEncodingState, terminalDragActive, sftpPaneOpen: sftpViewVisible, terminalTransferBusy,
    sftpViewActive,
    uploadSource, loadTransferConcurrency, loadTransferMaxActive, loadTransferDuplicatePolicy, transferDuplicateState,
    transferTasks,
    terminal: () => terminal.value,
    loadDirectory,
    refreshAfterUpload: refreshSftpAfterUpload,
    openTransferPanel: () => late.openTransferPanel(),
    joinRemote,
    // 与 useTransferQueue 同源判定：sudo 模式 + 可写连接（UploadSudo 车道）。
    sudoUpload: () => sudoMode.value && canWrite.value,
  });

  // —— 右键菜单「在线搜索」：引擎表/搜索动作收口在 composables/useOnlineSearch ——
  const {
    ctxSearchEnginesText,
    ctxSearchEngines,
    updateCtxSearchEngines,
    searchSelectionOnline,
    searchTextOnline,
  } = useOnlineSearch({
    terminal: () => terminal.value,
    terminalMenuOpen,
    copyTextToClipboard,
    persistPrefs: (patch) => persistTerminalFeaturePrefs(patch),
  });

  // —— 背景图：状态/持久化/WebGL 渲染器联动收口在 composables/useWallpaper ——

  function chooseZmodem() {
    terminalMenuOpen.value = false;
    zmodemInput.value?.click();
  }

  // 批量发送：状态/发送/跨工作台广播收口在 composables/useBatchSend。

  return {
    onFileAreaContextMenu, folderMenuAction, refreshSftpFolder, refreshSftpAfterUpload, togglePaneOrder, openRightPanel, closeRightPanel, toggleSftpPane,
    toggleMetricsPanel, toggleDockerPane, closeDockerPane, startDividerDrag, onDividerKeydown, nudgeSplitDivider, startDockerDividerDrag,
    onDockerDividerKeydown, nudgeDockerDivider, previewOpen, previewTitle, previewText,
    previewLoading, previewPath, previewSize, previewEditable, previewDraft, previewSaving, previewMode, previewImageUrl,
    previewImageZoomed, previewTruncated, previewDirty, previewEditableAllowed, jsonPreviewState, openEntry, openImagePreview, closePreview,
    beginPreviewEdit, cancelPreviewEdit, savePreview, MAX_INLINE_PREVIEW_BYTES, editorTabs, activeEditorId, hasEditorTabs, openEditorTab,
    activateEditorTab, updateEditorText, saveEditorTab, saveActive, closeEditorTab, handleEditorRename, closeEditorTabsUnder, editorPaneRef,
    terminalStackRef, editorSplitRatio, startEditorDividerDrag, onEditorDividerKeydown, editorPaneContainsFocus, archiveBusy, isArchiveName, archiveDirectoryName,
    archiveEntry, extractEntry, editorPrefsAdapter, operationDialog, operationDraft, createDirectory, bookmarksOpen,
    toggleBookmarksMenu, sftpBookmarks, bookmarkSaveOpen, bookmarkSaving, bookmarkLabelDraft, refreshBookmarks,
    toggleBookmarkSave, confirmBookmarkSave, removeBookmark, confirmDelete, confirmBatchDelete, batchArchive, attrsTarget, attrsInfo,
    attrsOwnerDisplay, attrsLoading, attrsMode, attrsSubmitting, openAttributes, closeAttributes, saveAttributesPermissions, clearRowSelection,
    selectFile, copySelectedEntries, pasteClipboard, externalEditBusy, activeExternalWatches, watchModifiedQueue, watchModifiedPrompt, alwaysUploadWatches,
    dismissWatchModified, uploadWatchedFileOnce, uploadWatchedFileAlways, handleWatchModified, joinLocalPath, openInExternalEditor, editorConfig, updateEditorConfig,
    externalEditors, ensureExternalEditors, availableExternalEditors, mappedEditorIdFor, openWithSystemEntry, openWithKnownEntry, openWithCustomEntry, customEditorOpen,
    customEditorName, customEditorCommand, customEditorRemember, customEditorPattern, beginCustomEditorCommand, commitCustomEditor, symlinkDialog, symlinkDraft,
    symlinkTargetDraft, symlinkSubmitting, linkTargets, beginSymlinkCreate, beginSymlinkEdit, commitSymlink, linkTargetTitle, hydrateLinkTargets,
    probeLocalCapabilities, downloadEntry, downloadRemoteFile, waitForDownloadChunk, waitForDownloadReady, batchDownload, saveHostFile, revealTransferTarget, openTransferTarget,
    focusSftpPaneOnPointerDown, dragActive, sftpDragDropTarget, onSftpTreeDrop, uploadInput, folderUploadInput, folderUploadSupported, folderUploadProgress,
    fileBatchProgress, dropUploadPrompt, dropUploadTarget, dropCwdTarget, dropCwdTargetWire, dropUploadPathInput, dropUploadPathInputEl, uploadDuplicatePrompt,
    uploadDuplicateApplyAll, resolveUploadDuplicateName, resolveUploadDuplicate, onUploadInput, chooseFolderUpload, onFolderUploadInput, onSftpClipboardPaste, handleHostFileDrop,
    onDrop, onSftpDragEnter, onSftpDragLeave, onTerminalDragEnter, onTerminalDragLeave, onTerminalDrop, chooseUpload, confirmDropUpload,
    resolveDropUpload, ctxSearchEnginesText, ctxSearchEngines, updateCtxSearchEngines, searchSelectionOnline, searchTextOnline, chooseZmodem,
  };
}

export type InstallWorkbenchSftpResult = ReturnType<typeof installWorkbenchSftp>;
