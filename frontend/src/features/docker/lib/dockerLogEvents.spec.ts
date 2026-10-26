import { afterEach, describe, expect, it, vi } from "vitest";
import {
  clearPendingDockerLogEvents,
  dispatchDockerLogEvent,
  PENDING_LOG_EVENT_LIMIT,
  PENDING_LOG_EVENT_TTL_MS,
  registerDockerLogStream,
} from "./dockerLogEvents";

/** docker/logs/{chunk,end} 的处理器表：全局单例，测试必须自行注销。 */
const disposers: Array<() => void> = [];

function register(streamId: string, handler: Parameters<typeof registerDockerLogStream>[1]): () => void {
  const dispose = registerDockerLogStream(streamId, handler);
  disposers.push(dispose);
  return dispose;
}

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  clearPendingDockerLogEvents();
  vi.useRealTimers();
});

describe("docker log event routing", () => {
  it("dispatches chunk and end to the registered stream only", () => {
    const first = vi.fn();
    const second = vi.fn();
    register("stream-1", first);
    register("stream-2", second);

    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "hello" });
    expect(first).toHaveBeenCalledWith({ method: "docker/logs/chunk", params: { streamId: "stream-1", text: "hello" } });
    expect(second).not.toHaveBeenCalled();

    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-2", exitCode: 0 });
    expect(second).toHaveBeenCalledWith({ method: "docker/logs/end", params: { streamId: "stream-2", exitCode: 0 } });
    expect(first).toHaveBeenCalledTimes(1);
  });

  it("ignores events for unregistered or missing stream ids", () => {
    const handler = vi.fn();
    register("stream-1", handler);
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-9", text: "x" });
    dispatchDockerLogEvent("docker/logs/chunk", { text: "x" });
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: 42, text: "x" });
    expect(handler).not.toHaveBeenCalled();
  });

  it("stops routing after the unregister function runs", () => {
    const handler = vi.fn();
    const dispose = register("stream-1", handler);
    dispose();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "x" });
    expect(handler).not.toHaveBeenCalled();
  });

  it("does not let a stale disposer detach a replacement handler", () => {
    const stale = vi.fn();
    const current = vi.fn();
    const disposeStale = register("stream-1", stale);
    register("stream-1", current);
    disposeStale();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "x" });
    expect(stale).not.toHaveBeenCalled();
    expect(current).toHaveBeenCalledTimes(1);
  });

  it("replays chunk and end that arrive before the stream registers, in order", () => {
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-early", text: "first" });
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-early", text: "second" });
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-early", exitCode: 0 });

    const handler = vi.fn();
    register("stream-early", handler);

    expect(handler.mock.calls.map(([event]) => event.method)).toEqual([
      "docker/logs/chunk",
      "docker/logs/chunk",
      "docker/logs/end",
    ]);
    expect(handler.mock.calls.map(([event]) => event.params.text ?? event.params.exitCode)).toEqual([
      "first",
      "second",
      0,
    ]);
  });

  it("drops buffered events once the pending window expires", () => {
    vi.useFakeTimers();
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-late", text: "stale" });
    vi.advanceTimersByTime(PENDING_LOG_EVENT_TTL_MS + 1);

    const handler = vi.fn();
    register("stream-late", handler);
    expect(handler).not.toHaveBeenCalled();
  });

  it("bounds the pending buffer by dropping the oldest events", () => {
    const overflow = 2;
    for (let i = 0; i < PENDING_LOG_EVENT_LIMIT + overflow; i += 1) {
      dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-flood", text: `chunk-${i}` });
    }

    const handler = vi.fn();
    register("stream-flood", handler);
    expect(handler).toHaveBeenCalledTimes(PENDING_LOG_EVENT_LIMIT);
    // 超限丢最旧的，最新输出（含紧随其后的 end）仍在缓冲里。
    expect(handler.mock.calls[0][0].params.text).toBe(`chunk-${overflow}`);
    expect(handler.mock.calls.at(-1)?.[0].params.text).toBe(`chunk-${PENDING_LOG_EVENT_LIMIT + overflow - 1}`);
  });
});
