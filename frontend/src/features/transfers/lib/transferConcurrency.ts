/** Run `run` for every item with at most `limit` in flight.
 *
 * A worker starts the next item as soon as it finishes one; the first worker
 * rejection stops the remaining queue via `Promise.all`, while already-running
 * items are awaited by their own worker. Callers that want per-item isolation
 * should catch inside `run`. */
export async function runLimited<T>(
  items: readonly T[],
  limit: number,
  run: (item: T) => Promise<void>,
): Promise<void> {
  if (!items.length) return;
  const width = Math.min(Math.max(1, Math.floor(limit)), items.length);
  let cursor = 0;
  let failed = false;
  await Promise.all(
    Array.from({ length: width }, async () => {
      for (;;) {
        if (failed) return;
        const index = cursor;
        cursor += 1;
        if (index >= items.length) return;
        try {
          await run(items[index]!);
        } catch (cause) {
          failed = true;
          throw cause;
        }
      }
    }),
  );
}

/** Batch download width from the user settings and the sidecar's per-session
 * depth. Compat mode is serial on the server; an "ask every time" save dialog
 * is single-instance, so both force 1. Otherwise leave room for transfers
 * that already occupy the session and let the server's depth gate be the
 * authority for the remainder. */
export function downloadBatchLimit(input: {
  compat: boolean;
  askDirectory: boolean;
  concurrency: number;
  maxActive: number;
  active: number;
}): number {
  if (input.compat || input.askDirectory) return 1;
  const available = Math.max(0, input.maxActive - input.active);
  return Math.max(1, Math.min(input.concurrency, available || 1));
}
