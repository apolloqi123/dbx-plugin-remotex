import { ref, watch, type Ref, onBeforeUnmount } from "vue";
import { describeReconnectCountdown, terminalReconnectDelay, TERMINAL_RECONNECT_DELAYS, type ReconnectCountdown } from "../../terminal/lib/behavior/terminalReconnect";

/** 传输断开重连梯子（有界退避）：pending/倒计时（250ms tick 供状态胶囊与
 * 横幅）/尝试计数。第一级先在现有连接上直接重开会话——单条 PTY 掉线不必让
 * 宿主断开并重建整条连接（那会连带杀掉同连接下其它标签的会话与 SFTP 通道）；
 * 只有直连落到 error 态（典型是宿主侧已更新凭据、sidecar 仍持旧配置）才请
 * 宿主按最新配置重放连接并再试一次。耗尽落到 disconnected 终态等待手动重连。
 * openSession/attachSession/cancelConnect/reconnectNow 经解构 ref 直接读写
 * 梯子状态（reconnectTimer.value/Attempt）。 */
export function useSessionReconnect(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  terminalState: Ref<string>;
  terminalError: Ref<string>;
  isDisposed: () => boolean;
  openSession: (forceNew?: boolean, bootRestore?: boolean, isRetry?: boolean, hostReopenAttempted?: boolean) => Promise<void>;
  requestHostReopenConnection: () => Promise<unknown>;
}) {
  const { t, terminalState, terminalError, isDisposed, openSession, requestHostReopenConnection } = options;

const reconnectTimer = ref(0);
const reconnectAttempt = ref(0);

const reconnectPending = ref(false);
// Pure-display reconnect countdown for the status pill: seconds until the
// next retry plus the progress through the current backoff delay.
const reconnectCountdown = ref<ReconnectCountdown | null>(null);
const reconnectNextAt = ref(0);
const reconnectDelayMs = ref(0);
const reconnectCountdownTimer = ref(0);
// Set the moment a backoff loop starts; consumed by afterSessionConnected to
// show the "connection restored" notice (with cwd context) only after a real
// reconnect, not on the initial connect.
const reconnectWasPending = ref(false);

watch(reconnectPending, (pending) => {
  if (pending) reconnectWasPending.value = true;
  if (reconnectCountdownTimer.value) {
    window.clearInterval(reconnectCountdownTimer.value);
    reconnectCountdownTimer.value = 0;
  }
  if (!pending) {
    reconnectCountdown.value = null;
    return;
  }
  const update = () => {
    reconnectCountdown.value = describeReconnectCountdown({
      pending: true,
      attempt: reconnectAttempt.value,
      nextAt: reconnectNextAt.value,
      now: Date.now(),
      delayMs: reconnectDelayMs.value,
    });
  };
  update();
  reconnectCountdownTimer.value = window.setInterval(update, 250);
});

/**
 * 梯子第一级：先在现有连接上直接重开会话。
 *
 * 只有这一级失败（落到 error 态）才请宿主按最新配置重放连接并再试一次——
 * 那条路径会先断开整条连接，连带杀掉同连接下其它标签的会话与 SFTP 通道，
 * 不应为单条 PTY 掉线付出这个代价。侧边栏改密码后凭据过期的场景仍被覆盖：
 * 旧凭据的 openSession 失败 → 宿主重放新配置 → 第二次成功。
 */
async function reconnectFirstAttempt() {
  try {
    await openSession();
  } catch {
    // openSession 自身把失败落成 error 态；这里只兜底异常不影响下面的判定。
  }
  if (isDisposed() || terminalState.value !== "error") return;
  await requestHostReopenConnection().catch(() => undefined);
  // hostReopenAttempted=true：本轮已经重放过连接，openSession 内部不再重复请求宿主。
  if (!isDisposed()) void openSession(false, false, false, true);
}

function scheduleSessionReconnect() {
  // 断开事件可能成对到达（sidecar 重启常伴双事件）：重复调度必须先撤旧
  // timer 再设新的——直接覆盖会让被顶掉的 setTimeout 仍到点触发 openSession，
  // 与新 timer 形成并发双开会话。
  window.clearTimeout(reconnectTimer.value);
  if (!isDisposed() && reconnectAttempt.value < TERMINAL_RECONNECT_DELAYS.length) {
    const delay = terminalReconnectDelay(reconnectAttempt.value++);
    terminalState.value = "connecting";
    reconnectPending.value = true;
    reconnectTimer.value = window.setTimeout(() => {
      if (isDisposed()) return;
      if (reconnectAttempt.value === 1) void reconnectFirstAttempt();
      else void openSession();
    }, delay);
    return;
  }
  terminalState.value = "disconnected";
  reconnectPending.value = false;
  terminalError.value = t("transportDisconnected");
}


  onBeforeUnmount(() => {
    window.clearTimeout(reconnectTimer.value);
    window.clearInterval(reconnectCountdownTimer.value);
  });

  return {
    reconnectPending,
    reconnectCountdown,
    reconnectWasPending,
    reconnectTimer,
    reconnectAttempt,
    reconnectCountdownTimer,
    reconnectNextAt,
    reconnectDelayMs,
    scheduleSessionReconnect,
  };
}
