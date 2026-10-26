/**
 * Tooltip, terminal appearance, font zoom, quick select, ghost suggestions and key routing.
 */
import { computed, watch } from "vue";
import { createCrossSurfaceStorageReader, subscribeStorageEvents } from "../lib/appearanceSync";
import { useAppearanceCrossSurfaceSync } from "../composables/useAppearanceCrossSurfaceSync";
import { useGhostSuggest } from "../../../features/completion/composables/useGhostSuggest";
import { useProtocolFrameBuffers } from "../../../shared/composables/useProtocolFrameBuffers";
import { useQuickSelect } from "../../../features/terminal/composables/ui/useQuickSelect";
import { useTerminalAppearance } from "../../../features/terminal/composables/ui/useTerminalAppearance";
import { useTerminalBell } from "../../../features/terminal/composables/ui/useTerminalBell";
import { useTerminalFontZoom } from "../../../features/terminal/composables/ui/useTerminalFontZoom";
import { useTerminalKeys } from "../../../features/terminal/composables/io/useTerminalKeys";
import { useTerminalMouse } from "../../../features/terminal/composables/ui/useTerminalMouse";
import { useTerminalTooltip } from "../../../features/terminal/composables/ui/useTerminalTooltip";
import { useTrzsz } from "../../../features/terminal/composables/ui/useTrzsz";
import { useZmodem } from "../../../features/terminal/composables/ui/useZmodem";
import type { SetupState } from "./setupState";
import type { InstallWorkbenchCoreResult } from "./step1-core";
import type { InstallWorkbenchTerminalResult } from "./step2-terminal";
import type { InstallWorkbenchPrefsResult } from "./step3-prefs";
import type { InstallWorkbenchSessionIoResult } from "./step4-sessions";
type Deps = InstallWorkbenchCoreResult & InstallWorkbenchTerminalResult & InstallWorkbenchPrefsResult & InstallWorkbenchSessionIoResult & { late: Record<string, any>; state: SetupState };

export function installWorkbenchAppearance(deps: Deps) {
  const {
  aiAssistFromTerminal, aiBridgeOk, aiFixBar, aiFixFromBar, aiSettingsState, appearance,
  applyChosenDirAsDefault, askDownloadTarget, canWrite, clearTerminal, clipboardDeps, closeHistoryPanel,
  closeTerminalSearch, commandHistory, commandRunning, completionController, completionOpen, connected,
  copyTerminalSelection, dispatchTerminalOutput, fileMenu, ghostKeycapMenuOpen, ghostTabAccept, handleCompletionKey,
  handleHistoryPanelKey, handleSuggestionKey, historyPanelEnabled, historyPanelGateOpen, historyPanelOpen, loadDirectory,
  loadDownloadDir, loadDownloadUseDefaultDir, localExitCode, localSession, localState, noticeActions,
  noticeOpen, openCompletionsManually, openHistoryPanel, openTerminalSearch, pasteTerminal, quickCommands,
  readTerminalCellFrame, resolveDownloadConflictFor, scheduleFit, searchOpen, selectAllTerminal, sendTerminalBytes,
  serialError, serialSession, serialState, session, showError, showNotice,
  stopCommandMarkerTick, suggestionMaxCharsState, suggestionMinCharsState, suggestionOpen, t, telnetError,
  telnetSession, telnetState, terminal, terminalAppearance, terminalBehavior, terminalCopyCache,
  terminalDiagVisible, terminalFontOverride, terminalFontSize, terminalHost, terminalHotkeys, terminalMenuOpen,
  terminalSyncOutput, terminalTransferBusy, terminalWriteThrottle, writeTerminalOutput, zmodemInput,
  } = deps;
  const { late } = deps;
  const M = deps.state;

  const { tooltip, tooltipBubble, hideTooltip, onTooltipOver, onTooltipOut, onTooltipFocusIn, onTooltipFocusOut } = useTerminalTooltip();

  // 宿主派生的终端主题（未启用配色方案时的最终结果）：DBX 面板色 + 内置 16 色
  // ANSI。作为「跟随宿主」基底，也是设置页预览的基准。
  // 终端外观/主题/字体跟随：收口在 composables/useTerminalAppearance。
  const {
    terminalTheme,
    hostTerminalTheme,
    applyTerminalPaddingVars,
    applyTerminalAppearance,
    updateTerminalAppearance,
    applyTerminalAppearanceTheme,
    saveTerminalAppearanceTheme,
    deleteTerminalAppearanceTheme,
    addImportedSchemes,
    removeImportedScheme,
    applyAppearance,
    registerOscColorQueryHandlers,
    hostTerminalFontFamily,
    hostFontObserver,
  } = useTerminalAppearance({
    t, showNotice, appearance, terminalAppearance, terminalFontOverride, terminalFontSize,
    terminalWriteThrottle, terminalSyncOutput,
    terminal: () => terminal.value,
    scheduleFit,
    setTerminalFont: (family, size) => setTerminalFont(family, size),
  });

  // 分屏终端主题：terminalTheme() 每次调用返回新对象，直接绑定会让分屏的
  // deep watcher 在主页面每次重渲染时都重设主题并重排；computed 按依赖缓存。
  const splitTerminalTheme = computed(() => terminalTheme());

  // 终端按键路由与浮层键权仲裁（热键派发/ghost 接受/Ctrl+F 分派/补全/建议/
  // 历史面板/Quick Select 消费顺序）收口在 io/useTerminalKeys.ts；
  // handleTerminalKey 由下方解构经 getKeyHandler 接入 xterm。

  // 终端响铃：视觉/听觉两态收口在 composables/useTerminalBell。
  const {
    terminalBellFlash,
    handleTerminalBell,
  } = useTerminalBell({ terminalBehavior });

  /**
 * 链接点击（对标 Tabby「Mouse → Require a key to click links」）：链接修饰键未按下
 * 时直接忽略，把点击还给下面的终端内容。
 *
 * 打开动作逐句复刻 addon 内置处理器（先开空窗拿句柄 → 清 opener → 改写 location），
 * 只是多了上面这道修饰键闸门：清 opener 是防「反向标签劫持」的关键，不能省。
 * 沙箱 iframe 未开 allow-popups 时 window.open 会返回 null，此时静默放弃。
 */
  // 终端鼠标交互：链接/中键粘贴/缩放滚轮/点击定位光标收口在 composables/useTerminalMouse。
  const {
    openTerminalLink,
    handleTerminalMiddleClick,
    handleTerminalWheel,
    handleTerminalMouseDown,
    handleTerminalMouseUp,
  } = useTerminalMouse({
    terminalBehavior, terminalMenuOpen,
    closeFileMenu: () => { fileMenu.value = undefined; },
    pasteTerminal,
    adjustTerminalZoom: (delta) => adjustTerminalZoom(delta),
    terminal: () => terminal.value,
    getTerminalHost: () => terminalHost.value,
    session,
    sendTerminalBytes,
    zmodemBusy: () => zmodemBusy.value,
    trzszBusy: () => trzszBusy.value,
  });

  // 终端缩放与字体设置落地：收口在 composables/useTerminalFontZoom。
  const {
    adjustTerminalZoom,
    resetTerminalZoom,
    setTerminalFont,
    applyTerminalFontSize,
    applyTerminalFontSettings,
  } = useTerminalFontZoom({
    t, showNotice, appearance, terminalAppearance, terminalFontOverride, terminalFontSize,
    terminal: () => terminal.value,
    scheduleFit,
    hostTerminalFontFamily,
  });

  // 跨表面终端外观同步（tab ↔ dock 底部栏）：两个表面是独立 webview，外观三键
  // 写穿宿主 storage 后对端无通知（桥无 storage 变更事件）——host 通道低频轮询
  // 直读桥、localStorage 档（浏览器直连）另有原生 storage 事件即时补拍；隐藏
  // 期间停轮询、复现即补。防「自写未落地被读回旧值回落」的双闸门在 lib 状态机，
  // 外观与字体写入分别经两条深度 watch 进闸。
  const appearanceCrossSurfaceSync = useAppearanceCrossSurfaceSync({
    reader: createCrossSurfaceStorageReader(),
    subscribe: subscribeStorageEvents,
    currentAppearance: () => terminalAppearance.value,
    currentFont: () => terminalFontOverride.value,
    adopt: (decision) => {
      // 只落地非空部分：字体未变时不重写终端字体（避免无谓的持久化回声）。
      if (decision.appearance) {
        terminalAppearance.value = decision.appearance;
        applyTerminalAppearance();
      }
      if (decision.font) setTerminalFont(decision.font.fontFamily, decision.font.fontSize);
    },
  });
  watch(terminalAppearance, () => appearanceCrossSurfaceSync.noteAppearanceWrite(), { deep: true });
  watch(terminalFontOverride, () => appearanceCrossSurfaceSync.noteFontWrite());

  // ---------------------------------------------------------------------------
  // Quick Select Mode（WT-1，对标 WezTerm Quick Select）：注册表动作唤起浮层，
  // 纯逻辑模块抽取可视区命中（正则/容量/去重见 lib/quickSelect.ts），这里只做
  // 状态与复制接线。复制链与选中复制一致（插件视图副本 + 剪贴板桥逐级降级）。
  // ---------------------------------------------------------------------------
  // Quick Select：命中抽取/浮层/按键消费收口在 composables/useQuickSelect。
  const {
    quickSelectOpen,
    quickSelectHits,
    quickSelectActive,
    openQuickSelect,
    closeQuickSelect,
    moveQuickSelectActive,
    copyQuickSelectHit,
    handleQuickSelectKey,
  } = useQuickSelect({
    t, showNotice, showError,
    terminal: () => terminal.value,
    terminalMenuOpen, terminalCopyCache, clipboardDeps,
  });

  // 浮层键权仲裁（claims 可见性/优先级表/键维度映射）随 useTerminalKeys 迁至
  // io/useTerminalKeys.ts；本篇只保留 AI 助手与行跟踪的解构接线。

  // AI 助手（Warp AI 对齐批）：设置/修复条/输出采集/推荐位在
  // composables/useAiAssistant；`#` 命令搜索状态机在 useTerminalAiSearch
  // （上面的 aiSearch）。三条体验都只「发送快照到宿主 AI 面板」——AI 生成内容
  // 一律不回写 PTY、不代执行（执行权在人）；插件零密钥，能力缺失全链路可见
  // 降级（aiBridgeOk 各调用点新鲜探测）。

  // 行改写追踪（EnterLineTracker）、回车行兜底采集与回显对照随
  // useTerminalInput 迁至 io/useTerminalInput.ts（captureEnterLine 的语义与
  // 门过滤仍在 lib/terminalEnterCapture.ts，行为不变）。

  // ---------------------------------------------------------------------------
  // 命令输入建议浮层（P1-1）：采集→抑制门→检索→定位→按键消费。
  // 采集走三条真实来源：① 命令弹窗/命令条执行（已入 commandHistory）；
  // ② OSC 633 shell-integration E 帧（applyCommandMarker）；③ 回车行兜底
  // 采集（trackPendingInput 的 \r 分支 → captureEnterLine，覆盖无 shell
  // integration 的 SSH/本地会话——远端没装 integration 时 E 帧不存在，
  // 手动键入的命令此前从不入史）。兜底路径的门过滤 + 回显对照（凭据类
  // 关回显输入绝不入史）在 lib/terminalEnterCapture.ts。Expect/OTP 自动
  // 应答由 sidecar 直接注入 PTY，与 onData 用户输入不同源，永不入历史。
  // ---------------------------------------------------------------------------

  /** 仅收历史建议浮层（不 dismiss 补全调度）：历史建议无匹配的回落分支用。
 *  worker 模式下本次键入的补全响应还在途——dismiss 会把 revision 顶掉，
 *  响应到达时被三重 guard 丢弃，表现为"敲错→退格→再输入，补全不再弹出"
 *  （每个键入都重复 dismiss → 永久失效，直到命中历史建议才恢复）。 */

  // ---------------------------------------------------------------------------
  // 终端行内 ghost 自动建议（对标 Warp/fish autosuggest）：门状态机与前缀扩展
  // 匹配在 lib/terminalGhostSuggest.ts，这里只做三件事——xterm buffer 行尾采样、
  // overlay 锚点计算、接受时向 PTY 注入剩余字节（等价用户键入，无协议改动）。
  // 渲染选 overlay DOM 而非 xterm decoration：ghost 逐键刷新，decoration 注册/
  // 销毁生命周期重；overlay 与既有 action-link-hint 同机制，零 buffer 侵入，
  // 不影响选区/搜索/屏幕阅读器。
  // ---------------------------------------------------------------------------

  /** 设置开关（SettingsDialog 自治持久化，经 update:ghost-suggest 即时上抛）。 */
  // ghost 行内建议：状态机/门/接受语义收口在 composables/useGhostSuggest。
  const {
    ghostEnabled,
    ghostMatch,
    ghostAnchor,
    setGhostEnabled,
    hideGhostSuggestion,
    resetGhostSuggestion,
    refreshGhostAfterInput,
    acceptGhostSuggestion,
    acceptGhostSuggestionWord,
    syncGhostAnchorOnSettle,
  } = useGhostSuggest({
    terminal: () => terminal.value,
    sendTerminalBytes,
    getPendingTerminalInput: () => M.pendingTerminalInput,
    appendToPendingTerminalInput: (data) => { M.pendingTerminalInput += data; },
    commandRunning, terminalTransferBusy, commandHistory, quickCommands,
    suggestionOpen, historyPanelOpen, completionController,
    suggestionMinCharsState, suggestionMaxCharsState,
    // 终端交互提示待答（issue #150 及其反馈扩展）：询问输入/选择处不出 ghost。
    isInteractivePromptPending: () => late.interactivePromptGatePending(),
    readTerminalCellFrame,
  });
  // 终端按键路由与浮层键权仲裁：热键派发/ghost 接受/Ctrl+F 分派/补全建议历史
  // 面板与 Quick Select 的消费顺序、按键维度仲裁表收口在 io/useTerminalKeys。
  // 状态均为本文件权威（Ref 直取），命令执行经回调注入；xterm 侧经上方
  // getKeyHandler 惰性取用（本段晚于实例装配）。
  const {
    handleTerminalKey,
  } = useTerminalKeys({
    terminalMenuOpen,
    hideGhostSuggestion,
    ghostMatch,
    ghostKeycapMenuOpen,
    suggestionOpen,
    completionOpen,
    commandRunning,
    terminalTransferBusy,
    acceptGhostSuggestionWord,
    acceptGhostSuggestion,
    dismissPromptHints: () => late.dismissPromptHints(),
    ghostTabAccept,
    openTerminalSearch,
    closeTerminalSearch,
    handleCompletionKey,
    handleSuggestionKey,
    historyPanelOpen,
    handleHistoryPanelKey,
    openHistoryPanel,
    closeHistoryPanel,
    historyPanelGateOpen,
    historyPanelEnabled,
    searchOpen,
    quickSelectOpen,
    handleQuickSelectKey,
    openQuickSelect,
    closeQuickSelect,
    closeGhostKeycapMenu: () => { ghostKeycapMenuOpen.value = false; },
    toggleTerminalDiag: () => { terminalDiagVisible.value = !terminalDiagVisible.value; },
    terminalHotkeys,
    openCompletionsManually,
    hasAiFixBar: () => !!aiFixBar.value,
    aiFixFromBar: () => aiFixFromBar(),
    aiBridgeOk: () => aiBridgeOk(),
    isAiAssistEnabled: () => aiSettingsState.value.assist,
    aiAssistFromTerminal: () => aiAssistFromTerminal(),
    showNotice,
    t: (key) => t(key),
    terminal: () => terminal.value,
    copyTerminalSelection: () => copyTerminalSelection(),
    selectAllTerminal: () => selectAllTerminal(),
    clearTerminal: () => clearTerminal(),
    adjustTerminalZoom: (delta) => adjustTerminalZoom(delta),
    resetTerminalZoom: () => resetTerminalZoom(),
    isPromptHintsVisible: () => late.promptHintsVisible.value,
  });

  // 主终端字节写入口（串口/Telnet/SSH·本地分派与队列入队）随 useTerminalInput
  // 迁至 io/useTerminalInput.ts；解构见上方 sendTerminalBytes。

  // 输出写入/dispatch 已迁至 useTerminalWriter（writeTerminalOutput /
  // dispatchTerminalOutput 由上方解构）；ZMODEM 占用期与 trzsz 过滤链顺序不变。

  // ZMODEM 上传/下载：sentry/检测/进度/落盘/收尾收口在 composables/useZmodem。
  const {
    zmodemState,
    zmodemSpeed,
    zmodemFileIndex,
    zmodemFileCount,
    zmodemBusy,
    zmodemPercent,
    zmodemStatusLabel,
    resetZmodemSentry,
    finishZmodemUpload,
    cancelZmodemUpload,
    onZmodemInput,
    consumeFrameViaZmodem,
  } = useZmodem({
    t, showNotice, showError, connected,
    terminal: () => terminal.value,
    sendTerminalBytes, dispatchTerminalOutput,
    loadDirectory: (path?: string) => loadDirectory(path),
    // 懒引用：save 链/trzszBusy 都在本调用点之后才声明（TDZ），闭包只在传输时执行。
    receiveAllowed: () => !trzszBusy.value,
    // 经典 rz 提示（issue #171）：选文件必须在用户手势内触发隐藏 input click；
    // 取消/超时后收起提示横幅。
    pickUploadFiles: () => zmodemInput.value?.click(),
    dismissNotice: () => { noticeOpen.value = false; noticeActions.value = []; },
    probeLocalCapabilities: () => late.probeLocalCapabilities(),
    saveHostFile: (chunks, fileName) => late.saveHostFile(chunks, fileName),
    loadDownloadUseDefaultDir, askDownloadTarget, loadDownloadDir, resolveDownloadConflictFor,
    applyChosenDirAsDefault,
    revealTransferTarget: (path) => late.revealTransferTarget(path),
  });

  // trzsz (trz / tsz)：状态/进度/落盘收口在 composables/useTrzsz（终端流集成点在上方）。
  const {
    trzszInput,
    trzszPhase,
    trzszDirection,
    trzszFileName,
    trzszFileIndex,
    trzszFileCount,
    trzszPercent,
    trzszSpeed,
    trzszMessage,
    trzszBusy,
    trzszOverlayVisible,
    trzszStatusLabel,
    ensureTrzszFilter,
    handleTrzszAnnounce,
    cancelTrzszTransfer,
    teardownTrzsz,
    chooseTrzszUpload,
    onTrzszPickInput,
    onTrzszPickCancel,
    feedTrzszTerminalInput,
    setTrzszTerminalColumns,
  } = useTrzsz({
    t, showNotice, showError, session, canWrite, zmodemBusy, terminalMenuOpen,
    terminal: () => terminal.value,
    writeTerminalOutput, sendTerminalBytes,
    probeLocalCapabilities: () => late.probeLocalCapabilities(),
    saveHostFile: (chunks, fileName) => late.saveHostFile(chunks, fileName),
    loadDownloadUseDefaultDir, askDownloadTarget, loadDownloadDir, resolveDownloadConflictFor,
    applyChosenDirAsDefault,
    revealTransferTarget: (path) => late.revealTransferTarget(path),
  });

  // 协议帧缓冲（local/telnet/serial）：收口在 composables/useProtocolFrameBuffers。
  const {
    localLastSequence,
    localPendingFrames,
    localReplayInFlight,
    localReplayNoProgress,
    markLocalExited,
    drainLocalTerminalFrames,
    telnetLastSequence,
    telnetPendingFrames,
    telnetReplayInFlight,
    telnetReplayNoProgress,
    markTelnetClosed,
    drainTelnetFrames,
    serialLastSequence,
    serialPendingFrames,
    serialReplayInFlight,
    serialReplayNoProgress,
    serialUnknownStreamFrames,
    markSerialClosed,
    drainSerialFrames,
    requestLocalReplay,
  } = useProtocolFrameBuffers({
    t,
    dispatchTerminalOutput, showNotice,
    localSession, localState, localExitCode, stopCommandMarkerTick,
    telnetSession, telnetState, telnetError,
    serialSession, serialState, serialError,
  });

  // 宿主二进制事件（local/telnet/serial/vnc/rdp/ssh 输出 + sftp 下载）与 SSH 帧
  // 重组/空洞回放已随输出管线迁入 useTerminalWriter（handleBinary 由上方解构）。

  // SSH 会话主干：协议路由查询、open/attach/关闭、重试退避、传输复用/降级、
  // 宿主重开与 host-key 应答收口在 composables/useSshConnection。重连梯子、
  // 传输断开监听与连接卡片在本段之后装配并消费其函数，故先于此创建、后段
  // 依赖经 getter 惰性接入；输出管线帧流仍是 App 的 `let`，经 frameStreams 读写。
  // 模板同名绑定（resolveHostKey/inactiveWaiting）经解构透出。

  return {
    tooltip, tooltipBubble, hideTooltip, onTooltipOver, onTooltipOut, onTooltipFocusIn, onTooltipFocusOut, terminalTheme,
    hostTerminalTheme, applyTerminalPaddingVars, applyTerminalAppearance, updateTerminalAppearance, applyTerminalAppearanceTheme, saveTerminalAppearanceTheme, deleteTerminalAppearanceTheme, addImportedSchemes,
    removeImportedScheme, applyAppearance, registerOscColorQueryHandlers, hostTerminalFontFamily, hostFontObserver, splitTerminalTheme, terminalBellFlash, handleTerminalBell,
    openTerminalLink, handleTerminalMiddleClick, handleTerminalWheel, handleTerminalMouseDown, handleTerminalMouseUp, adjustTerminalZoom, resetTerminalZoom, setTerminalFont,
    applyTerminalFontSize, applyTerminalFontSettings, appearanceCrossSurfaceSync, quickSelectOpen, quickSelectHits, quickSelectActive, openQuickSelect, closeQuickSelect,
    moveQuickSelectActive, copyQuickSelectHit, handleQuickSelectKey, ghostEnabled, ghostMatch, ghostAnchor, setGhostEnabled, hideGhostSuggestion,
    resetGhostSuggestion, refreshGhostAfterInput, acceptGhostSuggestion, acceptGhostSuggestionWord, syncGhostAnchorOnSettle, handleTerminalKey, zmodemState, zmodemSpeed,
    zmodemFileIndex, zmodemFileCount, zmodemBusy, zmodemPercent, zmodemStatusLabel, resetZmodemSentry, finishZmodemUpload, cancelZmodemUpload,
    onZmodemInput, consumeFrameViaZmodem, trzszInput, trzszPhase, trzszDirection, trzszFileName, trzszFileIndex, trzszFileCount,
    trzszPercent, trzszSpeed, trzszMessage, trzszBusy, trzszOverlayVisible, trzszStatusLabel, ensureTrzszFilter, handleTrzszAnnounce,
    cancelTrzszTransfer, teardownTrzsz, chooseTrzszUpload, onTrzszPickInput, onTrzszPickCancel, feedTrzszTerminalInput, setTrzszTerminalColumns, localLastSequence,
    localPendingFrames, localReplayInFlight, localReplayNoProgress, markLocalExited, drainLocalTerminalFrames, telnetLastSequence, telnetPendingFrames, telnetReplayInFlight,
    telnetReplayNoProgress, markTelnetClosed, drainTelnetFrames, serialLastSequence, serialPendingFrames, serialReplayInFlight, serialReplayNoProgress, serialUnknownStreamFrames,
    markSerialClosed, drainSerialFrames, requestLocalReplay,
  };
}

export type InstallWorkbenchAppearanceResult = ReturnType<typeof installWorkbenchAppearance>;
