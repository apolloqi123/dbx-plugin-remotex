/**
 * Batch send, connection info, recording, plugin events, metrics, disk usage, chmod, settings and modal wiring.
 */
import { computed, watch } from "vue";
import { type MetricsProcessSortKey, type ServerMetricsView } from "../../../features/metrics/components/ServerMetricsPanel.vue";
import { useBatchSend } from "../../../features/terminal/composables/ui/useBatchSend";
import { useConnectionInfo } from "../../../features/connections/composables/useConnectionInfo";
import { useDockMetrics } from "../../../features/metrics/composables/useDockMetrics";
import { useMetricsPanel } from "../../../features/metrics/composables/useMetricsPanel";
import { usePluginEvents } from "../composables/usePluginEvents";
import { useRecording } from "../../../features/recording/composables/useRecording";
import { useSftpChmod } from "../../../features/sftp/composables/actions/useSftpChmod";
import { useWorkbenchModals } from "../composables/useWorkbenchModals";
import type { DiskUsage, SftpEntry } from "../types";
import type { InstallWorkbenchCoreResult } from "./step1-core";
import type { InstallWorkbenchTerminalResult } from "./step2-terminal";
import type { InstallWorkbenchPrefsResult } from "./step3-prefs";
import type { InstallWorkbenchSessionIoResult } from "./step4-sessions";
import type { InstallWorkbenchAppearanceResult } from "./step5-appearance";
import type { InstallWorkbenchConnectionsResult } from "./step6-connections";
import type { InstallWorkbenchSftpResult } from "./step7-sftp";
type Deps = InstallWorkbenchCoreResult & InstallWorkbenchTerminalResult & InstallWorkbenchPrefsResult & InstallWorkbenchSessionIoResult & InstallWorkbenchAppearanceResult & InstallWorkbenchConnectionsResult & InstallWorkbenchSftpResult & { late: Record<string, any> };

export function installWorkbenchShell(deps: Deps) {
  const {
  agentModeOpen, agentPromptHead, agentPromptQueue, agentRunning, alertTriageOpen, appearance,
  applyChosenDirAsDefault, askDownloadTarget, attrsMode, attrsTarget, auditClearOpen, auditClearSubmitting,
  auditOpen, batchDeleteOpen, batchDeleteSubmitting, bookmarkSaveOpen, canWrite, clearSftpDetailCache,
  closeAttributes, closePreview, closeRightPanel, columnsOpen, commandHistory, commandOpen,
  confirmRiskyPaste, connectLog, connected, connectionReadOnly, currentPath, customEditorOpen,
  deleteTarget, diskUsage, dockerPanelOpen, downloadConflictPrompt, downloadPrompt, dropUploadPrompt,
  editorPaneContainsFocus, fileMenu, folderPickerTarget, handleWatchModified, hasEditorTabs, hostKeyPrompt,
  hostTerminalFontFamily, importPanelOpen, isLocalMode, loadDirectory, loadDownloadDir, loadDownloadUseDefaultDir,
  localOpenConfirmOpen, localSession, localShellRestored, localShellSurfaceOpen, locale, markLocalExited,
  markSerialClosed, markTelnetClosed, markVncClosed, newFileDialog, openLocalShellPrefs, openTransferTarget,
  operationDialog, otpPanelOpen, panelPaletteOpen, panelSurface, pasteConfirm, pathHistoryOpen,
  pendingConfirmDialog, persistCommandHistory, previewOpen, probeLocalCapabilities, profilesOpen, pushTerminalCommandHistory,
  quickCommands, quickMenuOpen, rdpCertPrompt, rdpCertRemember, rdpClipboardChunks, rdpDialogOpen,
  rdpPasteOpen, rdpResolutionOpen, rdpSession, rdpState, rdpSurface, reconnectPending,
  refreshAgentMode, resolveDownloadConflict, resolveDownloadConflictFor, resolveDownloadPrompt, resolveDropUpload, resolvePasteConfirm,
  resolvePendingConfirmDialog, resolveUploadDuplicate, revealTransferTarget, rightPanelView, saveActive, saveHostFile,
  scheduleReconnect, serialDialogOpen, serialSession, serialUpload, serialUploadDialogOpen, session,
  sessionMenuOpen, sessionMenuShellOpen, sessionMenuShellPickOpen, settingsDialog, settingsOpen, sftpPaneOpen,
  showError, showNotice, spawnSessionDialogOpen, sudoMode, symlinkDialog, t,
  telnetDialogOpen, telnetError, telnetSession, telnetState, terminalAppearance, terminalDiag,
  terminalFontOverride, terminalInputQueue, terminalMenuOpen, terminalState, terminalTheme, transferHistoryClearOpen,
  transferHistoryMenu, transferPanelOpen, updateTransfer, uploadAckWaiters, uploadDuplicatePrompt, vncDialogOpen,
  vncError, vncSession, vncState, watchModifiedQueue,
  } = deps;
  const { late } = deps;

  const {
    batchBarOpen,
    batchTargetsOpen,
    batchLoading,
    batchSending,
    batchTargets,
    batchSelected,
    batchDraft,
    batchError,
    batchSummary,
    batchQuickOpen,
    batchQuickSearch,
    filteredBatchQuickCommands,
    groupedBatchQuickCommands,
    batchQuickPickName,
    batchSaveMode,
    batchSaveName,
    batchSaving,
    batchHistoryIndex,
    batchHistoryBackup,
    batchBarSourceId,
    toggleBatchBar,
    broadcastBatchBarState,
    applyRemoteBatchBarState,
    refreshBatchTargets,
    toggleBatchTargetsPopover,
    toggleBatchTargetId,
    pickBatchTargets,
    pickBatchQuick,
    browseBatchHistoryUp,
    browseBatchHistoryDown,
    batchSessionLabel,
    sendBatchCommand,
    dismissBatchResult,
    openBatchBarSave,
    confirmBatchBarSave,
    cancelBatchBarSave,
  } = useBatchSend({
    showError,
    connected,
    session,
    localSession,
    quickCommands,
    commandHistory,
    panelSurface,
    sftpPaneOpen,
    transferPanelOpen,
    persistCommandHistory,
    pushTerminalCommandHistory,
    confirmRiskyPaste,
    terminalInputQueue,
    closeOtherPopovers: closeToolbarPopovers,
  });

  // 连接建立后刷新 AI 模式；断开时收起模式弹出层。
  watch(connected, (value) => {
    if (value) {
      void refreshAgentMode();
    } else {
      agentModeOpen.value = false;
    }
  });

  // ---------------------------------------------------------------------------
  // 连接信息面板（只读）

  // ---------------------------------------------------------------------------

  // 工具条弹层 toggle（quick/session/connection-info/columns/rdp/palette/batch/
  // transfer）已随菜单状态迁 composables/useTerminalMenus；shell 选项视图切换
  // 仍留在本地 shell 偏好域。
  // 下拉内切到 shell 选项视图：同弹层视图切换，菜单不关，顺带懒加载 shell 列表。
  function openSessionShellPrefs() {
    sessionMenuShellOpen.value = true;
    void openLocalShellPrefs();
  }

  // 认证方式：读取 ssh/sessions/list 当前会话行的 authMethod（只读方法名，
  // 不含任何凭据材料）。失败时面板显示占位符，不影响其他信息。
  // 连接信息面板：延迟探测/认证方式只读名称收口在 composables/useConnectionInfo。
  const {
    connectionInfoOpen,
    connectionLatency,
    connectionLatencyBusy,
    connectionLatencyFailed,
    connectionAuthMethod,
    refreshConnectionAuthMethod,
    measureLatency,
  } = useConnectionInfo({ t, session, connectionReadOnly });



  // 录制目标会话：SSH 连接优先（与既有语义一致）；本地终端（运行中的 shell）
  // 走 local/recording 家族——sidecar 侧与 ssh/recording 同形同目录。
  // SSH 断开 / 本地 shell 退出时目标消失，录制态由 useRecording 内的 watch 复位。
  const recordingTarget = computed<{ kind: "ssh" | "local"; sessionId: string } | undefined>(() => {
    if (connected.value && session.value) return { kind: "ssh", sessionId: session.value.sessionId };
    if (localSession.value) return { kind: "local", sessionId: localSession.value.sessionId };
    return undefined;
  });

  // 终端录制 + 回放（asciicast v2）：状态与交互收口在 composables/useRecording。
  const {
    recordingActive,
      recordingsOpen,
      recordings,
      recordingsLoading,
      recordingDeleteTarget,
      recordingDeleteSubmitting,
      recordingExportingId,
      replayState,
      replayPlaying,
      replaySpeed,
      replayPlayheadMs,
      replayExporting,
      replayHost,
      recordCountdown,
      recordingStartedAt,
      recordingElapsedSec,
      beginRecordCountdown,
      cancelRecordCountdown,
      startRecordingClock,
      stopRecordingClock,
      startRecordingNow,
      toggleRecording,
      loadRecordings,
      toggleRecordings,
      revealRecording,
      exportRecordingTranscript,
      recordingsQuery,
      recordingHits,
      recordingSearchBusy,
      filteredRecordings,
      runRecordingSearch,
      clearRecordingsSearch,
      deleteRecording,
      confirmRecordingDelete,
      recordingClearAllOpen,
      recordingClearAllSubmitting,
      confirmRecordingClearAll,
      REPLAY_EVENT_CAP,
      replayDurationMs,
      loadReplayEvents,
      openReplay,
      closeReplay,
      stopReplayLoop,
      replayFrame,
      toggleReplayPlay,
      onReplaySeek,
      exportRecordingGif,
      exportReplayGif,
      exportRecordingFromList,
      formatDuration,
      formatRecordedAt,
  } = useRecording({
    t, showNotice, showError, recordingTarget, appearance, terminalTheme, openTransferTarget, revealTransferTarget,
    terminalFontOverride, terminalAppearance,
    hostFontFamily: () => hostTerminalFontFamily(appearance.value),
    loadDownloadDir, loadDownloadUseDefaultDir, askDownloadTarget, applyChosenDirAsDefault, resolveDownloadConflictFor,
    probeLocalCapabilities, saveHostFile,
    closeMetrics: () => closeMetrics(),
  });

  /** 回放宿主元素由 TransferOverlays 内的模板 ref 上抛（T20）：播放器在弹窗
 *  挂载后的 nextTick 里读取该 ref 并 open xterm。 */
  function bindReplayHost(el: HTMLDivElement | null) {
    replayHost.value = el;
  }
  // 后端事件分派：收口在 composables/usePluginEvents（30 个 method 分支整体
  // 平移，method 键互斥、命中即 return，与原 if 链语义一致）。
  const { handlePluginEvent } = usePluginEvents({
    t, session, terminalState, showError, showNotice,
    terminalDiag, batchBarSourceId, applyRemoteBatchBarState,
    hostKeyPrompt, rdpCertPrompt, rdpCertRemember, connectLog,
    scheduleSessionReconnect: scheduleReconnect, reconnectPending,
    localSession, markLocalExited,
    telnetSession, telnetState, telnetError, markTelnetClosed,
    vncSession, vncState, vncError, markVncClosed,
    serialSession, markSerialClosed, serialUpload,
    rdpSession, rdpState, rdpSurface, rdpClipboardChunks,
    agentPromptQueue, agentRunning, recordingActive, startRecordingClock,
    handleWatchModified, uploadAckWaiters, updateTransfer,
  });

  // 宿主桥事件入口：env 事件由 composable 内窄化排除。
  function handleEvent(event: DbxPluginEvent) {
    handlePluginEvent(event);
  }


  // 指标卡 + 进程管理（F2）：状态/轮询/采样收口在 composables/useMetricsPanel。
  const {
    metricsOpen,
      processesOpen,
      processRows,
      processLoading,
      processSortKey,
      metrics,
      metricsLoading,
      metricsError,
      metricSamples,
      recordMetricSamples,
      metricsRxSparkline,
      metricsTxSparkline,
      metricsCpuSparkline,
      metricsMemSparkline,
      visibleDiskMounts,
      visibleNetworkInterfaces,
      metricsDistroBadge,
      refreshMetrics,
      openMetrics,
      closeMetrics,
      backfillMetricsHistory,
      toggleProcessPanel,
      refreshProcessList,
      sortedProcessRows,
      PROCESS_VISIBLE_LIMIT,
      visibleProcessRows,
      killProcessRow,
  } = useMetricsPanel({ t, showNotice, showError, session, transferPanelOpen, recordingsOpen });

  // 右侧栏指标视图快照：组件只接收解包后的普通值，避免把一整套 ref 当 props。
  const metricsView = computed<ServerMetricsView>(() => ({
    metrics: metrics.value,
    loading: metricsLoading.value,
    error: metricsError.value,
    distroBadge: metricsDistroBadge.value,
    cpuSparkline: metricsCpuSparkline.value,
    memSparkline: metricsMemSparkline.value,
    rxSparkline: metricsRxSparkline.value,
    txSparkline: metricsTxSparkline.value,
    cpuLast: metricSamples.cpu.at(-1) ?? 0,
    memLast: metricSamples.mem.at(-1) ?? 0,
    disks: visibleDiskMounts.value,
    networks: visibleNetworkInterfaces.value,
    processesOpen: processesOpen.value,
    processRows: processRows.value,
    processLoading: processLoading.value,
    processSortKey: processSortKey.value,
    sortedProcessTotal: sortedProcessRows.value.length,
    visibleProcessRows: visibleProcessRows.value,
  }));

  function setMetricsProcessSort(key: MetricsProcessSortKey) {
    processSortKey.value = key;
  }

  // 指标轮询被外部收起（录制/传输互斥会调用 closeMetrics）时同步收起右栏，
  // 否则会留下一个不再刷新的指标快照挡住终端。
  watch(metricsOpen, (open) => {
    if (!open && rightPanelView.value === "metrics" && sftpPaneOpen.value) closeRightPanel();
  });

  // —— dock 面板左上角系统信息（CPU/内存/网速）：状态/轮询/采样收口在
  // composables/useDockMetrics ——
  const {
    localTerminalActive,
    refreshPanelMetrics,
    panelMetricsVisible,
    panelMetricsFailed,
    panelMetricsErrorTitle,
    panelCpuPercent,
    panelMemPercent,
    panelNetRates,
  } = useDockMetrics({
    t, isLocalMode, localShellRestored, metrics, metricsError, metricsLoading,
    metricsOpen, connected, panelSurface, refreshMetrics,
  });

  async function refreshDiskUsage() {
    if (!session.value) return;
    const path = currentPath.value;
    const usage = await window.dbxPlugin
      .invoke<DiskUsage>("sftp/diskUsage", { sessionId: session.value.sessionId, path }, { timeoutMs: 30_000 })
      .catch(() => undefined);
    // fire-and-forget 无 epoch 守卫：慢响应/失败落回时目录可能已切换，
    // 按发起时的 path 口径校验，过期快照不回写页脚。
    if (path !== currentPath.value) return;
    diskUsage.value = usage;
  }

  // 权限矩阵换算与 chmod 对话框状态/提交：收口在 composables/useSftpChmod
  // （PERM_ROLES/PERM_COLUMNS 与八进制草稿换算同文件）。
  const {
    chmodTarget,
    chmodDraft,
    chmodSubmitting,
    toggleChmodPerm,
    toggleAttrsPerm,
    beginChmod,
    confirmChmod,
  } = useSftpChmod<SftpEntry>({
    canWrite: () => canWrite.value,
    session: () => session.value,
    sudoMode: () => sudoMode.value,
    attrsMode,
    closeFileMenu: () => { fileMenu.value = undefined; },
    loadDirectory: () => loadDirectory(),
    clearSftpDetailCache: () => clearSftpDetailCache(),
    showNotice: (message) => showNotice(message),
    showError: (cause) => showError(cause),
    t: (key) => t(key),
  });

  function openSettings() {
    settingsOpen.value = true;
    void probeLocalCapabilities();
  }


  /**
 * 工具栏弹出层互斥族统一收口（round2：收敛五处 + 三处模板内联的手抄互斥清单）。
 * 打开任一同族弹出层前调用，先关掉全部兄弟弹出层与右键菜单，再由各 toggle
 * 设定自身状态。族成员 = 模板 class="popover" 的九个弹出层（quick-commands /
 * agent-mode / highlight-rules / connection-info / columns / transfer /
 * batch-targets / bookmark-save / path-history）。语义差异说明：右侧栏的
 * 指标/容器视图（closeRightPanel 会停掉指标轮询并卸载容器面板）与批量保存态
 * （batchSaveMode，cancelBatchBarSave 带草稿清理）不属于本族，仍由 Esc 链收口；
 * batch-targets 弹层另有 mousedown-capture 点空白收起，此处再关一次幂等无害。
 */
  function closeToolbarPopovers() {
    fileMenu.value = undefined;
    terminalMenuOpen.value = false;
    transferHistoryMenu.value = undefined;
    transferPanelOpen.value = false;
    columnsOpen.value = false;
    pathHistoryOpen.value = false;
    quickMenuOpen.value = false;
    connectionInfoOpen.value = false;
    agentModeOpen.value = false;
    bookmarkSaveOpen.value = false;
    batchTargetsOpen.value = false;
    sessionMenuOpen.value = false;
    sessionMenuShellOpen.value = false;
    sessionMenuShellPickOpen.value = false;
    localShellSurfaceOpen.value = false;
    panelPaletteOpen.value = false;
    rdpPasteOpen.value = false;
    rdpResolutionOpen.value = false;
    batchQuickOpen.value = false;
    batchTargetsOpen.value = false;
  }

  // 弹层焦点管理与 Esc 关闭链：触发元素栈/幽灵点击守卫/焦点表/逐层退出收口在
  // composables/useWorkbenchModals（各弹层 open ref 与动作回调仍由 App 持有，
  // 监听器注册/注销留守本文件的 onMounted/onBeforeUnmount）。
  const { onDocumentMouseDownCapture, onDocumentClickCapture, trackStableFocus, onDocumentKeydown } = useWorkbenchModals({
    localOpenConfirmOpen,
    telnetDialogOpen,
    serialDialogOpen,
    serialUploadDialogOpen,
    vncDialogOpen,
    rdpDialogOpen,
    rdpCertPrompt,
    folderPickerTarget,
    otpPanelOpen,
    importPanelOpen,
    previewOpen,
    pasteConfirm,
    pendingConfirmDialog,
    dropUploadPrompt,
    downloadPrompt,
    downloadConflictPrompt,
    uploadDuplicatePrompt,
    attrsTarget,
    deleteTarget,
    batchDeleteOpen,
    recordingDeleteTarget,
    recordingClearAllOpen,
    transferHistoryClearOpen,
    chmodTarget,
    newFileDialog,
    operationDialog,
    symlinkDialog,
    watchModifiedQueue,
    customEditorOpen,
    commandOpen,
    profilesOpen,
    auditOpen,
    auditClearOpen,
    settingsOpen,
    alertTriageOpen,
    hostKeyPrompt,
    agentPromptHead,
    spawnSessionDialogOpen,
    batchDeleteSubmitting,
    recordingDeleteSubmitting,
    recordingClearAllSubmitting,
    auditClearSubmitting,
    metricsOpen,
    recordingsOpen,
    batchSaveMode,
    dockerPanelOpen,
    settingsDialog,
    recordCountdown,
    hasEditorTabs,
    editorPaneContainsFocus,
    saveActive,
    cancelRecordCountdown,
    closePreview,
    resolvePasteConfirm,
    resolvePendingConfirmDialog,
    resolveDropUpload,
    resolveDownloadPrompt,
    resolveDownloadConflict,
    resolveUploadDuplicate,
    closeAttributes,
    cancelBatchBarSave,
    closeRightPanel,
  });

  function openTransferPanel() {
    // 右键菜单项打开：菜单项自身随后卸载，互斥族统一收口后再开面板。
    closeToolbarPopovers();
    transferPanelOpen.value = true;
  }





  function formatModified(value?: number) {
    if (!value) return "";
    return new Intl.DateTimeFormat(locale.value, { dateStyle: "short", timeStyle: "short" }).format(new Date(value * 1000));
  }


  function writeU64(bytes: Uint8Array, offset: number, value: number) {
    new DataView(bytes.buffer, bytes.byteOffset + offset, 8).setBigUint64(0, BigInt(value), false);
  }

  // 宿主拖拽桥：SDK 监听与原始 postMessage 双通道共用同一份高亮/落点逻辑，
  // 收口在 workbench/composables/useHostDragBridge；高亮态与树实例仍是 App 权威态。

  return {
    batchBarOpen, batchTargetsOpen, batchLoading, batchSending, batchTargets, batchSelected, batchDraft, batchError,
    batchSummary, batchQuickOpen, batchQuickSearch, filteredBatchQuickCommands, groupedBatchQuickCommands, batchQuickPickName, batchSaveMode, batchSaveName,
    batchSaving, batchHistoryIndex, batchHistoryBackup, batchBarSourceId, toggleBatchBar, broadcastBatchBarState, applyRemoteBatchBarState, refreshBatchTargets,
    toggleBatchTargetsPopover, toggleBatchTargetId, pickBatchTargets, pickBatchQuick, browseBatchHistoryUp, browseBatchHistoryDown, batchSessionLabel, sendBatchCommand,
    dismissBatchResult, openBatchBarSave, confirmBatchBarSave, cancelBatchBarSave, openSessionShellPrefs, connectionInfoOpen, connectionLatency, connectionLatencyBusy,
    connectionLatencyFailed, connectionAuthMethod, refreshConnectionAuthMethod, measureLatency, recordingTarget, recordingActive, recordingsOpen, recordings,
    recordingsLoading, recordingDeleteTarget, recordingDeleteSubmitting, recordingExportingId, replayState, replayPlaying, replaySpeed, replayPlayheadMs,
    replayExporting, replayHost, recordCountdown, recordingStartedAt, recordingElapsedSec, beginRecordCountdown, cancelRecordCountdown, startRecordingClock,
    stopRecordingClock, startRecordingNow, toggleRecording, loadRecordings, toggleRecordings, revealRecording, exportRecordingTranscript, recordingsQuery,
    recordingHits, recordingSearchBusy, filteredRecordings, runRecordingSearch, clearRecordingsSearch, deleteRecording, confirmRecordingDelete, recordingClearAllOpen,
    recordingClearAllSubmitting, confirmRecordingClearAll, REPLAY_EVENT_CAP, replayDurationMs, loadReplayEvents, openReplay, closeReplay, stopReplayLoop,
    replayFrame, toggleReplayPlay, onReplaySeek, exportRecordingGif, exportReplayGif, exportRecordingFromList, formatDuration, formatRecordedAt,
    bindReplayHost, handlePluginEvent, handleEvent, metricsOpen, processesOpen, processRows, processLoading, processSortKey,
    metrics, metricsLoading, metricsError, metricSamples, recordMetricSamples, metricsRxSparkline, metricsTxSparkline, metricsCpuSparkline,
    metricsMemSparkline, visibleDiskMounts, visibleNetworkInterfaces, metricsDistroBadge, refreshMetrics, openMetrics, closeMetrics, backfillMetricsHistory,
    toggleProcessPanel, refreshProcessList, sortedProcessRows, PROCESS_VISIBLE_LIMIT, visibleProcessRows, killProcessRow, metricsView, setMetricsProcessSort,
    localTerminalActive, refreshPanelMetrics, panelMetricsVisible, panelMetricsFailed, panelMetricsErrorTitle, panelCpuPercent, panelMemPercent, panelNetRates,
    refreshDiskUsage, chmodTarget, chmodDraft, chmodSubmitting, toggleChmodPerm, toggleAttrsPerm, beginChmod, confirmChmod,
    openSettings, closeToolbarPopovers, onDocumentMouseDownCapture, onDocumentClickCapture, trackStableFocus, onDocumentKeydown, openTransferPanel, formatModified,
    writeU64,
  };
}

export type InstallWorkbenchShellResult = ReturnType<typeof installWorkbenchShell>;
