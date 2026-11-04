/**
 * Zmodem/trzsz, protocol frame buffers, SSH lifecycle, reconnect, connect card and spawn tabs.
 */
import { TERMINAL_TRANSPORT_PROBE_TIMEOUT_MS } from "../../../features/terminal/composables/session/useTerminalTransportWatch";
import { type ReplayResult, useSshConnection } from "../../../features/connections/composables/useSshConnection";
import { startSshSessionHeartbeat } from "../../../features/connections/composables/useSshSessionHeartbeat";
import { useAgentTerminalMode } from "../../../features/agent/composables/useAgentTerminalMode";
import { useAuditLogViewer } from "../../../features/security/composables/useAuditLogViewer";
import { useConnectCard } from "../../../features/connections/composables/useConnectCard";
import { useHighlightRules } from "../../../features/terminal/composables/ui/useHighlightRules";
import { useTerminalActionLinks } from "../../../features/terminal/composables/ui/useTerminalActionLinks";
import { useTerminalGutter } from "../../../features/terminal/composables/ui/useTerminalGutter";
import { useTerminalHighlight } from "../../../features/terminal/composables/ui/useTerminalHighlight";
import { useTerminalReconnect } from "../../../features/terminal/composables/session/useTerminalReconnect";
import { stopActiveDockerLogStreams } from "../../../features/docker/lib/dockerLogStreamRegistry";
import { stopActiveDockerTerminals } from "../../../features/docker/lib/dockerTerminalRegistry";
import { useTransferHistory } from "../../../features/transfers/composables/useTransferHistory";
import { useWorkbenchSpawnTabs } from "../composables/useWorkbenchSpawnTabs";
import { watch } from "vue";
import { workbenchClosePayload, workbenchCloseTargets } from "../../lib/workbenchCloseSessions";
import type { SetupState } from "./setupState";
import type { InstallWorkbenchCoreResult } from "./step1-core";
import type { InstallWorkbenchTerminalResult } from "./step2-terminal";
import type { InstallWorkbenchPrefsResult } from "./step3-prefs";
import type { InstallWorkbenchSessionIoResult } from "./step4-sessions";
import type { InstallWorkbenchAppearanceResult } from "./step5-appearance";
type Deps = InstallWorkbenchCoreResult & InstallWorkbenchTerminalResult & InstallWorkbenchPrefsResult & InstallWorkbenchSessionIoResult & InstallWorkbenchAppearanceResult & { late: Record<string, any>; state: SetupState };

export function installWorkbenchConnections(deps: Deps) {
  const {
  auditOpen, cancelZmodemUpload, clipboardDeps, connectCancelled, connectLog, connectSucceeded,
  connected, connectionId, connectionProtocol, createTerminal, currentPath, directoryTrackingSupported,
  fitTerminalDimensions, followDirectory, hostContext, hostKeyPrompt, initialState, initializeSftpTree,
  loadDirectory, loadRemoteShellHistory, localSession, locale, outputGate, panelSurface,
  pendingTerminalFrames, persistState, rdpClipboardChunks, rdpDesktopSize, rdpSession, refreshSftpHomePath,
  rememberHostKey, replaceTerminalLineWith, resetCommandMarker, resetSftpTree, restoreTransfers, routeRdpConnection,
  scheduleFit, sendTerminalBytes, serialDialogOpen, serialSession, session, setDirectoryTracking,
  sftpAutoConnect, sftpListingLoaded, sftpSoftError, shellTabSessions, showError, showNotice,
  startLocalTerminal, startSerialFromConnection, startTelnetFromConnection, startVncFromConnection, storedConnectionProtocol, t,
  teardownTrzsz, telnetDialogOpen, telnetSession, terminal, terminalError, terminalHost,
  terminalInputQueue, terminalState, terminalWriteThrottle, trackPendingInput, uploadSource, vncDialogOpen,
  vncSession, workbenchId, writeWorkbenchState,
  } = deps;
  const { late } = deps;
  const M = deps.state;

  const {
    openSession,
    attachSession,
    closeSession,
    requestHostReopenConnection,
    resolveHostKey,
    inactiveWaiting,
    resetTransportReuseState,
  } = useSshConnection({
    t,
    showError,
    showNotice,
    connectLog,
    connectCancelled,
    connectSucceeded,
    hostKeyPrompt,
    rememberHostKey,
    terminalState,
    terminalError,
    session,
    connectionId,
    workbenchId,
    connectionProtocol,
    storedConnectionProtocol,
    directoryTrackingSupported,
    sftpListingLoaded,
    panelSurface,
    hostContext,
    followDirectory,
    sftpAutoConnect,
    currentPath,
    terminalInputQueue,
    isDisposed: () => M.disposed,
    getTerminal: () => terminal.value,
    createTerminal,
    fitTerminalDimensions,
    scheduleFit,
    resetCommandMarker: () => resetCommandMarker(),
    initialState,
    persistState: () => persistState(),
    writeWorkbenchState: () => writeWorkbenchState(),
    loadRemoteShellHistory: (sessionId) => loadRemoteShellHistory(sessionId),
    restoreTransfers: () => restoreTransfers(),
    cancelZmodemUpload: () => cancelZmodemUpload(),
    teardownTrzsz: () => teardownTrzsz(),
    startTelnetFromConnection: () => startTelnetFromConnection(),
    telnetDialogOpen,
    startVncFromConnection: () => startVncFromConnection(),
    vncDialogOpen,
    startSerialFromConnection: () => startSerialFromConnection(),
    serialDialogOpen,
    routeRdpConnection: () => routeRdpConnection(),
    startLocalTerminal: (shellOverride, connection) => startLocalTerminal(shellOverride, connection),
    getReconnectState: () => ({ reconnectPending, reconnectAttempt, reconnectNextAt, reconnectDelayMs, reconnectTimer, reconnectWasPending }),
    getAgentTerminalMode: () => ({ clearAgentPrompts, agentRunning }),
    getSftpHooks: () => ({ loadDirectory, refreshSftpHomePath, setDirectoryTracking, initializeSftpTree, resetSftpTree }),
    getSftpSoftError: () => sftpSoftError,
    getExternalEditState: () => ({ activeExternalWatches: late.activeExternalWatches, watchModifiedQueue: late.watchModifiedQueue }),
    rescanHighlightViewport: () => rescanHighlightViewport(),
    probeLocalCapabilities: () => late.probeLocalCapabilities(),
    frameStreams: {
      setActiveTerminalSessionId: (next) => { M.activeTerminalSessionId = next; },
      getLastSequence: () => M.lastSequence,
      setLastSequence: (next) => { M.lastSequence = next; },
      pendingTerminalFrames,
      setReplayNoProgress: (next) => { M.replayNoProgress = next; },
    },
  });

  // 传输断开/会话被杀的统一入口：有界退避自动重连（useSessionReconnect）+ 浏览器/
  // 宿主失联探测（useTerminalTransportWatch），装配收口在 useTerminalReconnect。
  const {
    terminalTransportWatch,
    reconnectPending,
    reconnectCountdown,
    reconnectWasPending,
    reconnectTimer,
    reconnectAttempt,
    reconnectNextAt,
    reconnectDelayMs,
    scheduleReconnect,
    cancelReconnect,
    reconnectNow,
  } = useTerminalReconnect({
    t,
    terminalState,
    terminalError,
    session,
    isDisposed: () => M.disposed,
    openSession,
    requestHostReopenConnection,
    resetInputQueue: () => terminalInputQueue.reset(),
    probeTransport: (sessionId) => window.dbxPlugin.invoke<ReplayResult>(
      "ssh/terminal/replay",
      { sessionId, afterSequence: M.lastSequence },
      { timeoutMs: TERMINAL_TRANSPORT_PROBE_TIMEOUT_MS },
    ),
    attachSession,
    // 手动重连路径本体在连接卡片（useConnectCard）；点击时才求值，可引用下方解构。
    reconnectNow: () => connectCardReconnectNow(),
  });
  // 会话心跳：宿主在编辑其它连接/重建工作台时会发延迟拆除通知；只要本工作台
  // 仍持有活会话就持续续期，延迟拆除不会落到已显示的终端上。真实关页签后心跳
  // 随 webview 停止，拆除按原宽限落地。
  startSshSessionHeartbeat(() => {
    const sessionId = session.value?.sessionId;
    if (!sessionId) return null;
    return { sessionId, connectionId: connectionId.value, workbenchId: workbenchId.value };
  });
  // 连接卡片四态与手动重连入口：收口在 composables/useConnectCard（会话主干
  // openSession/closeSession/宿主重开由 useSshConnection 持有，经 options 注入）。
  const { connectCardState, connectLogEntries, reconnect, cancelConnect, startConnect, reconnectNow: connectCardReconnectNow } = useConnectCard({
    t,
    terminalState,
    connectCancelled, connectSucceeded, connectLog,
    reconnectPending, reconnectAttempt, reconnectTimer,
    getTerminal: () => terminal.value,
    // gutter 引擎在本文件后段装配（Task 8 抽取）：这里只登记回调，调用发生在连接建立后。
    resetGutterTimestamps: () => resetGutterTimestamps(),
    requestHostReopenConnection,
    openSession,
    closeSession,
  });

  // HOST_PLUGIN_UI_SPEC §8.3/§7.4 workbench/close 两段式关闭：宿主拆除 panel/tab webview
  // 前先通知本 workbench 释放自己的 sidecar scope（PTY 会话），避免孤儿 PTY 活到 sidecar
  // 退出。特 性探测：旧宿主不发 workbench/close，也无此 API。置 disposed 拦住在途的
  // 异步启动流程（startLocalTerminal 会据此回收刚开的会话）。
  if (window.dbxPlugin.workbench?.onClose) {
    window.dbxPlugin.workbench.onClose(async () => {
      M.disposed = true;
      // 容器日志流与容器 PTY 先收口：两者都是服务器/本机上的真实进程，宿主
      // 拆除 webview 后不能再靠组件卸载补发 stop/close。幂等，失败不阻断关闭。
      await Promise.allSettled([
        stopActiveDockerLogStreams(),
        stopActiveDockerTerminals(),
      ]);
      // 只枚举本工作台自有会话（workbenchCloseTargets 契约）：同连接其他 tab、
      // 底部 dock 终端等兄弟工作台的会话不在列表里，关闭本 tab 绝不波及它们。
      // 连接级清场（connection/disconnect）是宿主生命周期语义，插件不代劳。
      const ownedSessions = workbenchCloseTargets([
        ["local/session/close", localSession.value?.sessionId],
        ["ssh/session/close", session.value?.sessionId],
        // 分屏/内嵌页签 PTY 使用独立 workbenchId 后缀，workbench/close 不会扫到它们。
        ...[...shellTabSessions.values()].map((sessionId) => ["ssh/session/close", sessionId] as const),
        ["telnet/close", telnetSession.value?.sessionId],
        ["serial/close", serialSession.value?.sessionId],
        ["vnc/close", vncSession.value?.sessionId],
        ["rdp/close", rdpSession.value?.sessionId],
      ]);
      // SSH closes are deferred in the sidecar: a host workbench re-init
      // (measured up to ~16s) restores the same session via attach, which
      // cancels the close. Other session domains close immediately.
      await Promise.allSettled(ownedSessions.map(([method, sessionId]) =>
        window.dbxPlugin.notify(method, workbenchClosePayload(method, sessionId)),
      ));
      localSession.value = null;
      session.value = undefined;
      telnetSession.value = null;
      serialSession.value = null;
      vncSession.value = null;
      rdpSession.value = null;
      rdpDesktopSize.value = null;
      rdpClipboardChunks.clear();
    });
  }

  // 同工作台 spawn 新 tab（新建/复制/命令会话）：收口在
  // composables/useWorkbenchSpawnTabs（context 克隆与宿主桥细节见该文件）。
  const {
    openNewSessionTab,
    openCopiedSessionTab,
    spawnSessionDialogOpen,
    spawnSessionCommand,
    openCommandSessionTab,
    confirmCommandSessionTab,
  } = useWorkbenchSpawnTabs({
    hostContext,
    connectionId: () => connectionId.value,
    session,
    connected: () => connected.value,
  });


  // 传输历史 + 断点续传：状态与 sidecar 接线收口在 composables/useTransferHistory。
  const {
    transferHistory,
    transferHistoryLoading,
    transferHistoryFailed,
    transferHistoryClearOpen,
    refreshTransferHistory,
    confirmTransferHistoryClear,
    resumableTasks,
    resumableLoading,
    resumeTargetTaskId,
    refreshResumableUploads,
    beginResumeUpload,
    onResumeFilePicked,
  } = useTransferHistory({ t, showNotice, showError, uploadSource, loadDirectory: (path?: string) => loadDirectory(path) });


  // AI 终端同步执行：审批队列/执行横幅/模式切换收口在 composables/useAgentTerminalMode。
  const {
    agentPromptCommand,
    agentPromptRemaining,
    agentPromptExpired,
    agentPromptRemember,
    agentPromptQueue,
    agentRunning,
    agentModeOpen,
    agentMode,
    agentModeBusy,
    agentModeHint,
    agentPromptHead,
    clearAgentPrompts,
    stopAgentPromptTimer,
    dismissAgentPrompt,
    resolveAgentPrompt,
    interruptAgentRun,
    toggleAgentModeMenu,
    refreshAgentMode,
    applyAgentMode,
  } = useAgentTerminalMode({ t, showError, session, connectionId: () => connectionId.value, sendTerminalBytes, closeToolbarPopovers: () => late.closeToolbarPopovers() });

  // 容器管理已改为终端列底部的独立分屏（ContainerManagerPane），不再占用
  // 右侧栏；本步骤不再涉及容器视图。

  // 关键词高亮：规则表 CRUD 收口在 composables/useHighlightRules（渲染引擎在下方）。
  const {
    highlightRules,
    highlightSaving,
    highlightEnabled,
    highlightDisplayMode,
    compiledHighlightRules,
    hydrateHighlightRules,
    saveHighlightRule,
    toggleHighlightRule,
    deleteHighlightRule,
    setHighlightDisplayMode,
  } = useHighlightRules({ showError });

  // 关键词高亮的 decoration 渲染引擎收口在 useTerminalHighlight（规则表 CRUD 仍在
  // useHighlightRules）；终端实例经 getter 注入，跟随终端重建。
  const { attachHighlightRender, detachHighlightRender, rescanHighlightViewport } = useTerminalHighlight({
    terminal: () => terminal.value,
    highlightEnabled,
    compiledHighlightRules,
    highlightDisplayMode,
    outputGate,
  });

  // ---------------------------------------------------------------------------
  // 动作链接 + 行号/时间戳 gutter（IMPL_PLAN Task P1-2 / P1-3，均默认关闭）。
  // 偏好权威态在此，经 sidecar preferences.json（backend/src/preferences.rs 的
  // 固定 allowlist，local/preferences/get|set）持久化；设置页控件在
  // SettingsDialog「终端」分类，经 update:* 增量上抛。
  // ---------------------------------------------------------------------------

  async function persistTerminalFeaturePrefs(patch: Record<string, unknown>) {
    try {
      await window.dbxPlugin.invoke("local/preferences/set", patch);
    } catch {
      // 旧 sidecar 无这些键位：与 localShell 等同款，当前会话内存态兜底。
    }
  }

  // 动作链接（P1-2，默认关闭）：provider / 虚线装饰 / 预览浮签收口在
  // useTerminalActionLinks；偏好权威态与 update* 增量上抛也由它持有。
  const {
    actionLinksSettings,
    actionLinksEnabled,
    actionLinkHint,
    attachActionLinks,
    detachActionLinks,
    scheduleActionLinkScan,
    handleDockerOpenInTerminal,
    updateActionLinksSettings,
  } = useTerminalActionLinks({
    terminal: () => terminal.value,
    terminalHost,
    compiledHighlightRules,
    outputGate,
    t,
    showNotice,
    session,
    localSession,
    serialSession,
    telnetSession,
    sendTerminalBytes,
    replaceTerminalLineWith,
    persistPrefs: persistTerminalFeaturePrefs,
  });

  // 行号 / 时间戳 gutter（P1-3，默认关闭）：挂子、rAF 重算与行模型收口在
  // useTerminalGutter；写入时刻采集仍挂在 write 完成回调（stampGutterWrittenRows）。
  const {
    gutterSettings,
    gutterRows,
    gutterPaneVisible,
    gutterWidth,
    gutterVisible,
    gutterPaneStyle,
    isGutterActive,
    attachGutterListeners,
    detachGutterListeners,
    scheduleGutterRecompute,
    resetGutterTimestamps,
    stampGutterWrittenRows,
    updateGutterSettings,
  } = useTerminalGutter({
    terminal: () => terminal.value,
    outputGate,
    terminalWriteThrottle,
    scheduleFit,
    persistPrefs: persistTerminalFeaturePrefs,
  });


  watch(() => session.value?.sessionId, (next, previous) => {
    if (next !== previous) {
      // 录制挂在具体 session 上：换会话后本端标记复位（后端随旧会话自动收尾）。
      late.recordingActive.value = false;
      late.cancelRecordCountdown();
      late.stopRecordingClock();
    }
  });


  // 审计日志查看：状态/拉取/清空确认收口在 composables/useAuditLogViewer。
  const {
    auditEntries,
    auditLoading,
    auditLoadFailed,
    auditTruncated,
    auditKindFilter,
    loadAuditEntries,
    openAuditLog,
    visibleAuditEntries,
    auditClearOpen,
    auditClearSubmitting,
    confirmAuditClear,
    auditTime,
    auditRowKindClass,
  } = useAuditLogViewer({ locale, auditOpen });


  // —— SFTP 单栏文件树与目录导航（懒加载/祖先路径揭示/目录跟随/loadDirectory
  // 枢纽）已随工作区收口到 composables/listing/useSftpWorkspace，见 setup 早期
  // 的 useSftpWorkspace 解构点 ——

  // SFTP 文件树右键菜单与上传后增量刷新：收口在 composables/useSftpFolderActions
  // （导航/树工作区经工作区解构出的回调接线）。

  return {
    openSession, attachSession, closeSession, requestHostReopenConnection, resolveHostKey, inactiveWaiting, resetTransportReuseState, terminalTransportWatch,
    reconnectPending, reconnectCountdown, reconnectWasPending, reconnectTimer, reconnectAttempt, reconnectNextAt, reconnectDelayMs, scheduleReconnect,
    cancelReconnect, reconnectNow, connectCardState, connectLogEntries, reconnect, cancelConnect, startConnect, connectCardReconnectNow,
    openNewSessionTab, openCopiedSessionTab, spawnSessionDialogOpen, spawnSessionCommand, openCommandSessionTab, confirmCommandSessionTab, transferHistory, transferHistoryLoading,
    transferHistoryFailed, transferHistoryClearOpen, refreshTransferHistory, confirmTransferHistoryClear, resumableTasks, resumableLoading, resumeTargetTaskId, refreshResumableUploads,
    beginResumeUpload, onResumeFilePicked, agentPromptCommand, agentPromptRemaining, agentPromptExpired, agentPromptRemember, agentPromptQueue, agentRunning,
    agentModeOpen, agentMode, agentModeBusy, agentModeHint, agentPromptHead, clearAgentPrompts, stopAgentPromptTimer, dismissAgentPrompt,
    resolveAgentPrompt, interruptAgentRun, toggleAgentModeMenu, refreshAgentMode, applyAgentMode, highlightRules, highlightSaving,
    highlightEnabled, highlightDisplayMode, compiledHighlightRules, hydrateHighlightRules, saveHighlightRule, toggleHighlightRule, deleteHighlightRule, setHighlightDisplayMode,
    attachHighlightRender, detachHighlightRender, rescanHighlightViewport, persistTerminalFeaturePrefs, actionLinksSettings, actionLinksEnabled, actionLinkHint, attachActionLinks,
    detachActionLinks, scheduleActionLinkScan, handleDockerOpenInTerminal, updateActionLinksSettings, gutterSettings, gutterRows, gutterPaneVisible, gutterWidth,
    gutterVisible, gutterPaneStyle, isGutterActive, attachGutterListeners, detachGutterListeners, scheduleGutterRecompute, resetGutterTimestamps, stampGutterWrittenRows,
    updateGutterSettings, auditEntries, auditLoading, auditLoadFailed, auditTruncated, auditKindFilter, loadAuditEntries, openAuditLog,
    visibleAuditEntries, auditClearOpen, auditClearSubmitting, confirmAuditClear, auditTime, auditRowKindClass,
  };
}

export type InstallWorkbenchConnectionsResult = ReturnType<typeof installWorkbenchConnections>;
