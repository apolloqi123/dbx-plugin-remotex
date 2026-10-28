// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { raceCancelled } from "./downloadCancellation";

afterEach(() => vi.useRealTimers());

describe("raceCancelled", () => {
  it("rejects quickly when the task is cancelled while the RPC is pending", async () => {
    vi.useFakeTimers();
    const cancelled = new Set<string>();
    const pending = new Promise<string>(() => undefined);
    const raced = raceCancelled("t1", pending, (id) => cancelled.has(id));
    const assertion = expect(raced).rejects.toThrow("transfer cancelled");
    cancelled.add("t1");
    await vi.advanceTimersByTimeAsync(100);
    await assertion;
  });

  it("resolves with the RPC result and stops polling", async () => {
    vi.useFakeTimers();
    const raced = raceCancelled("t2", Promise.resolve("ok"), () => false);
    await expect(raced).resolves.toBe("ok");
    expect(vi.getTimerCount()).toBe(0);
  });
});
