// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { dockerChannelKey, runDockerChannel } from "./dockerChannelQueue";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("docker channel queue", () => {
  it("serialises operations of one connection", async () => {
    const key = "ssh:queue-order";
    const gate = deferred<void>();
    const order: string[] = [];
    const first = runDockerChannel(key, async () => {
      order.push("first:start");
      await gate.promise;
      order.push("first:end");
    });
    const second = runDockerChannel(key, async () => {
      order.push("second");
    });
    await Promise.resolve();
    expect(order).toEqual(["first:start"]);
    gate.resolve();
    await Promise.all([first, second]);
    expect(order).toEqual(["first:start", "first:end", "second"]);
  });

  it("keeps running after a failed operation", async () => {
    const key = "ssh:queue-failure";
    await expect(runDockerChannel(key, async () => {
      throw new Error("boom");
    })).rejects.toThrow("boom");
    await expect(runDockerChannel(key, async () => "ok")).resolves.toBe("ok");
  });

  it("runs different connections independently", async () => {
    const gate = deferred<void>();
    const order: string[] = [];
    const blocked = runDockerChannel("ssh:queue-a", async () => {
      await gate.promise;
      order.push("a");
    });
    const free = runDockerChannel("ssh:queue-b", async () => {
      order.push("b");
    });
    await free;
    expect(order).toEqual(["b"]);
    gate.resolve();
    await blocked;
    expect(order).toEqual(["b", "a"]);
  });

  it("keys by SSH session, falling back to the local connection", () => {
    expect(dockerChannelKey("sess-1", "conn-1")).toBe("ssh:sess-1");
    expect(dockerChannelKey(undefined, "conn-1")).toBe("local:conn-1");
    expect(dockerChannelKey(undefined)).toBe("local:default");
  });
});
