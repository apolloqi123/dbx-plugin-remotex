import { computed, onBeforeUnmount, watch, type Ref } from "vue";
import type { ServerMetrics } from "./useMetricsPanel";
import { filterNetworkInterfaces } from "../lib/metricsView";

/**
 * dock 面板左上角系统信息（CPU/内存/网速）：复用 ssh/metrics 既有链路。
 * 本地终端（运行中或恢复的 shell）没有 SSH 会话，改采本机样本；轮询在
 * 连接建立（或本地终端激活）后按 5s 环境间隔运行，指标卡打开时让位其
 * 自带轮询，隐藏的 webview 不打扰。定时器与卸载清理归本 composable。
 */
export function useDockMetrics(options: {
  t: (key: string, values?: Record<string, string | number>) => string;
  isLocalMode: Ref<boolean>;
  localShellRestored: Ref<boolean>;
  metrics: Ref<ServerMetrics | undefined>;
  metricsError: Ref<string>;
  metricsLoading: Ref<boolean>;
  metricsOpen: Ref<boolean>;
  connected: Ref<boolean>;
  panelSurface: Ref<boolean>;
  refreshMetrics: () => void;
}) {
  const {
    t, isLocalMode, localShellRestored, metrics, metricsError, metricsLoading,
    metricsOpen, connected, panelSurface, refreshMetrics,
  } = options;

  // 本地终端（运行中或恢复的 shell）没有 SSH 会话；telnet/serial/vnc/rdp 的
  // 用户语义在远端设备，本机指标会造成误导——不适用（connected 分支天然排除）。
  const localTerminalActive = computed(() => isLocalMode.value || localShellRestored.value);

  async function refreshPanelMetrics() {
    if (localTerminalActive.value) {
      const sample = await window.dbxPlugin
        .invoke<ServerMetrics>("local/metrics", {}, { timeoutMs: 30_000 })
        .catch(() => undefined);
      if (sample) {
        metrics.value = sample;
        metricsError.value = "";
      } else {
        metricsError.value = t("metricsLocalUnavailable");
      }
      return;
    }
    void refreshMetrics();
  }

  // 连接建立（或本地终端激活）后 5s 环境轮询（与指标卡同一采样环；指标卡打开
  // 时让位其自带轮询，避免双发）。隐藏的 webview 不打扰；断开/失败整体隐藏。
  let panelMetricsTimer = 0;
  watch([connected, panelSurface, localTerminalActive], ([isConnected, onPanel, isLocal]) => {
    window.clearInterval(panelMetricsTimer);
    if (!onPanel || (!isConnected && !isLocal)) return;
    void refreshPanelMetrics();
    panelMetricsTimer = window.setInterval(() => {
      if (!metricsOpen.value && !metricsLoading.value && document.visibilityState === "visible") void refreshPanelMetrics();
    }, 5000);
  }, { immediate: true });
  onBeforeUnmount(() => window.clearInterval(panelMetricsTimer));

  // 三项就绪才显示（首拍未回/旧 sidecar 缺 network 字段/请求失败都不渲染占位）。
  const panelMetricsVisible = computed(() =>
    panelSurface.value && (connected.value || localTerminalActive.value) && metrics.value != null && !metricsError.value,
  );
  // SSH 面板指标失败不再静默整条隐藏：降级显示占位带，tooltip 携带原因。
  // 本地终端的平台性缺失（Windows 无 sh）仍按既有约定整条隐藏，不制造噪音。
  const panelMetricsFailed = computed(() =>
    panelSurface.value && connected.value && !localTerminalActive.value && !!metricsError.value,
  );
  const panelMetricsErrorTitle = computed(() => `${t("metrics")}: ${metricsError.value}`);
  const panelCpuPercent = computed<number | null>(() => {
    const value = metrics.value?.cpu?.percent;
    return value == null ? null : Math.round(value);
  });
  const panelMemPercent = computed<number | null>(() => {
    const memory = metrics.value?.memory;
    if (!memory?.totalBytes) return null;
    return Math.round(((memory.usedBytes ?? 0) / memory.totalBytes) * 100);
  });
  const panelNetRates = computed<{ rx: number; tx: number } | null>(() => {
    // 与指标面板同源去噪：容器/回环网卡不重复计入网速带。
    const interfaces = filterNetworkInterfaces(metrics.value?.network);
    if (!interfaces.length) return null;
    let rx = 0;
    let tx = 0;
    for (const net of interfaces) {
      rx += Math.max(0, net.rxRate || 0);
      tx += Math.max(0, net.txRate || 0);
    }
    return { rx, tx };
  });

  return {
    localTerminalActive,
    refreshPanelMetrics,
    panelMetricsVisible,
    panelMetricsFailed,
    panelMetricsErrorTitle,
    panelCpuPercent,
    panelMemPercent,
    panelNetRates,
  };
}
