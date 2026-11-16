import { pick } from "./lib/pick";
import type { WorkbenchApi } from "./useWorkbench";

/** Bindings owned by the sftp domain, in declaration order. */
const SFTP_KEYS = [
  "MAX_INLINE_PREVIEW_BYTES", "SFTP_QUICK_PATHS", "TRANSFER_CONCURRENCY_KEY", "TRANSFER_DOWNLOAD_LIMIT_KEY", "activeTransfers",
  "alwaysUploadWatches", "archiveBusy", "archiveDirectoryName", "archiveEntry", "attrsMode",
  "attrsOwnerDisplay", "attrsTarget", "batchArchive", "batchBarSourceId", "batchDeleteOpen",
  "batchDeleteSubmitting", "batchDownload", "batchError", "batchHistoryIndex", "batchProgress",
  "batchQuickOpen", "batchQuickPickName", "batchSaveName", "batchSelected", "batchSending",
  "batchTargetsOpen", "beginPathBarEdit", "beginRename", "beginResumeUpload", "beginSymlinkCreate",
  "bookmarkLabelDraft", "bookmarkSaving", "cancelPreviewEdit", "chmodDraft",
  "chmodSubmitting", "chmodTarget", "chooseFolderUpload", "closeAllEditorTabs", "closeEditorTab", "closePreview",
  "commitCustomEditor", "commitSymlink", "confirmBookmarkSave", "confirmChmod", "confirmDelete",
  "confirmDropUpload", "createDirectory", "createNewFile", "ctxSearchEngines", "currentPath",
  "customEditorCommand", "customEditorOpen", "customEditorPattern", "deleteQuickCommand",
  "deleteSubmitting", "deleteTarget", "deleteTerminalAppearanceTheme", "downloadChunkWaiters", "downloadConflictState",
  "downloadDirState", "downloadEntry", "downloadPrefsAdapter", "downloadRemoteFile", "downloadUseDefaultState", "dragActive",
  "dropCwdTarget", "dropUploadPathInput", "dropUploadPathInputEl", "dropUploadPrompt", "dropUploadTarget",
  "editorConfig", "editorPaneContainsFocus", "editorPaneRef", "editorPrefsAdapter", "editorSplitRatio",
  "editorTabs", "enableSudoModeAndReload", "ensureExternalEditors", "entries", "externalEditBusy",
  "externalEditors", "extractEntry", "fileBatchProgress", "fileMenu", "folderMenuAction",
  "folderUploadInput", "folderUploadProgress", "folderUploadSupported", "followSftpPath", "handleEditorRename",
  "handleHostFileDrop", "handleWatchModified", "hasEditorTabs", "hydrateLinkTargets", "isArchiveName",
  "isLoadedDirectoryUri", "lastClickedUri", "loadSftpDetails", "loadTransferConcurrency", "loadTransferDownloadLimit",
  "loadTransferDuplicatePolicy", "loadTransferMaxActive", "loadedTreeEntries", "loadingFiles", "localCanSave",
  "localDownloadDir", "mappedEditorIdFor", "mkdirTargetDir", "newFileDialog", "newFileDraft",
  "newFileSubmitting", "noticeActions", "noticeKey", "noticeSftpErrorKey", "onEditorDividerKeydown",
  "onFileAreaContextMenu", "onFileRowKeydown", "onNoticeOpenChange", "onSftpClipboardPaste", "onSftpDragEnter",
  "onSftpDragLeave", "onTerminalDragLeave", "onTerminalDrop", "onUploadInput", "openAttributes",
  "openEditorTab", "openEntry", "openInExternalEditor", "openRightPanel", "openSftpTreeNode",
  "openTransferTarget", "openWithKnownEntry", "operationDialog", "pasteClipboard", "pathBarDraft",
  "bookmarksOpen", "pathBarEditing", "pathCrumbs", "previewDirty", "previewDraft",
  "previewEditable", "previewEditableAllowed", "previewImageUrl", "previewImageZoomed", "previewLoading",
  "previewMode", "previewOpen", "previewPath", "previewSaving", "previewSize",
  "previewText", "previewTitle", "previewTruncated", "reconcileActiveTransfers", "refreshBookmarks",
  "refreshSftpAfterMutation", "refreshSftpFolder", "refreshSftpHomePath", "refreshTransferHistory", "remoteBasename",
  "renameDraft", "renameSubmitting", "resetSftpTree", "resolveDropUpload", "resolveUploadDuplicateName",
  "reloadEditorTab", "resumableTasks", "resumeTargetTaskId", "revealSftpPath", "saveAttributesPermissions", "saveEditorTab",
  "saveHostFile", "searchSelectionOnline", "selectFile", "selectedEntries", "selectedEntry",
  "selectedPath", "selectedShellInjectable", "selectedUriSet", "selectedUris", "setDirectoryTracking",
  "sftpBookmarks", "sftpClipboard", "sftpColumnWidths", "sftpCompatModeState", "sftpDragDropTarget",
  "sftpErrorKey", "sftpErrorOpen", "sftpErrorRetry", "sftpFiltersActive", "sftpHomePath",
  "sftpListingLoaded", "sftpNameEncodingState", "sftpNameWidth", "sftpPane", "sftpPaneDefaultOpen",
  "sftpPaneOpen", "sftpSearch", "sftpSearchDraft", "sftpShowHidden", "sftpSoftError",
  "sftpTreeEntries", "sftpTreePane", "sftpTreeRoot", "sftpTreeRows", "sftpTreeTotalRows",
  "sftpTypeFilter", "sftpViewActive", "sftpViewVisible", "showError", "sort",
  "submitPathInput", "symlinkDialog", "symlinkDraft", "symlinkSubmitting", "symlinkTargetDraft",
  "terminalError", "toggleAttrsPerm", "toggleBookmarksMenu", "toggleTransferPause", "transferBarValue",
  "transferBytesTitle", "transferCompressModeState", "transferCompressThresholdState", "transferConcurrencyState", "transferDownloadLimitState",
  "transferDuplicateState", "transferHistory", "transferHistoryClearOpen", "transferHistoryFailed", "transferHistoryLoading",
  "transferHistoryMenu", "transferList", "transferMaxActiveState", "transferPanelOpen", "transferPercent",
  "transferPrefsAdapter", "transferShownBytes", "transferSpeeds", "transferTasks", "updateEditorConfig",
  "updateEditorText", "updateTransfer", "uploadAckWaiters", "uploadDuplicateApplyAll", "uploadDuplicatePrompt",
  "uploadInput", "uploadSource", "uploadWatchedFileAlways", "uploadWatchedFileOnce", "visibleColumns",
  "waitForDownloadReady", "watchModifiedQueue",
] as const;

export type SftpWorkbench = Pick<WorkbenchApi, (typeof SFTP_KEYS)[number]>;

/** Flat sftp-domain API: only selects bindings, never creates state. */
export function useSftpWorkbench(api: WorkbenchApi): SftpWorkbench {
  return pick(api, SFTP_KEYS);
}
