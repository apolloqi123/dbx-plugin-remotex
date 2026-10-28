// 上传 spool 的 ack 等待队列：分片窗口（UPLOAD_SPOOL_WINDOW）允许同时有
// 多片在途，旧实现每任务只保存一个等待器，只能严格「发一片等一片确认」，
// 把主机读盘/桥接往返串在每一片后面。队列按 nextOffset 命中——二进制通道
// 保序，但 ack 事件可能被状态轮询/错误路径穿插。
export interface UploadAckWaiter {
  nextOffset: number;
  resolve: () => void;
  reject: (error: Error) => void;
  timer: number;
}

export type UploadAckWaiters = Map<string, UploadAckWaiter[]>;

/** 同时在途（已发送、未确认）的分片数。约束的是未确认帧的数量（≤3MiB
 * 载荷在途），payload 在 sendBinary 返回后即可回收，不随等待器常驻。 */
export const UPLOAD_SPOOL_WINDOW = 3;

export function enqueueUploadAck(
  waiters: UploadAckWaiters,
  taskId: string,
  waiter: UploadAckWaiter,
): void {
  const queue = waiters.get(taskId);
  if (queue) queue.push(waiter);
  else waiters.set(taskId, [waiter]);
}

/** 按累计 offset 释放等待器：sidecar 的 ack 携带的是「已收到总量」，所以
 * `nextOffset <= ack` 的分片都确定已落盘（也覆盖某条 ack 事件丢失、后一条
 * 才到达的情况）。至少释放一个返回 true。 */
export function resolveUploadAck(
  waiters: UploadAckWaiters,
  taskId: string,
  nextOffset: number,
): boolean {
  const queue = waiters.get(taskId);
  if (!queue?.length) return false;
  let resolved = false;
  for (let index = 0; index < queue.length; ) {
    const waiter = queue[index];
    if (waiter.nextOffset > nextOffset) {
      index += 1;
      continue;
    }
    queue.splice(index, 1);
    window.clearTimeout(waiter.timer);
    waiter.resolve();
    resolved = true;
  }
  if (!queue.length) waiters.delete(taskId);
  return resolved;
}

/** 按 offset 摘除等待器（超时兜底走状态查询确认已落盘时用），不 resolve。 */
export function removeUploadAck(
  waiters: UploadAckWaiters,
  taskId: string,
  nextOffset: number,
): UploadAckWaiter | undefined {
  const queue = waiters.get(taskId);
  if (!queue?.length) return undefined;
  const index = queue.findIndex((waiter) => waiter.nextOffset === nextOffset);
  if (index < 0) return undefined;
  const [removed] = queue.splice(index, 1);
  window.clearTimeout(removed.timer);
  if (!queue.length) waiters.delete(taskId);
  return removed;
}

/** 失败/取消收口：拒绝该任务所有在途等待器并清定时器（不留 30s 悬挂）。 */
export function rejectUploadAcks(
  waiters: UploadAckWaiters,
  taskId: string,
  error: Error,
): void {
  const queue = waiters.get(taskId);
  if (!queue?.length) return;
  waiters.delete(taskId);
  for (const waiter of queue) {
    window.clearTimeout(waiter.timer);
    waiter.reject(error);
  }
}
