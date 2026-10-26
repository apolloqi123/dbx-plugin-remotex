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
import { LogBuffer } from "../lib/dockerLogBuffer";
import { clearPendingDockerLogEvents, registerDockerLogStream, type DockerLogEvent } from "../lib/dockerLogEvents";
import { LOGS_TIMEOUT_MS } from "../lib/dockerPanelModel";

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
}

export interface DockerLogStartOptions {
  tail?: string | number;
  sudo?: boolean;
}

/** restart 只覆盖 tail/sudo，容器沿用当前目标。 */
export type DockerLogRestartOptions = DockerLogStartOptions;

/** tail 与 sidecar 同口径钳到 10–2000，非法输入回落默认 200。 */
export function normalizeLogTail(value: string | number | undefined): number {
  if (value === undefined || value === "") return DEFAULT_LOG_TAIL;
  const parsed = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(parsed)) return DEFAULT_LOG_TAIL;
  return Math.min(2000, Math.max(10, Math.round(parsed)));
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

  const buffer = new LogBuffer();
  const text = ref("");
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
  /** 启动代际：stop 或新 start 会让仍在途的 start 响应作废并立即收流。 */
  let generation = 0;

  watch(follow, (value) => options.onFollowChange?.(value));

  function setFollow(value: boolean): void {
    if (follow.value !== value) follow.value = value;
  }

  function detach(): void {
    if (detachHandler) detachHandler();
    detachHandler = undefined;
  }

  /** 摘掉处理器并交出当前流 id：注册新流前先收走任何既有流。 */
  function takeCurrentStream(): string {
    detach();
    const streamId = currentStreamId;
    currentStreamId = "";
    return streamId;
  }

  /** 尽力收流：会话已断开/流已被 TTL 收口时后端报错也无妨。 */
  async function stopStream(streamId: string): Promise<void> {
    // 这条流已不再被本实例接管：丢掉可能还在缓冲的早到事件，避免复用 id 回放。
    clearPendingDockerLogEvents(streamId);
    try {
      await rpc("docker/logs/stop", { streamId }, { timeoutMs: STOP_TIMEOUT_MS });
    } catch {
      // 收流尽力而为。
    }
  }

  function startPayload(current: StreamTarget): Record<string, unknown> {
    const sessionId = readSessionId();
    return {
      ...(sessionId ? { sessionId } : { target: "local" as const }),
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
      buffer.append(chunk);
      // 后端的转发任务已按约 100ms/8KiB 合并分片，这里按缓冲重算文本即可
      // 保证与 clear/裁剪后的内容严格一致（行尾无多余空行）。
      text.value = buffer.text();
      truncated.value = buffer.truncated;
      return;
    }
    if (event.method !== "docker/logs/end") return;
    const exitCodeValue = typeof event.params.exitCode === "number" ? event.params.exitCode : null;
    const message = typeof event.params.error === "string" ? event.params.error : "";
    const sudo = event.params.needsSudo === true;
    detach();
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
  async function start(containerId: string, conditions: DockerLogStartOptions = {}): Promise<void> {
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

    // 先收旧流；不调用 stop()，否则它递增代际会把本次 token 立刻作废。
    const previousStreamId = takeCurrentStream();
    if (previousStreamId) await stopStream(previousStreamId);
    if (token !== generation) return;

    let streamId = "";
    try {
      const response = await rpc<{ streamId?: unknown }>("docker/logs/start", startPayload(next), {
        timeoutMs: LOGS_TIMEOUT_MS,
      });
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
    // 防御：注册前把任何残留的流收掉，保证同一时刻只有一条后端流。
    const leftoverStreamId = takeCurrentStream();
    if (leftoverStreamId && leftoverStreamId !== streamId) await stopStream(leftoverStreamId);
    currentStreamId = streamId;
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
    text.value = "";
    truncated.value = false;
  }

  onScopeDispose(() => {
    void stop();
  });

  return {
    lines,
    text,
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
