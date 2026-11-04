// 容器监控数据面：页签可见且容器 running 时每 3s 轮询一次 `docker/stats`，
// 采样进环缓冲（前端保留 1 小时）；切页签/切容器/关面板立即停轮询，失败
// 三连即停并给出重试入口。时间段切换只过滤本地缓冲，不发新请求。
//
// 与日志流同口径：工作台绑定会话存在时走 SSH，否则回落 target:"local"；
// 引擎设置（cli/socket/host）随每次调用附带。
import { computed, onScopeDispose, ref, watch } from "vue";
import { rpc } from "../../../shared/lib/rpc";
import { dockerEngineParams, loadDockerEngineSettings } from "../lib/dockerEngine";
import { dockerChannelKey, runDockerChannel } from "../lib/dockerChannelQueue";
import { documentHidden, onDocumentVisibilityChange } from "../../../shared/lib/documentVisibility";
import {
  DEFAULT_MONITOR_RANGE_SECONDS,
  MONITOR_POLL_MS,
  pushStatsSample,
  rangePoints,
  sampleFromPayload,
  statsPoints,
  type ContainerStatsSample,
} from "../lib/containerMonitor";

export type ContainerMonitorStatus = "idle" | "loading" | "live" | "stopped" | "denied" | "error";

const STATS_TIMEOUT_MS = 12_000;
const MAX_CONSECUTIVE_FAILURES = 3;

export interface UseContainerStatsOptions {
  containerId: () => string;
  /** 工作台绑定的 SSH 会话；缺省时按 target:"local" 查询本机 daemon。 */
  sessionId?: () => string | undefined;
  connectionId?: () => string | undefined;
  /** 容器是否 running（停止时只展示停止态，不发请求）。 */
  running: () => boolean;
  /** 监控页签是否可见（隐藏即停轮询）。 */
  visible: () => boolean;
}

export function useContainerStats(options: UseContainerStatsOptions) {
  const readSessionId = options.sessionId ?? (() => undefined);
  const readConnectionId = options.connectionId ?? (() => undefined);

  const samples = ref<ContainerStatsSample[]>([]);
  const status = ref<ContainerMonitorStatus>("idle");
  const error = ref("");
  const rangeSeconds = ref(DEFAULT_MONITOR_RANGE_SECONDS);
  /** daemon 主机总核数（一次性查询，用于“用量 / 总量”）。 */
  const hostCpus = ref<number>();
  /** 容器 CPU 上限（核）；null/undefined = 未限制。 */
  const cpuLimitCores = ref<number>();

  let pollTimer: ReturnType<typeof setInterval> | undefined;
  /** CPU 总量按容器目标只查一次；切容器/重试时清空。 */
  let limitsKey = "";
  let polling = false;
  let consecutiveFailures = 0;
  let generation = 0;

  const points = computed(() => statsPoints(samples.value));
  const current = computed(() => points.value[points.value.length - 1]);
  const rangedPoints = computed(() => rangePoints(points.value, rangeSeconds.value));

  function stopPolling(): void {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = undefined;
  }

  function reset(): void {
    generation += 1;
    stopPolling();
    samples.value = [];
    error.value = "";
    consecutiveFailures = 0;
    status.value = "idle";
    hostCpus.value = undefined;
    cpuLimitCores.value = undefined;
    limitsKey = "";
  }

  /**
   * 一次性读取 CPU 总量：容器有上限用上限，否则用 daemon 主机核数。
   * 不进入 3s 轮询；失败时静默，只显示用量。
   */
  async function loadLimits(): Promise<void> {
    const containerId = options.containerId();
    if (!containerId) return;
    const sessionId = readSessionId();
    const key = `${containerId}\u0000${sessionId ?? ""}\u0000${readConnectionId() ?? ""}`;
    if (limitsKey === key) return;
    limitsKey = key;
    try {
      const params = {
        containerId,
        ...(sessionId ? { sessionId } : { target: "local" as const }),
        ...dockerEngineParams(loadDockerEngineSettings(readConnectionId() || "")),
      };
      const payload = (await runDockerChannel(
        dockerChannelKey(sessionId, readConnectionId()),
        () =>
          rpc<Record<string, unknown>>("docker/stats/limits", params, {
            timeoutMs: STATS_TIMEOUT_MS,
          }),
      )) as Record<string, unknown>;
      if (limitsKey !== key) return;
      const host = typeof payload.hostCpus === "number" && payload.hostCpus > 0
        ? payload.hostCpus
        : undefined;
      const limit = typeof payload.cpuLimitCores === "number" && payload.cpuLimitCores > 0
        ? payload.cpuLimitCores
        : undefined;
      hostCpus.value = host;
      cpuLimitCores.value = limit;
    } catch {
      // 总量是可选信息：失败只影响分母，不影响采样。
    }
  }

  async function poll(): Promise<void> {
    if (polling) return;
    const containerId = options.containerId();
    if (!containerId) return;
    const currentGeneration = generation;
    polling = true;
    try {
      const sessionId = readSessionId();
      const params = {
        containerId,
        ...(sessionId ? { sessionId } : { target: "local" as const }),
        ...dockerEngineParams(loadDockerEngineSettings(readConnectionId() || "")),
      };
      // 与 list/logs/files/terminal 共用连接级 FIFO：3s 采样不与其它 channel
      // 同时开，服务器 MaxSessions 较小时也不会瞬时顶满。
      const payload = (await runDockerChannel(
        dockerChannelKey(sessionId, readConnectionId()),
        () =>
          rpc<Record<string, unknown>>("docker/stats", params, {
            timeoutMs: STATS_TIMEOUT_MS,
          }),
      )) as Record<string, unknown>;
      if (currentGeneration !== generation) return;
      consecutiveFailures = 0;
      error.value = "";
      const sample = sampleFromPayload(payload, Date.now());
      if (sample) {
        samples.value = pushStatsSample(samples.value, sample);
        status.value = "live";
        return;
      }
      if (payload.needsSudo === true) {
        stopPolling();
        status.value = "denied";
        return;
      }
      if (payload.available === true && payload.running === false) {
        stopPolling();
        samples.value = [];
        status.value = "stopped";
        return;
      }
      consecutiveFailures += 1;
      status.value = "error";
      error.value = typeof payload.error === "string" ? payload.error : "";
      if (consecutiveFailures >= MAX_CONSECUTIVE_FAILURES) stopPolling();
    } catch (cause) {
      if (currentGeneration !== generation) return;
      consecutiveFailures += 1;
      status.value = "error";
      error.value = cause instanceof Error ? cause.message : String(cause);
      if (consecutiveFailures >= MAX_CONSECUTIVE_FAILURES) stopPolling();
    } finally {
      polling = false;
    }
  }

  function start(): void {
    stopPolling();
    if (!options.visible() || !options.running() || documentHidden()) {
      if (!options.running()) status.value = "stopped";
      return;
    }
    generation += 1;
    consecutiveFailures = 0;
    if (status.value !== "live") status.value = "loading";
    void poll();
    void loadLimits();
    pollTimer = setInterval(() => void poll(), MONITOR_POLL_MS);
  }

  /** 错误态/无权态的手动重试：清空计数后立即采样并恢复轮询。 */
  async function retry(): Promise<void> {
    reset();
    start();
  }

  // 容器/运行态变化：旧采样属于旧目标，清空后按当前可见性重开。字符串键
  // 避免列表轮询替换同 id 容器对象（status 文本变化）时误清空曲线。
  watch(
    () => `${options.containerId()}\u0000${options.running()}`,
    () => {
      reset();
      if (options.visible()) start();
    },
    { immediate: true, flush: "post" },
  );

  // 页签显隐只停/启轮询，保留已采集的 1 小时缓冲：在日志/文件/终端之间
  // 来回切换不应把监控曲线清空。
  watch(
    () => options.visible(),
    (visible) => {
      if (visible) start();
      else stopPolling();
    },
  );

  // 窗口不可见时暂停 3s 采样（后台不打扰 SSH/daemon）；回来立即补一拍。
  const offVisibility = onDocumentVisibilityChange((hidden) => {
    if (hidden) stopPolling();
    else if (options.visible() && options.running()) start();
  });

  onScopeDispose(() => {
    offVisibility();
    reset();
  });

  return {
    samples,
    hostCpus,
    cpuLimitCores,
    points,
    rangedPoints,
    current,
    status,
    error,
    rangeSeconds,
    retry,
  };
}
