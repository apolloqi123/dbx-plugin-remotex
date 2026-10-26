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
import { registerDockerLogStream, type DockerLogEvent } from "../lib/dockerLogEvents";
import { dockerEngineParams, loadDockerEngineSettings } from "../lib/dockerEngine";

export type DockerLogStreamStatus = "idle" | "starting" | "streaming" | "ended" | "error";

export const DEFAULT_LOG_TAIL = 200;

export interface DockerLogStreamOptions {
  /** 工作台绑定的 SSH 会话；缺省时按 target:"local" 查询本机 daemon。 */
  sessionId?: () => string | undefined;
  /** 引擎设置（cli/socket/host）按连接持久化的键。 */
  connectionId?: () => string | undefined;
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
  const readConnectionId = options.connectionId ?? (() => undefined);

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

  function startPayload(current: StreamTarget): Record<string, unknown> {
    const sessionId = readSessionId();
    const connectionId = readConnectionId() || "";
    return {
      ...(sessionId ? { sessionId } : { target: "local" as const }),
      containerId: current.containerId,
      tail: current.tail,
      ...(current.sudo ? { sudo: true } : {}),
      ...dockerEngineParams(loadDockerEngineSettings(connectionId)),
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
    target = {
      containerId,
      tail: normalizeLogTail(conditions.tail),
      sudo: conditions.sudo === true,
    };
    await stop();
    const generationAtStart = ++generation;
    status.value = "starting";
    error.value = "";
    needsSudo.value = false;
    exitCode.value = null;
    setFollow(true);

    let streamId = "";
    try {
      const response = await rpc<{ streamId?: unknown }>("docker/logs/start", startPayload(target));
      streamId = typeof response?.streamId === "string" ? response.streamId : "";
    } catch (cause) {
      if (generationAtStart !== generation) return;
      status.value = "error";
      error.value = errorMessage(cause);
      return;
    }
    if (generationAtStart !== generation) {
      // start 在途期间被 stop/新 start 取代：后端已建好的流必须立即收掉。
      if (streamId) void rpc("docker/logs/stop", { streamId }).catch(() => undefined);
      return;
    }
    if (!streamId) {
      status.value = "error";
      error.value = "docker/logs/start returned no streamId";
      return;
    }
    currentStreamId = streamId;
    detachHandler = registerDockerLogStream(streamId, handleEvent);
    status.value = "streaming";
  }

  /** 停止并注销；幂等（无流时不发 RPC）。 */
  async function stop(): Promise<void> {
    generation += 1;
    detach();
    const streamId = currentStreamId;
    currentStreamId = "";
    if (status.value === "starting" || status.value === "streaming") status.value = "idle";
    if (!streamId) return;
    try {
      await rpc("docker/logs/stop", { streamId });
    } catch {
      // 收流尽力而为：会话已断开/流已被 TTL 收口时后端返回错误也无妨。
    }
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
