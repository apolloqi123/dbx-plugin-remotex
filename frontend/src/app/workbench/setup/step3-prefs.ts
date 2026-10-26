/**
 * Command history, quick commands, suggestions, settings preferences and the local/Telnet/serial session shells.
 */
import SettingsDialog from "../../../features/settings/components/SettingsDialog.vue";
import { Osc633CommandParser } from "../../../features/terminal/lib/panels/terminalCommandMarkers";
import { Osc7DirectoryParser } from "../../../features/terminal/lib/emulator/terminalDirectoryTracking";
import { PromptNewlineTransformer } from "../../../features/terminal/lib/emulator/terminalPromptNewline";
import { WebglAddon } from "@xterm/addon-webgl";
import { computed, ref } from "vue";
import { settingsPreferences } from "../../../features/settings/lib/settingsPreferences";
import { useCommandSuggestions } from "../../../features/completion/composables/useCommandSuggestions";
import { useConfirmDialogHost } from "../../../shared/lib/confirmDialog";
import { useHistoryPanel } from "../../../features/terminal/composables/ui/useHistoryPanel";
import { useLocalShellPreferences } from "../../../features/terminal/composables/session/useLocalShellPreferences";
import { useLocalShellSession } from "../../../features/terminal/composables/session/useLocalShellSession";
import { useSerialUpload } from "../../../features/serial/composables/useSerialUpload";
import { useSerialWorkbench } from "../../../features/serial/composables/useSerialWorkbench";
import { useTelnetWorkbench } from "../../../features/telnet/composables/useTelnetWorkbench";
import { useWallpaper } from "../../../features/terminal/composables/ui/useWallpaper";
import { useWorkbenchPrefs } from "../composables/useWorkbenchPrefs";
import { useWorkbenchToolbar } from "../composables/useWorkbenchToolbar";
import type { DiskUsage } from "../types";
import type { SetupState } from "./setupState";
import type { InstallWorkbenchCoreResult } from "./step1-core";
import type { InstallWorkbenchTerminalResult } from "./step2-terminal";
type Deps = InstallWorkbenchCoreResult & InstallWorkbenchTerminalResult & { late: Record<string, any>; state: SetupState };

export function installWorkbenchPrefs(deps: Deps) {
  const {
  TRANSFER_CONCURRENCY_KEY, TRANSFER_DOWNLOAD_LIMIT_KEY, TRANSFER_DUPLICATE_KEY, TRANSFER_MAX_ACTIVE_KEY, appearance, commandHistory,
  commandHistoryMeta, commandHistoryTimes, commandRunning, connection, connectionId, downloadConflictState,
  downloadDirState, downloadUseDefaultState, fitTerminalDimensions, isRdpMode, loadRemoteShellHistory, loadTransferConcurrency,
  loadTransferDownloadLimit, loadTransferDuplicatePolicy, loadTransferMaxActive, localDownloadDir, persistCommandHistory, persistTransferConcurrency,
  persistTransferDownloadLimit, persistTransferDuplicatePolicy, persistTransferMaxActive, pushTerminalCommandHistory, quickCommands, readEchoTextBeforeCursor,
  runtimeEndpoint, scheduleFit, searchOpen, sendTerminalBytes, serialInputQueue, session,
  sessionMenuOpen, sessionMenuShellOpen, sessionMenuShellPickOpen, settledTerminalDimensions, sftpCompatModeState, sftpNameEncodingState,
  sftpPaneOpen, sftpShowHidden, showError, suggestionMaxCharsState, suggestionMinCharsState, suggestionsEnabledState,
  terminal, terminalAppearance, terminalCursorPosition, terminalCwd, terminalError, terminalHost,
  terminalMenuOpen, terminalState, trackPendingInput, transferCompressModeState, transferCompressThresholdState, transferConcurrencyState,
  transferDownloadLimitState, transferDuplicateState, transferMaxActiveState, webglRecoveryOptions, webglRenderer, workbenchId,
  } = deps;
  const { late } = deps;
  const M = deps.state;

  const {
    suggestionOpen,
    suggestionItems,
    suggestionActiveIndex,
    suggestionAnchor,
    suggestionQuery,
    suggestionViewport,
    completionOpen,
    completionItems,
    completionActiveIndex,
    completionAnchor,
    completionLoading,
    closeSuggestions,
    handleSuggestionKey,
    handleCompletionKey,
    refreshSuggestionsAfterInput,
    refreshCompletionMenu,
    openCompletionsManually,
    sniffTerminalShell,
    replaceTerminalLineWith,
    fillSuggestion,
    acceptCompletionRow,
    syncSuggestionAnchorsOnSettle,
    readTerminalCellFrame,
    readTerminalSuggestionAnchor,
    completionController,
    lastTerminalCommand,
    resetSuggestionsForSession,
    ignoreSuggestion,
  } = useCommandSuggestions({
    terminal: () => terminal.value,
    getTerminalHost: () => terminalHost.value,
    sendTerminalBytes,
    getPendingTerminalInput: () => M.pendingTerminalInput,
    setPendingTerminalInput: (value) => {
      M.pendingTerminalInput = value;
      // 同 trackPendingInput：整行替换（回填/接受）也改变行空态。
      late.promptHintsLineEmpty.value = value === "";
    },
    commandRunning,
    isTerminalTransferBusy: () => late.terminalTransferBusy.value,
    terminalCwd, commandHistory, quickCommands,
    suggestionsEnabledState, suggestionMinCharsState, suggestionMaxCharsState,
    session,
    getSessionId: () => session.value?.sessionId ?? localSession.value?.sessionId ?? serialSession.value?.sessionId ?? "",
    getLocalSessionId: () => localSession.value?.sessionId,
    persistCommandHistory,
    isHistoryPanelOpen: () => historyPanelOpen.value,
    // 历史面板接线在下方 useHistoryPanel：惰性转发，规避声明顺序。
    updateHistoryPanelFilter: (syncLine?: boolean) => updateHistoryPanelFilter(syncLine),
    pushTerminalCommandHistory,
    syncHistoryPanelAnchor: () => { historyPanelAnchor.value = readTerminalSuggestionAnchor(); },
    // 数据分工（#138 批 2）：ghost 开启时前缀延伸命中让位行内 ghost，模糊
    // 浮层不弹。ghostEnabled 在下方 useGhostSuggest 才创建，惰性求值闭包
    // 仅在 onData 期调用，无初始化顺序问题。
    inlineGhostActive: () => late.ghostEnabled.value,
    // 终端交互提示待答（issue #150 及其反馈扩展）：询问输入/选择处不开建议/补全。
    isInteractivePromptPending: () => late.interactivePromptGatePending(),
  });

  // —— 快速命令数据面（M32-A3）：RPC 全部留在 App，编辑器/导入视图在
  // QuickCommandsSection（设置·终端），经 SettingsDialog 上抛意图。 ——

  // reka Select 不接受空串 option value（空串 = 未选中占位）；空值选项用哨兵值双向映射。
  const SELECT_EMPTY_SENTINEL = "__empty__";
  const diskUsage = ref<DiskUsage>();
  // Warp 式 history 面板（↑ 唤起）：状态/过滤/键位消费收口在
  // composables/useHistoryPanel；这里做数据与浮层互斥接线。
  const {
    historyPanelOpen,
    historyPanelEnabled,
    setHistoryPanelEnabled,
    historyPanelFocusSearch,
    historyPanelQuery,
    historyPanelEntries,
    historyPanelActiveIndex,
    historyPanelAnchor,
    historyPanelGateOpen,
    openHistoryPanel,
    closeHistoryPanel,
    updateHistoryPanelFilter,
    selectHistoryEntry,
    handleHistoryPanelKey,
    handleHistoryPanelPanelKey,
  } = useHistoryPanel({
    terminal: () => terminal.value,
    commandHistory: () => commandHistory.value,
    commandHistoryTimes: () => commandHistoryTimes.value,
    commandHistoryMeta: () => commandHistoryMeta.value,
    getPendingTerminalInput: () => M.pendingTerminalInput,
    getSessionId: () => session.value?.sessionId,
    isLocalMode: () => isLocalMode.value,
    commandRunning: () => commandRunning.value,
    terminalTransferBusy: () => late.terminalTransferBusy.value,
    completionOpen: () => completionOpen.value,
    suggestionOpen: () => suggestionOpen.value,
    quickSelectOpen: () => late.quickSelectOpen.value,
    searchOpen: () => searchOpen.value,
    interactivePromptPending: () => late.interactivePromptGatePending(),
    terminalMenuOpen,
    loadRemoteShellHistory,
    replaceTerminalLineWith,
    readEchoTextBeforeCursor,
    readTerminalSuggestionAnchor,
    hideGhostSuggestion: () => late.hideGhostSuggestion(),
    closeSuggestions,
    dismissPromptHints: () => late.dismissPromptHints(),
  });
  // 终端拖入文件的落点询问：null 表示取消；"cwd" 用解析后的 shell/SFTP 当前
  // 目录（resolveDropTargetDir：终端 cwd 跟随 → SFTP home → 面板当前目录），
  // 弹窗展示解析结果；{ dir } 是用户输入的目标目录（文件原名落其下）。
  // 设置弹窗（独立组件 SettingsDialog）：实例 ref 用于 Esc 内联分层消费与
  // 下载草稿回填；下载偏好权威态在本组件，经适配器交给组件读写。
  const settingsDialog = ref<InstanceType<typeof SettingsDialog>>();
  // 下载/传输/命令建议/SFTP 兼容/压缩传输偏好与 sidecar preferences 水合收口在
  // composables/useWorkbenchPrefs；本组状态在早期声明（设置弹窗适配器直接引用
  // load/persist 函数），后段偏好域经 accessor 惰性接入。
  const {
    loadDownloadDir,
    persistDownloadDir,
    loadDownloadUseDefaultDir,
    persistDownloadUseDefaultDir,
    loadDownloadConflictPolicy,
    persistDownloadConflictPolicy,
    loadSftpCompatMode,
    persistSftpCompatMode,
    loadSftpNameEncoding,
    persistSftpNameEncoding,
    loadTransferCompressMode,
    persistTransferCompressMode,
    loadTransferCompressThreshold,
    persistTransferCompressThreshold,
    loadSuggestionsEnabled,
    persistSuggestionsEnabled,
    loadSuggestionMinChars,
    persistSuggestionMinChars,
    loadSuggestionMaxChars,
    persistSuggestionMaxChars,
    syncPrefs: syncPrefsNow,
    hydratePrefs,
    downloadPrompt,
    resolveDownloadPrompt,
    askDownloadTarget,
    applyChosenDirAsDefault,
    folderPickerTarget,
    onFolderPicked,
    downloadConflictPrompt,
    askDownloadConflict,
    resolveDownloadConflict,
    resolveDownloadConflictFor,
  } = useWorkbenchPrefs({
    downloadDirState, downloadUseDefaultState, downloadConflictState,
    transferConcurrencyState, transferMaxActiveState, transferDownloadLimitState, transferDuplicateState,
    sftpCompatModeState, sftpNameEncodingState, transferCompressModeState, transferCompressThresholdState,
    suggestionsEnabledState, suggestionMinCharsState, suggestionMaxCharsState,
    transferKeys: {
      concurrency: TRANSFER_CONCURRENCY_KEY,
      maxActive: TRANSFER_MAX_ACTIVE_KEY,
      downloadLimit: TRANSFER_DOWNLOAD_LIMIT_KEY,
      duplicate: TRANSFER_DUPLICATE_KEY,
    },
    getLocalShell: () => localShellPref.value,
    setLocalShell: (value) => { localShellPref.value = value; },
    getLocalShellIntegration: () => localShellIntegrationPref.value,
    setLocalShellIntegration: (value) => { localShellIntegrationPref.value = value; },
    getLocalShellCwd: () => localShellCwdPref.value,
    setLocalShellCwd: (value) => { localShellCwdPref.value = value; },
    setLocalShellEnv: (value) => { localShellEnvPref.value = value; },
    setActionLinksSettings: (value) => { late.actionLinksSettings.value = value; },
    setGutterSettings: (value) => { late.gutterSettings.value = value; },
    setWallpaperEnabled: (value) => { wallpaperEnabled.value = value; },
    setWallpaperOpacity: (value) => { wallpaperOpacity.value = value; },
    loadWallpaperImage: () => loadWallpaperImage(),
    setCtxSearchEngines: (text) => { late.ctxSearchEnginesText.value = text; },
    settingsDialog,
    getLocalDownloadDir: () => localDownloadDir.value,
  });
  M.syncPrefsImpl = syncPrefsNow;
  const downloadPrefsAdapter = {
    loadDir: loadDownloadDir,
    loadUseDefault: loadDownloadUseDefaultDir,
    loadConflict: loadDownloadConflictPolicy,
    persistDir: persistDownloadDir,
    persistUseDefault: persistDownloadUseDefaultDir,
    persistConflict: persistDownloadConflictPolicy,
  };
  // 传输并发/重复策略与命令建议的读写适配器（权威态在本组件，同下载偏好）。
  const transferPrefsAdapter = {
    loadConcurrency: loadTransferConcurrency,
    loadDuplicatePolicy: loadTransferDuplicatePolicy,
    persistConcurrency: persistTransferConcurrency,
    persistDuplicatePolicy: persistTransferDuplicatePolicy,
    loadMaxActive: loadTransferMaxActive,
    persistMaxActive: persistTransferMaxActive,
    loadDownloadLimit: loadTransferDownloadLimit,
    persistDownloadLimit: persistTransferDownloadLimit,
    loadCompatMode: loadSftpCompatMode,
    persistCompatMode: persistSftpCompatMode,
    loadNameEncoding: loadSftpNameEncoding,
    persistNameEncoding: persistSftpNameEncoding,
    loadCompressMode: loadTransferCompressMode,
    persistCompressMode: persistTransferCompressMode,
    loadCompressThreshold: loadTransferCompressThreshold,
    persistCompressThreshold: persistTransferCompressThreshold,
  };
  const suggestionPrefsAdapter = {
    loadEnabled: loadSuggestionsEnabled,
    loadMinChars: loadSuggestionMinChars,
    loadMaxChars: loadSuggestionMaxChars,
    persistEnabled: persistSuggestionsEnabled,
    persistMinChars: persistSuggestionMinChars,
    persistMaxChars: persistSuggestionMaxChars,
  };
  // 终端 WebGL 渲染加速：偏好权威 = settingsPreferences store（与设置弹窗
  // 共享同一 ref + 唯一落盘点），默认开；WebGL 不可用（headless/无 context）
  // 时静默回退 DOM 渲染。只有主终端长期挂 renderer；回放弹窗保持 DOM 渲染，
  // GIF 导出在导出期间给离屏终端临时挂载（取像素依赖 canvas），导出完随终端
  // dispose 释放 context。
  const webglEnabled = settingsPreferences.webglEnabled;
  // 背景图（图源/开关/透明度 + 生效期强制 DOM 渲染器）收口在
  // composables/useWallpaper；WebGL renderer 引用仍由本域持有并经 getter 注入。
  const {
    wallpaperEnabled,
    wallpaperOpacity,
    wallpaperDataUrl,
    wallpaperSessionOnly,
    wallpaperActive,
    loadWallpaperImage,
    setWallpaperImage,
    clearWallpaperImage,
    updateWallpaperEnabled,
    updateWallpaperOpacity,
  } = useWallpaper({
    webglEnabled,
    getTerminal: () => terminal.value,
    getWebglRenderer: () => webglRenderer.value,
    setWebglRenderer: (renderer) => { webglRenderer.value = renderer; },
    createWebglAddon: () => new WebglAddon(),
    webglRecoveryOptions,
    persistPrefs: (patch) => late.persistTerminalFeaturePrefs(patch),
  });
  // True while attachSession sits inside its bounded backoff loop; turns the
  // status pill and overlay into the dedicated "reconnecting" phase.
  /** 显示/隐藏文件切换进全局布局偏好（issue #178），重启不再回落默认隐藏。 */
  function toggleSftpShowHidden() {
    sftpShowHidden.value = !sftpShowHidden.value;
    late.persistState();
  }

  // SetUserVar cwd 元数据（优先通道）的最新上报；null 表示通道未启用，OSC 7
  // 提示符通道照旧跟随（回落语义不变）。裁决规则见 shouldOsc7FollowOverrideUserVarCwd。

  // 通用应用内确认弹窗（SSH-H1）：原语在 lib/confirmDialog（SettingsDialog/
  // QuickCommandsSection 等子组件共用），此处只挂宿主渲染；模板/关框/Esc/
  // teardown 统一经 resolvePendingConfirmDialog 结算。
  const { pendingConfirmDialog, resolveConfirmDialog: resolvePendingConfirmDialog } = useConfirmDialogHost();
  /** 响铃视觉提示的短暂高亮（xterm 6.x 无 bellStyle，须自行实现）。 */
  // 宿主订阅/拖放注销句柄随工作台启动桥迁入 useWorkbenchBootstrap。



  // Consecutive replays that returned complete without filling the detected
  // sequence hole. A sidecar that keeps reporting complete on a gap that never
  // closes cannot self-heal by retrying — after a few attempts the drain must
  // resync past the hole instead of spinning the replay loop forever.



  const pendingTerminalFrames = new Map<number, { stream: number; data: Uint8Array }>();
  const directoryParser = new Osc7DirectoryParser();
  // 提示符换行修复（t8y2/dbx#10750）：sidecar 给每个交互 shell 会话武装了
  // 按提示符发 OSC 7 的钩子，这里把帧当作"提示符即将绘制"的信号——可见输出
  // 不在行首就在帧前补 CRLF。字节级透传，无钩子流恒等直通。
  const promptNewline = new PromptNewlineTransformer();
  // OSC 633 shell-integration markers (pure frontend parse; no-op streams pass through).
  const commandMarkerParser = new Osc633CommandParser();

  // —— 本地终端 / Telnet / 串口：会话状态机与生命周期（启停/互斥确认/恢复外壳/
  // 重新接回/连接驱动直启）收口在各自 composable。模板同名绑定经解构透出；
  // 帧流（useProtocolFrameBuffers）、偏好（useLocalShellPreferences）、其他协议
  // 会话与 xterm 实例均在本段之后声明/装配，故经 getter 惰性接入。
  const {
    localSession, localState, localExitCode, localOpenConfirmOpen, isLocalMode, localShellRestored,
    localLastCwd, localRecentCommands,
    startLocalTerminal, closeLocalTerminal, dismissRestoredLocalShell, toggleLocalTerminal,
    rerunLocalCommand, restartLocalTerminal, requestLocalTerminal, confirmLocalTerminal,
    reattachLocalSession,
  } = useLocalShellSession({
    t: (key) => late.t(key),
    showError,
    terminalState,
    terminalError,
    session,
    sftpPaneOpen,
    isDisposed: () => M.disposed,
    getTerminal: () => terminal.value,
    getWorkbenchId: () => workbenchId,
    closeRightPanel: () => late.closeRightPanel(),
    getBatchBarOpen: () => late.batchBarOpen,
    getSessionMenus: () => ({ sessionMenuOpen, sessionMenuShellOpen, sessionMenuShellPickOpen }),
    getLocalShellPrefs: () => ({ localShellPref, localShellIntegrationPref, localShellCwdPref, localShellEnvClean }),
    getLocalFrameStreams: () => ({ localLastSequence: late.localLastSequence, localPendingFrames: late.localPendingFrames, localReplayNoProgress: late.localReplayNoProgress, requestLocalReplay: late.requestLocalReplay }),
    resetCommandMarker: () => late.resetCommandMarker(),
    getReconnectPending: () => late.reconnectPending,
    getOtherModes: () => ({ isRdpMode, closeRdpSession: late.closeRdpSession, isVncMode: late.isVncMode, closeVncSession: late.closeVncSession, isSerialMode, closeSerialSession, isTelnetMode, closeTelnetSession }),
    settledTerminalDimensions,
    terminalCursorPosition,
    scheduleFit,
    trackPendingInput,
    sendConfirmedPaste: (payload) => late.sendConfirmedPaste(payload),
    // SSH 会话主干在 useSshConnection（后段装配），惰性转发避免 setup 期 TDZ。
    closeSession: () => late.closeSession(),
  });
  // Telnet 会话（P2-3）：互斥展示语义与状态机收口在 composables/useTelnetWorkbench。
  const {
    telnetSession, telnetDialogOpen, telnetState, telnetError, isTelnetMode, telnetTarget,
    startTelnetSession, closeTelnetSession, startTelnetFromConnection,
  } = useTelnetWorkbench({
    showError,
    isDisposed: () => M.disposed,
    getTerminal: () => terminal.value,
    getConnectionId: () => connectionId,
    getWorkbenchId: () => workbenchId,
    getRuntimeEndpoint: () => runtimeEndpoint,
    getConnection: () => connection,
    getTelnetFrameStreams: () => ({ telnetLastSequence: late.telnetLastSequence, telnetPendingFrames: late.telnetPendingFrames, telnetReplayNoProgress: late.telnetReplayNoProgress }),
    fitTerminalDimensions,
    scheduleFit,
  });
  // 串口会话（P3 + B1 增强）：二进制写通道/能力探测与状态机收口在
  // composables/useSerialWorkbench。
  const {
    serialSession, serialDialogOpen, serialState, serialError, serialBinaryInput,
    isSerialMode, serialTarget, startSerialSession, closeSerialSession, startSerialFromConnection,
  } = useSerialWorkbench({
    showError,
    isDisposed: () => M.disposed,
    getTerminal: () => terminal.value,
    localShellRestored,
    getWorkbenchId: () => workbenchId,
    getConnection: () => connection,
    getSerialFrameStreams: () => ({ serialLastSequence: late.serialLastSequence, serialPendingFrames: late.serialPendingFrames, serialUnknownStreamFrames: late.serialUnknownStreamFrames }),
    getSerialInputQueue: () => serialInputQueue,
    cancelSerialUploadForSessionClose: (sessionId) => cancelSerialUploadForSessionClose(sessionId),
    scheduleFit,
  });
  // 串口文件上传 overlay 状态：进度由 sidecar 的 serial/upload/progress 事件
  // 驱动；running 期间吞掉键入（协议控制字符窗口）且禁止并发第二次 upload。
  // 状态机/送数/取消收口在 composables/useSerialUpload（会话关闭经
  // cancelForSessionClose 静默复位）。
  const {
    serialUpload,
    serialUploadDialogOpen,
    serialUploadBusy,
    serialUploadOverlayVisible,
    serialUploadPercentValue,
    serialUploadStatusLabel,
    startSerialUpload,
    cancelSerialUpload,
    cancelForSessionClose: cancelSerialUploadForSessionClose,
  } = useSerialUpload({
    t: (key, values) => late.t(key, values),
    showError,
    getSessionId: () => serialSession.value?.sessionId,
  });
  // 本地/非 SSH 会话激活期（本地/Telnet/串口/VNC/RDP）：SSH 专属能力整体隐藏。
  // 依赖 useLocalShellSession/useTelnetWorkbench/useSerialWorkbench/useVncSession
  // 的会话态（本段之后装配），因此不进 state/useSessionState。
  const localUiMode = computed(() => isLocalMode.value || localShellRestored.value || isTelnetMode.value || isSerialMode.value || late.isVncMode.value || isRdpMode.value);
  // —— 本地终端偏好/shell 发现/选择器：收口在 composables/useLocalShellPreferences ——
  // （本地终端进程本身仍由本文件 start/close，牵动输出管线状态。）
  const {
    localShellPref,
    localShellIntegrationPref,
    localShellCwdPref,
    localShellEnvPref,
    localShellEnvClean,
    localShells,
    localShellsLoading,
    selectedShellInjectable,
    localShellSurfaceOpen,
    dockConnections,
    pickLocalShell,
    openSessionShellPick,
    openLocalShellPrefs,
    openLocalShellSurfaceMenu,
    openConnectionSurface,
    openLocalShellSurface,
    setLocalShellPref,
    setLocalShellIntegrationPref,
    setLocalShellCwd,
    setLocalShellEnv,
    addLocalShellEnvRow,
    removeLocalShellEnvRow,
    canOpenLocalTab,
    openLocalTerminalTab,
  } = useLocalShellPreferences({
    isLocalMode: () => isLocalMode.value,
    isLocalShellRestored: () => localShellRestored.value,
    restartLocalTerminal: (shellOverride) => { void restartLocalTerminal(shellOverride); },
    requestLocalTerminal: (shellOverride) => requestLocalTerminal(shellOverride),
    // 会话下拉状态在本段之后声明，惰性写入。
    closeSessionMenu: () => { sessionMenuOpen.value = false; },
    setShellPickOpen: (open) => { sessionMenuShellPickOpen.value = open; },
  });
  // Dock 面板配色快切 + 工具条隐藏偏好：收口在 composables/useWorkbenchToolbar。
  const {
    panelPaletteOpen,
    panelToolbarHidden,
    togglePanelToolbar,
    panelSchemeChoices,
    panelActiveSchemeId,
    pickPanelScheme,
  } = useWorkbenchToolbar({
    appearanceColorScheme: () => appearance.value.colorScheme,
    terminalAppearance: () => terminalAppearance.value,
    // 惰性转发：updateTerminalAppearance 在本段之后才由 useTerminalAppearance 解构。
    updateAppearance: (patch) => late.updateTerminalAppearance(patch),
  });

  // —— 输出管线：大输出背压门/合帧写节流、OSC 7/633 解析链、AI 修复输出采集、
  // 提示符换行修复、zmodem/trzsz 流分派、SSH 帧重组与宿主二进制事件分派收口在
  // features/terminal/composables/session/useTerminalWriter。装饰引擎与传输占用/
  // 断线梯子在本段之后装配，一律经 getter 惰性接入；SSH 帧序号与 pending 缓存
  // 仍归 App（useSshConnection/持久化/transport watch 同源读写），经 frameStream
  // 的 getter/setter 存取。

  return {
    suggestionOpen, suggestionItems, suggestionActiveIndex, suggestionAnchor, suggestionQuery, suggestionViewport, completionOpen, completionItems,
    completionActiveIndex, completionAnchor, completionLoading, closeSuggestions, handleSuggestionKey, handleCompletionKey, refreshSuggestionsAfterInput, refreshCompletionMenu,
    openCompletionsManually, sniffTerminalShell, replaceTerminalLineWith, fillSuggestion, acceptCompletionRow, syncSuggestionAnchorsOnSettle, readTerminalCellFrame, readTerminalSuggestionAnchor,
    completionController, lastTerminalCommand, resetSuggestionsForSession, ignoreSuggestion, SELECT_EMPTY_SENTINEL, diskUsage, historyPanelOpen, historyPanelEnabled,
    setHistoryPanelEnabled, historyPanelFocusSearch, historyPanelQuery, historyPanelEntries, historyPanelActiveIndex, historyPanelAnchor, historyPanelGateOpen, openHistoryPanel,
    closeHistoryPanel, updateHistoryPanelFilter, selectHistoryEntry, handleHistoryPanelKey, handleHistoryPanelPanelKey, settingsDialog, loadDownloadDir, persistDownloadDir,
    loadDownloadUseDefaultDir, persistDownloadUseDefaultDir, loadDownloadConflictPolicy, persistDownloadConflictPolicy, loadSftpCompatMode, persistSftpCompatMode, loadSftpNameEncoding, persistSftpNameEncoding,
    loadTransferCompressMode, persistTransferCompressMode, loadTransferCompressThreshold, persistTransferCompressThreshold, loadSuggestionsEnabled, persistSuggestionsEnabled, loadSuggestionMinChars, persistSuggestionMinChars,
    loadSuggestionMaxChars, persistSuggestionMaxChars, syncPrefsNow, hydratePrefs, downloadPrompt, resolveDownloadPrompt, askDownloadTarget, applyChosenDirAsDefault,
    folderPickerTarget, onFolderPicked, downloadConflictPrompt, askDownloadConflict, resolveDownloadConflict, resolveDownloadConflictFor, downloadPrefsAdapter, transferPrefsAdapter,
    suggestionPrefsAdapter, webglEnabled, wallpaperEnabled, wallpaperOpacity, wallpaperDataUrl, wallpaperSessionOnly, wallpaperActive, loadWallpaperImage,
    setWallpaperImage, clearWallpaperImage, updateWallpaperEnabled, updateWallpaperOpacity, toggleSftpShowHidden, pendingConfirmDialog, resolvePendingConfirmDialog, pendingTerminalFrames,
    directoryParser, promptNewline, commandMarkerParser, localSession, localState, localExitCode, localOpenConfirmOpen, isLocalMode,
    localShellRestored, localLastCwd, localRecentCommands, startLocalTerminal, closeLocalTerminal, dismissRestoredLocalShell, toggleLocalTerminal, rerunLocalCommand,
    restartLocalTerminal, requestLocalTerminal, confirmLocalTerminal, reattachLocalSession, telnetSession, telnetDialogOpen, telnetState, telnetError,
    isTelnetMode, telnetTarget, startTelnetSession, closeTelnetSession, startTelnetFromConnection, serialSession, serialDialogOpen, serialState,
    serialError, serialBinaryInput, isSerialMode, serialTarget, startSerialSession, closeSerialSession, startSerialFromConnection, serialUpload,
    serialUploadDialogOpen, serialUploadBusy, serialUploadOverlayVisible, serialUploadPercentValue, serialUploadStatusLabel, startSerialUpload, cancelSerialUpload, cancelSerialUploadForSessionClose,
    localUiMode, localShellPref, localShellIntegrationPref, localShellCwdPref, localShellEnvPref, localShellEnvClean, localShells, localShellsLoading,
    selectedShellInjectable, localShellSurfaceOpen, dockConnections, pickLocalShell, openSessionShellPick, openLocalShellPrefs, openLocalShellSurfaceMenu, openConnectionSurface,
    openLocalShellSurface, setLocalShellPref, setLocalShellIntegrationPref, setLocalShellCwd, setLocalShellEnv, addLocalShellEnvRow, removeLocalShellEnvRow, canOpenLocalTab,
    openLocalTerminalTab, panelPaletteOpen, panelToolbarHidden, togglePanelToolbar, panelSchemeChoices, panelActiveSchemeId, pickPanelScheme,
  };
}

export type InstallWorkbenchPrefsResult = ReturnType<typeof installWorkbenchPrefs>;
