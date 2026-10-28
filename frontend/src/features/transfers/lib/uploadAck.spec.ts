// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  enqueueUploadAck,
  rejectUploadAcks,
  removeUploadAck,
  resolveUploadAck,
  type UploadAckWaiter,
  type UploadAckWaiters,
} from "./uploadAck";

function waiter(nextOffset: number) {
  return {
    nextOffset,
    resolve: vi.fn<() => void>(),
    reject: vi.fn<(error: Error) => void>(),
    timer: window.setTimeout(() => undefined, 10_000),
  } satisfies UploadAckWaiter;
}

afterEach(() => {
  vi.useRealTimers();
});

describe("upload ack queue", () => {
  it("releases every waiter covered by a cumulative ack", () => {
    vi.useFakeTimers();
    const waiters: UploadAckWaiters = new Map();
    const first = waiter(1);
    const second = waiter(2);
    enqueueUploadAck(waiters, "t1", first);
    enqueueUploadAck(waiters, "t1", second);

    // sidecar 的 nextOffset 是“已收到总量”：ack 2 同时证明 1 已落盘
    //（也覆盖某条 ack 事件丢失的情况）。
    expect(resolveUploadAck(waiters, "t1", 2)).toBe(true);
    expect(first.resolve).toHaveBeenCalledTimes(1);
    expect(second.resolve).toHaveBeenCalledTimes(1);
    expect(waiters.size).toBe(0);

    expect(resolveUploadAck(waiters, "t1", 2)).toBe(false);
    expect(resolveUploadAck(waiters, "missing", 1)).toBe(false);
  });

  it("does not release a later waiter from a stale ack", () => {
    vi.useFakeTimers();
    const waiters: UploadAckWaiters = new Map();
    const later = waiter(2);
    enqueueUploadAck(waiters, "t1", later);

    expect(resolveUploadAck(waiters, "t1", 1)).toBe(false);
    expect(later.resolve).not.toHaveBeenCalled();
    expect(waiters.get("t1")).toEqual([later]);
  });

  it("removes a waiter without resolving it (status-confirmed chunk)", () => {
    vi.useFakeTimers();
    const waiters: UploadAckWaiters = new Map();
    const only = waiter(4096);
    enqueueUploadAck(waiters, "t1", only);

    expect(removeUploadAck(waiters, "t1", 4096)).toBe(only);
    expect(only.resolve).not.toHaveBeenCalled();
    expect(waiters.size).toBe(0);
    expect(removeUploadAck(waiters, "t1", 4096)).toBeUndefined();
  });

  it("rejects every in-flight waiter once and drops the queue", () => {
    vi.useFakeTimers();
    const waiters: UploadAckWaiters = new Map();
    const first = waiter(1);
    const second = waiter(2);
    enqueueUploadAck(waiters, "t1", first);
    enqueueUploadAck(waiters, "t1", second);
    const error = new Error("rejected");

    rejectUploadAcks(waiters, "t1", error);

    expect(first.reject).toHaveBeenCalledWith(error);
    expect(second.reject).toHaveBeenCalledWith(error);
    expect(waiters.size).toBe(0);
    // 队列已清空，重复收口是无害空操作。
    rejectUploadAcks(waiters, "t1", error);
    expect(first.reject).toHaveBeenCalledTimes(1);
  });
});
