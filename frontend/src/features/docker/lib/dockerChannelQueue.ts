// Container-manager channel queue: every short-lived SSH operation of one
// connection (list polling, file list/stat, log start/stop, terminal
// open/close) runs through a single FIFO chain. The server caps concurrent
// sessions per connection (`MaxSessions`); when several panes switch at the
// same time they used to open channels in parallel, overshoot the cap and
// surface `AdministrativelyProhibited` even though every channel closes
// correctly. Serialising the operations keeps the transient peak at one
// extra channel while still letting log/terminal sessions stay open.

const chains = new Map<string, Promise<unknown>>();

/** One queue per SSH transport; local targets share a queue per connection. */
export function dockerChannelKey(sessionId: string | undefined, connectionId?: string): string {
  return sessionId ? `ssh:${sessionId}` : `local:${connectionId ?? "default"}`;
}

/** Drops every queued chain (session teardown / test isolation). */
export function resetDockerChannelQueues(): void {
  chains.clear();
}

/** Runs `task` after the previous task of the same key settled. */
export function runDockerChannel<T>(key: string, task: () => Promise<T>): Promise<T> {
  const previous = chains.get(key) ?? Promise.resolve();
  const run = previous.then(task, task);
  const tail = run.then(
    () => undefined,
    () => undefined,
  );
  chains.set(key, tail);
  void tail.then(() => {
    if (chains.get(key) === tail) chains.delete(key);
  });
  return run;
}
