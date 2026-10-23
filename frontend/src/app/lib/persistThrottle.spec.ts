// persistThrottle 单测：首次立即、150 ms 合并、连续输出下 1 s 强制落盘、
// flush/cancel 生命周期。使用 fake timers（Date.now 也由 vitest 接管）。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPersistThrottle } from "./persistThrottle";

describe("persistThrottle", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("首次调用立即落盘（空闲期零延迟，替代原 150 ms 防抖的首帧等待）", () => {
    let runs = 0;
    const throttle = createPersistThrottle(() => { runs += 1; });
    throttle.schedule();
    expect(runs).toBe(1);
    expect(throttle.hasPending()).toBe(false);
  });

  it("窗口内的连续调用合并为一次 trailing 落盘", () => {
    let runs = 0;
    const throttle = createPersistThrottle(() => { runs += 1; });
    throttle.schedule(); // 立即 → 1
    vi.advanceTimersByTime(20);
    throttle.schedule();
    vi.advanceTimersByTime(20);
    throttle.schedule();
    expect(runs).toBe(1); // 仍在合并窗口内
    vi.advanceTimersByTime(150);
    expect(runs).toBe(2);
  });

  it("连续输出（每 10 ms 一次）不会饿死：从首个挂起起 1 s 内强制落盘", () => {
    let runs = 0;
    const throttle = createPersistThrottle(() => { runs += 1; }, { intervalMs: 150, maxWaitMs: 1000 });
    throttle.schedule(); // t=0 立即
    for (let tick = 0; tick < 200; tick += 1) {
      vi.advanceTimersByTime(10);
      throttle.schedule();
    }
    vi.advanceTimersByTime(50); // 让最后一轮 trailing 定时器到期
    // 2 s 连续输出：除首帧外至少再落盘两次（≈每 1 s 一次），且不逐帧写。
    expect(runs).toBeGreaterThanOrEqual(3);
    expect(runs).toBeLessThan(10);
  });

  it("挂起变更的落盘时刻不晚于 pendingSince + maxWaitMs", () => {
    const at: number[] = [];
    const throttle = createPersistThrottle(() => { at.push(Date.now()); }, { intervalMs: 150, maxWaitMs: 1000 });
    throttle.schedule(); // t0 立即
    const firstPending = Date.now() + 10;
    vi.advanceTimersByTime(10);
    for (let i = 0; i < 30; i += 1) {
      throttle.schedule();
      vi.advanceTimersByTime(10);
    }
    expect(at).toHaveLength(1); // t=300 仍在合并窗口内
    vi.advanceTimersByTime(1000);
    // 第一次 trailing 落盘必须发生在 firstPending + 1000 之前。
    expect(at[1]).toBeLessThanOrEqual(firstPending + 1000);
    expect(at[1]).toBeGreaterThan(firstPending);
  });

  it("flush() 立即落盘挂起变更；无挂起时为空操作", () => {
    let runs = 0;
    const throttle = createPersistThrottle(() => { runs += 1; });
    throttle.schedule();
    vi.advanceTimersByTime(20);
    throttle.schedule();
    expect(throttle.hasPending()).toBe(true);
    throttle.flush();
    expect(runs).toBe(2);
    expect(throttle.hasPending()).toBe(false);
    throttle.flush();
    expect(runs).toBe(2); // 无挂起 → 不重复写
  });

  it("cancel() 丢弃挂起变更，定时器不再触发", () => {
    let runs = 0;
    const throttle = createPersistThrottle(() => { runs += 1; });
    throttle.schedule();
    vi.advanceTimersByTime(10);
    throttle.schedule();
    throttle.cancel();
    vi.advanceTimersByTime(2000);
    expect(runs).toBe(1);
    expect(throttle.hasPending()).toBe(false);
  });

  it("距上次落盘超过 intervalMs 后再次调用 → 回到立即落盘", () => {
    let runs = 0;
    const throttle = createPersistThrottle(() => { runs += 1; });
    throttle.schedule();
    vi.advanceTimersByTime(500);
    throttle.schedule();
    expect(runs).toBe(2);
    expect(throttle.hasPending()).toBe(false);
  });

  it("落盘抛错（宿主桥缺失/序列化失败）不向 UI 处理器抛，也不把节流器卡死", () => {
    let runs = 0;
    let fail = true;
    const throttle = createPersistThrottle(() => {
      runs += 1;
      if (fail) throw new Error("window.dbxPlugin is undefined");
    });
    // leading 路径：schedule 不得抛错，且无残留 pending。
    expect(() => throttle.schedule()).not.toThrow();
    expect(runs).toBe(1);
    expect(throttle.hasPending()).toBe(false);
    // 失败后仍可继续调度：空闲窗口过去后恢复落盘。
    fail = false;
    vi.advanceTimersByTime(200);
    expect(() => throttle.schedule()).not.toThrow();
    expect(runs).toBe(2);
  });

  it("trailing 定时器与 flush 的落盘抛错同样被吞掉，状态不悬空", () => {
    let runs = 0;
    let fail = false;
    const throttle = createPersistThrottle(() => {
      runs += 1;
      if (fail) throw new Error("persist failed");
    });
    throttle.schedule(); // leading ok
    vi.advanceTimersByTime(20);
    fail = true;
    throttle.schedule(); // 进入 trailing
    expect(throttle.hasPending()).toBe(true);
    // 定时器回调里的抛错若逃逸会变成 unhandled error；这里不得抛。
    expect(() => vi.advanceTimersByTime(150)).not.toThrow();
    expect(runs).toBe(2);
    expect(throttle.hasPending()).toBe(false);

    // flush 路径同样吞错。
    vi.advanceTimersByTime(20);
    throttle.schedule();
    expect(throttle.hasPending()).toBe(true);
    expect(() => throttle.flush()).not.toThrow();
    expect(throttle.hasPending()).toBe(false);
    expect(runs).toBe(3);
  });

  it("注入时钟/定时器后不依赖 window（纯逻辑可测）", () => {
    let now = 0;
    const timers: Array<{ handler: () => void; due: number }> = [];
    let runs = 0;
    const throttle = createPersistThrottle(() => { runs += 1; }, {
      now: () => now,
      setTimeoutFn: (handler, timeout) => {
        timers.push({ handler, due: now + timeout });
        return timers.length - 1;
      },
      clearTimeoutFn: (id) => {
        timers[id] = { handler: () => undefined, due: Number.POSITIVE_INFINITY };
      },
    });
    throttle.schedule();
    now = 20;
    throttle.schedule();
    expect(timers).toHaveLength(1);
    now = timers[0].due;
    timers[0].handler();
    expect(runs).toBe(2);
  });
});
