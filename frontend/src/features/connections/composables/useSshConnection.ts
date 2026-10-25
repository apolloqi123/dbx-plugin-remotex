import { ref, type Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import {
  describeReconnectRestoredNotice,
  isConnectionInactiveError,
  shouldReattachTerminal,
  terminalReconnectDelay,
  TERMINAL_RECONNECT_DELAYS,
} from "../../terminal/lib/behavior/terminalReconnect";
import { classifyConnectError, connectErrorKey } from "../lib/connectError";
import { decideConnectRetry, isDuplicatedTransportUnavailableError } from "../lib/connectRetry";
import { reopenHostConnection } from "../lib/hostReopen";
import type { ConnectLog } from "../lib/connectLog";
import {
  createSessionTransportReuseState,
  fallbackToFreshTransport,
  markSessionTransportOpenSucceeded,
  sessionTransportOpenParams,
} from "../lib/sessionTransportReuse";
import type { SftpSoftError } from "../../sftp/composables/listing/useSftpNavigation";
import type { ModifiedPrompt, WatchRegistry } from "../../sftp/lib/editor/watchEdits";
import { randomUUID } from "../../../shared/lib/uuid";

export interface ReplayResult {
  frameCount: number;
  firstAvailableSequence: number;
  tailSequence: number;
  complete: boolean;
}

export interface SessionInfo {
  sessionId: string;
  connectionId: string;
  workbenchId?: string;
  connected: boolean;
  sequence: number;
  chunkSize: number;
  directoryTrackingSupported?: boolean;
  replay?: ReplayResult;
}

/** host-key 挑战应答所需的最小载荷（完整弹窗状态留在 App.vue）。 */
export interface HostKeyChallenge {
  challengeId: string;
  operationId: string;
}

/** 重连梯子（useSessionReconnect 持有，在本组合式函数之后装配）。 */
export interface SshReconnectState {
  reconnectPending: Ref<boolean>;
  reconnectAttempt: Ref<number>;
  reconnectNextAt: Ref<number>;
  reconnectDelayMs: Ref<number>;
  reconnectTimer: Ref<number>;
  reconnectWasPending: Ref<boolean>;
}

/** 连接成功后的 SFTP 侧效应（useSftpNavigation / useSftpTreeWorkspace 持有，
 *  均在本组合式函数之后装配）。 */
export interface SshSftpHooks {
  loadDirectory: (path: string, fromTerminal?: boolean, quiet?: boolean) => Promise<void>;
  refreshSftpHomePath: () => Promise<void>;
  setDirectoryTracking: (enabled: boolean) => Promise<void>;
  initializeSftpTree: () => void;
  resetSftpTree: () => void;
}

/** SSH 输出管线仍由 App.vue 持有（Task 16 再迁出）：会话启停只经这组读写口
 *  触及 sequence/pending/resync 计数，避免与本地/串口帧流互染。 */
export interface SshFrameStreams {
  setActiveTerminalSessionId: (sessionId: string) => void;
  getLastSequence: () => number;
  setLastSequence: (sequence: number) => void;
  pendingTerminalFrames: Map<number, { stream: number; data: Uint8Array }>;
  setReplayNoProgress: (value: number) => void;
}

export interface SshConnectionOptions {
  t: (key: string, values?: Record<string, string | number>) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  showNotice: (message: string) => void;
  connectLog: ConnectLog;
  connectCancelled: Ref<boolean>;
  connectSucceeded: Ref<boolean>;
  hostKeyPrompt: Ref<HostKeyChallenge | undefined>;
  rememberHostKey: Ref<boolean>;
  terminalState: Ref<"connecting" | "connected" | "disconnected" | "error">;
  terminalError: Ref<string>;
  session: Ref<SessionInfo | undefined>;
  connectionId: Readonly<Ref<string>>;
  workbenchId: Readonly<Ref<string>>;
  connectionProtocol: Readonly<Ref<"ssh" | "telnet" | "vnc" | "serial" | "rdp">>;
  storedConnectionProtocol: Ref<string>;
  directoryTrackingSupported: Ref<boolean | undefined>;
  sftpListingLoaded: Ref<boolean>;
  panelSurface: Readonly<Ref<boolean>>;
  hostContext: Ref<Record<string, unknown>>;
  followDirectory: Readonly<Ref<boolean>>;
  sftpAutoConnect: Readonly<Ref<boolean>>;
  currentPath: Readonly<Ref<string>>;
  terminalInputQueue: { reset: () => void };
  isDisposed: () => boolean;
  getTerminal: () => Terminal | undefined;
  createTerminal: () => void;
  fitTerminalDimensions: () => { cols: number; rows: number };
  scheduleFit: () => void;
  resetCommandMarker: () => void;
  initialState: () => { sessionId?: string };
  persistState: () => void;
  writeWorkbenchState: () => Promise<void>;
  loadRemoteShellHistory: (sessionId: string) => void | Promise<void>;
  restoreTransfers: () => void | Promise<void>;
  cancelZmodemUpload: () => void;
  teardownTrzsz: () => void;
  startTelnetFromConnection: () => Promise<boolean>;
  telnetDialogOpen: Ref<boolean>;
  startVncFromConnection: () => Promise<boolean>;
  vncDialogOpen: Ref<boolean>;
  startSerialFromConnection: () => Promise<boolean>;
  serialDialogOpen: Ref<boolean>;
  routeRdpConnection: () => Promise<void>;
  startLocalTerminal: (
    shellOverride?: string,
    connection?: { cwd?: string; env?: Array<{ name: string; value: string }> },
  ) => Promise<void>;
  // 装配依赖（声明/装配在本组合式函数之后）：一律 getter 惰性接入；函数内
  // 按原名解构，迁出代码保持逐行原样。
  getReconnectState: () => SshReconnectState;
  getAgentTerminalMode: () => { clearAgentPrompts: () => void; agentRunning: Ref<unknown> };
  getSftpHooks: () => SshSftpHooks;
  getSftpSoftError: () => Ref<SftpSoftError | null>;
  getExternalEditState: () => {
    activeExternalWatches: Ref<WatchRegistry>;
    watchModifiedQueue: Ref<ModifiedPrompt[]>;
  };
  rescanHighlightViewport: () => void;
  probeLocalCapabilities: () => void;
  frameStreams: SshFrameStreams;
}

const OPEN_RETRY_MAX = 3;

/** SSH 会话主干：协议路由查询、open/attach/关闭、重试退避、传输复用与降级、
 *  宿主重开（host reopen）与 host-key 挑战应答统一收口。
 *
 *  重连梯子（useSessionReconnect）、传输断开监听（useTerminalTransportWatch）、
 *  连接卡片（useConnectCard）都消费这里的 openSession/attachSession/
 *  closeSession，故本组合式函数先于它们装配、相关 ref 以 getter 惰性注入；
 *  连接成功后的 SFTP/编辑器侧效应同理。输出管线帧流仍是 App.vue 的 `let`，
 *  经 frameStreams 读写。模板同名绑定（resolveHostKey/inactiveWaiting）经
 *  解构透出。 */
export function useSshConnection(options: SshConnectionOptions) {
  const {
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
    isDisposed,
    getTerminal,
    createTerminal,
    fitTerminalDimensions,
    scheduleFit,
    resetCommandMarker,
    initialState,
    persistState,
    writeWorkbenchState,
    loadRemoteShellHistory,
    restoreTransfers,
    cancelZmodemUpload,
    teardownTrzsz,
    startTelnetFromConnection,
    telnetDialogOpen,
    startVncFromConnection,
    vncDialogOpen,
    startSerialFromConnection,
    serialDialogOpen,
    routeRdpConnection,
    startLocalTerminal,
    getReconnectState,
    getAgentTerminalMode,
    getSftpHooks,
    getSftpSoftError,
    getExternalEditState,
    rescanHighlightViewport,
    probeLocalCapabilities,
    frameStreams,
  } = options;

const {
  setActiveTerminalSessionId,
  getLastSequence,
  setLastSequence,
  pendingTerminalFrames,
  setReplayNoProgress,
} = frameStreams;

// 传输复用状态跨重试保留：宿主把已被占用的来源降级为普通登录时，同一
// sidecar 会话 id 可被后续 open 复用。boot 上下文落地后由 App.vue 经
// resetTransportReuseState 重新初始化。
let transportReuseState = createSessionTransportReuseState({});
let openRetryAttempt = 0;
// 手动重连撞上 "Connection is not active"（sidecar 注册表丢凭据）：卡片保持
// connecting 态并提示用户在 DBX 侧边栏重开连接，插件在有界窗口内自动轮等待宿主重放凭据。
const inactiveWaiting = ref(false);

/** 宿主上下文落地后按最新上下文重建传输复用状态（App.vue initialize 调用）。 */
function resetTransportReuseState(context: Record<string, unknown>) {
  transportReuseState = createSessionTransportReuseState(context);
}

// 宿主推给 webview 的 connection 上下文缺 external_config.protocol，向
// sidecar 查询 lifecycle 暂存的权威协议；宿主尚未推送该连接时返回空串。
// sidecar 现在对已登记的 SSH 连接也回答 "ssh"（backend/src/main.rs 的
// connection/protocol），所以 SSH 连接首个查询即可命中——这个窗口只用于吸收
// 恢复态标签更晚的凭据重推，以及非 SSH 协议的迟到推送：恢复态 4×250ms、常规
// 2×250ms。旧 sidecar 对 SSH 仍回空串，最多多等 0.75s，路由结果不变；真未命中
// 的连接落进既有重试梯子（每次重试都会重新查询，协议一到就路由）。
async function resolveStoredConnectionProtocol(connectionId: string, bootRestore: boolean): Promise<{ protocol: string; shell: string; cwd: string; env: Array<{ name: string; value: string }> }> {
  const attempts = bootRestore ? 4 : 2;
  const delayMs = 250;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      const result = await window.dbxPlugin.invoke<{ protocol?: string; shell?: string; cwd?: string; env?: Array<{ name?: unknown; value?: unknown }> }>("connection/protocol", { connectionId });
      const protocol = typeof result?.protocol === "string" ? result.protocol : "";
      if (protocol) {
        return {
          protocol,
          shell: typeof result.shell === "string" ? result.shell : "",
          // 连接表单的启动目录/环境变量（旧 sidecar 无键 → 空 = 未配置）。
          cwd: typeof result.cwd === "string" ? result.cwd.trim() : "",
          env: Array.isArray(result.env)
            ? result.env
                .filter((entry): entry is { name: string; value: string } => typeof entry === "object" && entry !== null && typeof (entry as { name?: unknown }).name === "string" && typeof (entry as { value?: unknown }).value === "string")
                .map((entry) => ({ name: entry.name, value: entry.value }))
            : [],
        };
      }
    } catch {
      // sidecar 未就绪/旧版 sidecar 无此方法：按未暂存处理，走推送协议。
      return { protocol: "", shell: "", cwd: "", env: [] };
    }
    if (attempt + 1 < attempts) await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
  return { protocol: "", shell: "", cwd: "", env: [] };
}

async function openSession(forceNew = false, bootRestore = false, isRetry = false, hostReopenAttempted = false) {
  if (!connectionId.value || !workbenchId.value) return;
  const { reconnectTimer, reconnectAttempt, reconnectPending } = getReconnectState();
  const { clearAgentPrompts, agentRunning } = getAgentTerminalMode();
  // 协议路由（对标 Tabby profile）：Telnet/VNC 连接不建 SSH 会话，直接驱动
  // 各自的会话启动；启动失败回落连接弹窗（预填 host/port，可改后重试）。
  // 宿主推给 webview 的 connection 上下文缺 external_config.protocol（该字
  // 段只随 lifecycle connection/connect 走向 sidecar），SSH 形态先向 sidecar
  // 查询权威协议再路由——lifecycle 推送与 webview 创建存在固有竞态，用短重
  // 试吸收（恢复态标签的凭据重推更晚，窗口放宽到 ~5s，对齐 BOOT_RESTORE 节奏）。
  if (connectionProtocol.value === "ssh" && connectionId.value) {
    const stored = await resolveStoredConnectionProtocol(connectionId.value, bootRestore);
    const storedProtocol = stored.protocol;
    // 权威协议写回响应式状态（storedConnectionProtocol）：路由之外的连接级
    // 门控（isRdpConnection）也以它为准，否则真实宿主下 RDP 连接的 SSH 入口
    // 整组漏出（宿主推送上下文缺 protocol，查询结果不落状态就只够当次路由）。
    if (storedProtocol) storedConnectionProtocol.value = storedProtocol;
    if (storedProtocol === "telnet") {
      if (!(await startTelnetFromConnection())) telnetDialogOpen.value = true;
      return;
    }
    if (storedProtocol === "vnc") {
      if (!(await startVncFromConnection())) vncDialogOpen.value = true;
      return;
    }
    if (storedProtocol === "serial") {
      if (!(await startSerialFromConnection())) serialDialogOpen.value = true;
      return;
    }
    if (storedProtocol === "rdp") {
      await routeRdpConnection();
      return;
    }
    if (storedProtocol === "local") {
      // 本地 Shell 连接（LOCAL_CONNECTION_PROVIDER_ID，见 shared/lib/pluginIds.ts）：无网络语义，直接在连接绑定的
      // 工作台里开本地终端；连接表单 shell/cwd/env 为空时逐级回落偏好/默认
      // 解析链（cwd/env 的合并优先级见 startLocalTerminal）。
      await startLocalTerminal(stored.shell.trim() || undefined, {
        cwd: stored.cwd || undefined,
        env: stored.env.length ? stored.env : undefined,
      });
      return;
    }
  }
  if (connectionProtocol.value === "telnet") {
    if (!(await startTelnetFromConnection())) telnetDialogOpen.value = true;
    return;
  }
  if (connectionProtocol.value === "vnc") {
    if (!(await startVncFromConnection())) vncDialogOpen.value = true;
    return;
  }
  // M32-B：serial/rdp 连接记录与 telnet/vnc 同模式路由——配置驱动会话启动，
  // 缺字段/启动失败回落各自连接表单（对话框自带上次参数预填）。
  if (connectionProtocol.value === "serial") {
    if (!(await startSerialFromConnection())) serialDialogOpen.value = true;
    return;
  }
  if (connectionProtocol.value === "rdp") {
    await routeRdpConnection();
    return;
  }
  window.clearTimeout(reconnectTimer.value);
  reconnectAttempt.value = 0;
  // Boot-time tab restore can race the host's plugin activation and fail the
  // very first ssh/session/open; a bounded retry self-heals the restored
  // terminal instead of parking it on a manual reconnect button.
  // P1-1：重试计数只在"新入口"（用户动作 / 断线重连 / 初次打开）归零；
  // 重试定时器重入时必须保留计数，否则 OPEN_RETRY_MAX 永远打不满，
  // 认证失败等秒级永久错误会无限重试、错误文案永不呈现。
  if (!isRetry) openRetryAttempt = 0;
  // 重试重入保留等待提示；新入口（用户动作 / 断线重连 / 初次打开）重置。
  if (!isRetry) inactiveWaiting.value = false;
  // A session opened over a stale one must not inherit a stuck ZMODEM
  // overlay (zmodemBusy would keep swallowing terminal input).
  cancelZmodemUpload();
  // 同理不继承上一个会话的 trzsz 传输占用（在途传输一并停掉）。
  teardownTrzsz();
  // 同理不继承上一个会话的 AI 审批队列 / 执行横幅（切换会话清空全部排队挑战）。
  clearAgentPrompts();
  agentRunning.value = undefined;
  terminalState.value = "connecting";
  terminalError.value = "";
  reconnectPending.value = false;
  terminalInputQueue.reset();
  resetCommandMarker();
  connectCancelled.value = false;
  connectSucceeded.value = false;
  connectLog.push("info", t("connectCard.log.attempt", { attempt: openRetryAttempt + 1 }));
  if (forceNew && session.value) await closeSession(false);
  createTerminal();
  // A retry is only worth it for fast failures (boot-restore races with
  // plugin activation). A real dial failure takes tens of seconds — retrying
  // those just turns one error into minutes of spinner.
  const attemptStarted = Date.now();
  const attemptTimeoutMs = openRetryAttempt === 0 ? 120_000 : 30_000;
  try {
    const requestedSessionId = randomUUID();
    setActiveTerminalSessionId(requestedSessionId);
    setLastSequence(0);
    pendingTerminalFrames.clear();
    setReplayNoProgress(0);
    const info = await window.dbxPlugin.invoke<SessionInfo>("ssh/session/open", {
      connectionId: connectionId.value,
      workbenchId: workbenchId.value,
      requestedSessionId,
      ...sessionTransportOpenParams(transportReuseState),
      // spawn 即真实面板尺寸，理由同 local/terminal/start（fitTerminalDimensions）。
      ...fitTerminalDimensions(),
    }, { timeoutMs: attemptTimeoutMs });
    // 用户取消后在途 open 仍可能成功（invoke 无法中止）：立即关闭这个孤儿
    // 会话并提前返回——不置 connected、不补发 replay，卡片停在已取消态。
    if (connectCancelled.value) {
      void window.dbxPlugin.invoke("ssh/session/close", { sessionId: info.sessionId }).catch(() => undefined);
      connectLog.push("warn", t("connectCard.log.orphanClosed"));
      return;
    }
    // The caller-provided id lets the binary listener accept first output before
    // the long-running open RPC resolves. The returned id remains authoritative.
    const requestedMatches = info.sessionId === requestedSessionId;
    markSessionTransportOpenSucceeded(transportReuseState, info.sessionId);
    setActiveTerminalSessionId(info.sessionId);
    session.value = info;
    directoryTrackingSupported.value = info.directoryTrackingSupported ?? true;
    if (!requestedMatches) {
      // Older sidecars ignore requestedSessionId, or the backend had to avoid a
      // collision. Discard frames captured under the speculative id and replay
      // the authoritative stream from its beginning.
      setLastSequence(0);
      pendingTerminalFrames.clear();
      setReplayNoProgress(0);
    }
    connectSucceeded.value = true;
    hostReopenFailed = false;
    inactiveWaiting.value = false;
    connectLog.push("info", t("connectCard.log.connected", { seconds: ((Date.now() - attemptStarted) / 1000).toFixed(1) }));
    const replay = await window.dbxPlugin.invoke<ReplayResult>("ssh/terminal/replay", {
      sessionId: info.sessionId,
      afterSequence: requestedMatches ? getLastSequence() : 0,
    });
    if (!replay.complete) throw new Error(t("sessionUnrecoverable"));
    connectSucceeded.value = false;
    terminalState.value = "connected";
    await afterSessionConnected();
    // WKWebView（macOS 宿主）在"卡片遮盖 → 终端显示"过渡后可能漏一帧重绘，
    // 高亮装饰停留在过渡前状态直到首次交互（Mac 用户反馈"连接成功后 IP 高亮
    // 闪烁、点一下就好"）。状态落定为 connected 后下一帧主动全量刷新并重扫
    // 高亮，等价于那次点击；对 Chromium 是无害的一次多余重绘。
    window.requestAnimationFrame(() => {
      const terminal = getTerminal();
      if (isDisposed() || !terminal || terminalState.value !== "connected") return;
      terminal.refresh(0, terminal.rows - 1);
      rescanHighlightViewport();
    });
  } catch (cause) {
    if (isDisposed()) return;
    connectSucceeded.value = false;
    // 用户已取消：不重试、不呈现错误，卡片停在已取消态等 Connect 重新发起。
    if (connectCancelled.value) return;
    // The selected source may close between opening the child workbench and
    // its first sidecar call. Downgrade once, explicitly, to a normal login;
    // subsequent failures follow the ordinary permanent-error policy.
    if (
      isDuplicatedTransportUnavailableError(cause)
      && fallbackToFreshTransport(transportReuseState)
    ) {
      connectLog.push("warn", t("connectCard.log.duplicateFallback"));
      await openSession(false, bootRestore, true);
      return;
    }
    const attemptMs = Date.now() - attemptStarted;
    // "Connection is not active"：sidecar 连接注册表还没有该连接。web 刷新后
    // 宿主进程不重启、不会重放 connection/connect（#202），boot 恢复与手动
    // 场景等价——首败都主动请求宿主按最新配置重开一次（幂等；webview 拿不到
    // 凭据，boot 态调用安全），成功后在有界窗口内轮询。宿主请求失败则立即
    // 呈现错误。认证 / host-key 拒绝是秒级永久错误，重试不可能自愈——跳过
    // 重试直接进 error 态，呈现 friendly 文案 + Reconnect 出口（P1-1）。
    // 决策细节见 connectRetry.ts。
    const inactive = isConnectionInactiveError(cause);
    // A live workbench can outlast its sidecar connection registration. Ask
    // the host to replay the vault-backed lifecycle once before polling for
    // the new registration. The webview never receives the credentials.
    if (inactive && openRetryAttempt === 0 && !hostReopenAttempted) {
      await requestHostReopenConnection();
    }
    // 预拨号面板的引导竞态：宿主在创建条目时已开始 connect，openSession 只
    // 是跑在了 connect 推送前面——用短间隔轮询等它落地，而不是 2s 退避梯子。
    const preconnect = bootRestore && panelSurface.value && hostContext.value.connectionPreconnected === true;
    const decision = decideConnectRetry({
      cause,
      attempt: openRetryAttempt,
      maxAttempts: OPEN_RETRY_MAX,
      attemptMs,
      inactive,
      bootRestore,
      preconnect,
      hostReopenFailed,
    });
    if (decision.kind === "retry") {
      openRetryAttempt = decision.attempt;
      // 只有手动入口的 inactive 轮询需要"请在侧边栏重开"提示；boot 恢复由宿主自动重放。
      inactiveWaiting.value = inactive && !bootRestore;
      terminalState.value = "connecting";
      connectLog.push("warn", t("connectCard.log.retry", { seconds: decision.delayMs / 1000 }));
      reconnectTimer.value = window.setTimeout(() => {
        if (!isDisposed()) void openSession(false, bootRestore, true);
      }, decision.delayMs);
      return;
    }
    inactiveWaiting.value = false;
    terminalState.value = "error";
    setActiveTerminalSessionId("");
    // 日志记录分类后的友好原因（与卡片错误行同源），未分类时保留原始错误串。
    const rawReason = cause instanceof Error ? cause.message : String(cause ?? "");
    const reasonKind = classifyConnectError(rawReason);
    connectLog.push("error", t("connectCard.log.failed", { reason: inactive ? t("connectionInactive") : reasonKind ? t(connectErrorKey(reasonKind)) : rawReason }));
    showError(inactive ? new Error(t("connectionInactive")) : cause, "terminal");
  }
}

async function attachSession(sessionId: string, retryReference: string = initialState().sessionId || "") {
  const { reconnectPending, reconnectAttempt, reconnectNextAt, reconnectDelayMs, reconnectTimer } = getReconnectState();
  terminalState.value = "connecting";
  terminalInputQueue.reset();
  connectCancelled.value = false;
  // attach（tab 恢复）不播成功过渡动画，直接进终端。
  connectSucceeded.value = false;
  setActiveTerminalSessionId(sessionId);
  try {
    const info = await window.dbxPlugin.invoke<SessionInfo>("ssh/session/attach", {
      connectionId: connectionId.value,
      workbenchId: workbenchId.value,
      afterSequence: getLastSequence(),
    }, { timeoutMs: 15_000 });
    if (info.sessionId !== sessionId) throw new Error(t("errors.sessionChanged"));
    session.value = info;
    terminalState.value = "connected";
    reconnectPending.value = false;
    if (info.replay && !info.replay.complete) {
      terminalState.value = "error";
      terminalError.value = t("sessionUnrecoverable");
      return;
    }
    reconnectAttempt.value = 0;
    await afterSessionConnected();
  } catch (cause) {
    if (!shouldReattachTerminal({ disposed: isDisposed(), state: terminalState.value, expectedSessionId: sessionId, currentSessionId: retryReference })) {
      terminalState.value = "error";
      reconnectPending.value = false;
      setActiveTerminalSessionId("");
      showError(cause, "terminal");
      return;
    }
    const delay = terminalReconnectDelay(reconnectAttempt.value++);
    // Once the backoff ladder is exhausted, the stored session is gone for
    // good (e.g. the app was killed while the tab was open): re-attaching a
    // dead session id can never succeed, so fall back to a fresh open.
    if (reconnectAttempt.value > TERMINAL_RECONNECT_DELAYS.length) {
      reconnectAttempt.value = 0;
      reconnectPending.value = false;
      reconnectTimer.value = window.setTimeout(() => void openSession(true), delay);
      terminalError.value = t("reattachingTerminal");
      return;
    }
    reconnectNextAt.value = Date.now() + delay;
    reconnectDelayMs.value = delay;
    reconnectPending.value = true;
    reconnectTimer.value = window.setTimeout(() => void attachSession(sessionId), delay);
    terminalError.value = t("reattachingTerminal");
  }
}

async function afterSessionConnected() {
  const { loadDirectory, refreshSftpHomePath, setDirectoryTracking, initializeSftpTree } = getSftpHooks();
  const { reconnectWasPending } = getReconnectState();
  terminalError.value = "";
  getTerminal()?.focus();
  scheduleFit();
  await writeWorkbenchState();
  if (followDirectory.value) await setDirectoryTracking(true);
  if (sftpAutoConnect.value) {
    // #209 自动首拉走安静失败模式：失败落面板内软提示条，不弹全局错误横幅
    // （服务端停在堡垒机列表等场景的失败对用户是噪音）。home 探测与首拉并行，
    // 完成后初始化单栏树（目标 = 跟随 cwd / home）。
    await Promise.all([refreshSftpHomePath(), loadDirectory(currentPath.value, false, true), restoreTransfers()]);
    void initializeSftpTree();
  } else {
    // 自动连接已关：连接成功不主动碰 SFTP 子系统——列表首拉/主目录探测只在
    // 面板打开时由 initializeSftpTree 惰性补载（面板收起零请求）。
    await restoreTransfers();
    void initializeSftpTree();
  }
  // 「在外部编辑器中打开」菜单项的可用性依赖本机落盘能力，连接后即探测
  // （结果按工作台生命周期缓存，web/docker 为 false → 菜单项保持禁用）。
  void probeLocalCapabilities();
  // 远端 shell 历史导入(↑ history 面板内容对齐 shell history):异步拉取,
  // 不阻塞会话建立;失败静默(受限 shell/无历史文件时回落仅本会话采集)。
  const connectedSessionId = session.value?.sessionId;
  if (connectedSessionId) void loadRemoteShellHistory(connectedSessionId);
  // After an auto-reconnect succeeds, tell the user the session is back and
  // which working directory context it resumed with (pure-function chosen).
  if (reconnectWasPending.value) {
    reconnectWasPending.value = false;
    const restored = describeReconnectRestoredNotice({ wasReconnecting: true, path: currentPath.value });
    // Dock panel surface：目录提示属 SFTP/目录跟随域，面板里不弹。
    if (restored && !panelSurface.value) showNotice(t(restored.key, restored.values));
  }
}

async function closeSession(updateStatus = true) {
  const { reconnectPending } = getReconnectState();
  const { clearAgentPrompts, agentRunning } = getAgentTerminalMode();
  const { resetSftpTree } = getSftpHooks();
  const sftpSoftError = getSftpSoftError();
  const sessionId = session.value?.sessionId;
  session.value = undefined;
  setActiveTerminalSessionId("");
  clearAgentPrompts();
  agentRunning.value = undefined;
  // Closing mid-ZMODEM aborts the transfer silently instead of leaving the
  // busy overlay and the dead sentry attached to the workbench.
  cancelZmodemUpload();
  // trzsz 在途传输同样静默停止（死会话上的 sendToServer 会因无 sessionId 空转）。
  teardownTrzsz();
  pendingTerminalFrames.clear();
  setLastSequence(0);
  terminalInputQueue.reset();
  reconnectPending.value = false;
  resetCommandMarker();
  // 新会话的 SFTP 目录视为未加载：自动连接关闭时，重连后手动打开面板会重新补载。
  sftpListingLoaded.value = false;
  // 单栏树按会话重建（根 "/"），避免旧连接的 home/展开分支串到新会话。
  resetSftpTree();
  // #209：面板内软错误条（跟随/自动首拉失败态）不跨会话存留。
  sftpSoftError.value = null;
  // 外部编辑器监听挂在会话上：断开前先停掉（后端 ssh/session/close 兜底）。
  // M15 多文件：清空整个 watch 注册表与逐文件确认队列。
  if (sessionId) {
    const { activeExternalWatches, watchModifiedQueue } = getExternalEditState();
    void window.dbxPlugin.invoke("watch/stop-all", { sessionId }).catch(() => undefined);
    activeExternalWatches.value = {};
    watchModifiedQueue.value = [];
  }
  if (sessionId) await window.dbxPlugin.invoke("ssh/session/close", { sessionId }).catch(() => undefined);
  if (updateStatus) {
    terminalState.value = "disconnected";
    terminalError.value = t("disconnected");
  }
  persistState();
}

// 手动重连前请宿主按当前最新配置重开连接：连接在侧边栏被编辑（如改密码）后，
// 宿主会摘掉 connected 标记且不回推插件，sidecar 里存的凭据就此过期——不先
// 重开的话 openSession 只会拿旧凭据反复失败。宿主以打开连接的同款流程重新
// 下发配置（含 vault 里的最新凭据）。失败记录下来，避免后续对空连接表
// 反复发起 ssh/session/open；旧宿主仍可在连接表已有记录时继续拨号。
let hostReopenFailed = false;
async function requestHostReopenConnection(): Promise<boolean> {
  if (!connectionId.value) return false;
  const restored = await reopenHostConnection(window.dbxPlugin, connectionId.value);
  hostReopenFailed = !restored;
  return restored;
}

async function resolveHostKey(accept: boolean) {
  const prompt = hostKeyPrompt.value;
  if (!prompt) return;
  hostKeyPrompt.value = undefined;
  try {
    await window.dbxPlugin.invoke("connection/challenge/resolve", {
      challengeId: prompt.challengeId,
      operationId: prompt.operationId,
      accept,
      remember: accept && rememberHostKey.value,
    });
    connectLog.push("info", t("connectCard.log.hostKeyResolved"));
  } catch (cause) {
    showError(cause, "terminal");
  }
}

  return {
    resolveStoredConnectionProtocol,
    openSession,
    attachSession,
    afterSessionConnected,
    closeSession,
    requestHostReopenConnection,
    resolveHostKey,
    inactiveWaiting,
    resetTransportReuseState,
  };
}
