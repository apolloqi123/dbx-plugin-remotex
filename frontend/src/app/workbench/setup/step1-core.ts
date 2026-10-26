/**
 * Workbench core: shared preference state, notices, terminal search, AI assistant bridge, view preferences and the workbench context (domain state factories).
 */
import { computed, ref } from "vue";
import { isApplePlatform } from "../../../features/terminal/lib/behavior/terminalInteraction";
import { loadGhostTabAccept, saveGhostTabAccept } from "../../../features/completion/lib/ghostAcceptKey";
import { type DownloadConflictPolicy } from "../../../features/sftp/lib/transfer/downloadPrefs";
import { type SftpCompressMode } from "../lib/preferencesMirror";
import { type SftpNameEncoding } from "../../../features/sftp/lib/entries/sftpName";
import { useAiAssistant } from "../../../features/terminal/composables/ui/useAiAssistant";
import { useNotices } from "../composables/useNotices";
import { useTerminalAiSearch } from "../../../features/terminal/composables/ui/useTerminalAiSearch";
import { useTerminalSearch } from "../../../features/terminal/composables/ui/useTerminalSearch";
import { useWorkbenchContext } from "../state/useWorkbenchContext";
import { useWorkbenchViewPrefs } from "../composables/useWorkbenchViewPrefs";
import { workbenchMessage } from "../../../shared/lib/i18n";
import type { SetupState } from "./setupState";
type Deps = { late: Record<string, any>; state: SetupState };

export function installWorkbenchCore(deps: Deps) {
  const { late } = deps;
  const M = deps.state;

  // 粘贴防护：内容含换行或达到该字符数时先确认（对齐 tiny-rdm TerminalPane 阈值）。
  const PASTE_CONFIRM_CHAR_THRESHOLD = 200;
  // SFTP 路径历史：每连接最多保留 10 条，存 pluginStore（宿主 host.storage；对齐 tiny-rdm pathHistory）。
  // 命令历史 / 终端字号：pluginStore 持久化（敏感命令不入持久层；快速命令已迁 sidecar，见 QUICK_COMMANDS_KEY）。
  const COMMAND_HISTORY_KEY = "ssh-command-history";
  // 执行时间并行映射（Warp 式 history 面板的右侧相对时间）：command → 最近一次
  // 执行时刻。不改 commandHistory（string[]）的形状——suggestion/ghost/命令弹窗
  // 多处消费零波及；时间只有面板用。
  const COMMAND_HISTORY_TIMES_KEY = "ssh-command-history-times";
  const COMMAND_HISTORY_META_KEY = "ssh-command-history-meta";
  // 终端字号/字体族键移入 lib/terminalFont.ts（issue #31 字体单独设置）统一管理。
  // 是否默认下载到「下载保存目录」（默认开）；关闭则每次下载打开目录选择窗口。
  // 文件已存在时的处理策略：rename（自动重命名，默认）/ ask（询问我）/ overwrite（覆盖）。
  // 上传并发（P1-5，1..10，默认 3）与重复目标策略（rename 默认）：sidecar
  // 压缩传输（M33，gzip 混合方案）：策略三态（auto=智能综合判断含弱 CPU
  // 门槛，默认 / on=始终尝试 / off=关闭）+ 生效阈值 MiB（默认 64，0=不限
  // 下限）；sidecar preferences 同步，键名与后端白名单一致。
  // 命令输入建议（P1-1）：开关 + 查询长度上下限；同一偏好链路持久化。
  // 下载偏好的内存权威态：setup 早期（downloadUseDefaultDraft 初始化）就会被读，
  // 必须声明在所有读取点之前（存储语义见下方 loadDownloadDir 一带的注释）。
  const downloadDirState = ref("");
  const downloadUseDefaultState = ref(true);
  const downloadConflictState = ref<DownloadConflictPolicy>("rename");
  // 上传并发/重复策略与命令建议的内存权威态（hydratePrefs 时被 sidecar 值覆盖）。
  // M14-B 三键：会话级并发深度（1-8，默认 3）、老旧服务器兼容模式（默认关）、
  // 文件名显示编码（auto/latin-1，默认 auto）。权威态在此，sidecar preferences 同步。
  // 下载限速权威态（KiB/s，0=不限速）；sidecar preferences 同步。
  const sftpCompatModeState = ref(false);
  const sftpNameEncodingState = ref<SftpNameEncoding>("auto");
  // 压缩传输（M33）：策略默认 auto、阈值默认 64 MiB（0=不限下限，上限 65536）。
  const transferCompressModeState = ref<SftpCompressMode>("auto");
  const transferCompressThresholdState = ref(64);
  const suggestionsEnabledState = ref(true);
  const suggestionMinCharsState = ref(1);
  const suggestionMaxCharsState = ref(64);
  // ghost 接受键（Warp 多键模型）：→ / Ctrl+F / Ctrl+E 整段、Ctrl+→ 逐词恒可用
  // （lib/ghostAcceptKey.ts 匹配矩阵）；唯一可配置项是「Tab 接受建议」开关
  // （Warp Tab key behavior），pluginStore 权威 + 内存态供按键分支与键帽回显。
  // issue #108a 起 Ctrl+F 只在 ghost 建议可见待接受时作接受键，无 ghost 归终端
  // 搜索（iTerm2 同位，resolveCtrlFRoute 判定）；→ / Ctrl+E 行为不变。
  const ghostTabAccept = ref(loadGhostTabAccept());
  const ghostAcceptKeyLabel = computed(() => (ghostTabAccept.value ? "Tab" : "→"));
  function setGhostTabAccept(value: boolean) {
    ghostTabAccept.value = value;
    saveGhostTabAccept(value);
  }

  // 通知/错误横幅状态收口在 composables/useNotices；t 为后段初始化的惰性引用。
  const {
    terminalError,
    sftpError,
    sftpErrorRetry,
    sftpErrorOpen,
    // 错误横幅 key 经 useSftpDialogs 聚合透出（App 解构口不变），这里先别名。
    sftpErrorKey: noticeSftpErrorKey,
    notice,
    noticeActions,
    noticeOpen,
    noticeKey,
    showNotice,
    onNoticeOpenChange,
    onSftpErrorOpenChange,
    showError,
  } = useNotices({ t: (key) => workbenchMessage(locale.value, key) });

  // 主终端搜索面板状态/动作收口在 composables/useTerminalSearch；addon 与菜单在
  // 后段初始化，getter/回调延迟读取。
  const {
    searchOpen,
    searchSeedQuery,
    searchSeedOptions,
    searchMatchState,
    searchResultIndex,
    searchResultCount,
    openTerminalSearch,
    closeTerminalSearch,
    clearTerminalSearch,
    resetSearchResults,
    handleSearchResults,
    runTerminalSearch,
  } = useTerminalSearch({
    terminal: () => terminal.value,
    searchAddon: () => late.searchAddon(),
    closeMenu: () => {
      late.terminalMenuOpen.value = false;
    },
  });

  // 存储迁移批 3：传输/下载/建议偏好消毒器已下沉 workbench/lib/preferencesMirror
  // （镜像 schema 同处、可单测），App 统一从那里导入。
  // —— AI 助手（Warp AI 对齐批，IMPL_PLAN_WARP_AI_TERMINAL）——
  // 设置权威值/修复条/执行期输出采集/推荐位收口在 composables/useAiAssistant；
  // 终端与后声明偏好经 getter 惰性读取。`#` 命令搜索状态机留在 App（模板直接
  // 消费其解构态）。
  const {
    aiSettingsState,
    aiBridgeReady,
    aiFixBar,
    aiBridgeOk,
    updateAiSettings,
    resetAiFixConsent,
    aiContextExtras,
    aiFixFromBar,
    aiAssistFromTerminal,
    aiFixBarStyle,
    maybeOfferAiFix,
    captureOutputChunk,
    beginAiCommand,
    resetAiFixForSession,
    clearFixRecommendation,
  } = useAiAssistant({
    t: (key, values) => late.t(key, values),
    showNotice,
    getLocale: () => locale.value,
    getConnectionId: () => connectionId.value,
    getTerminalCwd: () => terminalCwd.value,
    isLocalMode: () => late.isLocalMode.value,
    getLocalShellPref: () => late.localShellPref.value,
    getLocalShells: () => late.localShells.value,
    isApplePlatform: () => applePlatform,
    sniffShell: () => late.sniffTerminalShell(),
    terminal: () => terminal.value,
    commandMarker: () => late.commandMarker,
    suggestionViewportHeight: () => late.suggestionViewport.value.height,
    readSuggestionAnchor: () => late.readTerminalSuggestionAnchor(),
    replaceLine: (command) => {
      late.replaceTerminalLineWith(command, false);
      terminal.value?.focus();
    },
  });
  // `#` 命令搜索状态机/等待冻结/锚点/直连生成与分屏窗格共用同一 composable。
  const aiSearch = useTerminalAiSearch({
    terminal: () => terminal.value,
    getHost: () => terminalHost.value,
    t: (key, values) => late.t(key, values),
    getLine: () => M.pendingTerminalInput,
    isEnabled: () => aiBridgeOk() && aiSettingsState.value.search,
    gates: () => ({
      alternateActive: terminal.value?.buffer.active.type === "alternate",
      commandRunning: late.commandRunning.value,
      transferBusy: late.terminalTransferBusy.value,
      suggestionOpen: late.suggestionOpen.value,
      completionOpen: late.completionOpen.value,
      historyPanelOpen: late.historyPanelOpen.value,
      quickSelectOpen: late.quickSelectOpen.value,
      searchOpen: searchOpen.value,
    }),
    replaceLine: (command) => {
      late.replaceTerminalLineWith(command, false);
      terminal.value?.focus();
    },
    notify: showNotice,
    context: aiContextExtras,
    onActivate: () => late.dismissPromptHints(),
  });
  const {
    state: aiSearchState,
    pending: aiSearchPending,
    pendingQuery: aiSearchPendingQuery,
    anchor: aiSearchAnchor,
    barEl: aiSearchBarEl,
    barStyle: aiSearchBarStyle,
  } = aiSearch;

  // Apple 平台判定（Cmd 为主修饰键）：既有的全选语义与新增的快捷键默认键位都要用，
  // 因此在此单点声明，供后面的偏好初始值与终端选项复用。
  const applePlatform = isApplePlatform();

  /** SftpWorkspacePanel 把内部树实例回填到上面的 ref，保持目录跟随与宿主拖拽落点。
 *  markDropTargetAt 接宿主转发的 iframe 内坐标，命中目录行并返回目标路径。 */
  function bindSftpWorkspaceTreePane(api: {
    scrollPathToTop(path: string): Promise<boolean>;
    markDropTargetAt(point: { x: number; y: number }): string;
    resetHostDragCalibration(): void;
  } | null) {
    sftpTreePane.value = api;
  }
  /** 路径输入框随工具栏移入子组件：回填 composable 的 ref，进入编辑态仍能聚焦。 */
  function bindSftpPathBarInput(el: HTMLInputElement | null) {
    late.pathBarInputEl.value = el;
  }
  /** 拖入上传落点弹窗随 T21 迁入 WorkbenchDialogs：路径输入框 ref 经回调回填
 *  useUploadChain 持有的 ref（打开弹窗时自动聚焦的既有语义不变）。 */
  function bindDropUploadPathInputEl(el: HTMLInputElement | null) {
    late.dropUploadPathInputEl.value = el ?? undefined;
  }
  const zmodemInput = ref<HTMLInputElement>();
  // Bottom dock panel surface (surface=panel, host §8.3): hide the workbench identity block so the panel
  // and focus the terminal itself; multi-open/shell switching goes through the panel "+" menu (bridge openWorkbench opens another panel).
  // Declared early: the batch bar / sftp pane initializers below must know the surface at setup time.
  // 宿主表面词表（开放集合，未知回退）："tab" 主工作台多标签 / "dock" 底部栏
  // （2026-09-30 起，旧值 "panel" 兼容期保留）/ "sidebar-left|right" / "window" 预留。
  const panelSurface = computed(() => hostContext.value.surface === "dock" || hostContext.value.surface === "panel");
  // 工作台视图/终端行为偏好（布局轴向、SFTP 默认/自动连接、目录跟随、终端行为
  // 与快捷键）：收口在 composables/useWorkbenchViewPrefs。
  const {
    paneDirection,
    sftpPaneDefaultOpen,
    sftpAutoConnect,
    terminalBehavior,
    termSelectCopy,
    terminalHotkeys,
    followDirectory,
    toggleSftpPaneDefaultOpen,
    loadSftpPaneDefaultOpen,
    toggleSftpAutoConnect,
    loadDirectoryFollowPref,
    persistDirectoryFollowPref,
    setPaneDirection,
    updateTerminalBehavior,
    updateTerminalHotkeys,
  } = useWorkbenchViewPrefs({
    applePlatform,
    terminal: () => terminal.value,
    showNotice: (message) => showNotice(message),
    t: (key, values) => late.t(key, values),
    scheduleFit: () => late.scheduleFit(),
  });
  // —— 工作台状态上下文（Task 21b）：按域创建的 refs 收口在 workbench/state/*，
  // 本文件解构同名绑定保持模板/接线不变（行为、初始值与消毒器均未改动）。 ——
  const ctx = useWorkbenchContext({
    sftpPaneDefaultOpen: loadSftpPaneDefaultOpen(),
    paneDirection,
  });
  const { hostContext, appearance, locale, workbenchId } = ctx;
  const {
    terminal,
    stage,
    terminalHost,
    terminalState,
    terminalCwd,
    terminalCopyCache,
    directoryTrackingSupported,
    terminalFontSize,
    terminalFontOverride,
    terminalAppearance,
    terminalAppearanceState,
    activeAppearanceThemeId,
    terminalDiag,
    terminalDiagVisible,
  } = ctx.terminal;
  const {
    session,
    connectCancelled,
    connectSucceeded,
    connectLogsOpen,
    connectLog,
    hostKeyPrompt,
    rememberHostKey,
    connectionReadOnly,
    rdpSession,
    rdpDialogOpen,
    rdpState,
    rdpScaleMode,
    rdpSurface,
    rdpClipboardChunks,
    isRdpMode,
    rdpTarget,
    connectionId,
    restored,
    runtimeEndpoint,
    connection,
    storedConnectionProtocol,
    connectionProtocol,
    isRdpConnection,
    canWrite,
    connected,
    toolbarStyle,
  } = ctx.session;
  const {
    sftpPane,
    sftpTreePane,
    sftpPaneOpen,
    rightPanelView,
    dockerPanelOpen,
    sftpViewActive,
    sftpViewVisible,
    visibleColumns,
    sftpColumnWidths,
    sftpNameWidth,
    sort,
    sftpSearch,
    sftpTypeFilter,
    sftpShowHidden,
    batchProgress,
    terminalDragActive,
    localDownloadDir,
    localCanSave,
    SFTP_QUICK_PATHS,
  } = ctx.sftp;
  const {
    splitRatio,
    dockerSplitRatio,
    paneOrder,
    sudoMode,
    paneContainer,
    paneLayout,
    terminalBasis,
    paneSplitStyle,
    orderedPaneClass,
  } = ctx.layout;
  const {
    settingsOpen,
    auditOpen,
    profilesOpen,
    otpPanelOpen,
    importPanelOpen,
    forwardsOpen,
    columnsOpen,
    ghostKeycapMenuOpen,
    sessionMenuOpen,
    sessionMenuShellOpen,
    sessionMenuShellPickOpen,
  } = ctx.dialogs;

  // 主终端输入、行跟踪与有序输入队列收口在
  // features/terminal/composables/io/useTerminalInput；实例只做 onData 接线
  // （WKWebView 输入丢失兜底共用同一路由）。依赖（会话态/传输占用/建议与 ghost
  // 刷新/断线梯子）多在后段声明，故一律经 getter 惰性读取（setup 期不求值）；
  // 行缓冲 pendingTerminalInput 本体仍在本文件，经 getter/setter 注入。

  return {
    stage,
    PASTE_CONFIRM_CHAR_THRESHOLD, COMMAND_HISTORY_KEY, COMMAND_HISTORY_TIMES_KEY, COMMAND_HISTORY_META_KEY, downloadDirState, downloadUseDefaultState, downloadConflictState, sftpCompatModeState,
    sftpNameEncodingState, transferCompressModeState, transferCompressThresholdState, suggestionsEnabledState, suggestionMinCharsState, suggestionMaxCharsState, ghostTabAccept, ghostAcceptKeyLabel,
    setGhostTabAccept, terminalError, sftpError, sftpErrorRetry, sftpErrorOpen, noticeSftpErrorKey, notice, noticeActions,
    noticeOpen, noticeKey, showNotice, onNoticeOpenChange, onSftpErrorOpenChange, showError, searchOpen, searchSeedQuery,
    searchSeedOptions, searchMatchState, searchResultIndex, searchResultCount, openTerminalSearch, closeTerminalSearch, clearTerminalSearch, resetSearchResults,
    handleSearchResults, runTerminalSearch, aiSettingsState, aiBridgeReady, aiFixBar, aiBridgeOk, updateAiSettings, resetAiFixConsent,
    aiContextExtras, aiFixFromBar, aiAssistFromTerminal, aiFixBarStyle, maybeOfferAiFix, captureOutputChunk, beginAiCommand, resetAiFixForSession,
    clearFixRecommendation, aiSearch, aiSearchState, aiSearchPending, aiSearchPendingQuery, aiSearchAnchor, aiSearchBarEl, aiSearchBarStyle,
    applePlatform, bindSftpWorkspaceTreePane, bindSftpPathBarInput, bindDropUploadPathInputEl, zmodemInput, panelSurface, paneDirection, sftpPaneDefaultOpen,
    sftpAutoConnect, terminalBehavior, termSelectCopy, terminalHotkeys, followDirectory, toggleSftpPaneDefaultOpen, loadSftpPaneDefaultOpen, toggleSftpAutoConnect,
    loadDirectoryFollowPref, persistDirectoryFollowPref, setPaneDirection, updateTerminalBehavior, updateTerminalHotkeys, ctx, hostContext, appearance,
    locale, workbenchId, terminal, terminalHost, terminalState, terminalCwd, terminalCopyCache, directoryTrackingSupported,
    terminalFontSize, terminalFontOverride, terminalAppearance, terminalAppearanceState, activeAppearanceThemeId, terminalDiag, terminalDiagVisible, session,
    connectCancelled, connectSucceeded, connectLogsOpen, connectLog, hostKeyPrompt, rememberHostKey, connectionReadOnly, rdpSession,
    rdpDialogOpen, rdpState, rdpScaleMode, rdpSurface, rdpClipboardChunks, isRdpMode, rdpTarget, connectionId,
    restored, runtimeEndpoint, connection, storedConnectionProtocol, connectionProtocol, isRdpConnection, canWrite, connected,
    toolbarStyle, sftpPane, sftpTreePane, sftpPaneOpen, rightPanelView, dockerPanelOpen, sftpViewActive, sftpViewVisible,
    visibleColumns, sftpColumnWidths, sftpNameWidth, sort, sftpSearch, sftpTypeFilter, sftpShowHidden, batchProgress,
    terminalDragActive, localDownloadDir, localCanSave, SFTP_QUICK_PATHS, splitRatio, dockerSplitRatio, paneOrder, sudoMode,
    paneContainer, paneLayout, terminalBasis, paneSplitStyle, orderedPaneClass, settingsOpen, auditOpen, profilesOpen,
    otpPanelOpen, importPanelOpen, forwardsOpen, columnsOpen, ghostKeycapMenuOpen, sessionMenuOpen, sessionMenuShellOpen, sessionMenuShellPickOpen,
  };
}

export type InstallWorkbenchCoreResult = ReturnType<typeof installWorkbenchCore>;
