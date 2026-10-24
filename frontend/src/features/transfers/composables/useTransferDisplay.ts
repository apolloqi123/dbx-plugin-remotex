import { type TransferTask } from "../lib/transferQueue";

/** 本机 CPU/磁盘阶段（staging/compressing/decompressing）与 prep 就绪标记
 * 走不定态进度条；网络阶段（uploading/fetching/transferring）用真实百分比
 * （压缩任务各阶段事件自带分母，切换自然衔接）。 */
const INDETERMINATE_BAR_PHASES: ReadonlySet<string> = new Set(["staging", "compressing", "decompressing", "ready"]);

/** 传输卡片展示助手（纯函数，无状态）：百分比/进度条值/阶段字节数/tooltip。 */
export function useTransferDisplay() {
  function transferPercent(task: TransferTask) {
    return task.size > 0 ? Math.min(100, Math.round((task.transferred / task.size) * 100)) : task.status === "completed" ? 100 : 0;
  }

  function transferBarValue(task: TransferTask): number | undefined {
    return task.phase !== undefined && INDETERMINATE_BAR_PHASES.has(task.phase) ? undefined : transferPercent(task);
  }

  /** staging/compressing 行展示的字节数：本地阶段计数（spool 缓存/压缩输入）；
   * 其余阶段是已推送/已接收计数。 */
  function transferShownBytes(task: TransferTask): number {
    return task.phase === "staging" || task.phase === "compressing" ? task.staged ?? 0 : task.transferred;
  }

  /** 精确字节数 tooltip：化解 5.9GB(十进制) vs 5.49GiB(二进制) 的口径困惑（issue #60）。 */
  function transferBytesTitle(task: TransferTask): string | undefined {
    if (!(task.size > 0)) return undefined;
    return `${transferShownBytes(task).toLocaleString()} / ${task.size.toLocaleString()} bytes`;
  }

  return {
    transferPercent,
    transferBarValue,
    transferShownBytes,
    transferBytesTitle,
  };
}
