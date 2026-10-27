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
  disks: Array<{ filesystem: string; mount: string; totalBytes: number; usedBytes: number; availableBytes: number; percentUsed: number; inodeUsePercent?: number | null }>;
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
import {
  ArrowDown,
  ArrowUp,
  Cpu,
  HardDrive,
  ListTree,
  Loader2,
  MemoryStick,
  Network,
  Server,
} from "@lucide/vue";
import GpuNpuMonitor from "./GpuNpuMonitor.vue";
import MetricCard from "./MetricCard.vue";
import MetricsCoreGrid from "./MetricsCoreGrid.vue";
import MetricsMemoryDonut from "./MetricsMemoryDonut.vue";
import MetricsTrendChart from "./MetricsTrendChart.vue";
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

const procGridStyle = { gridTemplateColumns: "64px 80px 56px 56px 96px 56px 96px minmax(0, 1fr) 132px" };

const cpuPercent = computed(() => props.view.metrics?.cpu?.percent ?? null);
const cpuCores = computed(() => props.view.metrics?.cpu?.cores ?? null);
const coreRows = computed(() =>
  (props.view.metrics?.cpu?.perCore ?? []).filter((core) => core && core.index != null),
);
const loadText = computed(() => {
  const cpu = props.view.metrics?.cpu;
  if (!cpu || (cpu.load1 == null && cpu.load5 == null && cpu.load15 == null)) return "";
  return [cpu.load1, cpu.load5, cpu.load15]
    .map((value) => (value == null ? "–" : value.toFixed(2)))
    .join(" / ");
});

const memTotal = computed(() => props.view.metrics?.memory?.totalBytes ?? 0);
const memUsed = computed(() => {
  const memory = props.view.metrics?.memory;
  if (!memory?.totalBytes) return 0;
  return memory.usedBytes ?? memory.totalBytes - (memory.availableBytes ?? memory.totalBytes);
});
const memCache = computed(() => props.view.metrics?.memory?.cacheBytes ?? 0);
const swapTotal = computed(() => props.view.metrics?.memory?.swapTotalBytes ?? 0);
const swapUsed = computed(() => props.view.metrics?.memory?.swapUsedBytes ?? 0);
const swapPercent = computed(() =>
  swapTotal.value > 0 ? Math.min(100, (swapUsed.value / swapTotal.value) * 100) : 0,
);

const diskTotals = computed(() =>
  props.view.disks.reduce(
    (totals, disk) => ({
      total: totals.total + (disk.totalBytes ?? 0),
      used: totals.used + (disk.usedBytes ?? 0),
    }),
    { total: 0, used: 0 },
  ),
);
const diskMeta = computed(() =>
  diskTotals.value.total > 0
    ? `${formatBytes(diskTotals.value.used)} / ${formatBytes(diskTotals.value.total)}`
    : "",
);

const netRates = computed(() =>
  props.view.networks.reduce(
    (rates, net) => ({ rx: rates.rx + (net.rxRate ?? 0), tx: rates.tx + (net.txRate ?? 0) }),
    { rx: 0, tx: 0 },
  ),
);

const topProcesses = computed(() => props.view.metrics?.processes ?? []);

const hostname = computed(() => props.view.metrics?.hostname || "");
const osPretty = computed(() => props.view.metrics?.osPretty || "");
// 发行版徽标已经展示完整名称时不再重复“系统版本”一行。
const osFact = computed(() =>
  osPretty.value && osPretty.value !== props.view.distroBadge?.name ? osPretty.value : "",
);

function clampPercent(value: number | null | undefined) {
  return `${Math.max(0, Math.min(100, value ?? 0))}%`;
}

function formatUptime(seconds: number) {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days >= 1) return props.t("uptimeDays", { count: days, hours });
  if (hours >= 1) return props.t("uptimeHours", { count: hours, minutes });
  return props.t("uptimeMinutes", { count: minutes });
}

const uptimeText = computed(() =>
  props.view.metrics?.uptimeSeconds != null ? formatUptime(props.view.metrics.uptimeSeconds) : "",
);
</script>

<template>
  <div class="metrics-float-body metrics-side-body">
    <div v-if="view.loading && !view.metrics" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
    <p v-else-if="view.error" class="task-error">{{ view.error }} <button class="link-button" @click="emit('refresh')">{{ t("refresh") }}</button></p>
    <div v-else-if="view.metrics" class="metrics-dash">
      <MetricCard :icon="Server" :title="t('metricsSystem')" :meta="uptimeText">
        <div class="metrics-host-row">
          <strong class="metrics-host-name" :title="hostname">{{ hostname || "–" }}</strong>
          <span v-if="view.distroBadge" class="metrics-distro" :title="view.distroBadge.name">
            <span class="metrics-distro-dot" :style="{ background: view.distroBadge.color }" />
            {{ view.distroBadge.name }}
          </span>
        </div>
        <dl class="metrics-facts">
          <div v-if="osFact"><dt>{{ t("metricsOs") }}</dt><dd :title="osFact">{{ osFact }}</dd></div>
          <div v-if="view.metrics.kernel"><dt>{{ t("metricsKernel") }}</dt><dd class="mono" :title="view.metrics.kernel">{{ view.metrics.kernel }}</dd></div>
          <div v-if="cpuCores"><dt>{{ t("metricsCores") }}</dt><dd>{{ cpuCores }} vCPU</dd></div>
          <div v-if="loadText"><dt>{{ t("metricsLoad") }}</dt><dd class="mono" :title="loadText">{{ loadText }}</dd></div>
        </dl>
      </MetricCard>

      <MetricCard :icon="Cpu" tone="emerald" :title="t('metricsCpu')" :meta="cpuCores ? `${cpuCores} vCPU` : undefined">
        <div class="metrics-hero">
          <strong class="metrics-hero-value">{{ cpuPercent ?? "–" }}<span>%</span></strong>
          <span v-if="loadText" class="metrics-hero-note">{{ t("metricsLoad") }} <b>{{ loadText }}</b></span>
        </div>
        <MetricsTrendChart :points="view.cpuSparkline" tone="emerald" />
        <p v-if="!view.cpuSparkline" class="metrics-chart-hint">{{ t("metricsChartPending") }}</p>
        <div v-if="coreRows.length" class="metrics-core-section">
          <div class="metrics-subhead"><span>{{ t("metricsPerCore") }}</span><span>{{ coreRows.length }}</span></div>
          <MetricsCoreGrid :cores="coreRows" />
        </div>
      </MetricCard>

      <MetricCard v-if="memTotal" :icon="MemoryStick" tone="blue" :title="t('metricsMemory')" :meta="formatBytes(memTotal)">
        <MetricsMemoryDonut
          :total-bytes="memTotal"
          :used-bytes="memUsed"
          :cache-bytes="memCache"
          :used-label="t('metricsUsed')"
          :cache-label="t('metricsCache')"
          :free-label="t('metricsFree')"
        />
        <div v-if="swapTotal" class="metrics-swap">
          <div class="metrics-swap-head">
            <span>{{ t("metricsSwap") }}</span>
            <span>{{ formatBytes(swapUsed) }} / {{ formatBytes(swapTotal) }}</span>
          </div>
          <div class="metrics-bar is-violet"><span :style="{ width: `${swapPercent}%` }" /></div>
        </div>
      </MetricCard>

      <MetricCard v-if="view.networks.length" :icon="Network" tone="cyan" :title="t('metricsNetwork')">
        <div class="metrics-legend-row">
          <span class="metrics-legend-item is-down"><ArrowDown />{{ t("metricsDownload") }} <b>{{ formatRate(netRates.rx) }}</b></span>
          <span class="metrics-legend-item is-up"><ArrowUp />{{ t("metricsUpload") }} <b>{{ formatRate(netRates.tx) }}</b></span>
        </div>
        <MetricsTrendChart
          v-if="view.rxSparkline || view.txSparkline"
          :points="view.rxSparkline"
          :secondary-points="view.txSparkline"
          tone="cyan"
          secondary-tone="violet"
          :view-width="60"
          :fill="false"
        />
        <p v-else class="metrics-chart-hint">{{ t("metricsChartPending") }}</p>
        <div class="metrics-net-list">
          <div v-for="net in view.networks" :key="net.name" class="metrics-net-row">
            <span class="metrics-net-name mono" :title="net.name">{{ net.name }}</span>
            <span class="metrics-net-total">↓ {{ formatBytes(net.rxTotal) }} · ↑ {{ formatBytes(net.txTotal) }}</span>
            <span class="metrics-net-rate is-down">↓ {{ formatRate(net.rxRate) }}</span>
            <span class="metrics-net-rate is-up">↑ {{ formatRate(net.txRate) }}</span>
          </div>
        </div>
      </MetricCard>

      <MetricCard v-if="view.disks.length" :icon="HardDrive" tone="amber" :title="t('metricsDisk')" :meta="diskMeta">
        <div v-for="disk in view.disks" :key="disk.mount" class="metrics-disk">
          <div class="metrics-disk-head">
            <span class="metrics-disk-mount mono" :title="disk.mount">{{ disk.mount }}</span>
            <span class="metrics-disk-chip" :class="{ 'is-warn': disk.percentUsed >= 85 }">{{ Math.round(disk.percentUsed) }}%</span>
          </div>
          <div class="metrics-bar" :class="disk.percentUsed >= 85 ? 'is-rose' : 'is-amber'">
            <span :style="{ width: clampPercent(disk.percentUsed) }" />
          </div>
          <div class="metrics-disk-meta">
            <span class="mono" :title="disk.filesystem">{{ disk.filesystem }}</span>
            <span>{{ formatBytes(disk.usedBytes) }} / {{ formatBytes(disk.totalBytes) }}</span>
          </div>
          <div v-if="disk.inodeUsePercent != null" class="metrics-disk-meta is-inode">
            <span>{{ t("metricsInodes") }}</span>
            <span>{{ Math.round(disk.inodeUsePercent) }}%</span>
          </div>
        </div>
      </MetricCard>

      <GpuNpuMonitor :locale="locale" :gpu="view.metrics.gpu" :npu="view.metrics.npu" />

      <MetricCard v-if="topProcesses.length" :icon="ListTree" tone="violet" :title="t('metricsProc')">
        <template #actions>
          <span class="metrics-card-action">
            <button class="link-button" @click="emit('toggle-processes')">{{ t(view.processesOpen ? "procCollapse" : "metricsViewAll") }}</button>
          </span>
        </template>
        <div class="metrics-proc-list">
          <div v-for="proc in topProcesses" :key="proc.pid" class="metrics-proc-row">
            <div class="metrics-proc-main">
              <span class="mono metrics-proc-command" :title="proc.command">{{ proc.command }}</span>
              <small>PID {{ proc.pid }} · {{ proc.user }}</small>
            </div>
            <div class="metrics-proc-stat">
              <span>CPU</span><b :class="{ 'is-hot': proc.cpuPercent >= 50 }">{{ proc.cpuPercent }}%</b>
              <div class="metrics-proc-bar"><span :style="{ width: clampPercent(proc.cpuPercent) }" /></div>
            </div>
            <div class="metrics-proc-stat">
              <span>MEM</span><b :class="{ 'is-hot': proc.memPercent >= 30 }">{{ proc.memPercent }}%</b>
              <div class="metrics-proc-bar is-mem"><span :style="{ width: clampPercent(proc.memPercent) }" /></div>
            </div>
          </div>
        </div>
      </MetricCard>

      <div v-if="view.processesOpen" class="metrics-proc-manager">
        <div class="metrics-proc-manager-head">
          <span>{{ t("procTitle", { count: view.processRows.length }) }}</span>
          <button class="link-button" @click="emit('refresh-processes')">{{ t("refresh") }}</button>
        </div>
        <div class="metrics-sort-row">
          <label v-for="key in (['cpu', 'mem', 'pid', 'fd', 'ports'] as const)" :key="key" class="metrics-sort-chip">
            <input type="radio" name="procSort" :value="key" :checked="view.processSortKey === key" @change="emit('update:sort', key)" />{{ t(`procSort.${key}`) }}
          </label>
        </div>
        <div v-if="view.processLoading && !view.visibleProcessRows.length" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
        <div v-else-if="!view.visibleProcessRows.length" class="empty compact">{{ t("procEmpty") }}</div>
        <template v-else>
          <div class="metrics-proc-table">
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
          </div>
          <p v-if="view.sortedProcessTotal > view.visibleProcessRows.length" class="metrics-hint muted">{{ t("procCapped", { shown: view.visibleProcessRows.length, total: view.sortedProcessTotal }) }}</p>
        </template>
      </div>
      <p class="metrics-hint muted">{{ t("metricsRefreshHint") }}</p>
    </div>
  </div>
</template>

<style scoped>
.metrics-dash { display: flex; flex-direction: column; gap: 10px; }

/* —— 系统卡 —— */
.metrics-host-row { display: flex; min-width: 0; align-items: center; gap: 8px; }
.metrics-host-name {
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
  color: var(--foreground);
  font-size: 15px;
  font-weight: 650;
  letter-spacing: -0.01em;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.metrics-distro {
  display: inline-flex;
  max-width: 55%;
  flex: 0 0 auto;
  align-items: center;
  gap: 5px;
  overflow: hidden;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-pill);
  padding: 2px 8px 2px 6px;
  color: var(--muted-foreground);
  font-size: 10px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.metrics-distro-dot { width: 7px; height: 7px; flex: 0 0 7px; border-radius: 50%; }
.metrics-facts {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 7px 12px;
  margin: 0;
}
.metrics-facts > div { display: flex; min-width: 0; flex-direction: column; gap: 1px; }
.metrics-facts dt {
  color: var(--muted-foreground);
  font-size: 9px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
.metrics-facts dd {
  margin: 0;
  overflow: hidden;
  color: var(--foreground);
  font-size: 10.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* —— 数值主行 —— */
.metrics-hero { display: flex; align-items: flex-end; justify-content: space-between; gap: 10px; }
.metrics-hero-value {
  color: var(--foreground);
  font-size: 30px;
  font-weight: 700;
  letter-spacing: -0.02em;
  line-height: 1;
  font-variant-numeric: tabular-nums;
}
.metrics-hero-value > span { margin-left: 2px; color: var(--muted-foreground); font-size: 13px; font-weight: 500; }
.metrics-hero-note { color: var(--muted-foreground); font-size: 9.5px; text-align: right; }
.metrics-hero-note b { display: block; color: var(--foreground); font-family: var(--terminal-font-family); font-size: 10px; font-variant-numeric: tabular-nums; }
.metrics-chart-hint { margin: -4px 0 0; color: var(--muted-foreground); font-size: 9.5px; }

.metrics-subhead {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
  border-top: 1px solid color-mix(in srgb, var(--card-border) 65%, transparent);
  padding-top: 8px;
  color: var(--muted-foreground);
  font-size: 9px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
.metrics-core-section { display: flex; flex-direction: column; }

/* —— 通用进度条 —— */
.metrics-bar {
  height: 6px;
  overflow: hidden;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--foreground) 9%, transparent);
}
.metrics-bar > span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--tone-amber);
  transition: width 400ms cubic-bezier(0.4, 0, 0.2, 1);
}
.metrics-bar.is-amber > span { background: linear-gradient(90deg, var(--tone-emerald), var(--tone-amber)); }
.metrics-bar.is-rose > span { background: linear-gradient(90deg, var(--tone-amber), var(--tone-rose)); }
.metrics-bar.is-violet > span { background: var(--tone-violet); }

/* —— 内存 —— */
.metrics-swap { display: flex; flex-direction: column; gap: 5px; border-top: 1px solid color-mix(in srgb, var(--card-border) 65%, transparent); padding-top: 8px; }
.metrics-swap-head { display: flex; justify-content: space-between; gap: 8px; color: var(--muted-foreground); font-size: 9.5px; font-variant-numeric: tabular-nums; }

/* —— 网络 —— */
.metrics-legend-row { display: flex; flex-wrap: wrap; gap: 4px 14px; }
.metrics-legend-item { display: inline-flex; align-items: center; gap: 4px; color: var(--muted-foreground); font-size: 9.5px; }
.metrics-legend-item > svg { width: 11px; height: 11px; stroke-width: 2; }
.metrics-legend-item.is-down > svg { color: var(--tone-cyan); }
.metrics-legend-item.is-up > svg { color: var(--tone-violet); }
.metrics-legend-item b { color: var(--foreground); font-family: var(--terminal-font-family); font-size: 10px; font-variant-numeric: tabular-nums; }
.metrics-net-list { display: flex; flex-direction: column; }
.metrics-net-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 2px 10px;
  padding: 7px 0;
}
.metrics-net-row + .metrics-net-row { border-top: 1px dashed color-mix(in srgb, var(--card-border) 70%, transparent); }
.metrics-net-name { grid-column: 1; grid-row: 1; overflow: hidden; color: var(--foreground); font-size: 10.5px; text-overflow: ellipsis; white-space: nowrap; }
.metrics-net-total { grid-column: 1; grid-row: 2; color: var(--muted-foreground); font-size: 9px; font-variant-numeric: tabular-nums; }
.metrics-net-rate { font-family: var(--terminal-font-family); font-size: 10px; font-variant-numeric: tabular-nums; text-align: right; }
.metrics-net-rate.is-down { grid-column: 2; grid-row: 1; color: var(--tone-cyan); }
.metrics-net-rate.is-up { grid-column: 2; grid-row: 2; color: var(--tone-violet); }

/* —— 磁盘 —— */
.metrics-disk { display: flex; flex-direction: column; gap: 5px; }
.metrics-disk + .metrics-disk {
  margin-top: 3px;
  border-top: 1px dashed color-mix(in srgb, var(--card-border) 70%, transparent);
  padding-top: 8px;
}
.metrics-disk-head { display: flex; min-width: 0; align-items: center; justify-content: space-between; gap: 8px; }
.metrics-disk-mount { min-width: 0; overflow: hidden; color: var(--foreground); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.metrics-disk-chip {
  flex: 0 0 auto;
  border-radius: var(--radius-pill);
  padding: 1px 7px;
  background: var(--tone-amber-soft);
  color: var(--tone-amber);
  font-size: 9.5px;
  font-variant-numeric: tabular-nums;
}
.metrics-disk-chip.is-warn { background: var(--tone-rose-soft); color: var(--tone-rose); }
.metrics-disk-meta { display: flex; justify-content: space-between; gap: 10px; color: var(--muted-foreground); font-size: 9px; font-variant-numeric: tabular-nums; }
.metrics-disk-meta > span:first-child { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.metrics-disk-meta.is-inode { font-size: 8.5px; opacity: 0.8; }

/* —— 进程 —— */
.metrics-card-action { margin-left: auto; }
.metrics-card-action .link-button { font-size: 9.5px; white-space: nowrap; }
.metrics-proc-list { display: flex; flex-direction: column; }
.metrics-proc-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 64px 64px;
  align-items: center;
  gap: 8px;
  padding: 6px 0;
}
.metrics-proc-row + .metrics-proc-row { border-top: 1px dashed color-mix(in srgb, var(--card-border) 70%, transparent); }
.metrics-proc-main { display: flex; min-width: 0; flex-direction: column; gap: 1px; }
.metrics-proc-command { overflow: hidden; color: var(--foreground); font-size: 10.5px; text-overflow: ellipsis; white-space: nowrap; }
.metrics-proc-main small { overflow: hidden; color: var(--muted-foreground); font-size: 8.5px; text-overflow: ellipsis; white-space: nowrap; }
.metrics-proc-stat { display: grid; grid-template-columns: 1fr auto; align-items: baseline; gap: 2px 4px; }
.metrics-proc-stat > span { color: var(--muted-foreground); font-size: 8.5px; letter-spacing: 0.03em; }
.metrics-proc-stat > b { color: var(--foreground); font-family: var(--terminal-font-family); font-size: 9.5px; font-variant-numeric: tabular-nums; }
.metrics-proc-stat > b.is-hot { color: var(--tone-rose); }
.metrics-proc-bar { grid-column: 1 / -1; height: 3px; overflow: hidden; border-radius: var(--radius-pill); background: color-mix(in srgb, var(--foreground) 9%, transparent); }
.metrics-proc-bar > span { display: block; height: 100%; border-radius: inherit; background: linear-gradient(90deg, var(--tone-emerald), var(--tone-amber)); transition: width 400ms cubic-bezier(0.4, 0, 0.2, 1); }
.metrics-proc-bar.is-mem > span { background: linear-gradient(90deg, var(--tone-blue), var(--tone-violet)); }

/* —— 进程管理表 —— */
.metrics-proc-manager { display: flex; flex-direction: column; gap: 8px; border-top: 1px solid var(--card-border); padding-top: 10px; }
.metrics-proc-manager-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: 11px; font-weight: 600; }
.metrics-sort-row { display: flex; flex-wrap: wrap; gap: 6px; }
.metrics-sort-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-pill);
  padding: 3px 8px;
  color: var(--muted-foreground);
  font-size: 9.5px;
  cursor: pointer;
}
.metrics-sort-chip:has(input:checked) { border-color: color-mix(in srgb, var(--primary) 45%, var(--card-border)); color: var(--foreground); }
.metrics-sort-chip input { width: 10px; height: 10px; margin: 0; accent-color: var(--primary); }
.metrics-proc-table {
  overflow-x: auto;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-sm);
  background: var(--card-surface);
}
.metrics-proc-table .file-header,
.metrics-proc-table .file-row { min-width: 620px; }
.metrics-proc-table .file-header { position: static; }
.proc-hot { color: var(--warning); font-weight: 600; }
.proc-kill-group { display: flex; gap: 8px; justify-content: flex-end; }
.proc-kill-force { color: var(--destructive); }
</style>
