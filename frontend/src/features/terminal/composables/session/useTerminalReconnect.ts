import type { Ref } from "vue";
import { useSessionReconnect } from "../../../connections/composables/useSessionReconnect";
import { useTerminalTransportWatch } from "./useTerminalTransportWatch";

type SessionRef = Readonly<Ref<{ sessionId: string } | undefined>>;

/**
 * 传输断开/会话被杀后的重连装配：有界退避梯子（useSessionReconnect）与浏览器/
 * 宿主失联探测（useTerminalTransportWatch）在同一次 setup 里装配，状态、调度、
 * 取消与倒计时经由本 composable 统一出口暴露给 App 与模板。
 *
 * - 退避延迟、重试条件、pending/attempt/nextAt/倒计时 tick 全部沿用
 *   useSessionReconnect 与 lib/behavior/terminalReconnect 的既有实现；
 * - 探测间隔/超时与 online/offline、visibilitychange 监听仍由
 *   useTerminalTransportWatch 持有，卸载时自行拆除；
 * - 手动重连入口（reconnectNow）的完整路径（清屏 → closeSession → 宿主重开 →
 *   openSession）由连接卡片 useConnectCard 持有，这里只接收并转发，保证 App
 *   只有一条重连出口。该回调仅在用户点击时求值，可安全引用后声明的实现。
 */
export function useTerminalReconnect(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  terminalState: Ref<string>;
  terminalError: Ref<string>;
  session: SessionRef;
  isDisposed: () => boolean;
  openSession: (forceNew?: boolean, bootRestore?: boolean, isRetry?: boolean, hostReopenAttempted?: boolean) => Promise<void>;
  requestHostReopenConnection: () => Promise<unknown>;
  resetInputQueue: () => void;
  probeTransport: (sessionId: string) => Promise<unknown>;
  canReachHost?: () => Promise<boolean>;
  attachSession: (sessionId: string, retryReference: string) => Promise<void>;
  reconnectNow: () => Promise<void>;
}) {
  const {
    reconnectPending,
    reconnectCountdown,
    reconnectWasPending,
    reconnectTimer,
    reconnectAttempt,
    reconnectCountdownTimer,
    reconnectNextAt,
    reconnectDelayMs,
    scheduleSessionReconnect,
  } = useSessionReconnect({
    t: options.t,
    terminalState: options.terminalState,
    terminalError: options.terminalError,
    isDisposed: options.isDisposed,
    openSession: options.openSession,
    requestHostReopenConnection: options.requestHostReopenConnection,
  });

  const terminalTransportWatch = useTerminalTransportWatch({
    t: options.t,
    terminalState: options.terminalState,
    terminalError: options.terminalError,
    session: options.session,
    resetInputQueue: options.resetInputQueue,
    probeTransport: options.probeTransport,
    canReachHost: options.canReachHost,
    attachSession: options.attachSession,
  });

  /** 卸载兜底：丢弃待触发的重连/重试定时器与倒计时 tick（与抽取前的卸载清理
   * 同一组语句，pending/attempt 状态保持原样）。 */
  function cancelReconnect() {
    window.clearTimeout(reconnectTimer.value);
    window.clearInterval(reconnectCountdownTimer.value);
  }

  return {
    terminalTransportWatch,
    reconnectPending,
    reconnectCountdown,
    reconnectWasPending,
    reconnectTimer,
    reconnectAttempt,
    reconnectCountdownTimer,
    reconnectNextAt,
    reconnectDelayMs,
    scheduleReconnect: scheduleSessionReconnect,
    cancelReconnect,
    reconnectNow: () => options.reconnectNow(),
  };
}
