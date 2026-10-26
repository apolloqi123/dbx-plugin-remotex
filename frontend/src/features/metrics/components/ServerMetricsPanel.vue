<script lang="ts">
// 右侧栏服务器指标视图：纯展示组件，轮询/采样/进程动作仍由 useMetricsPanel
// 与 App.vue 持有。single `view` object 把 ref 解包后的快照一次传入，避免
// 十多个 props 的模板噪声。
import type { ServerMetrics } from "../composables/useMetricsPanel";
import type { DistroBadge } from "../../connections/lib/distroBadge";

export interface MetricsProcessRow {
  pid: number;
  ppid: number;
  user: string;
  cpuPercent: number;
  memPercent: number;
  etime: string;
  state: string;
  command: string;
  fdCount?: number | null;
  listenPorts?: number[];
}

export type MetricsProcessSortKey = "cpu" | "mem" | "pid" | "fd" | "ports";

export interface ServerMetricsView {
  metrics?: ServerMetrics;
  loading: boolean;
  error: string;
  distroBadge: DistroBadge | null;
  cpuSparkline: string;
  memSparkline: string;
  rxSparkline: string;
  txSparkline: string;
  cpuLast: number;
  memLast: number;
  disks: Array<{ filesystem: string; mount: string; totalBytes: number; usedBytes: number; availableBytes: number; percentUsed: number }>;
  networks: Array<{ name: string; rxRate: number; txRate: number; rxTotal: number; txTotal: number }>;
  processesOpen: boolean;
  processRows: MetricsProcessRow[];
  processLoading: boolean;
  processSortKey: MetricsProcessSortKey;
  sortedProcessTotal: number;
  visibleProcessRows: MetricsProcessRow[];
}
</script>

<script setup lang="ts">
import { computed } from "vue";
import { Loader2 } from "@lucide/vue";
import GpuNpuMonitor from "./GpuNpuMonitor.vue";
import { formatBytes, formatRate } from "../../../shared/lib/format";

const props = defineProps<{
  view: ServerMetricsView;
  t: (key: string, values?: Record<string, string | number>) => string;
  locale: string;
}>();

const emit = defineEmits<{
  refresh: [];
  "toggle-processes": [];
  "refresh-processes": [];
  "kill-process": [row: MetricsProcessRow, signal: 15 | 9];
  "update:sort": [key: MetricsProcessSortKey];
}>();

const metricsProcGridStyle = { gridTemplateColumns: "64px 80px 56px 56px minmax(0, 1fr)" };
const procGridStyle = { gridTemplateColumns: "64px 80px 56px 56px 96px 56px 96px minmax(0, 1fr) 132px" };

const metricsRatePeak = computed(() => {
  let peak = 0;
  for (const net of props.view.networks) peak = Math.max(peak, net.rxRate, net.txRate);
  return peak > 0 ? peak : 1;
});

function networkRateShare(net: { rxRate: number; txRate: number }) {
  return Math.min(100, Math.round((Math.max(net.rxRate, net.txRate) / metricsRatePeak.value) * 100));
}

function formatUptime(seconds: number) {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days >= 1) return props.t("uptimeDays", { count: days, hours });
  if (hours >= 1) return props.t("uptimeHours", { count: hours, minutes });
  return props.t("uptimeMinutes", { count: minutes });
}
</script>

<template>
  <div class="metrics-float-body metrics-side-body">
    <div v-if="view.loading && !view.metrics" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
    <p v-else-if="view.error" class="task-error">{{ view.error }} <button class="link-button" @click="emit('refresh')">{{ t("refresh") }}</button></p>
    <template v-else-if="view.metrics">
      <div class="metrics-grid">
        <div class="metric-card">
          <strong>{{ view.metrics.cpu?.percent ?? "–" }}%</strong>
          <span>{{ t("metricsCpu") }}</span>
          <small v-if="view.metrics.cpu?.cores">{{ view.metrics.cpu.cores }} vCPU · {{ view.metrics.cpu?.load1 ?? "–" }} / {{ view.metrics.cpu?.load5 ?? "–" }} / {{ view.metrics.cpu?.load15 ?? "–" }}</small>
        </div>
        <div class="metric-card">
          <strong>{{ view.metrics.memory?.totalBytes ? Math.round(((view.metrics.memory.usedBytes ?? 0) / view.metrics.memory.totalBytes) * 100) : "–" }}%</strong>
          <span>{{ t("metricsMemory") }}</span>
          <small v-if="view.metrics.memory?.totalBytes">{{ formatBytes(view.metrics.memory.usedBytes) }} / {{ formatBytes(view.metrics.memory.totalBytes) }}<template v-if="view.metrics.memory.swapTotalBytes"> · {{ t("metricsSwap") }} {{ formatBytes(view.metrics.memory.swapUsedBytes ?? 0) }}</template></small>
        </div>
        <div v-if="view.metrics.uptimeSeconds != null" class="metric-card metric-card--wide">
          <strong>{{ formatUptime(view.metrics.uptimeSeconds) }}</strong>
          <span>{{ t("metricsUptime") }}</span>
          <small v-if="view.metrics.kernel">{{ view.metrics.kernel }}</small>
        </div>
      </div>
      <div v-if="view.cpuSparkline || view.memSparkline" class="metrics-disks metrics-trend">
        <div class="disk-row">
          <span class="mono">{{ t("metricsCpu") }}</span>
          <svg class="metrics-sparkline metrics-trend-line" width="120" height="18" viewBox="0 0 120 18" role="img" aria-label="cpu trend"><polyline :points="view.cpuSparkline" fill="none" style="stroke: var(--info)" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round" /></svg>
          <span class="numeric">{{ Math.round(view.cpuLast) }}%</span>
        </div>
        <div class="disk-row">
          <span class="mono">{{ t("metricsMemory") }}</span>
          <svg class="metrics-sparkline metrics-trend-line" width="120" height="18" viewBox="0 0 120 18" role="img" aria-label="memory trend"><polyline :points="view.memSparkline" fill="none" style="stroke: var(--success)" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round" /></svg>
          <span class="numeric">{{ Math.round(view.memLast) }}%</span>
        </div>
      </div>
      <div v-if="view.disks.length" class="metrics-disks">
        <div v-for="disk in view.disks" :key="disk.mount" class="disk-row">
          <span class="mono" :title="disk.mount">{{ disk.mount }}</span>
          <progress :value="Math.min(100, disk.percentUsed)" max="100" :class="{ 'disk-warn': disk.percentUsed >= 85 }" />
          <span class="numeric">{{ formatBytes(disk.usedBytes) }} / {{ formatBytes(disk.totalBytes) }} · {{ Math.round(disk.percentUsed) }}%</span>
        </div>
      </div>
      <div v-if="view.networks.length">
        <h3 class="settings-section-title metrics-net-title">
          <span>{{ t("metricsNetwork") }}</span>
          <span v-if="view.rxSparkline || view.txSparkline" class="metrics-sparkline-group">
            <svg class="metrics-sparkline" width="60" height="18" viewBox="0 0 60 18" role="img" aria-label="rx"><polyline :points="view.rxSparkline" fill="none" style="stroke: var(--info)" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round" /></svg>
            <svg class="metrics-sparkline" width="60" height="18" viewBox="0 0 60 18" role="img" aria-label="tx"><polyline :points="view.txSparkline" fill="none" style="stroke: var(--success)" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round" /></svg>
          </span>
        </h3>
        <div class="metrics-disks">
          <div
            v-for="net in view.networks"
            :key="net.name"
            class="disk-row"
            :title="`rx ${formatBytes(net.rxTotal)} · tx ${formatBytes(net.txTotal)}`"
          >
            <span class="mono" :title="net.name">{{ net.name }}</span>
            <progress :value="networkRateShare(net)" max="100" />
            <span class="numeric">↓ {{ formatRate(net.rxRate) }} · ↑ {{ formatRate(net.txRate) }}</span>
          </div>
        </div>
      </div>
      <GpuNpuMonitor :locale="locale" :gpu="view.metrics.gpu" :npu="view.metrics.npu" />
      <div v-if="view.metrics.processes?.length">
        <h3 class="settings-section-title metrics-proc-title"><span>{{ t("metricsProc") }}</span><button class="link-button" @click="emit('toggle-processes')">{{ t(view.processesOpen ? "procCollapse" : "procManage") }}</button></h3>
        <div class="file-header" :style="metricsProcGridStyle">
          <span>{{ t("metricsProcPid") }}</span>
          <span>{{ t("metricsProcUser") }}</span>
          <span class="numeric">{{ t("metricsProcCpu") }}</span>
          <span class="numeric">{{ t("metricsProcMem") }}</span>
          <span>{{ t("metricsProcCommand") }}</span>
        </div>
        <div v-for="proc in view.metrics.processes" :key="proc.pid" class="file-row" :style="metricsProcGridStyle">
          <span class="mono">{{ proc.pid }}</span>
          <span class="mono">{{ proc.user }}</span>
          <span class="numeric" :class="{ 'proc-hot': proc.cpuPercent >= 50 }">{{ proc.cpuPercent }}%</span>
          <span class="numeric" :class="{ 'proc-hot': proc.memPercent >= 30 }">{{ proc.memPercent }}%</span>
          <span class="mono" :title="proc.command">{{ proc.command }}</span>
        </div>
      </div>
      <div v-if="view.processesOpen" class="proc-manage">
        <div class="command-history-header">
          <span>{{ t("procTitle", { count: view.processRows.length }) }}</span>
          <span class="batch-target-actions">
            <button class="link-button" @click="emit('refresh-processes')">{{ t("refresh") }}</button>
          </span>
        </div>
        <div class="proc-sort-row">
          <label v-for="key in (['cpu', 'mem', 'pid', 'fd', 'ports'] as const)" :key="key" class="proc-sort-option">
            <input type="radio" name="procSort" :value="key" :checked="view.processSortKey === key" @change="emit('update:sort', key)" />{{ t(`procSort.${key}`) }}
          </label>
        </div>
        <div v-if="view.processLoading && !view.visibleProcessRows.length" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
        <div v-else-if="!view.visibleProcessRows.length" class="empty compact">{{ t("procEmpty") }}</div>
        <template v-else>
          <div class="file-header" :style="procGridStyle">
            <span>{{ t("metricsProcPid") }}</span>
            <span>{{ t("metricsProcUser") }}</span>
            <span class="numeric">{{ t("metricsProcCpu") }}</span>
            <span class="numeric">{{ t("metricsProcMem") }}</span>
            <span>{{ t("procEtime") }}</span>
            <span class="numeric">{{ t("procFd") }}</span>
            <span>{{ t("procPorts") }}</span>
            <span>{{ t("metricsProcCommand") }}</span>
            <span></span>
          </div>
          <div v-for="proc in view.visibleProcessRows" :key="proc.pid" class="file-row" :style="procGridStyle">
            <span class="mono">{{ proc.pid }}</span>
            <span class="mono">{{ proc.user }}</span>
            <span class="numeric" :class="{ 'proc-hot': proc.cpuPercent >= 50 }">{{ proc.cpuPercent }}%</span>
            <span class="numeric" :class="{ 'proc-hot': proc.memPercent >= 30 }">{{ proc.memPercent }}%</span>
            <span class="mono">{{ proc.etime }}</span>
            <span class="numeric mono">{{ proc.fdCount ?? "—" }}</span>
            <span class="mono" :title="proc.listenPorts?.join(', ')">{{ proc.listenPorts?.length ? proc.listenPorts.join(", ") : "—" }}</span>
            <span class="mono" :title="proc.command">{{ proc.command }}</span>
            <span class="proc-kill-group">
              <button class="link-button" @click="emit('kill-process', proc, 15)">{{ t("procKill") }}</button>
              <button class="link-button proc-kill-force" @click="emit('kill-process', proc, 9)">{{ t("procKillForce") }}</button>
            </span>
          </div>
          <p v-if="view.sortedProcessTotal > view.visibleProcessRows.length" class="muted metrics-hint">{{ t("procCapped", { shown: view.visibleProcessRows.length, total: view.sortedProcessTotal }) }}</p>
        </template>
      </div>
      <p class="metrics-hint muted">{{ t("metricsRefreshHint") }}</p>
    </template>
  </div>
</template>

<style scoped>
.metrics-trend .metrics-trend-line { width: 120px; height: 18px; }
.proc-manage { margin-top: 10px; }
.proc-manage .settings-section-title { display: flex; justify-content: space-between; align-items: center; }
.proc-sort-row { display: flex; gap: 16px; margin: 6px 0; font-size: 12px; color: var(--muted-foreground); }
.proc-sort-option { display: inline-flex; align-items: center; gap: 4px; }
.proc-kill-group { display: flex; gap: 8px; justify-content: flex-end; }
.proc-kill-force { color: var(--destructive); }
</style>
