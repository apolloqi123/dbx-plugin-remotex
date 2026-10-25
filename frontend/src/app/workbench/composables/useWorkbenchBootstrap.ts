import { nextTick, type Ref } from "vue";
import { subscribeHostEnvironment } from "../../../../../shared/frontend/hostThemeRuntime";
import type { DbxPluginAppearanceInput } from "../../../shared/lib/appearance";
import { pluginStore } from "../../../shared/lib/pluginStore";
import { readPluginMode, readPluginShell } from "../../../shared/lib/pluginContext";
import { isDbxPluginTheme, onHostThemeChange, themeToAppearance } from "../../../shared/lib/hostTheme";
import type { PersistThrottle } from "../../lib/persistThrottle";
import type { RdpSessionStateView } from "../../../features/remote-desktop/lib/rdpFrame";
import type { SessionSummary } from "../../../features/connections/lib/sessionRestore";
import { pickLiveSessionForReattach, pickProtocolSessionForReattach } from "../../../features/connections/lib/sessionRestore";
import type { ReplayResult } from "../../../features/connections/lib/sshConnectionTypes";
import {
  extractMcpSettingsMirror,
  loadMcpSettingsMirror,
  mcpSettingsMirrorEquals,
  persistMcpSettingsMirror,
} from "../../../features/settings/lib/settingsModel";
import type { HostDragTreePane } from "./useHostDragBridge";

/** 宿主桥 filedrop 条目（与 useHostFileDrag 的入参同形）。 */
type HostFileDropFile = { handleId: string; name: string; size: number; contentType: string; relativePath?: string };

/** Telnet/串口帧缓冲（useProtocolFrameBuffers 持有）。 */
type PendingFrames = Map<number, { stream: number; data: Uint8Array }>;

/**
 * 工作台启动/生命周期桥：宿主 API 握手 → UI 状态水合 → 会话接回/新开 →
 * 事件订阅的启动顺序，宿主事件分派入口，MCP 设置镜像，以及挂载/卸载的
 * 监听器与资源收口。状态本体（会话 refs、终端实例、树实例、通知）仍由
 * App.vue 持有，全部经 options 注入；函数体逐行原样迁移。
 */
export function useWorkbenchBootstrap(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  hostContext: Ref<Record<string, unknown>>;
  locale: Ref<string>;
  terminalState: Ref<"connecting" | "connected" | "disconnected" | "error">;
  terminalError: Ref<string>;
  panelSurface: Readonly<Ref<boolean>>;
  connectionId: Readonly<Ref<string>>;
  workbenchId: Readonly<Ref<string>>;
  restored: Readonly<Ref<boolean>>;
  // 宿主拖拽桥（useHostDragBridge）：SDK 监听与原始 postMessage 共用同一份逻辑。
  sftpDragDropTarget: Ref<string>;
  sftpTreePane: Ref<HostDragTreePane | null>;
  applyHostDragState: (active: boolean, position?: { x: number; y: number }) => void;
  onHostDragMessage: (event: MessageEvent) => void;
  // SSH 会话主干（useSshConnection）。
  openSession: (forceNew?: boolean, bootRestore?: boolean) => Promise<unknown>;
  attachSession: (sessionId: string, retryReference?: string) => Promise<unknown>;
  requestHostReopenConnection: () => Promise<boolean>;
  resetTransportReuseState: (context: Record<string, unknown>) => void;
  // 本地终端会话（useLocalShellSession）。
  localSession: Ref<{ sessionId: string; shell: string } | null>;
  localState: Ref<"starting" | "running" | "exited">;
  localShellRestored: Ref<boolean>;
  startLocalTerminal: (
    shellOverride?: string,
    connection?: { cwd?: string; env?: Array<{ name: string; value: string }> },
  ) => Promise<void>;
  reattachLocalSession: () => Promise<boolean>;
  // Telnet / 串口 / VNC / RDP 会话态。
  telnetSession: Ref<{ sessionId: string; host: string; port: number } | null>;
  telnetState: Ref<"idle" | "connecting" | "running" | "closed">;
  telnetLastSequence: Ref<number>;
  telnetPendingFrames: PendingFrames;
  reattachVncSession: (session: { sessionId: string; host?: string; port?: number }) => Promise<void>;
  rdpSession: Ref<{ sessionId: string; host: string; port: number } | null>;
  rdpState: Ref<RdpSessionStateView>;
  serialSession: Ref<{ sessionId: string; port: string; baudRate: number } | null>;
  serialState: Ref<"idle" | "running" | "closed">;
  serialLastSequence: Ref<number>;
  serialPendingFrames: PendingFrames;
  serialBinaryInput: Ref<boolean>;
  // 终端实例与输出管线。
  terminalHost: Ref<HTMLElement | undefined>;
  createTerminal: () => void;
  scheduleFit: () => void;
  detachTerminalHost: (host: HTMLElement) => void;
  disposeTerminalInstance: () => void;
  handleBinary: (event: DbxPluginBinaryEvent) => void;
  disposeOutputPipeline: () => void;
  // 持久化 / 偏好 / 外观。
  initialState: () => { sessionId?: string };
  restoreUiState: () => void;
  writeWorkbenchState: () => Promise<unknown>;
  persistThrottle: PersistThrottle;
  hydratePrefs: () => Promise<void>;
  applyTerminalPaddingVars: () => void;
  applyAppearance: (next: DbxPluginAppearanceInput) => void;
  hostFontObserver: MutationObserver;
  hydrateQuickCommands: () => Promise<void>;
  hydrateHighlightRules: () => Promise<void>;
  // 通知 / 提示气泡 / 弹层焦点与 Esc 链。
  showNotice: (message: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  hideTooltip: () => void;
  onTooltipOver: (event: MouseEvent) => void;
  onTooltipOut: (event: MouseEvent) => void;
  onTooltipFocusIn: (event: FocusEvent) => void;
  onTooltipFocusOut: (event: FocusEvent) => void;
  onDocumentClickCapture: (event: MouseEvent) => void;
  onDocumentMouseDownCapture: (event: MouseEvent) => void;
  onDocumentKeydown: (event: KeyboardEvent) => void;
  trackStableFocus: (event: FocusEvent) => void;
  onDocumentClickCloseMenus: (event: MouseEvent) => void;
  resolvePasteConfirm: (accepted: boolean) => void;
  resolvePendingConfirmDialog: (accepted: boolean) => void;
  // 终端装饰引擎 / 命令标记 / Agent / 重连梯子 / 提示条。
  handleDockerOpenInTerminal: (event: Event) => void;
  detachHighlightRender: () => void;
  detachActionLinks: () => void;
  detachGutterListeners: () => void;
  stopCommandMarkerTick: () => void;
  stopAgentPromptTimer: () => void;
  cancelReconnect: () => void;
  disposePromptHints: () => void;
  // SFTP 宿主拖放 / 在途传输等待者。
  handleHostFileDrop: (files: HostFileDropFile[], targetDir?: string) => Promise<void>;
  uploadAckWaiters: Map<string, { nextOffset: number; resolve: () => void; reject: (error: Error) => void; timer: number }>;
  downloadChunkWaiters: Map<string, { offset: number; resolve: (bytes: Uint8Array) => void; reject: (error: Error) => void; timer: number }>;
  // 后端事件分派入口（usePluginEvents）。
  handlePluginEvent: (event: DbxPluginEvent) => void;
  // AI 桥能力位 / 文件夹上传能力位 / 窗口级拖拽兜底 / 卸载标志。
  aiBridgeReady: Ref<boolean>;
  aiBridgeOk: () => boolean;
  folderUploadSupported: Ref<boolean>;
  resetDragHighlightOnWindowLeave: (event: DragEvent) => void;
  markDisposed: () => void;
  clearTerminalDiagTimer: () => void;
}) {
  const {
    t,
    hostContext,
    locale,
    terminalState,
    terminalError,
    panelSurface,
    connectionId,
    workbenchId,
    restored,
    sftpDragDropTarget,
    sftpTreePane,
    applyHostDragState,
    onHostDragMessage,
    openSession,
    attachSession,
    requestHostReopenConnection,
    resetTransportReuseState,
    localSession,
    localState,
    localShellRestored,
    startLocalTerminal,
    reattachLocalSession,
    telnetSession,
    telnetState,
    telnetLastSequence,
    telnetPendingFrames,
    reattachVncSession,
    rdpSession,
    rdpState,
    serialSession,
    serialState,
    serialLastSequence,
    serialPendingFrames,
    serialBinaryInput,
    terminalHost,
    createTerminal,
    scheduleFit,
    detachTerminalHost,
    disposeTerminalInstance,
    handleBinary,
    disposeOutputPipeline,
    initialState,
    restoreUiState,
    writeWorkbenchState,
    persistThrottle,
    hydratePrefs,
    applyTerminalPaddingVars,
    applyAppearance,
    hostFontObserver,
    hydrateQuickCommands,
    hydrateHighlightRules,
    showNotice,
    showError,
    hideTooltip,
    onTooltipOver,
    onTooltipOut,
    onTooltipFocusIn,
    onTooltipFocusOut,
    onDocumentClickCapture,
    onDocumentMouseDownCapture,
    onDocumentKeydown,
    trackStableFocus,
    onDocumentClickCloseMenus,
    resolvePasteConfirm,
    resolvePendingConfirmDialog,
    handleDockerOpenInTerminal,
    detachHighlightRender,
    detachActionLinks,
    detachGutterListeners,
    stopCommandMarkerTick,
    stopAgentPromptTimer,
    cancelReconnect,
    disposePromptHints,
    handleHostFileDrop,
    uploadAckWaiters,
    downloadChunkWaiters,
    handlePluginEvent,
    aiBridgeReady,
    aiBridgeOk,
    folderUploadSupported,
    resetDragHighlightOnWindowLeave,
    markDisposed,
    clearTerminalDiagTimer,
  } = options;

  // 宿主订阅句柄：启动时赋值，卸载时逐个注销（顺序即注册顺序）。
  let unsubscribeEvent: (() => void) | undefined;
  let unsubscribeBinary: (() => void) | undefined;
  // 宿主 fileTransfer 桥拖放事件（宿主 1.1 optional）注销句柄：OS 级拖入上传
  // 与终端/SFTP 面板共享同一道门禁，见 handleHostFileDrop。
  let unsubscribeFileDrag: (() => void) | undefined;
  let unsubscribeFileDrop: (() => void) | undefined;
  // X-P2/P3/P4 收敛：宿主环境订阅聚合句柄（shared/frontend/hostThemeRuntime）。
  let unsubscribeEnvironment: (() => void) | undefined;

  // 宿主桥事件入口：env 事件由 composable 内窄化排除。
  function handleEvent(event: DbxPluginEvent) {
    handlePluginEvent(event);
  }

  async function waitForHostApi(timeoutMs = 8000) {
    const deadline = Date.now() + timeoutMs;
    while (!window.dbxPlugin && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 50));
    if (!window.dbxPlugin) throw new Error(t("hostApiUnavailable"));
    return window.dbxPlugin;
  }

  async function initialize() {
    const api = await waitForHostApi();
    hostContext.value = await Promise.any([
      api.ready,
      api.request<Record<string, unknown>>("host.getContext"),
    ]);
    resetTransportReuseState(hostContext.value);
    locale.value = api.locale || "zh-CN";
    // UI 状态恢复必须等 pluginStore 水合（宿主 storage 白名单键异步灌入）：
    // 否则 workbenchUi 镜像与 setup 期的 sftp-pane-open 等键全部读到空档，
    // 布局偏好整组"不持久"（issue #178 的半个根因，另一半是宿主缺桥）。
    await pluginStore.ready.catch(() => undefined);
    restoreUiState();
    const appearanceAppliedAtBoot = Boolean(api.appearance || api.theme);
    // 外观偏好的 CSS 部分（终端内边距变量）与宿主是否推送 appearance 无关，
    // 开机先落一次，否则用户设了内边距要等下次主题推送才生效。
    applyTerminalPaddingVars();
    // FIG wave-1：legacy 动态 fs provider 随 legacy spec 目录整体退役；
    // generator 动态候选在批次 2 经 completion/execute 目标机执行接线
    // （前端不直连 shell）。
    if (api.appearance) applyAppearance(api.appearance);
    else if (isDbxPluginTheme(api.theme)) applyAppearance(themeToAppearance(api.theme));
    // 宿主可能在 init 前先应答 host.getContext（如重推连接期间 init 被延迟）：
    // 此时 api.locale/appearance 仍是 bootstrap 默认值。init 落地后重读一次，
    // 否则会话会停在默认英文/默认主题。ready 已解决时 then 立即执行，是幂等重读。
    void api.ready.then(() => {
      if (api.locale) locale.value = api.locale;
      if (!appearanceAppliedAtBoot) {
        if (api.appearance) applyAppearance(api.appearance);
        else if (isDbxPluginTheme(api.theme)) applyAppearance(themeToAppearance(api.theme));
      }
    });
    // X-P2/P3/P4 收敛：env（locale/theme）+ context + appearance 订阅统一走
    // shared/frontend/hostThemeRuntime 单点（真桥无 onLocaleChange/onContextChange
    // 幽灵 API；onContext 旧桥回退 onContextChange；appearance 契约缺失时经
    // theme 通道兜底，两套不同时挂）。
    unsubscribeEnvironment = subscribeHostEnvironment<DbxPluginAppearance, DbxPluginTheme>(api, {
      onLocale: (nextLocale) => (locale.value = nextLocale || "zh-CN"),
      onContext: (context) => {
        hostContext.value = context;
      },
      onAppearance: applyAppearance,
      onTheme: (theme) => applyAppearance(themeToAppearance(theme)),
    }, { themeChannel: onHostThemeChange });
    unsubscribeEvent = api.onEvent(handleEvent);
    unsubscribeBinary = api.onBinary(handleBinary);
    unsubscribeFileDrag = api.fileTransfer?.onDragState((active, position) => {
      applyHostDragState(active, position);
    });
    unsubscribeFileDrop = api.fileTransfer?.onDrop((files, drop) => {
      // 宿主目录展开撞上限（如单文件夹 2000 文件）只交付前缀：明确提示而不是
      // 静默丢文件；完整上传指引工具栏「上传文件夹」选择器（浏览器枚举，无此限）。
      if (drop?.truncated) showNotice(t("hostDrop.truncated"));
      // 新宿主附带落点坐标：以落点命中的目录行为准（空白/文件 = 当前目录）；
      // 旧宿主没有坐标时回落到拖拽期间记录的目标。
      const pointed = drop?.position ? sftpTreePane.value?.markDropTargetAt(drop.position) : "";
      const target = drop?.position ? (pointed || undefined) : (sftpDragDropTarget.value || undefined);
      sftpDragDropTarget.value = "";
      void handleHostFileDrop(files, target);
    });
    // 原始 dragstate 通道：宿主消息本身带 position；即使注入 SDK 是旧版/缓存版，
    // 也能拿到坐标。与上面的 SDK 监听重复消费是幂等的。
    window.addEventListener("message", onHostDragMessage);
    // 分屏 PTY 不随 workbenchState 恢复。webview 重载后旧分屏会话仍在 sidecar
    // 中，先按独立 workbenchId 后缀清掉，避免孤儿 PTY 泄漏（主会话不受影响）。
    void closeOrphanSplitSessions();
    // §8.3 面板加载生命周期：探活与终端创建并行。探活只是一次 sidecar 往返，
    // 串行执行会把真正耗时的 openSession 压到整个引导的最后。
    const reattachLookup = readPluginMode(hostContext.value) !== "local-terminal" && connectionId.value && workbenchId.value && !restored.value
      ? findReattachSession()
      : Promise.resolve("");
    await nextTick();
    createTerminal();
    // P0 connectionless local-terminal passthrough (HOST_PLUGIN_UI_SPEC §4/§7.1): when the host opens this workbench with
    // the workbench is opened with the command context (plugin.mode="local-terminal") (command
    // panel / toolbar entry / self-open bridge), the SSH connection flow is skipped and the local terminal opens directly.
    if (readPluginMode(hostContext.value) === "local-terminal") {
      if (!workbenchId.value) throw new Error(t("errors.hostBridgeMissing"));
      await hydratePrefs();
      // Bottom dock / webview rebuild reopen: first reattach the live shell still bound to this workbenchId in the
      // live shell (reopening never leaks a new PTY; spec §10 leaves no PTY behind on close).
      if (await reattachLocalSession()) return;
      // A4 restore semantics (spec §7.6/§8.4): restoring is not re-running the command — a restored tab
      // no automatic shell — just the exit shell until the user explicitly hits "Reopen".
      if (restored.value) {
        localSession.value = null;
        localState.value = "exited";
        localShellRestored.value = true;
        return;
      }
      // Dock "+" creates with the selected shell type (context.plugin.shell); when unset the preference applies.
      await startLocalTerminal(readPluginShell(hostContext.value) || undefined);
      return;
    }
    if (!connectionId.value || !workbenchId.value) throw new Error(t("errors.hostBridgeMissing"));
    const state = initialState();
    if (restored.value) {
      terminalState.value = "disconnected";
      terminalError.value = t("restartDisconnected");
      return;
    }
    // Webview 重建必须先按 workbenchId 接回所有非 SSH 协议会话；这些会话不
    // 写 SSH workbench state，若先走 openSession 会重新创建 Telnet/VNC/RDP/Serial。
    if (await reattachProtocolSession()) return;
    if (typeof state.sessionId === "string" && state.sessionId) await attachSession(state.sessionId);
    else {
      // 宿主切 tab / 左侧菜单重开可能整体重建工作台 webview。只恢复
      // 同一 workbench 的 live session；不能按 connectionId 复用任意会话，
      // 否则打开同一连接的新 Tab 会接管已有 Tab 的 PTY。
      const reattach = await reattachLookup;
      if (reattach) await attachSession(reattach, reattach);
      // 上一轮本地终端还活着（webview 重建但 sidecar 未退出）：接回并补发，
      // 避免孤儿 shell 挂在 sidecar 里。
      else if (await reattachLocalSession()) {
        // 本地模式接管终端。
      }
      // 上来直接连（§8.3 面板加载生命周期）：宿主已在点击创建条目时
      // ensureConnected 预拨（connectionPreconnected 旗标），这里跳过 force
      // 重开（force 会复位共享连接），直接 openSession——拨号未完成时由
      // preconnect 短间隔轮询自愈（250ms 固定节奏，不再是 2s 退避梯子），
      // 拨号失败/永久错误走既有分类报错。
      // 旧宿主无旗标：保留原有 force 重开路径。
      else {
        if (panelSurface.value && !hostContext.value.connectionPreconnected) await requestHostReopenConnection();
        await openSession(false, true);
      }
    }
  }

  /**
   * Asks the sidecar for the live session bound to this workbench and connection
   * (sidecar `ssh/sessions/list`); "" when none — caller dials a fresh session.
   * Failures degrade to a fresh open instead of blocking the workbench.
   */
  async function findReattachSession(): Promise<string> {
    try {
      const result = await window.dbxPlugin.invoke<{ sessions?: SessionSummary[] }>("ssh/sessions/list", {}, { timeoutMs: 10_000 });
      return pickLiveSessionForReattach(result?.sessions, { connectionId: connectionId.value, workbenchId: workbenchId.value });
    } catch {
      return "";
    }
  }

  /** 清理 webview 重载遗留的分屏 PTY（workbenchId 带 #split 后缀）。 */
  async function closeOrphanSplitSessions(): Promise<void> {
    if (!workbenchId.value) return;
    const splitWorkbenchId = `${workbenchId.value}#split`;
    try {
      const result = await window.dbxPlugin.invoke<{ sessions?: Array<{ sessionId?: string; workbenchId?: string }> }>("ssh/sessions/list", {}, { timeoutMs: 10_000 });
      const orphans = (result?.sessions ?? []).filter((entry) => entry.workbenchId === splitWorkbenchId && entry.sessionId);
      await Promise.allSettled(orphans.map((entry) => window.dbxPlugin.invoke("ssh/session/close", { sessionId: entry.sessionId })));
    } catch {
      // 旧 sidecar / 无会话列表时静默；没有分屏就没有孤儿。
    }
  }

  type ProtocolSessionKind = "telnet" | "vnc" | "rdp" | "serial";
  interface ProtocolSessionSummary extends SessionSummary {
    host?: string;
    port?: number;
    baudRate?: number;
    binaryInput?: boolean;
  }

  async function reattachProtocolSession(): Promise<boolean> {
    const lists: Array<{ kind: ProtocolSessionKind; method: string }> = [
      { kind: "telnet", method: "telnet/list" },
      { kind: "vnc", method: "vnc/list" },
      { kind: "rdp", method: "rdp/list" },
      { kind: "serial", method: "serial/list" },
    ];
    const candidates = await Promise.all(lists.map(async ({ kind, method }) => {
      try {
        const result = await window.dbxPlugin.invoke<{ sessions?: ProtocolSessionSummary[] }>(method, {}, { timeoutMs: 10_000 });
        const sessionId = pickProtocolSessionForReattach(result?.sessions, workbenchId.value);
        const session = result?.sessions?.find((entry) => entry?.sessionId === sessionId);
        return session ? { kind, session } : null;
      } catch {
        return null;
      }
    }));
    const live = candidates
      .filter((candidate): candidate is { kind: ProtocolSessionKind; session: ProtocolSessionSummary } => candidate !== null)
      .sort((left, right) => (right.session.createdAt ?? 0) - (left.session.createdAt ?? 0))[0];
    if (!live) return false;

    if (live.kind === "telnet") {
      telnetSession.value = { sessionId: live.session.sessionId, host: live.session.host || "", port: live.session.port || 23 };
      telnetState.value = "running";
      telnetLastSequence.value = 0;
      telnetPendingFrames.clear();
      await window.dbxPlugin.invoke<ReplayResult>("telnet/replay", { sessionId: live.session.sessionId, afterSequence: 0 });
    } else if (live.kind === "vnc") {
      await reattachVncSession(live.session);
    } else if (live.kind === "rdp") {
      rdpSession.value = { sessionId: live.session.sessionId, host: live.session.host || "", port: live.session.port || 3389 };
      rdpState.value = { state: "running", error: "", errorKind: "", attempt: 0, maxAttempts: 0 };
      await window.dbxPlugin.invoke("rdp/replay", { sessionId: live.session.sessionId });
    } else {
      serialSession.value = {
        sessionId: live.session.sessionId,
        port: live.session.port ? String(live.session.port) : "",
        baudRate: live.session.baudRate || 115_200,
      };
      serialState.value = "running";
      serialLastSequence.value = 0;
      serialPendingFrames.clear();
      serialBinaryInput.value = live.session.binaryInput !== false;
      await window.dbxPlugin.invoke<ReplayResult>("serial/replay", { sessionId: live.session.sessionId, afterSequence: 0 });
    }
    await nextTick();
    scheduleFit();
    return true;
  }

  // ---------------------------------------------------------------------------
  // MCP 设置镜像（存储迁移批 1，IMPL_PLAN_STORAGE_SYNC）：sidecar 在 MCP serve
  // 时直接消费 mcp-settings.json，pluginStore 里只存「云同步载荷」。启动时：
  // store 缺失 → 从 sidecar persisted 值播种；存在但与 sidecar 不一致（云同步
  // 恢复后）→ 把 store 推回 sidecar。set 是部分更新，localTransferRoot 由
  // sidecar 保留，不进镜像。保存侧的双写在 SettingsDialog.saveMcpSettings。
  // ---------------------------------------------------------------------------

  async function syncMcpSettingsMirror() {
    const stored = loadMcpSettingsMirror();
    try {
      const remote = extractMcpSettingsMirror(await window.dbxPlugin.invoke<unknown>("mcp/settings/get", {}));
      if (stored === null) {
        persistMcpSettingsMirror(remote);
        return;
      }
      if (!mcpSettingsMirrorEquals(stored, remote)) {
        await window.dbxPlugin.invoke("mcp/settings/set", {
          maxReadBytes: stored.maxReadBytes,
          maxUploadBytes: stored.maxUploadBytes,
          maxDownloadBytes: stored.maxDownloadBytes,
          execPermissionMode: stored.execPermissionMode,
          connectionScope: stored.connectionScope,
        });
      }
    } catch {
      // sidecar 不可用（旧版/降级）：跳过，下次启动重试。
    }
  }

  function mount() {
    document.addEventListener("click", onDocumentClickCloseMenus);
    // AI 桥能力位（Warp AI 对齐批）：桥 bootstrap 与 UI 挂载同批完成，挂载后
    // 探一次供设置页横幅展示；运行时调用点各做新鲜探测（aiBridgeOk）。
    aiBridgeReady.value = aiBridgeOk();
    // 文件夹上传能力探测（issue #78）：webkitdirectory 非标准属性，缺失环境
    // （老 webview）隐藏入口，文件级上传不受影响。
    const folderProbe = document.createElement("input");
    folderProbe.type = "file";
    folderUploadSupported.value = "webkitdirectory" in folderProbe;
    document.addEventListener("click", onDocumentClickCapture, true);
    document.addEventListener("mousedown", onDocumentMouseDownCapture, true);
    document.addEventListener("keydown", onDocumentKeydown);
    document.addEventListener("focusin", trackStableFocus);
    document.addEventListener("mouseover", onTooltipOver);
    document.addEventListener("mouseout", onTooltipOut);
    document.addEventListener("focusin", onTooltipFocusIn);
    document.addEventListener("focusout", onTooltipFocusOut);
    document.addEventListener("pointerdown", hideTooltip, true);
    document.addEventListener("wheel", hideTooltip, true);
    window.addEventListener("dbx:docker-open-in-terminal", handleDockerOpenInTerminal);
    window.addEventListener("dragleave", resetDragHighlightOnWindowLeave);
    hostFontObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["style"] });
    void hydrateQuickCommands();
    void hydrateHighlightRules();
    void syncMcpSettingsMirror();
    void hydratePrefs();
    void initialize().catch((cause) => {
      terminalState.value = "error";
      showError(cause, "terminal");
    });
  }

  function unmount() {
    markDisposed();
    window.removeEventListener("dbx:docker-open-in-terminal", handleDockerOpenInTerminal);
    window.removeEventListener("dragleave", resetDragHighlightOnWindowLeave);
    document.removeEventListener("mouseover", onTooltipOver);
    document.removeEventListener("mouseout", onTooltipOut);
    document.removeEventListener("focusin", onTooltipFocusIn);
    document.removeEventListener("focusout", onTooltipFocusOut);
    document.removeEventListener("pointerdown", hideTooltip, true);
    document.removeEventListener("wheel", hideTooltip, true);
    hostFontObserver.disconnect();
    hideTooltip();
    persistThrottle.cancel(); // 卸载兜底：丢弃挂起定时器后直接落盘（保留既有语义）
    void writeWorkbenchState();
    cancelReconnect();
    clearTerminalDiagTimer();
    disposePromptHints();
    stopCommandMarkerTick();
    stopAgentPromptTimer();
    resolvePasteConfirm(false);
    resolvePendingConfirmDialog(false);
    if (terminalHost.value) detachTerminalHost(terminalHost.value);
    document.removeEventListener("click", onDocumentClickCloseMenus);
    document.removeEventListener("click", onDocumentClickCapture, true);
    document.removeEventListener("mousedown", onDocumentMouseDownCapture, true);
    document.removeEventListener("keydown", onDocumentKeydown);
    document.removeEventListener("focusin", trackStableFocus);
    unsubscribeEvent?.();
    unsubscribeBinary?.();
    unsubscribeEnvironment?.();
    unsubscribeFileDrag?.();
    unsubscribeFileDrop?.();
    window.removeEventListener("message", onHostDragMessage);
    // 输出管线（写节流 flush + 在途写入作废 + 背压门复位）按原语义一次收口。
    disposeOutputPipeline();
    detachHighlightRender();
    detachActionLinks();
    detachGutterListeners();
    // 实例持有的 timer/observer/parser/输入与铃铛 handler 连同终端本体一起释放。
    disposeTerminalInstance();
    for (const waiter of uploadAckWaiters.values()) {
      window.clearTimeout(waiter.timer);
      waiter.reject(new Error(t("errors.workbenchDetached")));
    }
    for (const waiter of downloadChunkWaiters.values()) {
      window.clearTimeout(waiter.timer);
      waiter.reject(new Error(t("errors.workbenchDetached")));
    }
  }

  return {
    initialize,
    findReattachSession,
    closeOrphanSplitSessions,
    reattachProtocolSession,
    syncMcpSettingsMirror,
    mount,
    unmount,
  };
}
