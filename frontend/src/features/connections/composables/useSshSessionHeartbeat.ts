/**
 * 工作台会话心跳。
 *
 * 宿主在编辑其它连接、重建工作台时会发延迟 SSH 拆除（connection/disconnect、
 * workbench/close、ssh/session/close）。sidecar 会先挂起这些通知等 webview
 * 回来；本心跳在会话仍存活时定期续期，让同一条宽限窗口一直向后推，从而不会
 * 落到已显示的终端上。真实关闭页签后 webview 消失、心跳停止，挂起的拆除按
 * 宽限落地，不会遗留孤儿 PTY。
 */
export const SSH_SESSION_HEARTBEAT_INTERVAL_MS = 5_000;

export interface SshSessionHeartbeatTarget {
  sessionId: string;
  connectionId: string;
  workbenchId: string;
}

/** Starts the ping loop; returns a stop function (tests / teardown). */
export function startSshSessionHeartbeat(
  readTarget: () => SshSessionHeartbeatTarget | null,
  intervalMs = SSH_SESSION_HEARTBEAT_INTERVAL_MS,
): () => void {
  const timer = window.setInterval(() => {
    const target = readTarget();
    if (!target?.sessionId) return;
    // Fire-and-forget: a lost beat is harmless (the next one is 5s later and
    // the grace is 20s), and skipping the response round-trip keeps the
    // background-tab cost to one notification instead of an RPC.
    void window.dbxPlugin.notify("ssh/session/alive", target).catch(() => undefined);
  }, intervalMs);
  return () => window.clearInterval(timer);
}
