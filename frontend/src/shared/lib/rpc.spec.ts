// rpc<T>() 薄包装单测：桥缺失时明确拒绝（RpcUnavailableError），在位时
// 原样透传 method/params/options 并返回宿主结果。
// @vitest-environment happy-dom（需要 window.dbxPlugin 宿主桥面）

import { afterEach, describe, expect, it, vi } from "vitest";
import { rpc, rpcAvailable, RpcUnavailableError } from "./rpc";

const globalWindow = window as unknown as { dbxPlugin?: unknown };

afterEach(() => {
  delete globalWindow.dbxPlugin;
});

describe("rpc", () => {
  it("桥缺失：rpcAvailable=false 且以 RpcUnavailableError 拒绝", async () => {
    delete globalWindow.dbxPlugin;
    expect(rpcAvailable()).toBe(false);
    await expect(rpc("ssh/test")).rejects.toBeInstanceOf(RpcUnavailableError);
    await expect(rpc("ssh/test")).rejects.toMatchObject({ method: "ssh/test" });
  });

  it("桥在位：透传 method/params/timeout 并返回宿主结果", async () => {
    const invoke = vi.fn(async () => ({ ok: true }));
    globalWindow.dbxPlugin = { invoke };
    expect(rpcAvailable()).toBe(true);
    const result = await rpc<{ ok: boolean }>("ssh/terminal/start", { profileId: "p1" }, { timeoutMs: 5000 });
    expect(result).toEqual({ ok: true });
    expect(invoke).toHaveBeenCalledWith("ssh/terminal/start", { profileId: "p1" }, { timeoutMs: 5000 });
  });

  it("桥在位且未传 options：保持两参调用（与直连 invoke 的实参形状一致）", async () => {
    const invoke = vi.fn(async () => undefined);
    globalWindow.dbxPlugin = { invoke };
    await rpc("vnc/close", { sessionId: "vnc-1" });
    expect(invoke).toHaveBeenCalledWith("vnc/close", { sessionId: "vnc-1" });
    expect(invoke.mock.calls[0]).toHaveLength(2);
  });

  it("宿主拒绝原样上抛（不吞错、不改错误类型）", async () => {
    const boom = new Error("sidecar down");
    globalWindow.dbxPlugin = { invoke: vi.fn(async () => Promise.reject(boom)) };
    await expect(rpc("ssh/test")).rejects.toBe(boom);
  });
});
