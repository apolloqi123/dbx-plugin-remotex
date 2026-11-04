// 本 webview 持有的远端日志流登记表：工作台关闭（宿主的 workbench/close）
// 发生在组件卸载之前，且宿主随后会直接拆除 webview；不能只依赖 Vue 的
// onScopeDispose 去发 `docker/logs/stop`。这里按 webview 记录活跃 streamId 与
// 日志页实例（owner），关闭钩子一次性收流并按 owner 收口，保证服务器上的
// `docker logs -f` 立即结束。
import { rpc } from "../../../shared/lib/rpc";

const activeStreamIds = new Set<string>();
const activeOwners = new Set<string>();
/** 工作台关闭后不再登记新流：迟到的 start 响应必须立刻收掉。 */
let closed = false;

/** 工作台是否已进入关闭流程（迟到的 start 响应据此立即收流）。 */
export function isDockerLogStreamRegistryClosed(): boolean {
  return closed;
}

/** 登记一个日志页实例（owner）；返回注销函数（幂等）。 */
export function registerDockerLogOwner(owner: string): () => void {
  if (closed) {
    // 收尾阶段才挂载的日志页不进入快照（onClose 已取过 owners）：立即按
    // owner 打墓碑，迟到的 start 在后端会被拒绝。
    void closeDockerLogOwner(owner);
    return () => undefined;
  }
  activeOwners.add(owner);
  return () => {
    activeOwners.delete(owner);
  };
}

/** 登记一条已建立的流；返回注销函数（幂等）。已关闭时立即收流。 */
export function trackDockerLogStream(streamId: string): () => void {
  if (closed) {
    void stopDockerLogStream(streamId);
    return () => undefined;
  }
  activeStreamIds.add(streamId);
  return () => {
    activeStreamIds.delete(streamId);
  };
}

function stopDockerLogStream(streamId: string): Promise<void> {
  // 有界等待：宿主桥卡死时也不能把工作台关闭流程拖住。
  return rpc("docker/logs/stop", { streamId }, { timeoutMs: 2_000 }).then(
    () => undefined,
    () => undefined,
  );
}

/** 收口一个日志页实例：按 owner 取消（覆盖还没拿到 streamId 的迟到 start）。 */
export function closeDockerLogOwner(owner: string): Promise<void> {
  return rpc("docker/logs/close", { owner }, { timeoutMs: 2_000 }).then(
    () => undefined,
    () => undefined,
  );
}

/**
 * 停止本 webview 登记的所有日志流与实例。后台 stop/close 幂等，已结束/未知
 * id 返回 `stopped:false` 而不是错误；桥已断开等失败也不阻断工作台关闭。
 */
export async function stopActiveDockerLogStreams(): Promise<void> {
  closed = true;
  const streamIds = [...activeStreamIds];
  const owners = [...activeOwners];
  activeStreamIds.clear();
  activeOwners.clear();
  if (!streamIds.length && !owners.length) return;
  // stop 只等 2s（宿主桥卡死时不拖住关闭）；close(owner) 立即取消、不等待
  // forwarder，是关闭路径的权威收口——即使 stop 超时/丢失也不会留下流。
  await Promise.allSettled([
    ...streamIds.map(stopDockerLogStream),
    ...owners.map(closeDockerLogOwner),
  ]);
}

/** 测试复位：生产流程里 closed 一旦置位 webview 就走完生命周期。 */
export function resetDockerLogStreamRegistry(): void {
  activeStreamIds.clear();
  activeOwners.clear();
  closed = false;
}
