// 容器监控（docker/stats）纯逻辑：载荷归一、采样环缓冲、网络速率差分、时间段
// 过滤。与 dockerPanelModel 同风格：零 Vue 依赖，composable 与组件共用，
// 假数据即可单测。
//
// 采样模型：前端每 3s 取一次快照（`docker stats --no-stream`），环缓冲保留
// 最近 1 小时（1200 个样本）；时间段切换只过滤本地缓冲，不触发新请求。
// NetIO/BlockIO 是累计值，曲线用相邻样本差分得到 bytes/s（计数器重置/回绕
// 时钳 0，不出现负速率）。

export const MONITOR_POLL_MS = 3_000;
/** 1h × 3s 的样本容量。 */
export const MONITOR_CAPACITY = 1_200;

export interface ContainerStatsSample {
  /** 采集时间（epoch ms）。 */
  at: number;
  cpuPercent: number;
  memPercent: number;
  memUsedBytes: number;
  memLimitBytes: number;
  /** 累计收发字节（Docker NetIO）。 */
  netRxBytes: number;
  netTxBytes: number;
  blockReadBytes: number;
  blockWriteBytes: number;
  pids: number;
}

export interface ContainerStatsPoint extends ContainerStatsSample {
  /** 相邻样本差分得到的收发速率（bytes/s，首点 0）。 */
  rxRate: number;
  txRate: number;
}

export interface ContainerMonitorRange {
  seconds: number;
  /** 七语文案 key；标签固定为 1m/5m/15m/1h，key 保留以便本地化单位。 */
  labelKey: string;
}

export const CONTAINER_MONITOR_RANGES: readonly ContainerMonitorRange[] = [
  { seconds: 60, labelKey: "docker.monitorRange1m" },
  { seconds: 300, labelKey: "docker.monitorRange5m" },
  { seconds: 900, labelKey: "docker.monitorRange15m" },
  { seconds: 3_600, labelKey: "docker.monitorRange1h" },
];

export const DEFAULT_MONITOR_RANGE_SECONDS = 300;

/**
 * `docker stats` CPUPerc 以 100% = 1 核归一。小于 1 核时用毫核（Kubernetes
 * 的 `m`，12.5% → 125m）更好读；达到 1 核后显示两位小数的核数。
 */
export function cpuUsage(cpuPercent: number): { value: number; millicores: boolean } {
  const percent = Number.isFinite(cpuPercent) ? Math.max(0, cpuPercent) : 0;
  const cores = percent / 100;
  const millicores = Math.round(cores * 1_000);
  if (millicores < 1_000) return { value: millicores, millicores: true };
  return { value: Math.round(cores * 100) / 100, millicores: false };
}

/**
 * 网络速率纵轴上限的分档：以 1024 进制单位为主刻度，尾数在 1 个数量级内
 * 取 1/1.5/2/2.5/3/4/5/6/8，再叠加 10×/100×，得到 30 KiB/s、2 MiB/s 这类
 * 整齐刻度，而不是 29.3 KiB/s。
 */
const NICE_RATE_STEPS = [1, 1.5, 2, 2.5, 3, 4, 5, 6, 8] as const;
const NICE_RATE_DECADES = [1, 10, 100] as const;

/**
 * 峰值 → 稳定的纵轴上限（bytes/s）。至少 1 KiB/s，避免全零曲线出现 0 刻度；
 * 上限始终落在整 KiB/MiB/GiB 档，如 30 KiB/s、1.5 MiB/s。低于 1 KiB/s 的
 * 低速流量会被压在底部（用刻度可读性换低流量形状），这是有意的取舍。
 */
export function niceRateMax(peak: number): number {
  const value = Number.isFinite(peak) && peak > 0 ? peak : 0;
  if (value <= 1024) return 1024;
  const exponent = Math.floor(Math.log(value) / Math.log(1024));
  const unit = 1024 ** exponent;
  const mantissa = value / unit;
  for (const decade of NICE_RATE_DECADES) {
    for (const step of NICE_RATE_STEPS) {
      const candidate = step * decade;
      if (mantissa <= candidate) return candidate * unit;
    }
  }
  return 1024 * unit;
}

/**
 * Docker `CPUPerc`（100% = 1 核）→ 已知核数预算的占比。7 核用 1.03 核显示
 * 14.6%，而不是让头部出现 102.5% 的“超 100%”；总量未知时保留 Docker 原值。
 */
export function cpuSharePercent(cpuPercent: number, totalCores: number | null | undefined): number {
  const raw = Number.isFinite(cpuPercent) ? Math.max(0, cpuPercent) : 0;
  const total = typeof totalCores === "number" && Number.isFinite(totalCores) && totalCores > 0 ? totalCores : 0;
  return total > 0 ? raw / total : raw;
}

/** Core-count label for the denominator: `2`, `2.5`, `0.5` (no trailing zeros). */
export function formatCores(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0";
  return Number.isInteger(value)
    ? String(value)
    : value.toFixed(2).replace(/0+$/, "").replace(/\.$/, "");
}

function numberAt(source: Record<string, unknown>, key: string): number {
  const value = source[key];
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : 0;
}

/**
 * `docker/stats` 载荷 → 采样；容器未运行或 daemon 不可用时返回 null（调用方
 * 按 available/needsSudo 展示停止/无权态，不应把 0 值当真实样本画线）。
 */
export function sampleFromPayload(payload: unknown, at: number): ContainerStatsSample | null {
  if (!payload || typeof payload !== "object") return null;
  const source = payload as Record<string, unknown>;
  if (source.available !== true || source.running !== true) return null;
  return {
    at,
    cpuPercent: numberAt(source, "cpuPercent"),
    memPercent: numberAt(source, "memPercent"),
    memUsedBytes: numberAt(source, "memUsedBytes"),
    memLimitBytes: numberAt(source, "memLimitBytes"),
    netRxBytes: numberAt(source, "netRxBytes"),
    netTxBytes: numberAt(source, "netTxBytes"),
    blockReadBytes: numberAt(source, "blockReadBytes"),
    blockWriteBytes: numberAt(source, "blockWriteBytes"),
    pids: numberAt(source, "pids"),
  };
}

/** 推入采样并裁剪到容量（不可变语义，同 metricsSparkline.pushSample）。 */
export function pushStatsSample(
  ring: readonly ContainerStatsSample[],
  sample: ContainerStatsSample,
  capacity = MONITOR_CAPACITY,
): ContainerStatsSample[] {
  if (capacity <= 0) return [];
  return [...ring, sample].slice(-capacity);
}

/** 累计计数器 → 差分速率；时间回退/计数器重置一律钳 0。 */
export function statsPoints(samples: readonly ContainerStatsSample[]): ContainerStatsPoint[] {
  const points: ContainerStatsPoint[] = [];
  let previous: ContainerStatsSample | undefined;
  for (const sample of samples) {
    const seconds = previous ? (sample.at - previous.at) / 1_000 : 0;
    const rate = (current: number, before: number): number =>
      seconds > 0 ? Math.max(0, (current - before) / seconds) : 0;
    points.push({
      ...sample,
      rxRate: previous ? rate(sample.netRxBytes, previous.netRxBytes) : 0,
      txRate: previous ? rate(sample.netTxBytes, previous.netTxBytes) : 0,
    });
    previous = sample;
  }
  return points;
}

/** 只保留最近 `seconds` 秒的采样（按采样时间戳，不依赖缓冲顺序）。 */
export function rangePoints(
  points: readonly ContainerStatsPoint[],
  seconds: number,
  now = Date.now(),
): ContainerStatsPoint[] {
  const cutoff = now - seconds * 1_000;
  return points.filter((point) => point.at >= cutoff);
}
