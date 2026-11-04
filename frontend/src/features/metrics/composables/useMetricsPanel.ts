import { computed, onBeforeUnmount, reactive, ref, watch, type Ref } from "vue";
import { confirmDialog } from "../../../shared/lib/confirmDialog";
import { documentHidden, onDocumentVisibilityChange } from "../../../shared/lib/documentVisibility";
import { distroBadge, type DistroBadge } from "../../connections/lib/distroBadge";
import { pushSample, sparklinePath, METRICS_SAMPLE_CAPACITY } from "../lib/metricsSparkline";
import { filterDiskMounts, filterNetworkInterfaces } from "../lib/metricsView";
import { pluginStore } from "../../../shared/lib/pluginStore";
import { canKillProcess, sortProcessRows, type ProcessSortKey } from "../../docker/lib/processActions";
import { type GpuOverviewView, type NpuOverviewView } from "../lib/metricsGpuNpu";

export interface ServerMetrics {
  hostname?: string | null;
  kernel?: string | null;
  uptimeSeconds?: number | null;
  cpu?: {
    cores?: number | null;
    percent?: number | null;
    load1?: number | null;
    load5?: number | null;
    load15?: number | null;
    // New sidecars report one row per logical core; old ones omit the field.
    perCore?: Array<{ index: number; percent?: number | null }> | null;
  };
  memory?: {
    totalBytes?: number;
    availableBytes?: number;
    usedBytes?: number;
    // Subset of usedBytes (buffers + page cache) and the non-used remainder.
    cacheBytes?: number;
    freeBytes?: number;
    swapTotalBytes?: number;
    swapUsedBytes?: number;
  };
  disks?: Array<{ filesystem: string; mount: string; totalBytes: number; usedBytes: number; availableBytes: number; percentUsed: number; inodeUsePercent?: number | null }>;
  // Extensions reported by newer sidecars; when absent the network and
  // process sections simply stay hidden instead of erroring.
  network?: Array<{ name: string; rxRate: number; txRate: number; rxTotal: number; txTotal: number }>;
  processes?: Array<{ pid: number; user: string; cpuPercent: number; memPercent: number; command: string }>;
  // §1.5 发行版识别（/etc/os-release）：读不到时两字段整体缺省，
  // 旧 sidecar 自然缺失，前端不渲染徽标（optional 降级）。
  osId?: string;
  osPretty?: string;
  // GPU / Ascend NPU 总览（Task P1-4）：仅新 sidecar 输出，缺省时监控面板
  // 不渲染 GPU/NPU 区（optional 降级）。
  gpu?: GpuOverviewView;
  npu?: NpuOverviewView;
}

/** 右侧栏服务器指标视图 + 进程管理（F2）：5s 轮询、落盘历史回填、sparkline
 * 采样环、发行版徽标与进程查杀确认。打开录制列表/传输抽屉时互斥收口。 */
export function useMetricsPanel(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  showNotice: (message: string) => void;
  showError: (cause: unknown, target?: "terminal" | "sftp") => void;
  session: Ref<{ sessionId?: string } | undefined>;
  transferPanelOpen: Ref<boolean>;
  recordingsOpen: Ref<boolean>;
}) {
  const { t, showNotice, showError, session, transferPanelOpen, recordingsOpen } = options;

const metricsOpen = ref(false);
// F2：进程管理面板 + 排序键；F3：录制/回放状态。
interface ProcessRow {
  pid: number;
  ppid: number;
  user: string;
  cpuPercent: number;
  memPercent: number;
  etime: string;
  state: string;
  command: string;
  // iShell 对标列（best-effort）：句柄数与监听端口。旧 sidecar / 非 Linux
  // 主机可能缺失，未知以 null/空数组表示并显示占位符。
  fdCount?: number | null;
  listenPorts?: number[];
}
const processesOpen = ref(false);
const processRows = ref<ProcessRow[]>([]);
const processLoading = ref(false);
const processSortKey = ref<ProcessSortKey>("cpu");
const metrics = ref<ServerMetrics>();
const metricsLoading = ref(false);
const metricsError = ref("");

// 磁盘/网卡各自的去噪开关（默认只显示主要信息；「显示全部」时伪文件系统或
// 虚拟网卡一并列出）。偏好全局持久化，随宿主 storage 同步；beta.82 的合并
// 开关键 ssh-metrics-only-main 作为旧值回退，升级后保持用户上次的选择。
const METRICS_SHOW_ALL_DISKS_KEY = "ssh-metrics-show-all-disks";
const METRICS_SHOW_ALL_NICS_KEY = "ssh-metrics-show-all-nics";
const METRICS_LEGACY_ONLY_MAIN_KEY = "ssh-metrics-only-main";
function readShowAllPref(key: string): boolean {
  try {
    const stored = pluginStore.getItem(key);
    if (stored !== null) return stored === "true";
    return pluginStore.getItem(METRICS_LEGACY_ONLY_MAIN_KEY) === "false";
  } catch {
    return false;
  }
}
function persistShowAllPref(key: string, value: boolean) {
  try {
    pluginStore.setItem(key, String(value));
  } catch {
    // 持久化失败不阻断本次会话内的开关。
  }
}
const showAllDisks = ref(readShowAllPref(METRICS_SHOW_ALL_DISKS_KEY));
const showAllNics = ref(readShowAllPref(METRICS_SHOW_ALL_NICS_KEY));
function toggleDisksDensity() {
  showAllDisks.value = !showAllDisks.value;
  persistShowAllPref(METRICS_SHOW_ALL_DISKS_KEY, showAllDisks.value);
}
function toggleNicsDensity() {
  showAllNics.value = !showAllNics.value;
  persistShowAllPref(METRICS_SHOW_ALL_NICS_KEY, showAllNics.value);
}

// ---------------------------------------------------------------------------
// metrics sparkline + 发行版徽标（IMPL_PLAN_NETCATTY_PARITY §3-B2）
// ---------------------------------------------------------------------------

// 每方向环形采样（60 帧 × 5s 轮询 ≈ 5 分钟）；跨重连（新 session）清空。
// F2：cpu/mem 环形同样 60 帧，打开指标卡时用落盘历史回填（跨重启可见趋势）。
const metricSamples = reactive({ rx: [] as number[], tx: [] as number[], cpu: [] as number[], mem: [] as number[] });

function recordMetricSamples() {
  let rx = 0;
  let tx = 0;
  // 趋势与卡片同源：只统计可见网卡，容器/回环流量不再重复计入总速率。
  for (const net of visibleNetworkInterfaces.value) {
    rx += Math.max(0, net.rxRate || 0);
    tx += Math.max(0, net.txRate || 0);
  }
  metricSamples.rx = pushSample(metricSamples.rx, rx, METRICS_SAMPLE_CAPACITY);
  metricSamples.tx = pushSample(metricSamples.tx, tx, METRICS_SAMPLE_CAPACITY);
  const cpuPercent = metrics.value?.cpu?.percent;
  if (cpuPercent != null) metricSamples.cpu = pushSample(metricSamples.cpu, cpuPercent, METRICS_SAMPLE_CAPACITY);
  const totalBytes = metrics.value?.memory?.totalBytes ?? 0;
  if (totalBytes > 0) metricSamples.mem = pushSample(metricSamples.mem, ((metrics.value?.memory?.usedBytes ?? 0) / totalBytes) * 100, METRICS_SAMPLE_CAPACITY);
}

// 网络速率没有绝对上限：按双向共同峰值留 15% 顶部余量，峰值不会贴边。
const metricsNetScale = computed(() => {
  const peak = Math.max(0, ...metricSamples.rx, ...metricSamples.tx);
  return peak > 0 ? peak * 1.15 : 0;
});
const metricsRxSparkline = computed(() => sparklinePath(metricSamples.rx, 60, 18, metricsNetScale.value));
const metricsTxSparkline = computed(() => sparklinePath(metricSamples.tx, 60, 18, metricsNetScale.value));
// CPU/内存是百分比：固定 0–100 刻度，低占用不会被拉伸到顶边。
const metricsCpuSparkline = computed(() => sparklinePath(metricSamples.cpu, 120, 18, 100));
const metricsMemSparkline = computed(() => sparklinePath(metricSamples.mem, 120, 18, 100));
// 视图层去噪：伪文件系统/overlay 重复挂载/虚拟网卡默认不进渲染（纯函数在
// lib/metricsView）；「显示全部」开关把它们放回来。
const visibleDiskMounts = computed(() => filterDiskMounts(metrics.value?.disks, { includeNoise: showAllDisks.value }));
const visibleNetworkInterfaces = computed(() => filterNetworkInterfaces(metrics.value?.network, { includeNoise: showAllNics.value }));
// 旧 sidecar 无 osId/osPretty 时整体缺徽标（optional 降级，§6.6）。
const metricsDistroBadge = computed<DistroBadge | null>(() => (metrics.value ? distroBadge(metrics.value.osId, metrics.value.osPretty) : null));

let metricsTimer = 0;

async function refreshMetrics() {
  if (!session.value) return;
  metricsLoading.value = true;
  try {
    metrics.value = await window.dbxPlugin.invoke<ServerMetrics>("ssh/metrics", { sessionId: session.value.sessionId }, { timeoutMs: 30_000 });
    metricsError.value = "";
    recordMetricSamples();
  } catch (cause) {
    // 错误卡直渲染 metricsError 原文：裸 sidecar 错误串不进 UI（showError
    // 约定同源），细节留给 console 排查。
    console.warn("[metrics] refresh failed", cause);
    metricsError.value = t("metricsRefreshFailed");
  } finally {
    metricsLoading.value = false;
  }
}

// 指标视图：打开即刷新并启动 5s 轮询；不阻塞终端操作，随时开关。
// 与传输抽屉、录制视图互斥，避免叠加。
function openMetrics() {
  transferPanelOpen.value = false;
  recordingsOpen.value = false;
  metricsOpen.value = true;
  void backfillMetricsHistory();
  void refreshMetrics();
  window.clearInterval(metricsTimer);
  metricsTimer = window.setInterval(() => {
    // 面板还开着但窗口在后台：不发这次远端采集（一个 poll 是若干 exec
    // 通道），复现时由 visibilitychange 立即补一拍。
    if (metricsOpen.value && !metricsLoading.value && !documentHidden()) void refreshMetrics();
  }, 5000);
}

function toggleMetrics() {
  if (metricsOpen.value) closeMetrics();
  else openMetrics();
}

function closeMetrics() {
  metricsOpen.value = false;
  window.clearInterval(metricsTimer);
}

// —— F2：指标历史回填 + 进程管理 ———

// 打开指标卡时拉一次落盘历史（connectionId 维度，跨重启可见趋势）；
// 旧 sidecar 无该方法时静默降级。
async function backfillMetricsHistory() {
  if (!session.value) return;
  try {
    const result = await window.dbxPlugin.invoke<{ samples: Array<{ cpuPercent?: number; memoryPercent?: number; rxRate?: number; txRate?: number }> }>("ssh/metrics/history", { sessionId: session.value.sessionId, limit: METRICS_SAMPLE_CAPACITY });
    for (const sample of result.samples ?? []) {
      if (sample.cpuPercent != null) metricSamples.cpu = pushSample(metricSamples.cpu, sample.cpuPercent, METRICS_SAMPLE_CAPACITY);
      if (sample.memoryPercent != null) metricSamples.mem = pushSample(metricSamples.mem, sample.memoryPercent, METRICS_SAMPLE_CAPACITY);
      metricSamples.rx = pushSample(metricSamples.rx, Math.max(0, sample.rxRate ?? 0), METRICS_SAMPLE_CAPACITY);
      metricSamples.tx = pushSample(metricSamples.tx, Math.max(0, sample.txRate ?? 0), METRICS_SAMPLE_CAPACITY);
    }
  } catch {
    // optional 降级：无历史则趋势从本次打开开始累计。
  }
}

async function toggleProcessPanel() {
  processesOpen.value = !processesOpen.value;
  if (processesOpen.value) await refreshProcessList();
}

async function refreshProcessList() {
  if (!session.value) return;
  processLoading.value = true;
  try {
    const result = await window.dbxPlugin.invoke<{ processes: ProcessRow[] }>("ssh/processes/list", { sessionId: session.value.sessionId }, { timeoutMs: 20_000 });
    processRows.value = result.processes ?? [];
  } catch (cause) {
    showError(cause);
  } finally {
    processLoading.value = false;
  }
}

const sortedProcessRows = computed(() => sortProcessRows(processRows.value, processSortKey.value));
const PROCESS_VISIBLE_LIMIT = 100;
const visibleProcessRows = computed(() => sortedProcessRows.value.slice(0, PROCESS_VISIBLE_LIMIT));

async function killProcessRow(row: ProcessRow, signal: 15 | 9) {
  if (!session.value || !canKillProcess(row.pid)) return;
  const confirmKey = signal === 9 ? "procKillForceConfirm" : "procKillConfirm";
  if (!(await confirmDialog(t(confirmKey, { pid: row.pid, command: row.command })))) return;

  try {
    await window.dbxPlugin.invoke("ssh/processes/kill", { sessionId: session.value.sessionId, pid: row.pid, signal });
    showNotice(t("procKilled", { pid: row.pid }));
    await refreshProcessList();
  } catch (cause) {
    showError(cause);
  }
}


const stopVisibilityWatch = onDocumentVisibilityChange((hidden) => {
  if (!hidden && metricsOpen.value && !metricsLoading.value) void refreshMetrics();
});

onBeforeUnmount(() => {
  window.clearInterval(metricsTimer);
  stopVisibilityWatch();
});

// 换会话清空采样环（跨重连不残留旧趋势）。
watch(() => session.value?.sessionId, (next, previous) => {
  if (next !== previous) {
    metricSamples.rx = [];
    metricSamples.tx = [];
    metricSamples.cpu = [];
    metricSamples.mem = [];
  }
});

  return {
    metricsOpen,
    processesOpen,
    processRows,
    processLoading,
    processSortKey,
    metrics,
    metricsLoading,
    metricsError,
    metricSamples,
    recordMetricSamples,
    metricsRxSparkline,
    metricsTxSparkline,
    metricsCpuSparkline,
    metricsMemSparkline,
    visibleDiskMounts,
    visibleNetworkInterfaces,
    showAllDisks,
    showAllNics,
    toggleDisksDensity,
    toggleNicsDensity,
    metricsDistroBadge,
    refreshMetrics,
    openMetrics,
    toggleMetrics,
    closeMetrics,
    backfillMetricsHistory,
    toggleProcessPanel,
    refreshProcessList,
    sortedProcessRows,
    PROCESS_VISIBLE_LIMIT,
    visibleProcessRows,
    killProcessRow,
  };
}
