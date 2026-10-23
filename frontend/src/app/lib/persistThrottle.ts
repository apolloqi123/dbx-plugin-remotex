// 工作台持久化的节流器（纯逻辑，时钟/定时器可注入）。
//
// 背景：App.vue 原实现是 150 ms 防抖——终端持续输出时 drain 每帧都调用
// persistState()，定时器被不断重置，工作台状态（终端序列号/布局）在最坏
// 情况下**永不落盘**（关闭标签页前丢失全部进度）。
//
// 语义（leading throttle + 有上限的 trailing debounce）：
// - 首次变更立即落盘（空闲期的第一次调用零延迟）；
// - 随后的变更合并到 intervalMs 后的一次写入；
// - 无论输入多密集，从“第一次未落盘变更”起最多 maxWaitMs 必强制落盘一次，
//   因此持久化不会被连续输出饿死；
// - flush() 供卸载/隐藏路径立即落盘，cancel() 丢弃挂起。

export interface PersistThrottleOptions {
  /** trailing 合并窗口（毫秒）；缺省 150（沿用既有防抖窗口）。 */
  intervalMs?: number;
  /** 强制落盘上限（毫秒），从第一次未落盘变更起算；缺省 1000。 */
  maxWaitMs?: number;
  /** 时钟（测试注入）；缺省 Date.now。 */
  now?: () => number;
  /** 定时器（测试注入）；缺省 window.setTimeout。 */
  setTimeoutFn?: (handler: () => void, timeout: number) => number;
  /** 取消定时器（测试注入）；缺省 window.clearTimeout。 */
  clearTimeoutFn?: (id: number) => void;
}

export interface PersistThrottle {
  /** 记录一次状态变更（可能立即落盘或合并）。 */
  schedule(): void;
  /** 立即落盘挂起的变更（无挂起则空操作）。 */
  flush(): void;
  /** 丢弃挂起的变更（不再落盘）。 */
  cancel(): void;
  /** 是否有等待落盘的变更。 */
  hasPending(): boolean;
}

export const DEFAULT_PERSIST_INTERVAL_MS = 150;
export const DEFAULT_PERSIST_MAX_WAIT_MS = 1000;

export function createPersistThrottle(
  run: () => void,
  options: PersistThrottleOptions = {},
): PersistThrottle {
  const intervalMs = normalizePositive(options.intervalMs, DEFAULT_PERSIST_INTERVAL_MS);
  const maxWaitMs = normalizePositive(options.maxWaitMs, DEFAULT_PERSIST_MAX_WAIT_MS);
  const clock = options.now ?? (() => Date.now());
  const setTimer =
    options.setTimeoutFn ?? ((handler, timeout) => globalThis.setTimeout(handler, timeout) as unknown as number);
  const clearTimer =
    options.clearTimeoutFn ?? ((id) => globalThis.clearTimeout(id as unknown as ReturnType<typeof setTimeout>));

  let timer: number | null = null;
  /** 最近一次实际落盘时刻；null = 尚未落盘过（首次调用走立即路径）。 */
  let lastRunAt: number | null = null;
  /** 本轮第一个未落盘变更的时刻；null = 无挂起。 */
  let pendingSince: number | null = null;

  function runNow(at: number): void {
    // 先清状态再落盘：落盘失败（宿主桥缺失、序列化异常等）不得把节流器
    // 卡在“有挂起但无定时器”的僵死态；下一轮 schedule 仍会重新尝试。
    timer = null;
    pendingSince = null;
    lastRunAt = at;
    try {
      run();
    } catch {
      // 持久化是尽力而为：UI 事件处理器与异步定时器都不得因落盘失败抛错。
    }
  }

  function scheduleTrailing(now: number): void {
    if (pendingSince === null) pendingSince = now;
    // 正常按 intervalMs 合并；但不得晚于 pendingSince + maxWaitMs（强制上限）。
    const dueAt = Math.min(now + intervalMs, pendingSince + maxWaitMs);
    const delay = Math.max(0, dueAt - now);
    if (timer !== null) clearTimer(timer);
    timer = setTimer(() => {
      runNow(clock());
    }, delay);
  }

  return {
    schedule(): void {
      const now = clock();
      if (timer === null && (lastRunAt === null || now - lastRunAt >= intervalMs)) {
        runNow(now); // 空闲期首次（或距上次落盘够远）→ 立即落盘
        return;
      }
      scheduleTrailing(now);
    },
    flush(): void {
      if (timer === null && pendingSince === null) return;
      if (timer !== null) {
        clearTimer(timer);
        timer = null;
      }
      runNow(clock());
    },
    cancel(): void {
      if (timer !== null) {
        clearTimer(timer);
        timer = null;
      }
      pendingSince = null;
    },
    hasPending(): boolean {
      return timer !== null || pendingSince !== null;
    },
  };
}

function normalizePositive(value: number | undefined, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : fallback;
}
