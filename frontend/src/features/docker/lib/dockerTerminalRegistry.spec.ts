// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  resetDockerTerminalRegistry,
  stopActiveDockerTerminals,
  trackDockerTerminal,
} from "./dockerTerminalRegistry";

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

afterEach(() => {
  resetDockerTerminalRegistry();
  vi.unstubAllGlobals();
});

describe("docker terminal registry", () => {
  it("closes every tracked PTY exactly once", async () => {
    const calls = stubHost();
    trackDockerTerminal("term-a");
    const untrackB = trackDockerTerminal("term-b");
    untrackB();

    await stopActiveDockerTerminals();
    expect(calls).toEqual([{ method: "docker/terminal/close", params: { terminalId: "term-a" } }]);

    await stopActiveDockerTerminals();
    expect(calls).toHaveLength(1);
  });

  it("keeps shutting down when one close fails", async () => {
    const calls = stubHost(true);
    trackDockerTerminal("term-a");
    trackDockerTerminal("term-b");
    await expect(stopActiveDockerTerminals()).resolves.toBeUndefined();
    expect(calls.map((call) => call.params.terminalId).sort()).toEqual(["term-a", "term-b"]);
  });

  it("is a no-op without tracked terminals even when the bridge is unavailable", async () => {
    await expect(stopActiveDockerTerminals()).resolves.toBeUndefined();
  });
});
