// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { effectScope, nextTick } from "vue";
import { useDockerLogStream } from "./useDockerLogStream";
import { clearPendingDockerLogEvents, dispatchDockerLogEvent } from "../lib/dockerLogEvents";
import { DOCKER_ENGINE_STORE_KEY, saveDockerEngineSettings } from "../lib/dockerEngine";
import { pluginStore } from "../../../shared/lib/pluginStore";

type Call = { method: string; params: Record<string, unknown> };

function stubHost(options: { streamIds?: string[]; failStart?: string } = {}): Call[] {
  const calls: Call[] = [];
  let index = 0;
  vi.stubGlobal("dbxPlugin", {
    invoke: async (method: string, params: Record<string, unknown>) => {
      calls.push({ method, params });
      if (method === "docker/logs/start") {
        if (options.failStart) throw new Error(options.failStart);
        const ids = options.streamIds ?? ["stream-1"];
        const streamId = ids[Math.min(index, ids.length - 1)];
        index += 1;
        return { streamId };
      }
      if (method === "docker/logs/stop") return { stopped: true };
      throw new Error(`unexpected rpc ${method}`);
    },
  });
  return calls;
}

function createStream(options: Parameters<typeof useDockerLogStream>[0] = {}) {
  const scope = effectScope();
  const stream = scope.run(() => useDockerLogStream(options));
  if (!stream) throw new Error("effect scope did not produce a stream");
  return { stream, scope };
}

afterEach(() => {
  clearPendingDockerLogEvents();
  pluginStore.removeItem(DOCKER_ENGINE_STORE_KEY);
  vi.unstubAllGlobals();
});

describe("useDockerLogStream", () => {
  it("starts an SSH stream, appends chunks and stops on scope dispose", async () => {
    const calls = stubHost();
    const { stream, scope } = createStream({ sessionId: () => "sess-1", connectionId: () => "conn-1" });
    await stream.start("abc123def456", { tail: "200" });

    expect(calls[0]).toEqual({
      method: "docker/logs/start",
      params: { sessionId: "sess-1", containerId: "abc123def456", tail: 200 },
    });
    expect(stream.status.value).toBe("streaming");
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "hel" });
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "lo\nwor" });
    expect(stream.text.value).toBe("hello\nwor");
    expect(stream.lines.value).toEqual(["hello", "wor"]);

    scope.stop();
    expect(calls.at(-1)).toEqual({ method: "docker/logs/stop", params: { streamId: "stream-1" } });
  });

  it("targets the local daemon when no session is bound", async () => {
    const calls = stubHost();
    const { stream, scope } = createStream();
    await stream.start("abc123def456");
    expect(calls[0].params).toEqual({ target: "local", containerId: "abc123def456", tail: 200 });
    scope.stop();
  });

  it("carries the per-connection engine settings", async () => {
    saveDockerEngineSettings("conn-1", { cli: "podman", socket: "/run/podman.sock", host: "" });
    const calls = stubHost();
    const { stream, scope } = createStream({ sessionId: () => "sess-1", connectionId: () => "conn-1" });
    await stream.start("abc123def456");
    expect(calls[0].params).toMatchObject({ sessionId: "sess-1", cli: "podman", socket: "/run/podman.sock" });
    scope.stop();
  });

  it("keeps the buffer on natural end and records the exit code", async () => {
    stubHost();
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    await stream.start("abc123def456");
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "last line\n" });
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", exitCode: 137 });

    expect(stream.status.value).toBe("ended");
    expect(stream.exitCode.value).toBe(137);
    expect(stream.error.value).toBe("");
    expect(stream.needsSudo.value).toBe(false);
    expect(stream.text.value).toBe("last line");

    // 已注销：结束后的分片不再进入缓冲。
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "ghost" });
    expect(stream.text.value).toBe("last line");
    scope.stop();
  });

  it("flags sudo and retries the same container and tail with sudo", async () => {
    const calls = stubHost({ streamIds: ["stream-1", "stream-2"] });
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    await stream.start("abc123def456", { tail: "500" });
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", error: "permission denied", needsSudo: true });

    expect(stream.needsSudo.value).toBe(true);
    expect(stream.status.value).toBe("error");
    expect(stream.error.value).toBe("permission denied");

    await stream.retrySudo();
    expect(stream.needsSudo.value).toBe(false);
    expect(stream.status.value).toBe("streaming");
    expect(calls.filter((call) => call.method === "docker/logs/start").at(-1)?.params).toMatchObject({
      sessionId: "sess-1",
      containerId: "abc123def456",
      tail: 500,
      sudo: true,
    });
    scope.stop();
  });

  it("records a timeout end as an actionable error", async () => {
    stubHost();
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    await stream.start("abc123def456");
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", error: "stream timed out" });
    expect(stream.status.value).toBe("error");
    expect(stream.error.value).toBe("stream timed out");
    scope.stop();
  });

  it("ignores the cancellation end event that follows a manual stop", async () => {
    stubHost();
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    await stream.start("abc123def456");
    await stream.stop();
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-1", error: "cancelled" });
    expect(stream.status.value).toBe("idle");
    expect(stream.error.value).toBe("");
    scope.stop();
  });

  it("stops idempotently", async () => {
    const calls = stubHost();
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    await stream.start("abc123def456");
    await stream.stop();
    await stream.stop();
    expect(calls.filter((call) => call.method === "docker/logs/stop")).toHaveLength(1);
    scope.stop();
  });

  it("stops the previous stream when the container changes", async () => {
    const calls = stubHost({ streamIds: ["stream-1", "stream-2"] });
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    await stream.start("abc123def456");
    await stream.start("def456abc123");
    expect(calls.map((call) => call.method)).toEqual([
      "docker/logs/start",
      "docker/logs/stop",
      "docker/logs/start",
    ]);
    expect(stream.status.value).toBe("streaming");
    scope.stop();
  });

  it("clears the view without stopping the stream", async () => {
    const calls = stubHost();
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    await stream.start("abc123def456");
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: "one\ntwo\n" });
    stream.clear();
    expect(stream.text.value).toBe("");
    expect(stream.lines.value).toEqual([]);
    expect(stream.status.value).toBe("streaming");
    expect(calls.some((call) => call.method === "docker/logs/stop")).toBe(false);
    scope.stop();
  });

  it("surfaces start failures", async () => {
    stubHost({ failStart: "Quick Sudo is not available for local docker targets" });
    const { stream, scope } = createStream();
    await stream.start("abc123def456", { sudo: true });
    expect(stream.status.value).toBe("error");
    expect(stream.error.value).toContain("Quick Sudo is not available");
    scope.stop();
  });

  it("restarts with an overridden tail and keeps the container", async () => {
    const calls = stubHost({ streamIds: ["stream-1", "stream-2"] });
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    await stream.start("abc123def456", { tail: "200" });
    await stream.restart({ tail: "500" });
    const starts = calls.filter((call) => call.method === "docker/logs/start");
    expect(starts.at(-1)?.params).toMatchObject({ containerId: "abc123def456", tail: 500 });
    expect(stream.status.value).toBe("streaming");
    scope.stop();
  });

  it("stops a stream whose start resolves after the caller stopped", async () => {
    const calls: Call[] = [];
    let resolveStart: ((value: unknown) => void) | undefined;
    vi.stubGlobal("dbxPlugin", {
      invoke: (method: string, params: Record<string, unknown>) => {
        calls.push({ method, params });
        if (method === "docker/logs/start") {
          return new Promise((resolve) => {
            resolveStart = resolve;
          });
        }
        return Promise.resolve({ stopped: true });
      },
    });
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    const pending = stream.start("abc123def456");
    await nextTick();
    await stream.stop();
    resolveStart?.({ streamId: "late-stream" });
    await pending;

    expect(stream.status.value).toBe("idle");
    expect(calls.some((call) => call.method === "docker/logs/stop" && call.params.streamId === "late-stream")).toBe(true);
    scope.stop();
  });

  it("delivers chunk and a fast end that arrive before the start response", async () => {
    let resolveStart: ((value: unknown) => void) | undefined;
    vi.stubGlobal("dbxPlugin", {
      invoke: (method: string) => {
        if (method === "docker/logs/start") {
          return new Promise((resolve) => {
            resolveStart = resolve;
          });
        }
        return Promise.resolve({ stopped: true });
      },
    });
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    const pending = stream.start("abc123def456");
    await nextTick();
    expect(stream.status.value).toBe("starting");

    // start 尚未返回：事件没有处理器，必须先进缓冲而不是被丢弃。
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-fast", text: "fatal: no such container\n" });
    dispatchDockerLogEvent("docker/logs/end", { streamId: "stream-fast", exitCode: 1, error: "no such container" });

    resolveStart?.({ streamId: "stream-fast" });
    await pending;

    // 回放 end 后必须停在 error（可显示横幅），不能被 register 之后的状态写回覆盖。
    expect(stream.status.value).toBe("error");
    expect(stream.error.value).toBe("no such container");
    expect(stream.exitCode.value).toBe(1);
    expect(stream.text.value).toBe("fatal: no such container");

    // 结束即注销：后续迟到分片不再进入缓冲。
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-fast", text: "ghost" });
    expect(stream.text.value).toBe("fatal: no such container");
    scope.stop();
  });

  it("ignores a stale A start that resolves after the rapid A→B switch", async () => {
    const calls: Array<Call & { resolve?: (value: unknown) => void }> = [];
    vi.stubGlobal("dbxPlugin", {
      invoke: (method: string, params: Record<string, unknown>) => {
        const call: Call & { resolve?: (value: unknown) => void } = { method, params };
        calls.push(call);
        if (method === "docker/logs/start") {
          return new Promise((resolve) => {
            call.resolve = resolve;
          });
        }
        return Promise.resolve({ stopped: true });
      },
    });
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });

    const slowA = stream.start("aaa111aaa111");
    await nextTick();
    const fastB = stream.start("bbb222bbb222");
    await nextTick();

    const starts = calls.filter((call) => call.method === "docker/logs/start");
    expect(starts.map((call) => call.params.containerId)).toEqual(["aaa111aaa111", "bbb222bbb222"]);

    // B 先返回并注册；随后 A 才返回——A 必须作废并收掉自己的后端流。
    starts[1].resolve?.({ streamId: "stream-b" });
    await fastB;
    expect(stream.status.value).toBe("streaming");

    starts[0].resolve?.({ streamId: "stream-a" });
    await slowA;

    expect(calls.some((call) => call.method === "docker/logs/stop" && call.params.streamId === "stream-a")).toBe(true);
    expect(stream.status.value).toBe("streaming");

    // 只有 B 的流还活着：A 的分片被丢弃，B 的分片只计一次。
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-a", text: "stale\n" });
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-b", text: "live\n" });
    expect(stream.text.value).toBe("live");
    expect(stream.lines.value).toEqual(["live"]);
    scope.stop();
  });

  it("follows by default, notifies follow changes and resets on restart", async () => {
    stubHost();
    const onFollowChange = vi.fn();
    const { stream, scope } = createStream({ sessionId: () => "sess-1", onFollowChange });
    expect(stream.follow.value).toBe(true);
    stream.follow.value = false;
    await nextTick();
    expect(onFollowChange).toHaveBeenLastCalledWith(false);
    await stream.start("abc123def456");
    await nextTick();
    expect(stream.follow.value).toBe(true);
    expect(onFollowChange).toHaveBeenLastCalledWith(true);
    scope.stop();
  });

  it("exposes truncation once the buffer trims", async () => {
    stubHost();
    const { stream, scope } = createStream({ sessionId: () => "sess-1" });
    await stream.start("abc123def456");
    const bulk = Array.from({ length: 10_050 }, (_, i) => `line-${i}`).join("\n");
    dispatchDockerLogEvent("docker/logs/chunk", { streamId: "stream-1", text: bulk });
    expect(stream.truncated.value).toBe(true);
    expect(stream.lines.value).toHaveLength(10_000);
    scope.stop();
  });
});
