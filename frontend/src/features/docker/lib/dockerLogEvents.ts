// docker/logs 事件的处理器表：事件经 `usePluginEvents` 的单一订阅入口进入，
// 按 streamId 分派给当前活跃的 useDockerLogStream 实例。放在独立 lib 里是因为
// 订阅入口（app 层）不能反向依赖 feature composable，而流实例需要按流过滤，
// 一个 Map 是最小契约。
//
// `docker/logs/start` 是 RPC 往返，chunk/end 可能先于响应到达（快流、快失败：
// 缺 CLI/容器/权限时 end 往往紧跟 start 发出）。没有处理器时不能直接丢弃，
// 否则整段输出丢失、fast end 还会让页面停在 streaming。这里为每个 streamId
// 保留一个有界短时缓冲，注册处理器时按到达顺序回放后清空。

export interface DockerLogEvent {
  method: string;
  params: Record<string, unknown>;
}

export type DockerLogHandler = (event: DockerLogEvent) => void;

/** 未注册流的待分派事件上限：超出后丢最旧的，保留最新的输出与 end。 */
export const PENDING_LOG_EVENT_LIMIT = 64;
/** 待分派事件的存活时间：start 往返之外的迟到事件不再回放。 */
export const PENDING_LOG_EVENT_TTL_MS = 5_000;

interface PendingEvents {
  events: DockerLogEvent[];
  /** 以首次缓冲时刻起算，避免持续写入无限延长存活期。 */
  expiresAt: number;
}

const handlers = new Map<string, DockerLogHandler>();
const pending = new Map<string, PendingEvents>();

/** 惰性清理过期条目：没有定时器，只在分派/注册时顺带扫描。 */
function pruneExpired(now: number): void {
  for (const [streamId, entry] of pending) {
    if (entry.expiresAt <= now) pending.delete(streamId);
  }
}

/**
 * 注册某条流的处理器，返回注销函数。同一 streamId 再次注册会替换旧处理器；
 * 旧注销函数随后调用不会摘掉新处理器（避免竞态的旧清理）。
 * 注册时回放该流在注册前到达且未过期的事件（严格按到达顺序）。
 */
export function registerDockerLogStream(streamId: string, handler: DockerLogHandler): () => void {
  const now = Date.now();
  pruneExpired(now);
  handlers.set(streamId, handler);
  const buffered = pending.get(streamId);
  pending.delete(streamId);
  if (buffered && buffered.expiresAt > now) {
    for (const event of buffered.events) handler(event);
  }
  return () => {
    if (handlers.get(streamId) === handler) handlers.delete(streamId);
  };
}

/**
 * 分派 chunk/end：缺少 streamId 的事件静默丢弃；有处理器直接交付，未知 id
 * 则进入有界短时缓冲，等待 `register` 回放。
 */
export function dispatchDockerLogEvent(method: string, params: Record<string, unknown>): void {
  const streamId = params.streamId;
  if (typeof streamId !== "string" || !streamId) return;
  const event: DockerLogEvent = { method, params };
  const handler = handlers.get(streamId);
  if (handler) {
    handler(event);
    return;
  }
  const now = Date.now();
  pruneExpired(now);
  let entry = pending.get(streamId);
  if (!entry) {
    entry = { events: [], expiresAt: now + PENDING_LOG_EVENT_TTL_MS };
    pending.set(streamId, entry);
  }
  entry.events.push(event);
  if (entry.events.length > PENDING_LOG_EVENT_LIMIT) entry.events.shift();
}

/**
 * 清空待分派缓冲（可只清某条流）：流被显式收掉时丢弃迟到事件，避免后端复用
 * streamId 时把旧输出回放给新流。测试也用它做隔离。
 */
export function clearPendingDockerLogEvents(streamId?: string): void {
  if (streamId === undefined) pending.clear();
  else pending.delete(streamId);
}
