import { computed, nextTick, ref, watch, type Ref } from "vue";

/** 本地 shell 帧流状态（useProtocolFrameBuffers 持有，在 App.vue 中装配于本
 * 组合式函数之后）：会话启停时复位 sequence/pending/resync 计数并触发补发。 */
export interface LocalShellFrameStreams {
  localLastSequence: Ref<number>;
  localPendingFrames: Map<number, { stream: number; data: Uint8Array }>;
  localReplayNoProgress: Ref<number>;
  requestLocalReplay: (holeAt: number, resyncTarget: number | null) => void;
}

/** 其他互斥会话（VNC/RDP/串口/Telnet）的模式判定与关闭入口。 */
export interface OtherSessionModes {
  isRdpMode: Readonly<Ref<boolean>>;
  closeRdpSession: () => void | Promise<void>;
  isVncMode: Readonly<Ref<boolean>>;
  closeVncSession: () => void | Promise<void>;
  isSerialMode: Readonly<Ref<boolean>>;
  closeSerialSession: () => void | Promise<void>;
  isTelnetMode: Readonly<Ref<boolean>>;
  closeTelnetSession: () => void | Promise<void>;
}

/** 本地终端偏好/shell 选择（useLocalShellPreferences 持有）。 */
export interface LocalShellPreferenceRefs {
  localShellPref: Ref<string>;
  localShellIntegrationPref: Ref<boolean>;
  localShellCwdPref: Ref<string>;
  localShellEnvClean: Readonly<Ref<Array<{ name: string; value: string }>>>;
}

/** 会话下拉菜单（新建/复制/命令会话/本地终端/Shell 选项）状态。 */
export interface SessionMenuRefs {
  sessionMenuOpen: Ref<boolean>;
  sessionMenuShellOpen: Ref<boolean>;
  sessionMenuShellPickOpen: Ref<boolean>;
}

export interface LocalShellSessionOptions {
  t: (key: string) => string;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  terminalState: Ref<"connecting" | "connected" | "disconnected" | "error">;
  terminalError: Ref<string>;
  session: Ref<{ sessionId?: string } | undefined>;
  sftpPaneOpen: Ref<boolean>;
  isDisposed: () => boolean;
  getTerminal: () => { focus(): void } | undefined;
  // 装配依赖（声明/装配在本组合式函数之后，或在其他组合式函数内）：一律
  // getter/回调惰性接入；函数内按原名解构，迁出代码保持逐行原样。
  getWorkbenchId: () => Ref<string>;
  closeRightPanel: () => void;
  getBatchBarOpen: () => Ref<boolean>;
  getSessionMenus: () => SessionMenuRefs;
  getLocalShellPrefs: () => LocalShellPreferenceRefs;
  getLocalFrameStreams: () => LocalShellFrameStreams;
  resetCommandMarker: () => void;
  getReconnectPending: () => Ref<boolean>;
  getOtherModes: () => OtherSessionModes;
  settledTerminalDimensions: () => Promise<{ cols: number; rows: number }>;
  terminalCursorPosition: () => { cursorRow: number; cursorCol: number };
  scheduleFit: () => void;
  trackPendingInput: (data: string) => void;
  sendConfirmedPaste: (payload: string) => Promise<void>;
  closeSession: () => Promise<void>;
}

/** 本地终端会话生命周期（sidecar 所在机器的交互式登录 shell）：状态机/启停/
 * 互斥确认/A4 恢复外壳/重新接回统一收口在本组合式函数。
 *
 * App.vue 只做装配：模板同名绑定经解构透出。`useProtocolFrameBuffers` 的帧流、
 * `useLocalShellPreferences` 的偏好、其他协议会话与 xterm 实例均在本组合式
 * 函数之后声明/装配，故经 getter 惰性接入——各函数内按原名解构后，迁出的
 * 代码（含注释、语句与顺序）保持逐行原样。 */
export function useLocalShellSession(options: LocalShellSessionOptions) {
  const {
    t,
    showError,
    terminalState,
    terminalError,
    session,
    sftpPaneOpen,
    isDisposed,
    getTerminal,
    getWorkbenchId,
    closeRightPanel,
    getBatchBarOpen,
    getSessionMenus,
    getLocalShellPrefs,
    getLocalFrameStreams,
    resetCommandMarker,
    getReconnectPending,
    getOtherModes,
    settledTerminalDimensions,
    terminalCursorPosition,
    scheduleFit,
    trackPendingInput,
    sendConfirmedPaste,
    closeSession,
  } = options;

// —— 本地终端（sidecar 所在机器的交互式登录 shell）——
// 与 SSH 会话共用同一条渲染管线（乱序重组/补发/633 命令标记/输出节流），
// 同一时刻只展示一个会话：进入本地模式前先经确认关闭 SSH 会话。会话帧用
// 独立的 sequence/pending 状态，避免与 SSH 流互染。
const localSession = ref<{ sessionId: string; shell: string } | null>(null);
const localState = ref<"starting" | "running" | "exited">("exited");
const localExitCode = ref<number | null>(null);
const localOpenConfirmOpen = ref(false);
const isLocalMode = computed(() => localSession.value !== null);
// A4 restored shell (spec §7.6/§8.4): a restored local tab never auto-starts a shell; the exit
// overlay as the shell state waiting for an explicit start; cleared once a shell actually comes up (startLocalTerminal succeeds)
// cleared. localUiMode = "local-terminal UI state" (running session or restored shell); SSH-only toolbar actions gate on it,
// SSH-only toolbar actions and display branches gate on it, decoupled from session existence.
const localShellRestored = ref(false);
// 上次本地会话跟踪到的 cwd：重开时继承（VS Code 新终端继承工作区目录惯例）。
const localLastCwd = ref("");
// 本地终端最近命令（633;E 收集， newest-first，cap 20）：右键菜单"重跑"用。
// 注入关闭时无命令边界，本功能静默缺席。
const localRecentCommands = ref<string[]>([]);

// —— 本地终端生命周期 ——
// 退出态统一入口：exitCode 为 null 表示 sidecar 未上报（进程被杀/会话已回收），
// 覆盖层对 null 只显示"已退出"，有值时显示退出码。

async function startLocalTerminal(
  shellOverride?: string,
  connection?: { cwd?: string; env?: Array<{ name: string; value: string }> },
) {
  const { localShellPref, localShellIntegrationPref, localShellCwdPref, localShellEnvClean } = getLocalShellPrefs();
  const { localLastSequence, localPendingFrames, localReplayNoProgress } = getLocalFrameStreams();
  const workbenchId = getWorkbenchId();
  const batchBarOpen = getBatchBarOpen();
  if (localState.value === "starting" || isLocalMode.value) return;
  localState.value = "starting";
  // 本地模式与 SFTP 分栏/批量栏互斥（对齐 RDP 进入会话前的收口惯例 4348）：
  // 按钮层的 !localUiMode 门控只藏入口、不收已开的面板——SSH 工作台带分栏切
  // 本地、或恢复态 tab 从布局镜像水合出 sftpPaneOpen=true，都会留下对着空
  // 会话的幽灵分栏。persistState 由既有 watch 带上。
  if (sftpPaneOpen.value) closeRightPanel();
  if (batchBarOpen.value) batchBarOpen.value = false;
  try {
    const info = await window.dbxPlugin.invoke<{ sessionId: string; shell: string }>("local/terminal/start", {
      workbenchId: workbenchId.value,
      // spawn 尺寸必须等于面板稳定后的尺寸（单靠 fitTerminalDimensions 不够——
      // dock「+」新 tab 此时可能还没布局完，fit 回落 80×24，稍后的真实 fit 是
      // 一次跨几何 resize，ConPTY 整屏重绘＝提示符/横幅渲染两遍）。
      ...(await settledTerminalDimensions()),
      // ConPTY 以 PSEUDOCONSOLE_INHERIT_CURSOR 创建，开场 CSI 6n 问的是
      // 「终端光标在哪」，子进程从该位置开始输出——必须回真实光标位置；
      // 回视口尺寸等于宣称光标在最后一行，横幅会被从底部重画（顶部孤字/
      // 双提示符的来源）。
      ...terminalCursorPosition(),
      // Shell precedence: explicit dock choice > user preference > auto-detection.
      ...(shellOverride?.trim() ? { shell: shellOverride.trim() } : localShellPref.value ? { shell: localShellPref.value } : {}),
      ...(localShellIntegrationPref.value ? {} : { shellIntegration: false }),
      // cwd 优先级：重开继承上次跟踪目录 > 连接表单启动目录 > 偏好启动目录
      // > 不传（家目录）。目录可能已被删，sidecar 会回落家目录。
      ...(localLastCwd.value
        ? { cwd: localLastCwd.value }
        : connection?.cwd
          ? { cwd: connection.cwd }
          : localShellCwdPref.value
            ? { cwd: localShellCwdPref.value }
            : {}),
      // 环境变量：偏好与连接表单合并（同名时连接表单覆盖），sidecar 侧还有
      // 一道同规则净化兜底。
      ...(() => {
        const pairs = [...localShellEnvClean.value, ...(connection?.env ?? [])];
        return pairs.length ? { env: Object.fromEntries(pairs.map((pair) => [pair.name, pair.value])) } : {};
      })(),
      // spawn 是快路径，但宿主桥丢响应时 promise 永久 pending＝纯黑屏；
      // 与同文件其他交互调用一致地给超时，失败落入 catch 走 showError。
    }, { timeoutMs: 10_000 });
    if (isDisposed()) {
      void window.dbxPlugin.invoke("local/session/close", { sessionId: info.sessionId }, { timeoutMs: 10_000 }).catch(() => undefined);
      return;
    }
    localSession.value = { sessionId: info.sessionId, shell: info.shell };
    localState.value = "running";
    // The restored shell ends here: the session was explicitly restarted.
    localShellRestored.value = false;
    localExitCode.value = null;
    localLastSequence.value = 0;
    localPendingFrames.clear();
    localReplayNoProgress.value = 0;
    resetCommandMarker();
    await nextTick();
    scheduleFit();
    getTerminal()?.focus();
  } catch (cause) {
    localState.value = "exited";
    showError(cause, "terminal");
  }
}

async function closeLocalTerminal() {
  const { sessionMenuOpen, sessionMenuShellOpen, sessionMenuShellPickOpen } = getSessionMenus();
  const { localPendingFrames } = getLocalFrameStreams();
  const sessionId = localSession.value?.sessionId;
  localSession.value = null;
  localState.value = "exited";
  localPendingFrames.clear();
  localOpenConfirmOpen.value = false;
  sessionMenuOpen.value = false;
  sessionMenuShellOpen.value = false;
  sessionMenuShellPickOpen.value = false;
  if (!sessionId) return;
  await window.dbxPlugin.invoke("local/session/close", { sessionId }, { timeoutMs: 10_000 }).catch(() => undefined);
  getTerminal()?.focus();
}

// "Close" on a restored shell: there is no session to close — fall into the same disconnected state as an SSH restored tab
// (no connection replay, spec §7.6) and the exit overlay steps aside.
function dismissRestoredLocalShell() {
  localShellRestored.value = false;
  localState.value = "exited";
  terminalState.value = "disconnected";
  terminalError.value = t("restartDisconnected");
}

// Toolbar local-terminal button: running -> close; restored shell -> reopen directly (nothing to close, skipping
// SSH confirm flow); serial/telnet mode -> close that session; an SSH state walks the existing confirm flow.
function toggleLocalTerminal() {
  const { isRdpMode, closeRdpSession, isVncMode, closeVncSession, isSerialMode, closeSerialSession, isTelnetMode, closeTelnetSession } = getOtherModes();
  if (isRdpMode.value) {
    void closeRdpSession();
    return;
  }
  if (isVncMode.value) {
    void closeVncSession();
    return;
  }
  if (isSerialMode.value) {
    void closeSerialSession();
    return;
  }
  if (isTelnetMode.value) {
    void closeTelnetSession();
    return;
  }
  if (localShellRestored.value) {
    void restartLocalTerminal();
    return;
  }
  if (isLocalMode.value) {
    void closeLocalTerminal();
    return;
  }
  requestLocalTerminal();
}

// 右键菜单"重跑最近命令"：把命令写入本地 PTY（危险命令复用粘贴确认），
// 补回车立即执行；多行命令归一为回车分隔。
async function rerunLocalCommand(command: string) {
  if (!localSession.value) return;
  const payload = command.replace(/\r\n|\r|\n/g, "\r") + "\r";
  trackPendingInput(payload);
  await sendConfirmedPaste(payload);
}

async function restartLocalTerminal(shellOverride?: string) {
  await closeLocalTerminal();
  await startLocalTerminal(shellOverride);
}

// 一次性 shell override 的确认挂起：requestLocalTerminal 在 SSH 在连时先走
// 确认弹窗，所选 shell 存这里；确认/取消两路各自消费/清空（watch 兜底）。
let pendingLocalShellOverride = "";

// SSH 会话在连/连接中/重连中时先经确认关闭（本地模式与 SSH 会话互斥展示，
// connecting 途中放行会让在途 ssh/session/open 成功后与本地会话抢同一终端
// 视图），再开本地终端。
function requestLocalTerminal(shellOverride?: string) {
  const { isSerialMode, isTelnetMode, isVncMode, isRdpMode } = getOtherModes();
  const reconnectPending = getReconnectPending();
  // 串口/Telnet/VNC/RDP 会话占用终端视图时不开本地终端（互斥展示）。
  if (isSerialMode.value || isTelnetMode.value || isVncMode.value || isRdpMode.value) return;
  if (isLocalMode.value || localState.value === "starting") return;
  if (session.value || reconnectPending.value || terminalState.value === "connecting") {
    pendingLocalShellOverride = shellOverride ?? "";
    localOpenConfirmOpen.value = true;
    return;
  }
  void startLocalTerminal(shellOverride);
}

async function confirmLocalTerminal() {
  localOpenConfirmOpen.value = false;
  const shellOverride = pendingLocalShellOverride || undefined;
  pendingLocalShellOverride = "";
  await closeSession();
  await startLocalTerminal(shellOverride);
}

// 确认弹窗取消（按钮/X/外点/Esc 多路）统一清掉挂起的 override，避免残留到
// 下一次无 override 的确认；confirm 路径在本行同步消费后才会触发本 watch。
watch(localOpenConfirmOpen, (open) => {
  if (!open) pendingLocalShellOverride = "";
});

// webview 重建后接回 sidecar 里仍活着的本地 shell（workbench/close 才回收）。
async function reattachLocalSession(): Promise<boolean> {
  const workbenchId = getWorkbenchId();
  const { localLastSequence, localPendingFrames, requestLocalReplay } = getLocalFrameStreams();
  try {
    const result = await window.dbxPlugin.invoke<{ sessions?: Array<{ sessionId: string; workbenchId: string; shell: string }> }>(
      "local/session/list",
      {},
      { timeoutMs: 10_000 },
    );
    const match = (result?.sessions || []).find((session) => session.workbenchId === workbenchId.value);
    if (!match) return false;
    localSession.value = { sessionId: match.sessionId, shell: match.shell };
    localState.value = "running";
    localExitCode.value = null;
    localLastSequence.value = 0;
    localPendingFrames.clear();
    // 回放走统一补发路径（requestLocalReplay，自带超时与 resync）：环形缓冲
    // 淘汰过帧的活会话（累计输出 > 2MiB 必然发生）接回尾部继续，而不是被
    // !complete 误判成已退出——shell 在 sidecar 里还活着。
    requestLocalReplay(0, null);
    await nextTick();
    scheduleFit();
    return true;
  } catch {
    return false;
  }
}

  return {
    localSession,
    localState,
    localExitCode,
    localOpenConfirmOpen,
    isLocalMode,
    localShellRestored,
    localLastCwd,
    localRecentCommands,
    startLocalTerminal,
    closeLocalTerminal,
    dismissRestoredLocalShell,
    toggleLocalTerminal,
    rerunLocalCommand,
    restartLocalTerminal,
    requestLocalTerminal,
    confirmLocalTerminal,
    reattachLocalSession,
  };
}
