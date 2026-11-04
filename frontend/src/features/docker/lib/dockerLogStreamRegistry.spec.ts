// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  registerDockerLogOwner,
  resetDockerLogStreamRegistry,
  stopActiveDockerLogStreams,
  trackDockerLogStream,
} from "./dockerLogStreamRegistry";

type Call = { method: string; params: Record<string, unknown> };

function stubHost(fail = false): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal("dbxPlugin", {
    invoke: async (method: string, params: Record<string, unknown>) => {
      calls.push({ method, params });
      if (fail) throw new Error("bridge gone");
      return { stopped: true };
    },
  });
  return calls;
}

afterEach(async () => {
  // 清空登记表并复位 closed（生产里 webview 关闭后不会再用）。
  await stopActiveDockerLogStreams();
  resetDockerLogStreamRegistry();
  vi.unstubAllGlobals();
});

describe("docker log stream registry", () => {
  it("stops every tracked stream exactly once", async () => {
    const calls = stubHost();
    trackDockerLogStream("stream-a");
    const untrackB = trackDockerLogStream("stream-b");
    untrackB();

    await stopActiveDockerLogStreams();
    expect(calls).toEqual([{ method: "docker/logs/stop", params: { streamId: "stream-a" } }]);

    // 已清空：重复调用不再发 RPC。
    await stopActiveDockerLogStreams();
    expect(calls).toHaveLength(1);
  });

  it("closes registered owners so in-flight starts are covered", async () => {
    const calls = stubHost();
    const release = registerDockerLogOwner("pane-1");
    trackDockerLogStream("stream-a");

    await stopActiveDockerLogStreams();
    expect(calls).toContainEqual({ method: "docker/logs/close", params: { owner: "pane-1" } });
    release();
  });

  it("closes an owner registered after the workbench started closing", async () => {
    const calls = stubHost();
    await stopActiveDockerLogStreams();
    const release = registerDockerLogOwner("late-pane");
    await vi.waitFor(() =>
      expect(calls).toContainEqual({ method: "docker/logs/close", params: { owner: "late-pane" } }),
    );
    release();
  });

  it("stops a stream tracked after the workbench started closing", async () => {
    const calls = stubHost();
    await stopActiveDockerLogStreams();
    expect(calls).toHaveLength(0);

    trackDockerLogStream("late-stream");
    await vi.waitFor(() =>
      expect(calls).toContainEqual({ method: "docker/logs/stop", params: { streamId: "late-stream" } }),
    );
  });

  it("keeps shutting down when one stop fails", async () => {
    const calls = stubHost(true);
    trackDockerLogStream("stream-a");
    trackDockerLogStream("stream-b");
    registerDockerLogOwner("pane-1");
    await expect(stopActiveDockerLogStreams()).resolves.toBeUndefined();
    expect(calls.filter((call) => call.method === "docker/logs/stop").map((call) => call.params.streamId).sort()).toEqual([
      "stream-a",
      "stream-b",
    ]);
    expect(calls.some((call) => call.method === "docker/logs/close")).toBe(true);
  });

  it("is a no-op without tracked streams even when the bridge is unavailable", async () => {
    await expect(stopActiveDockerLogStreams()).resolves.toBeUndefined();
  });
});
