// 容器日志真流式的客户端状态：start/stop/restart/sudo 重试、分片缓冲与结束态。
//
// 事件不在这里直接订阅：chunk/end 由 `usePluginEvents` 统一接收入口分派到
// `lib/dockerLogEvents.ts` 的处理器表，本 composable 只在拿到 streamId 后注册、
// 在 stop/结束时注销（stop 幂等，先摘处理器再发 RPC）。
//
// 纯逻辑（拼行/裁剪/follow 判定）在 `lib/dockerLogBuffer.ts`；`follow` 的切换
// 由 UI 层决定，这里只持有状态并在重置时回调 onFollowChange。
import { computed, onScopeDispose, ref, watch } from "vue";
import { rpc } from "../../../shared/lib/rpc";
import { dockerChannelKey, runDockerChannel } from "../lib/dockerChannelQueue";
import { LogBuffer } from "../lib/dockerLogBuffer";
import { clearPendingDockerLogEvents, registerDockerLogStream, type DockerLogEvent } from "../lib/dockerLogEvents";
import {
  closeDockerLogOwner,
  isDockerLogStreamRegistryClosed,
  registerDockerLogOwner,
  trackDockerLogStream,
} from "../lib/dockerLogStreamRegistry";
import { LOGS_TIMEOUT_MS } from "../lib/dockerPanelModel";
import { createDockerScopeOwner } from "../lib/dockerScopeOwner";

export type DockerLogStreamStatus = "idle" | "starting" | "streaming" | "ended" | "error";

export const DEFAULT_LOG_TAIL = 200;

/** 收流 RPC 的有界等待：流可能已被 TTL/断线收口，不能无限挂起。 */
const STOP_TIMEOUT_MS = 10_000;


export interface DockerLogStreamOptions {
  /** 工作台绑定的 SSH 会话；缺省时按 target:"local" 查询本机 daemon。 */
  sessionId?: () => string | undefined;
  /** 每次开流附带的引擎设置（cli/socket/host），与 list/files/exec 同口径。 */
  engineParams?: () => Record<string, string>;
  /** follow 由 UI 切换；这里只做变更通知（例如埋点/外部同步）。 */
  onFollowChange?: (follow: boolean) => void;
  /** 每个分片新完成的整行（增量渲染入口；不再整段 join/切分）。 */
  onLines?: (lines: readonly string[]) => void;
  /** 缓冲被清空（clear/切换容器前的复位）：UI 丢弃已渲染行。 */
  onReset?: () => void;
}

export interface DockerLogStartOptions {
  tail?: string | number;
  sudo?: boolean;
}

/** restart 只覆盖 tail/sudo，容器沿用当前目标。 */
export type DockerLogRestartOptions = DockerLogStartOptions;

/** tail 与 sidecar 同口径钳到 10–1000（每次读取最多 1000 行），非法输入回落默认 200。 */
export function normalizeLogTail(value: string | number | undefined): number {
  if (value === undefined || value === "") return DEFAULT_LOG_TAIL;
  const parsed = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(parsed)) return DEFAULT_LOG_TAIL;
  return Math.min(1000, Math.max(10, Math.round(parsed)));
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

interface StreamTarget {
  containerId: string;
  tail: number;
  sudo: boolean;
}

export function useDockerLogStream(options: DockerLogStreamOptions = {}) {
  const readSessionId = options.sessionId ?? (() => undefined);
  const owner = createDockerScopeOwner("log");
  const releaseOwner = registerDockerLogOwner(owner);

  const buffer = new LogBuffer();
  /** 每次追加/清空递增；text 改成惰性 computed，UI 不再每次分片整段 join。 */
  const textVersion = ref(0);
  const text = computed(() => {
    void textVersion.value;
    return buffer.text();
  });
  const hasOutput = computed(() => {
    void textVersion.value;
    return buffer.lineCount > 0;
  });
  const truncated = ref(false);
  const lines = computed(() => (text.value ? text.value.split("\n") : []));
  const follow = ref(true);
  const status = ref<DockerLogStreamStatus>("idle");
  const error = ref("");
  const needsSudo = ref(false);
  const exitCode = ref<number | null>(null);

  /** 当前目标；restart/retrySudo 复用它（tail/sudo 可被覆盖）。 */
  let target: StreamTarget | undefined;
  let currentStreamId = "";
  let detachHandler: (() => void) | undefined;
  let untrackStream: (() => void) | undefined;
  /** 启动代际：stop 或新 start 会让仍在途的 start 响应作废并立即收流。 */
  let generation = 0;
  /** 启动串行链：同一时刻最多一个 stop/start RPC 在途，快速切容器只保留最新目标。 */
  let startChain: Promise<void> = Promise.resolve();

  watch(follow, (value) => options.onFollowChange?.(value));

  function setFollow(value: boolean): void {
    if (follow.value !== value) follow.value = value;
  }

  function detach(): void {
    if (detachHandler) detachHandler();
    detachHandler = undefined;
  }

  function untrackCurrent(): void {
    untrackStream?.();
    untrackStream = undefined;
  }

  /** 摘掉处理器并交出当前流 id：注册新流前先收走任何既有流。 */
  function takeCurrentStream(): string {
    detach();
    untrackCurrent();
    const streamId = currentStreamId;
    currentStreamId = "";
    return streamId;
  }

  /** 尽力收流：会话已断开/流已被 TTL 收口时后端报错也无妨。 */
  async function stopStream(streamId: string): Promise<void> {
    // 这条流已不再被本实例接管：丢掉可能还在缓冲的早到事件，避免复用 id 回放。
    clearPendingDockerLogEvents(streamId);
    try {
      // stop 只取消既有流、不新开 SSH channel：不能排在 list/stats/files 的
      // 连接级 FIFO 后面，否则关闭日志页时远端 docker logs -f 还要多活几秒。
      await rpc("docker/logs/stop", { streamId }, { timeoutMs: STOP_TIMEOUT_MS });
    } catch {
      // 收流尽力而为。
    }
  }

  function startPayload(current: StreamTarget): Record<string, unknown> {
    const sessionId = readSessionId();
    return {
      ...(sessionId ? { sessionId } : { target: "local" as const }),
      owner,
      containerId: current.containerId,
      tail: current.tail,
      ...(current.sudo ? { sudo: true } : {}),
      ...(options.engineParams?.() ?? {}),
    };
  }

  function handleEvent(event: DockerLogEvent): void {
    if (event.method === "docker/logs/chunk") {
      const chunk = typeof event.params.text === "string" ? event.params.text : "";
      if (!chunk) return;
      // 只把新完成的整行交给 UI 增量渲染：不再每次分片 join 整段缓冲
      // （万行日志下 join+split 是 O(总字节) 的主线程热点）。
      const added = buffer.append(chunk);
      truncated.value = buffer.truncated;
      if (added.length) {
        textVersion.value += 1;
        options.onLines?.(added);
      }
      return;
    }
    if (event.method !== "docker/logs/end") return;
    const exitCodeValue = typeof event.params.exitCode === "number" ? event.params.exitCode : null;
    const message = typeof event.params.error === "string" ? event.params.error : "";
    const sudo = event.params.needsSudo === true;
    detach();
    untrackCurrent();
    currentStreamId = "";
    exitCode.value = exitCodeValue;
    needsSudo.value = sudo;
    if (message && message !== "cancelled") {
      error.value = message;
      status.value = "error";
    } else {
      error.value = "";
      status.value = "ended";
    }
  }

  /** 启动一条新流：先收掉旧的；旧缓冲保留（清空由调用方显式决定）。 */
  function start(containerId: string, conditions: DockerLogStartOptions = {}): Promise<void> {
    // 代际与目标必须在入口同步快照：await 期间的 stop/新 start 会让本次启动作废，
    // 且 payload 不能等到 await 之后再读可变的 target。
    const token = ++generation;
    const next: StreamTarget = {
      containerId,
      tail: normalizeLogTail(conditions.tail),
      sudo: conditions.sudo === true,
    };
    target = next;
    status.value = "starting";
    error.value = "";
    needsSudo.value = false;
    exitCode.value = null;
    setFollow(true);
    // 串行执行：A→B→C 三次快速切换只保留 C，前两次在链上看到代际过期直接退出，
    // 避免并发 stop/start 把 SSH channel 瞬时堆到服务器 MaxSessions 上限。
    startChain = startChain.then(() => runStart(token, next)).catch(() => undefined);
    return startChain;
  }

  async function runStart(token: number, next: StreamTarget): Promise<void> {
    if (token !== generation) return;
    // 先收旧流；不调用 stop()，否则它递增代际会把本次 token 立刻作废。
    const previousStreamId = takeCurrentStream();
    if (previousStreamId) await stopStream(previousStreamId);
    if (token !== generation) return;

    let streamId = "";
    try {
      // 与 stop/list/files/terminal 共用同一连接队列：切换容器时不让多个
      // channel 同时开，避免瞬时超过服务器 MaxSessions。
      const response = await runDockerChannel(dockerChannelKey(readSessionId()), () =>
        rpc<{ streamId?: unknown }>("docker/logs/start", startPayload(next), {
          timeoutMs: LOGS_TIMEOUT_MS,
        }),
      );
      streamId = typeof response?.streamId === "string" ? response.streamId : "";
    } catch (cause) {
      if (token !== generation) return;
      status.value = "error";
      error.value = errorMessage(cause);
      return;
    }
    if (token !== generation) {
      // start 在途期间被 stop/新 start 取代：后端已建好的流必须立即收掉。
      if (streamId) void stopStream(streamId);
      return;
    }
    if (!streamId) {
      status.value = "error";
      error.value = "docker/logs/start returned no streamId";
      return;
    }
    if (isDockerLogStreamRegistryClosed()) {
      // 工作台已进入关闭流程：响应回来时没人会再发 stop，这里立即收掉。
      void stopStream(streamId);
      return;
    }
    // 防御：注册前把任何残留的流收掉，保证同一时刻只有一条后端流。
    const leftoverStreamId = takeCurrentStream();
    if (leftoverStreamId && leftoverStreamId !== streamId) await stopStream(leftoverStreamId);
    currentStreamId = streamId;
    untrackStream = trackDockerLogStream(streamId);
    // register 会同步回放早到的 chunk/end：fast end 可能在返回前就改写状态并清空
    // currentStreamId，此时不能再把状态覆盖回 streaming（否则页面停在收流中）。
    const detach = registerDockerLogStream(streamId, handleEvent);
    if (currentStreamId !== streamId) {
      detach();
      return;
    }
    detachHandler = detach;
    status.value = "streaming";
  }

  /** 停止并注销；幂等（无流时不发 RPC）。 */
  async function stop(): Promise<void> {
    generation += 1;
    const streamId = takeCurrentStream();
    if (status.value === "starting" || status.value === "streaming") status.value = "idle";
    if (!streamId) return;
    await stopStream(streamId);
  }

  /** 保留容器与 tail/sudo，重新开流（tail 变更后由 UI 清空缓冲）。 */
  async function restart(overrides: DockerLogRestartOptions = {}): Promise<void> {
    if (!target) return;
    const next: StreamTarget = {
      containerId: target.containerId,
      tail: overrides.tail === undefined ? target.tail : normalizeLogTail(overrides.tail),
      sudo: overrides.sudo === undefined ? target.sudo : overrides.sudo,
    };
    await start(next.containerId, { tail: next.tail, sudo: next.sudo });
  }

  /** 权限不足后的显式提权重试（只读连接由 UI 禁用入口）。 */
  async function retrySudo(): Promise<void> {
    if (!target) return;
    await restart({ sudo: true });
  }

  /** 清空视图（不断流）。 */
  function clear(): void {
    buffer.clear();
    textVersion.value += 1;
    truncated.value = false;
    options.onReset?.();
  }

  onScopeDispose(() => {
    releaseOwner();
    void stop();
    // 关闭 scope：覆盖 start 还在途、拿不到 streamId 的窗口。后端对同一
    // owner 的迟到 start 直接拒绝（短期墓碑），不会留下无人认领的流。
    void closeDockerLogOwner(owner);
  });

  return {
    lines,
    text,
    hasOutput,
    truncated,
    follow,
    status,
    error,
    needsSudo,
    exitCode,
    start,
    stop,
    restart,
    retrySudo,
    clear,
  };
}
