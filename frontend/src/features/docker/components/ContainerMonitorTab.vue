<script setup lang="ts">
// 容器监控页签：CPU / 内存 / 网络三块卡片 + 时间段切换。采样与轮询在
// useContainerStats（仅页签可见且容器 running 时 3s 一次）；这里只做展示。
// 图表复用服务器指标页的 sparkline 采样与趋势图组件，观感保持一致。
import { computed } from "vue";
import MetricsTrendChart from "../../metrics/components/MetricsTrendChart.vue";
import { sparklinePath } from "../../metrics/lib/metricsSparkline";
import { formatBytes, formatRate } from "../../../shared/lib/format";
import { isDockerContainerRunning, type DockerContainer } from "../lib/dockerPanelModel";
import { CONTAINER_MONITOR_RANGES, cpuSharePercent, cpuUsage, formatCores, niceRateMax } from "../lib/containerMonitor";
import { useContainerStats } from "../composables/useContainerStats";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  container: DockerContainer;
  sessionId?: string;
  connectionId?: string;
  /** 页签是否可见：隐藏即停轮询。 */
  visible: boolean;
}>();

const { rangedPoints, current, status, error, rangeSeconds, hostCpus, cpuLimitCores, retry } = useContainerStats({
  containerId: () => props.container.id,
  sessionId: () => props.sessionId,
  connectionId: () => props.connectionId,
  running: () => isDockerContainerRunning(props.container),
  visible: () => props.visible,
});

const CHART_WIDTH = 120;
const CHART_HEIGHT = 18;

/** 内存是百分比刻度，固定 0–100%。 */
const memoryAxis = ["100%", "50%", "0%"];

/**
 * CPU 曲线与头部百分比都按“总容量占比”画：Docker `CPUPerc` 的 100% 只是 1 个
 * 核，7 核跑 1.03 核应显示 14.6% 而不是 102.5%。总量未知时退回 Docker 口径，
 * 纵轴上限按峰值向上取整到 10% 档，避免多核曲线顶死在 100%。
 */
const cpuChartValues = computed(() =>
  rangedPoints.value.map((point) => cpuSharePercent(point.cpuPercent, cpuTotalCores.value)),
);
const cpuChartMax = computed(() => {
  if (cpuTotalCores.value) return 100;
  const peak = cpuChartValues.value.reduce((top, value) => Math.max(top, value), 100);
  return Math.ceil(peak / 10) * 10;
});
const cpuPoints = computed(() =>
  sparklinePath(cpuChartValues.value, CHART_WIDTH, CHART_HEIGHT, cpuChartMax.value),
);
const cpuAxis = computed(() => {
  const max = cpuChartMax.value;
  const tick = (value: number) => (Number.isInteger(value) ? String(value) : value.toFixed(1));
  return [`${tick(max)}%`, `${tick(max / 2)}%`, "0%"];
});
const memPoints = computed(() =>
  sparklinePath(rangedPoints.value.map((point) => point.memPercent), CHART_WIDTH, CHART_HEIGHT, 100),
);
/** 上下行共用同一刻度，双线高度可横向比较；纵轴上限取整到 1/1.5/2… 档。 */
const netPeak = computed(() =>
  rangedPoints.value.reduce(
    (peak, point) => Math.max(peak, point.rxRate, point.txRate),
    0,
  ),
);
const netAxisMax = computed(() => niceRateMax(netPeak.value));
const netAxis = computed(() => [
  formatRate(netAxisMax.value),
  formatRate(netAxisMax.value / 2),
  formatRate(0),
]);
const rxPoints = computed(() =>
  sparklinePath(rangedPoints.value.map((point) => point.rxRate), CHART_WIDTH, CHART_HEIGHT, netAxisMax.value),
);
const txPoints = computed(() =>
  sparklinePath(rangedPoints.value.map((point) => point.txRate), CHART_WIDTH, CHART_HEIGHT, netAxisMax.value),
);

const percent = (value: number | undefined): string =>
  value === undefined ? "—" : `${value.toFixed(1)}%`;

/** 分母：有 CPU 上限用上限，否则用 daemon 主机总核数；未知则不显示分母。 */
const cpuTotalCores = computed(() => cpuLimitCores.value ?? hostCpus.value);
const cpuTotalTitle = computed(() => {
  const total = cpuTotalCores.value;
  if (!total) return "";
  const value = formatCores(total);
  return cpuLimitCores.value
    ? props.t("docker.monitorCpuLimitHint", { value })
    : props.t("docker.monitorCpuHostHint", { value });
});

/**
 * CPU 主值：<1 核用毫核（125m），≥1 核用核数（1.57）；已知总量时显示
 * “用量 / 总量”（125m / 16 核、1.57 / 2 核），否则只显示用量。
 */
const cpuUsageText = computed(() => {
  const value = current.value?.cpuPercent;
  if (value === undefined) return "—";
  const usage = cpuUsage(value);
  const usageText = usage.millicores ? `${usage.value}m` : formatCores(usage.value);
  const total = cpuTotalCores.value;
  if (!total) {
    return usage.millicores
      ? usageText
      : props.t("docker.monitorCpuCores", { value: usageText });
  }
  return `${usageText} / ${props.t("docker.monitorCpuCores", { value: formatCores(total) })}`;
});

/** 头部百分比 = 占总容量/限制的比例；总量未知时退回 Docker 原值（100% = 1 核）。 */
const cpuDisplayPercent = computed(() => {
  const value = current.value?.cpuPercent;
  return value === undefined ? undefined : cpuSharePercent(value, cpuTotalCores.value);
});
const cpuPercentTitle = computed(() => {
  const raw = current.value?.cpuPercent;
  if (raw === undefined) return "";
  const total = cpuTotalCores.value;
  return total
    ? props.t("docker.monitorCpuPercentHint", { total: formatCores(total), percent: percent(raw) })
    : props.t("docker.monitorCpuRawHint", { percent: percent(raw) });
});

const memoryUsage = computed(() => {
  const sample = current.value;
  if (!sample) return "—";
  return `${formatBytes(sample.memUsedBytes)} / ${formatBytes(sample.memLimitBytes)}`;
});

const hasSamples = computed(() => rangedPoints.value.length > 0);
const stateLabel = computed(() => {
  switch (status.value) {
    case "live":
      return props.t("docker.monitorLive");
    case "loading":
      return props.t("docker.monitorCollecting");
    case "stopped":
      return props.t("docker.monitorStopped");
    case "denied":
      return props.t("docker.monitorDenied");
    case "error":
      return props.t("docker.monitorFailed");
    default:
      return "";
  }
});
</script>

<template>
  <div class="container-monitor-tab" data-tab-panel="monitor">
    <div class="container-monitor-toolbar">
      <span v-if="status === 'live' || status === 'loading'" class="container-monitor-state" :data-state="status">{{ stateLabel }}</span>
      <span class="container-monitor-spacer" />
      <div class="container-monitor-ranges" role="group" :aria-label="t('docker.monitorRangeLabel')">
        <button
          v-for="range in CONTAINER_MONITOR_RANGES"
          :key="range.seconds"
          type="button"
          class="container-monitor-range"
          :class="{ 'is-active': rangeSeconds === range.seconds }"
          :aria-pressed="rangeSeconds === range.seconds"
          :data-range="range.seconds"
          @click="rangeSeconds = range.seconds"
        >
          {{ t(range.labelKey) }}
        </button>
      </div>
    </div>

    <div
      v-if="status === 'stopped' || status === 'denied'"
      class="container-monitor-empty"
      data-role="monitor-empty"
    >
      <p class="container-monitor-empty-title">{{ stateLabel }}</p>
      <p v-if="status === 'denied'" class="muted container-monitor-empty-hint">
        {{ t("docker.monitorDeniedHint") }}
      </p>
    </div>

    <div v-else-if="status === 'error'" class="container-monitor-empty" data-role="monitor-empty">
      <p class="container-monitor-empty-title">{{ t("docker.monitorFailed") }}</p>
      <p v-if="error" class="muted container-monitor-empty-hint" :title="error">{{ error }}</p>
      <button type="button" class="container-monitor-retry" data-action="monitor-retry" @click="retry">
        {{ t("docker.monitorRetry") }}
      </button>
    </div>

    <div v-else-if="!hasSamples" class="container-monitor-empty" role="status" data-role="monitor-empty">
      <p class="container-monitor-empty-title">{{ t("docker.monitorCollecting") }}</p>
    </div>

    <div v-else class="container-monitor-grid">
      <article class="container-monitor-card" data-metric="cpu">
        <header class="container-monitor-card-head">
          <span class="container-monitor-card-title">
            {{ t("docker.monitorCpu") }}
            <span class="container-monitor-card-title-sub">
              · {{ t("docker.monitorPids", { count: current?.pids ?? 0 }) }}
            </span>
          </span>
          <span class="container-monitor-card-value" :title="cpuPercentTitle">{{ percent(cpuDisplayPercent) }}</span>
        </header>
        <MetricsTrendChart :points="cpuPoints" tone="emerald" :y-ticks="cpuAxis" />
        <footer class="container-monitor-card-foot">
          <span class="mono" :title="cpuTotalTitle">{{ cpuUsageText }}</span>
        </footer>
      </article>

      <article class="container-monitor-card" data-metric="memory">
        <header class="container-monitor-card-head">
          <span class="container-monitor-card-title">{{ t("docker.monitorMemory") }}</span>
          <span class="container-monitor-card-value">{{ percent(current?.memPercent) }}</span>
        </header>
        <MetricsTrendChart :points="memPoints" tone="amber" :y-ticks="memoryAxis" />
        <footer class="container-monitor-card-foot">
          <span class="mono">{{ memoryUsage }}</span>
        </footer>
      </article>

      <article class="container-monitor-card" data-metric="network">
        <header class="container-monitor-card-head">
          <span class="container-monitor-card-title">{{ t("docker.monitorNetwork") }}</span>
          <span class="container-monitor-card-value mono">
            ↓ {{ formatRate(current?.rxRate ?? 0) }} ↑ {{ formatRate(current?.txRate ?? 0) }}
          </span>
        </header>
        <MetricsTrendChart :points="rxPoints" :secondary-points="txPoints" tone="cyan" secondary-tone="violet" :fill="false" :y-ticks="netAxis" />
        <footer class="container-monitor-card-foot">
          <span>{{ t("docker.monitorBlockIo", { read: formatBytes(current?.blockReadBytes ?? 0), write: formatBytes(current?.blockWriteBytes ?? 0) }) }}</span>
        </footer>
      </article>
    </div>
  </div>
</template>

<style scoped>
.container-monitor-tab {
  display: flex;
  flex: 1;
  min-width: 0;
  min-height: 0;
  flex-direction: column;
  overflow: hidden;
}

.container-monitor-toolbar {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
  padding: 2px 6px;
  font-size: 11px;
}

.container-monitor-state {
  min-width: 0;
  overflow: hidden;
  color: var(--muted-foreground);
  font-size: 10px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.container-monitor-state[data-state="live"] {
  color: var(--success);
}

.container-monitor-state[data-state="denied"],
.container-monitor-state[data-state="error"] {
  color: var(--destructive);
}

.container-monitor-spacer {
  flex: 1;
}

.container-monitor-ranges {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 2px;
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  padding: 1px;
  background: var(--background);
}

.container-monitor-range {
  min-width: 26px;
  border: 0;
  border-radius: 4px;
  padding: 1px 6px;
  background: transparent;
  color: var(--muted-foreground);
  font-size: 10px;
  font-variant-numeric: tabular-nums;
  cursor: pointer;
}

.container-monitor-range:hover {
  color: var(--foreground);
}

.container-monitor-range.is-active {
  background: color-mix(in srgb, var(--primary) 20%, transparent);
  color: var(--foreground);
}

.container-monitor-grid {
  display: grid;
  flex: 1;
  min-height: 0;
  align-content: start;
  gap: 6px;
  overflow: auto;
  padding: 4px 6px 8px;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
}

.container-monitor-card {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 4px;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-md);
  padding: 8px 10px;
  background: var(--card-surface-inset);
}

.container-monitor-card-head {
  display: flex;
  min-width: 0;
  align-items: baseline;
  gap: 6px;
}

.container-monitor-card-title {
  color: var(--muted-foreground);
  font-size: 10px;
  letter-spacing: 0.02em;
  white-space: nowrap;
}

.container-monitor-card-value {
  margin-left: auto;
  overflow: hidden;
  color: var(--foreground);
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.container-monitor-card-title-sub {
  margin-left: 2px;
  color: var(--muted-foreground);
  font-size: 9px;
  opacity: 0.8;
}

.container-monitor-card-foot {
  display: flex;
  min-width: 0;
  overflow: hidden;
  color: var(--muted-foreground);
  font-size: 10px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.container-monitor-card-foot .mono {
  font-size: 10px;
}

.container-monitor-empty {
  display: flex;
  flex: 1;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 16px;
  text-align: center;
}

.container-monitor-empty-title {
  margin: 0;
  color: var(--muted-foreground);
  font-size: 12px;
}

.container-monitor-empty-hint {
  margin: 0;
  max-width: 360px;
  font-size: 11px;
}

.container-monitor-retry {
  margin-top: 4px;
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 2px 10px;
  background: var(--background);
  color: var(--foreground);
  font-size: 11px;
  cursor: pointer;
}

.container-monitor-retry:hover {
  background: var(--accent);
}
</style>
