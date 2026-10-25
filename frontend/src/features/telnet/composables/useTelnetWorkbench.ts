import { computed, nextTick, ref, type Ref } from "vue";
import { loadLastConnectParams } from "../../connections/lib/connectLastParams";
import type { TelnetConnectOptions } from "../components/TelnetConnectDialog.vue";

/** Telnet 帧流状态（useProtocolFrameBuffers 持有，在 App.vue 中装配于本
 * 组合式函数之后）：会话启停时复位 sequence/pending/resync 计数。 */
export interface TelnetFrameStreams {
  telnetLastSequence: Ref<number>;
  telnetPendingFrames: Map<number, { stream: number; data: Uint8Array }>;
  telnetReplayNoProgress: Ref<number>;
}

/** 连接上下文（host/port 直启 Telnet 用）。 */
export interface TelnetConnectionContext {
  host?: string;
  port?: number;
}

export interface TelnetWorkbenchOptions {
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  isDisposed: () => boolean;
  getTerminal: () => { focus(): void } | undefined;
  // 装配依赖（声明/装配在本组合式函数之后）：一律 getter 惰性接入；函数内按
  // 原名解构，迁出代码保持逐行原样。
  getConnectionId: () => Ref<string>;
  getWorkbenchId: () => Ref<string>;
  getRuntimeEndpoint: () => Ref<{ host?: string; port?: number }>;
  getConnection: () => Ref<TelnetConnectionContext>;
  getTelnetFrameStreams: () => TelnetFrameStreams;
  fitTerminalDimensions: () => { cols: number; rows: number };
  scheduleFit: () => void;
}

/** Telnet 会话生命周期（P2-3，与本地终端同款互斥与补发机制）：会话状态机/
 * 启停/连接驱动直启收口在本组合式函数。
 *
 * App.vue 只做装配：模板同名绑定经解构透出。帧流/连接上下文/xterm 实例均在
 * 本组合式函数之后声明或装配，故经 getter 惰性接入——各函数内按原名解构后，
 * 迁出代码（含注释、语句与顺序）保持逐行原样。 */
export function useTelnetWorkbench(options: TelnetWorkbenchOptions) {
  const {
    showError,
    isDisposed,
    getTerminal,
    getConnectionId,
    getWorkbenchId,
    getRuntimeEndpoint,
    getConnection,
    getTelnetFrameStreams,
    fitTerminalDimensions,
    scheduleFit,
  } = options;

// Telnet 会话（P2-3）：与 SSH/本地终端同款互斥展示。并入 localUiMode 后，
// 所有 SSH-only 工具栏分支对 Telnet 自动隐藏；Telnet 专属分支按 isTelnetMode
// 优先接在既有 localSession 分支前面。
const telnetSession = ref<{ sessionId: string; host: string; port: number } | null>(null);
const telnetDialogOpen = ref(false);
const telnetState = ref<"idle" | "connecting" | "running" | "closed">("idle");
const telnetError = ref("");
const isTelnetMode = computed(() => telnetSession.value !== null);
const telnetTarget = computed(() => (telnetSession.value ? `${telnetSession.value.host}:${telnetSession.value.port}` : ""));

// —— Telnet 会话生命周期（P2-3，与本地终端同款互斥与补发机制）——
// 退出态统一入口：error 为 null 表示 sidecar 未带原因（会话已被回收），
// 非空时在退出覆盖层展示（连接失败/对端断开）。

async function startTelnetSession(options: TelnetConnectOptions): Promise<boolean> {
  const connectionId = getConnectionId();
  const workbenchId = getWorkbenchId();
  const runtimeEndpoint = getRuntimeEndpoint();
  const { telnetLastSequence, telnetPendingFrames, telnetReplayNoProgress } = getTelnetFrameStreams();
  // 同一终端视图互斥：残留的 closed 会话先清场再开新连接。
  if (telnetSession.value && telnetState.value !== "closed") await closeTelnetSession();
  // 自动登录两种形态互斥：声明式（提示正则 + 凭据，sidecar 落内置默认正则）
  // 优先；否则走 Expect 规则 + 密文槽。凭据只进 start 载荷与发送计划，
  // sidecar 侧不落日志/事件。
  const autoLogin = options.declarative
    ? { declarative: options.declarative }
    : options.rules
      ? {
          rules: options.rules,
          secrets: [options.secret1 ?? "", options.secret2 ?? ""],
        }
      : undefined;
  try {
    const info = await window.dbxPlugin.invoke<{ sessionId: string; host: string; port: number }>("telnet/start", {
      connectionId: connectionId.value || undefined,
      workbenchId: workbenchId.value,
      host: options.host,
      port: options.port,
      runtimeHost: runtimeEndpoint.value.host || options.host,
      runtimePort: runtimeEndpoint.value.port || options.port,
      enterMode: options.enterMode,
      backspaceMode: options.backspaceMode,
      ...fitTerminalDimensions(),
      ...(autoLogin ? { autoLogin } : {}),
    });
    if (isDisposed()) {
      void window.dbxPlugin.invoke("telnet/close", { sessionId: info.sessionId }).catch(() => undefined);
      return false;
    }
    telnetSession.value = { sessionId: info.sessionId, host: info.host, port: info.port };
    telnetState.value = "connecting";
    telnetError.value = "";
    telnetLastSequence.value = 0;
    telnetPendingFrames.clear();
    telnetReplayNoProgress.value = 0;
    await nextTick();
    scheduleFit();
    getTerminal()?.focus();
    return true;
  } catch (cause) {
    showError(cause, "terminal");
    return false;
  }
}

async function closeTelnetSession() {
  const { telnetPendingFrames } = getTelnetFrameStreams();
  const sessionId = telnetSession.value?.sessionId;
  telnetSession.value = null;
  telnetState.value = "idle";
  telnetError.value = "";
  telnetPendingFrames.clear();
  if (!sessionId) return;
  await window.dbxPlugin.invoke("telnet/close", { sessionId }).catch(() => undefined);
  getTerminal()?.focus();
}

// —— 连接驱动的会话启动（对标 Tabby profile 打开）：非 SSH 连接从宿主
// 连接表单直启各自会话；参数 = 连接 host/port + 上次使用记忆的偏好。
// 凭据不随连接持久化，需要自动登录/VNC 密码时从工具栏弹窗进入；直启
// 失败回落弹窗（预填 host/port，可改后重试）。
async function startTelnetFromConnection(): Promise<boolean> {
  const connection = getConnection();
  const conn = connection.value;
  if (!conn.host) return false;
  const last = loadLastConnectParams<TelnetConnectOptions>("telnet-connect-last");
  return startTelnetSession({
    host: conn.host,
    port: conn.port && conn.port > 0 ? conn.port : 23,
    enterMode: last.enterMode === "cr" || last.enterMode === "lf" ? last.enterMode : "crlf",
    backspaceMode: last.backspaceMode === "ctrl_h" ? "ctrl_h" : "del",
  });
}

  return {
    telnetSession,
    telnetDialogOpen,
    telnetState,
    telnetError,
    isTelnetMode,
    telnetTarget,
    startTelnetSession,
    closeTelnetSession,
    startTelnetFromConnection,
  };
}
