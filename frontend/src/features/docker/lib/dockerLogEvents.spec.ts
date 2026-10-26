import { afterEach, describe, expect, it, vi } from "vitest";
import { dispatchDockerLogEvent, registerDockerLogStream } from "./dockerLogEvents";

/** docker/logs/{chunk,end} 的处理器表：全局单例，测试必须自行注销。 */
const disposers: Array<() => void> = [];

function register(streamId: string, handler: Parameters<typeof registerDockerLogStream>[1]): () => void {
  const dispose = registerDockerLogStream(streamId, handler);
  disposers.push(dispose);
  return dispose;
}

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
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
});
