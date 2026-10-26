// docker/logs 事件的处理器表：事件经 `usePluginEvents` 的单一订阅入口进入，
// 按 streamId 分派给当前活跃的 useDockerLogStream 实例。放在独立 lib 里是因为
// 订阅入口（app 层）不能反向依赖 feature composable，而流实例需要按流过滤，
// 一个 Map 是最小契约。

export interface DockerLogEvent {
  method: string;
  params: Record<string, unknown>;
}

export type DockerLogHandler = (event: DockerLogEvent) => void;

const handlers = new Map<string, DockerLogHandler>();

/**
 * 注册某条流的处理器，返回注销函数。同一 streamId 再次注册会替换旧处理器；
 * 旧注销函数随后调用不会摘掉新处理器（避免竞态的旧清理）。
 */
export function registerDockerLogStream(streamId: string, handler: DockerLogHandler): () => void {
  handlers.set(streamId, handler);
  return () => {
    if (handlers.get(streamId) === handler) handlers.delete(streamId);
  };
}

/** 分派 chunk/end：未注册或缺少 streamId 的事件静默丢弃。 */
export function dispatchDockerLogEvent(method: string, params: Record<string, unknown>): void {
  const streamId = params.streamId;
  if (typeof streamId !== "string" || !streamId) return;
  handlers.get(streamId)?.({ method, params });
}
