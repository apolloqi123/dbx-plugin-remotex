/**
 * Output writer, command markers, clipboard, remote-desktop sessions, shell tabs and session status.
 */
import { classifyConnectError, connectErrorKey } from "../../../features/connections/lib/connectError";
import { computed, watch } from "vue";
import { describeWorkbenchSessionStatus, type WorkbenchSessionStatus } from "../../../features/connections/lib/sessionStatus";
import { findTerminalPane, hasTerminalSplit, shouldFollowTabCwd } from "../../../features/terminal/lib/appearance/terminalLayout";
import { formatAuthMethodLabel, type KnownAuthMethod } from "../../../features/connections/lib/connectionInfo";
import { resolveTerminalFont } from "../../../features/terminal/lib/appearance/terminalFont";
import { sanitizeCommandOutput } from "../../../features/terminal/lib/behavior/terminalOutputText";
import { useCommandMarker } from "../../../features/terminal/composables/session/useCommandMarker";
import { useRdpSession } from "../../../features/remote-desktop/composables/useRdpSession";
import { useRdpWorkbench } from "../../../features/remote-desktop/composables/useRdpWorkbench";
import { useSftpListLayout } from "../../../features/sftp/composables/listing/useSftpListLayout";
import { useShellTabs } from "../../../features/terminal/composables/ui/useShellTabs";
import { useTerminalClipboard } from "../../../features/terminal/composables/io/useTerminalClipboard";
import { useTerminalWriter } from "../../../features/terminal/composables/session/useTerminalWriter";
import { useVncSession } from "../../../features/remote-desktop/composables/useVncSession";
import { useVncWorkbench } from "../../../features/remote-desktop/composables/useVncWorkbench";
import { useWorkbenchPersistence } from "../composables/useWorkbenchPersistence";
import { workbenchMessage } from "../../../shared/lib/i18n";
import type { SetupState } from "./setupState";
import type { InstallWorkbenchCoreResult } from "./step1-core";
import type { InstallWorkbenchTerminalResult } from "./step2-terminal";
import type { InstallWorkbenchPrefsResult } from "./step3-prefs";
type Deps = InstallWorkbenchCoreResult & InstallWorkbenchTerminalResult & InstallWorkbenchPrefsResult & { late: Record<string, any>; state: SetupState };

export function installWorkbenchSessionIo(deps: Deps) {
  const {
  aiSearch, appearance, beginAiCommand, canWrite, captureOutputChunk, closeHistoryPanel,
  closeSuggestions, commandMarkerParser, connected, connection, connectionId, currentPath,
  directoryParser, directoryTrackingSupported, dockerPaneOpen, downloadChunkWaiters, entries,
  followDirectory, followSftpPath, hostContext, isLocalMode, isRdpConnection, isRdpMode,
  isSerialMode, isTelnetMode, lastTerminalCommand, loadDirectory, loadDirectoryFollowPref, localLastCwd,
  localRecentCommands, localSession, localUiMode, locale, maybeOfferAiFix, menuSelectionSnapshot,
  paneDirection, paneLayout, paneOrder, panelSurface, pendingTerminalFrames, promptNewline,
  pushTerminalCommandHistory, rdpClipboardChunks, rdpDialogOpen, rdpScaleMode, rdpSession, rdpState,
  rdpSurface, rdpTarget, recordCommandExecutionMeta, resetAiFixForSession, resetSuggestionsForSession, rightPanelView,
  runtimeEndpoint, scheduleFit, sendTerminalBytes, serialSession, serialTarget, session,
  sftpColumnWidths, sftpNameWidth, sftpPaneOpen, sftpSearch, sftpShowHidden, sftpTypeFilter,
  showError, showNotice, sort, splitRatio, storedConnectionProtocol, sudoMode,
  syncSuggestionAnchorsOnSettle, telnetSession, telnetState, telnetTarget, terminal, terminalBehavior,
  terminalCopyCache, terminalCwd, terminalDiag, terminalError, terminalFontOverride, terminalMenuOpen,
  terminalState, trackPendingInput, visibleColumns, workbenchId,
  } = deps;
  const { late } = deps;
  const M = deps.state;

  const {
    outputGate,
    terminalWriteThrottle,
    terminalSyncOutput,
    writeTerminalOutput,
    dispatchTerminalOutput,
    handleBinary,
    disposeOutputPipeline,
  } = useTerminalWriter({
    t: (key) => t(key),
    showNotice,
    showError,
    terminal: () => terminal.value,
    isTransferBusy: () => terminalTransferBusy.value,
    frameStream: {
      pendingFrames: pendingTerminalFrames,
      get lastSequence() { return M.lastSequence; },
      set lastSequence(value: number) { M.lastSequence = value; },
      get replayNoProgress() { return M.replayNoProgress; },
      set replayNoProgress(value: number) { M.replayNoProgress = value; },
      get activeSessionId() { return M.activeTerminalSessionId; },
      set activeSessionId(value: string) { M.activeTerminalSessionId = value; },
    },
    session,
    terminalState,
    terminalError,
    terminalCwd,
    followDirectory,
    directoryTrackingSupported,
    directoryParser,
    promptNewline,
    commandMarkerParser,
    getUserVarCwdEvent: () => M.userVarCwdEvent,
    commandMarker: () => commandMarker,
    // 命令标记/AI 采集/prompt hints 均由本段之后的 composable 装配，惰性转发。
    applyCommandMarker: (updates) => applyCommandMarker(updates),
    captureOutputChunk,
    maybeOfferAiFix,
    markTerminalOutputActive: () => late.markTerminalOutputActive(),
    // 装饰引擎（gutter/高亮/动作链接）同样装配在后段。
    actionLinksEnabled: () => late.actionLinksEnabled.value,
    stampGutterWrittenRows: () => late.stampGutterWrittenRows(),
    syncSuggestionAnchorsOnSettle: () => syncSuggestionAnchorsOnSettle(),
    syncGhostAnchorOnSettle: () => late.syncGhostAnchorOnSettle(),
    rescanHighlightViewport: () => late.rescanHighlightViewport(),
    scheduleActionLinkScan: (start, end) => late.scheduleActionLinkScan(start, end),
    scheduleGutterRecompute: () => late.scheduleGutterRecompute(),
    // 传输占用与断线梯子（useZmodem/useTrzsz/useSessionReconnect/transport watch）。
    zmodemBusy: () => late.zmodemBusy.value,
    handleTrzszAnnounce: (announce) => late.handleTrzszAnnounce(announce),
    ensureTrzszFilter: () => late.ensureTrzszFilter(),
    consumeFrameViaZmodem: (data) => late.consumeFrameViaZmodem(data),
    terminalTransportWatch: () => late.terminalTransportWatch,
    isReconnectPending: () => late.reconnectPending.value,
    scheduleSessionReconnect: () => late.scheduleReconnect(),
    isDisposed: () => M.disposed,
    persistState: () => persistState(),
    // 其余二进制通道的会话/帧流由各自 composable 持有（useProtocolFrameBuffers 等）。
    localFrames: () => ({
      session: localSession,
      lastSequence: late.localLastSequence,
      pendingFrames: late.localPendingFrames,
      drain: late.drainLocalTerminalFrames,
    }),
    telnetFrames: () => ({
      session: telnetSession,
      lastSequence: late.telnetLastSequence,
      pendingFrames: late.telnetPendingFrames,
      drain: late.drainTelnetFrames,
    }),
    serialFrames: () => ({
      session: serialSession,
      lastSequence: late.serialLastSequence,
      pendingFrames: late.serialPendingFrames,
      unknownStreamFrames: late.serialUnknownStreamFrames,
      drain: late.drainSerialFrames,
    }),
    vncFrames: () => ({ session: vncSession, handleFrame: handleVncFrame }),
    rdpFrames: () => ({ session: rdpSession, state: rdpState, surface: rdpSurface }),
    getDownloadChunkWaiters: () => downloadChunkWaiters,
  });
  // 诊断开关（dbx-term-diag=1）只控制打印频率：计数对象本体在
  // state/useTerminalState，计时器句柄仍由本文件在卸载时清理。

  if (typeof window !== "undefined") {
    (window as unknown as Record<string, unknown>).__dbxTerminalDiag = terminalDiag;
    let diagEnabled = false;
    try {
      diagEnabled = window.localStorage?.getItem("dbx-term-diag") === "1";
    } catch {
      // 沙箱策略禁止 localStorage 时诊断保持关闭。
    }
    if (diagEnabled) {
      // 句柄入 onBeforeUnmount 清理：webview 反复重建时残留的 interval 会一直
      // 打 console 并持有 reactive 对象。
      M.terminalDiagTimer = window.setInterval(() => {
        console.info("[term-diag]", JSON.stringify(terminalDiag));
      }, 2000);
    }
  }

  // SSH/本地/Telnet 与串口的有序输入队列（含通道选择与失败落梯子）随
  // useTerminalInput 迁至 io/useTerminalInput.ts；解构见上方 terminalInputQueue。

  const t = (key: string, values: Record<string, string | number> = {}) => workbenchMessage(locale.value, key, values);
  // 宿主用 iframe 的 document.title 命名工作台标签：标题跟随 locale，
  // 中文环境显示「终端」而不是静态的英文插件名。
  watch(locale, (value) => { document.title = workbenchMessage(value, "pluginName"); }, { immediate: true });
  // html lang 同步跟随 locale：屏幕阅读器/拼写检查按它选语言，滞留 zh-CN 会让
  // 其他语种读错音（i18n 走查实录：locale=ja 时翻译已切换而 lang 仍为 zh-CN）。
  watch(locale, (value) => { document.documentElement.lang = value; }, { immediate: true });

  // 命令标记条（OSC 633）：状态/计时/帧消费收口在
  // composables/session/useCommandMarker；后段声明的会话态一律经 getter/回调惰性接入。
  const {
    commandMarker,
    commandMarkerElapsed,
    commandMarkerDetails,
    resetCommandMarker,
    startCommandMarkerTick,
    stopCommandMarkerTick,
    applyCommandMarker,
  } = useCommandMarker({
    t,
    commandMarkerParser,
    promptNewline,
    aiSearch,
    setUserVarCwdEvent: (event) => { M.userVarCwdEvent = event; },
    resetSuggestionsForSession,
    closeSuggestions,
    closeHistoryPanel,
    resetGhostSuggestion: () => late.resetGhostSuggestion(),
    resetPromptHintsQuiet: () => late.resetPromptHintsQuiet(),
    resetAiFixForSession,
    beginAiCommand,
    lastTerminalCommand,
    localRecentCommands,
    terminalCwd,
    localLastCwd,
    isLocalMode,
    pushTerminalCommandHistory,
    recordCommandExecutionMeta,
  });

  // 剪贴板/粘贴链（复制、右键粘贴、危险命令确认、全选/清屏）收口在
  // composables/io/useTerminalClipboard；OSC 52 写剪贴板仍留在 createTerminal。
  const {
    pasteConfirm,
    copyTextToClipboard,
    clipboardDeps,
    copyTerminalSelection,
    pasteTerminal,
    interceptTerminalPaste,
    sendConfirmedPaste,
    confirmRiskyPaste,
    resolvePasteConfirm,
    selectAllTerminal,
    clearTerminal,
  } = useTerminalClipboard({
    t,
    terminal: () => terminal.value,
    terminalCopyCache,
    terminalMenuOpen,
    menuSelectionSnapshot,
    terminalBehavior,
    terminalTransferBusy: () => terminalTransferBusy.value,
    session,
    localSession,
    telnetSession,
    serialSession,
    isDisposed: () => M.disposed,
    closeHistoryPanel,
    resetGutterTimestamps: () => late.resetGutterTimestamps(),
    hideGhostSuggestion: () => late.hideGhostSuggestion(),
    showNotice,
    showError,
    trackPendingInput,
    sendTerminalBytes,
  });
  // RDP 会话生命周期与证书确认：收口在 composables/useRdpSession（rdpSession/
  // rdpState/rdpSurface 等模板共享态留守 App.vue，经 options 注入）。
  const {
    rdpCertPrompt, rdpCertRemember, rdpCertRemaining,
    startRdpSession, closeRdpSession, sendRdpInput, sendRdpClipboard,
    reconnectRdpSession, resolveRdpCertificate,
  } = useRdpSession({
    t, showError, showNotice,
    rdpSession, rdpState, rdpSurface, rdpScaleMode, rdpClipboardChunks,
    workbenchId,
    connectionId,
    isDisposed: () => M.disposed,
    getTerminal: () => terminal.value,
  });
  // —— RDP 工作台专属逻辑（大文本粘贴 + 分辨率入口）：收口在 rdp/useRdpWorkbench，
  // App.vue 只保留薄挂载点（模板绑定经解构同名透出）。 ——
  const {
    rdpPasteOpen, rdpPasteText, rdpTypingBusy, rdpTypingSent, rdpTypingTotal,
    rdpWriteRemoteClipboard, rdpTypeAsKeystrokes,
    RDP_RESOLUTION_PRESETS, rdpResolutionOpen, rdpCustomWidth, rdpCustomHeight, rdpResizing,
    rdpDesktopSize, rdpPersistedSize, rdpResolutionLabel, isRdpResolutionActive,
    rdpWindowProportionPreset, applyRdpResolution, applyRdpCustomResolution,
    routeRdpConnection, connectViaRdpDialog, startRdpFromConnection, rdpClosedTitle,
  } = useRdpWorkbench({
    t, showError, showNotice, persistState,
    isRdpMode, rdpSession, rdpSurface,
    sendRdpInput, sendRdpClipboard,
    rdpState, rdpDialogOpen, startRdpSession,
    getConnection: () => connection,
    getStoredConnectionProtocol: () => storedConnectionProtocol,
    sftpPaneOpen,
    closeRightPanel: () => late.closeRightPanel(),
    getBatchBarOpen: () => late.batchBarOpen,
  });

  // VNC 会话生命周期与输入/帧入口：收口在 composables/useVncSession
  // （vncSession/vncState/vncSurface 等模板共享态由组合式函数持有并解构透出）。
  const {
    vncSession, vncDialogOpen, vncState, vncError, vncScaleMode, vncSurface,
    isVncMode, vncTarget,
    markVncClosed, startVncSession, closeVncSession, sendVncInput, sendVncClipboard,
    handleVncFrame, reattachVncSession,
  } = useVncSession({
    t, showError, showNotice,
    connectionId,
    workbenchId,
    runtimeEndpoint,
    isDisposed: () => M.disposed,
    getTerminal: () => terminal.value,
  });
  // —— VNC 连接路由（连接记录直启）：收口在 composables/useVncWorkbench，
  // App.vue 只保留薄挂载点（模板绑定经解构同名透出）。 ——
  const { startVncFromConnection } = useVncWorkbench({
    connection,
    startVncSession,
  });
  // 连接切换（宿主改推另一连接）时失效 sidecar 缓存的协议，避免把上一连接的
  // 协议带给下一个连接（权威协议与 connectionProtocol 派生在 state/useSessionState）。
  watch(connectionId, () => {
    storedConnectionProtocol.value = "";
  });
  // 终端分屏：布局树状态与窗格/页签操作收口在 composables/useShellTabs；
  // 「页签上报 cwd」「窗格激活」两个目录跟随入口仍在本文件（结构测试锁定）。
  const {
    terminalLayout,
    shellTabSessions,
    activeTerminalPaneId,
    shellTabCwd,
    activePaneCwd,
    openTerminalSplit,
    splitTerminalPane,
    closeTerminalSplit,
    closePane,
    createPaneTab,
    activatePaneTab,
    closePaneTab,
    onShellTabOpened,
    resetTerminalLayout,
  } = useShellTabs({
    terminalCwd,
    canSplit: () => canSplitTerminal.value,
    canCreateTab: () => canOpenShellTab.value,
    scheduleFit,
    syncSftpToActivePane: () => syncSftpToActivePane(),
  });
  const canSplitTerminal = computed(() => connected.value && !localUiMode.value && !isRdpConnection.value);
  const terminalHasSplit = computed(() => hasTerminalSplit(terminalLayout.value));
  const splitTerminalFont = computed(() =>
    resolveTerminalFont(terminalFontOverride.value, {
      fontFamily: late.hostTerminalFontFamily(appearance.value),
      fontSize: appearance.value.terminal.fontSize,
    }),
  );

  /** 分屏页签上报 OSC 7：记录并只在真实目录变化时驱动跟随。 */
  function onShellTabCwd(tabId: string, path: string) {
    const previous = shellTabCwd.get(tabId);
    shellTabCwd.set(tabId, path);
    // 首次上报是该页签的初始提示符；只有它本来就是活动窗格的活动页签（cwd
    // 迟到）才跟随。重复上报（回车/resize/提示符重绘）不重复导航。
    const activeTabId = findTerminalPane(terminalLayout.value, activeTerminalPaneId.value)?.activeTabId;
    if (!shouldFollowTabCwd(previous, path, activeTabId === tabId)) return;
    if (!followDirectory.value || !connected.value) return;
    void followSftpPath(path);
  }

  /** 任意窗格内按下/聚焦：记录最后交互窗格，并把 SFTP 同步到该窗格的目录。 */
  function onPaneActivated(paneId: string) {
    if (!paneId || activeTerminalPaneId.value === paneId) return;
    activeTerminalPaneId.value = paneId;
    syncSftpToActivePane();
  }

  /**
 * 聚焦/切换窗格或页签时显示该窗格的活动目录。只在“真正切换了窗格/页签”时
 * 调用（同一窗格内重复点击终端不会触发），因此会重新置顶但不会打断用户在
 * 当前窗格里的连续操作。
 */
  function syncSftpToActivePane() {
    if (!followDirectory.value || !connected.value) return;
    const cwd = activePaneCwd.value;
    if (!cwd) return;
    void followSftpPath(cwd);
  }

  const canOpenShellTab = computed(() => connected.value && !localUiMode.value && !isRdpConnection.value);
  // 主会话断开/切换本地/RDP 时销毁所有分屏与额外标签，避免留下不可见的 PTY。
  watch(canSplitTerminal, (allowed) => {
    if (!allowed) resetTerminalLayout();
  });
  // 指标视图依赖 SSH 会话；断开后收起右栏，避免保留一份不再刷新的快照。
  watch(connected, (isConnected) => {
    if (!isConnected && rightPanelView.value === "metrics") late.closeRightPanel();
  });
  const sessionStatus = computed<WorkbenchSessionStatus | "local">(() => (localUiMode.value ? "local" : describeWorkbenchSessionStatus(terminalState.value, { reattaching: late.reconnectPending.value })));
  // 本地模式徽标附带 shell 名（Local · Zsh），一眼可见当前在哪种 shell 里。
  const sessionPillText = computed(() => {
    // 串口徽标显示 port@baud（Serial · /dev/ttyUSB0@115200），一眼可见线路参数。
    if (isSerialMode.value) return `${t("serial.pillPrefix")} · ${serialTarget.value}`;
    // Telnet 徽标显示明文目标（Telnet · host:port），提示这是非 SSH 连接。
    if (isTelnetMode.value) return telnetState.value === "connecting" ? t("telnet.connecting") : `${t("telnet.pillPrefix")} · ${telnetTarget.value}`;
    // VNC 徽标显示远端桌面目标（VNC · host:port）。
    if (isVncMode.value) return vncState.value === "connecting" ? t("vnc.connecting") : `${t("vnc.pillPrefix")} · ${vncTarget.value}`;
    // RDP 徽标：连接中/重连中（带退避进度）显示状态，其余显示 host:port。
    if (isRdpMode.value) {
      if (rdpState.value.state === "connecting") return t("rdp.connecting");
      if (rdpState.value.state === "reconnecting") {
        return rdpState.value.maxAttempts > 0
          ? t("rdp.reconnectingAttempt", { attempt: rdpState.value.attempt, max: rdpState.value.maxAttempts })
          : t("rdp.reconnecting");
      }
      return `${t("rdp.pillPrefix")} · ${rdpTarget.value}`;
    }
    if (!isLocalMode.value || !localSession.value) return t(`sessionStatus.${sessionStatus.value}`);
    const kind = localSession.value.shell.split(/[\\/]/).pop() || localSession.value.shell;
    return `${t("sessionStatus.local")} · ${kind}`;
  });
  // Connect-error friendlification: raw sidecar/russh error strings stay as the
  // tooltip detail while the primary line renders a localized per-category hint
  // (auth / refused / DNS / timeout / host key). Non-connect errors pass through.
  const terminalErrorDetail = computed(() => terminalError.value);
  const terminalErrorFriendly = computed(() => {
    const kind = classifyConnectError(terminalError.value);
    return kind ? t(connectErrorKey(kind)) : "";
  });
  // Reconnect countdown lifecycle: while the backoff loop is pending a 250ms
  // tick recomputes the pure countdown; any exit from "reconnecting" stops it.
  const commandOutputText = computed(() => (late.commandResult.value ? sanitizeCommandOutput(late.commandResult.value.output) : ""));
  const connectionIdentity = computed(() => {
    // Connectionless local-terminal tab (including the restored shell): there is no connection identity to show.
    if (localUiMode.value && !connectionId.value) {
      if (isSerialMode.value) return `${t("serial.pillPrefix")} ${serialTarget.value}`;
      if (isTelnetMode.value) return `${t("telnet.pillPrefix")} ${telnetTarget.value}`;
      if (isVncMode.value) return `${t("vnc.pillPrefix")} ${vncTarget.value}`;
      return t("localTerminal.active");
    }
    const host = connection.value.host || connection.value.name || connectionId.value || "–";
    const identity = connection.value.username ? `${connection.value.username}@${host}` : host;
    const port = connection.value.port && connection.value.port !== 22 ? `:${connection.value.port}` : "";
    return `${identity}${port}`;
  });
  // 认证方式的本地化标签：已知方法名走 i18n，未知值原样展示（只读信息）。
  const connectionAuthMethodLabel = computed(() => formatAuthMethodLabel(late.connectionAuthMethod.value, (method) => {
    const labels: Record<KnownAuthMethod, string> = {
      password: t("authMethodPassword"),
      "private-key": t("authMethodPrivateKey"),
      "private-key-password": t("authMethodPrivateKeyPassword"),
      agent: t("authMethodAgent"),
      auto: t("authMethodAuto"),
      none: t("authMethodNone"),
    };
    return labels[method];
  }));
  // 容器分屏不跨连接存续：切到 RDP（分屏不再适用）或主会话断开时收起，
  // 避免留下一个不再刷新的容器快照；下次打开回到默认比例。
  watch([isRdpConnection, connected], ([rdp, online]) => {
    if (dockerPaneOpen.value && (rdp || !online)) late.closeDockerPane();
  });
  // 把手提示按轴向：纵向拖的是高度、横向拖的是宽度（与 onDividerKeydown 的
  // 方向键映射一致）。
  const dividerResizeHint = computed(() =>
    paneLayout.value.direction === "column" ? t("dividerResizeHintHeight") : t("dividerResizeHint"),
  );
  // 搜索防抖与过滤态：列表排序/列布局/虚拟滚动器保留供全局列设置与偏好兼容，
  // 单栏树只消费搜索草稿、清空入口与"过滤是否生效"。
  const {
    sftpSearchDraft,
    clearSftpSearch,
    sftpFiltersActive,
  } = useSftpListLayout({
    entries, sort, visibleColumns, sftpColumnWidths, sftpNameWidth,
    sftpSearch, sftpTypeFilter, sftpShowHidden,
    session, sudoMode,
    loadDirectory: (path?: string) => loadDirectory(path),
    persistState,
  });

  // 文件传输占用统一语义：ZMODEM 或 trzsz 任一持有终端流即视为 busy。
  const terminalTransferBusy = computed(() => late.zmodemBusy.value || late.trzszBusy.value);

  // 跨重启持久化（布局偏好档 / UI 镜像 / 宿主 workbenchState 桥）收口在
  // composables/useWorkbenchPersistence；布局 refs 与会话仍是 App 的权威态，
  // 终端序列号是 `let`，经 getter/setter 注入。
  const {
    initialState,
    applyWorkbenchLayoutPrefs,
    restoreUiState,
    writeWorkbenchState,
    persistThrottle,
    persistState: persistWorkbenchState,
  } = useWorkbenchPersistence({
    hostContext,
    session,
    currentPath,
    splitRatio,
    paneOrder,
    paneDirection,
    sftpPaneOpen,
    followDirectory,
    sudoMode,
    canWrite,
    visibleColumns,
    sftpColumnWidths,
    sftpNameWidth,
    sftpShowHidden,
    sort,
    panelSurface,
    rdpPersistedSize,
    lastSequence: () => M.lastSequence,
    setLastSequence: (value) => { M.lastSequence = value; },
    loadDirectoryFollowPref: () => loadDirectoryFollowPref(),
  });

  // 早于本段声明点的 setup 期调用方（useRdpWorkbench / useSftpListLayout 按值取用
  // persistState）依赖函数提升，保留同名薄包装转发到 composable 实例。
  function persistState() {
    persistWorkbenchState();
  }

  // —— 自定义 tooltip（对齐 DBX 宿主的气泡提示）：收口在 composables/useTerminalTooltip ——

  return {
    outputGate, terminalWriteThrottle, terminalSyncOutput, writeTerminalOutput, dispatchTerminalOutput, handleBinary, disposeOutputPipeline, t,
    commandMarker, commandMarkerElapsed, commandMarkerDetails, resetCommandMarker, startCommandMarkerTick, stopCommandMarkerTick, applyCommandMarker, pasteConfirm,
    copyTextToClipboard, clipboardDeps, copyTerminalSelection, pasteTerminal, interceptTerminalPaste, sendConfirmedPaste, confirmRiskyPaste, resolvePasteConfirm,
    selectAllTerminal, clearTerminal, rdpCertPrompt, rdpCertRemember, rdpCertRemaining, startRdpSession, closeRdpSession, sendRdpInput,
    sendRdpClipboard, reconnectRdpSession, resolveRdpCertificate, rdpPasteOpen, rdpPasteText, rdpTypingBusy, rdpTypingSent, rdpTypingTotal,
    rdpWriteRemoteClipboard, rdpTypeAsKeystrokes, RDP_RESOLUTION_PRESETS, rdpResolutionOpen, rdpCustomWidth, rdpCustomHeight, rdpResizing, rdpDesktopSize,
    rdpPersistedSize, rdpResolutionLabel, isRdpResolutionActive, rdpWindowProportionPreset, applyRdpResolution, applyRdpCustomResolution, routeRdpConnection, connectViaRdpDialog,
    startRdpFromConnection, rdpClosedTitle, vncSession, vncDialogOpen, vncState, vncError, vncScaleMode, vncSurface,
    isVncMode, vncTarget, markVncClosed, startVncSession, closeVncSession, sendVncInput, sendVncClipboard, handleVncFrame,
    reattachVncSession, startVncFromConnection, terminalLayout, shellTabSessions, activeTerminalPaneId, shellTabCwd, activePaneCwd, openTerminalSplit,
    splitTerminalPane, closeTerminalSplit, closePane, createPaneTab, activatePaneTab, closePaneTab, onShellTabOpened, resetTerminalLayout,
    canSplitTerminal, terminalHasSplit, splitTerminalFont, onShellTabCwd, onPaneActivated, syncSftpToActivePane, canOpenShellTab, sessionStatus,
    sessionPillText, terminalErrorDetail, terminalErrorFriendly, commandOutputText, connectionIdentity, connectionAuthMethodLabel, dividerResizeHint, sftpSearchDraft,
    clearSftpSearch, sftpFiltersActive, terminalTransferBusy, initialState, applyWorkbenchLayoutPrefs, restoreUiState, writeWorkbenchState, persistThrottle,
    persistWorkbenchState, persistState,
  };
}

export type InstallWorkbenchSessionIoResult = ReturnType<typeof installWorkbenchSessionIo>;
