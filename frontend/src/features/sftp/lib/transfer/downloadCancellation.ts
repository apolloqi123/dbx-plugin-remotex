/** Races an in-flight download RPC against the workbench's cancellation set.
 *
 * `saveToLocal`/tree downloads intentionally have no binary-frame waiter, so a
 * cancel would otherwise only be noticed when the current `next` RPC settles
 * (one chunk, up to the request timeout). Polling the shared set keeps cancel
 * latency at ~100ms without adding another cross-composable waiter channel. */
export function raceCancelled<T>(
  taskId: string,
  promise: Promise<T>,
  isCancelled: (taskId: string) => boolean,
): Promise<T> {
  let timer: number | undefined;
  const cancel = new Promise<never>((_, reject) => {
    timer = window.setInterval(() => {
      if (!isCancelled(taskId)) return;
      if (timer !== undefined) window.clearInterval(timer);
      timer = undefined;
      reject(new Error("transfer cancelled"));
    }, 100);
  });
  return Promise.race([promise, cancel]).finally(() => {
    if (timer !== undefined) window.clearInterval(timer);
  });
}
