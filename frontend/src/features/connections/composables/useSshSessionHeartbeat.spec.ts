import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  SSH_SESSION_HEARTBEAT_INTERVAL_MS,
  startSshSessionHeartbeat,
} from "./useSshSessionHeartbeat";

const notify = vi.fn();

function installWindow() {
  vi.stubGlobal("window", {
    dbxPlugin: { notify },
    setInterval: (...args: Parameters<typeof setInterval>) => globalThis.setInterval(...args),
    clearInterval: (timer: ReturnType<typeof setInterval>) => globalThis.clearInterval(timer),
  });
}

describe("startSshSessionHeartbeat", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    notify.mockReset().mockResolvedValue({ success: true });
    installWindow();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("renews the live session's deferred teardown on every interval", () => {
    const stop = startSshSessionHeartbeat(() => ({
      sessionId: "session-1",
      connectionId: "connection-1",
      workbenchId: "workbench-1",
    }));

    vi.advanceTimersByTime(SSH_SESSION_HEARTBEAT_INTERVAL_MS * 2);

    expect(notify).toHaveBeenCalledTimes(2);
    expect(notify).toHaveBeenCalledWith("ssh/session/alive", {
      sessionId: "session-1",
      connectionId: "connection-1",
      workbenchId: "workbench-1",
    });

    stop();
    vi.advanceTimersByTime(SSH_SESSION_HEARTBEAT_INTERVAL_MS * 2);
    expect(notify).toHaveBeenCalledTimes(2);
  });

  it("stays silent while the workbench owns no live session", () => {
    const stop = startSshSessionHeartbeat(() => null);

    vi.advanceTimersByTime(SSH_SESSION_HEARTBEAT_INTERVAL_MS * 3);

    expect(notify).not.toHaveBeenCalled();
    stop();
  });

  it("swallows host errors instead of surfacing them as unhandled rejections", async () => {
    notify.mockRejectedValue(new Error("host gone"));
    const stop = startSshSessionHeartbeat(() => ({
      sessionId: "session-1",
      connectionId: "connection-1",
      workbenchId: "workbench-1",
    }));

    vi.advanceTimersByTime(SSH_SESSION_HEARTBEAT_INTERVAL_MS);
    await Promise.resolve();

    expect(notify).toHaveBeenCalledTimes(1);
    stop();
  });
});
