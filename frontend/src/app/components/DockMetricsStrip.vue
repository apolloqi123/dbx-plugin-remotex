<script setup lang="ts">
/** Dock 面板工具条左组的系统信息带（CPU/内存/网速快照）。
 *  数据未就绪/不支持时整条隐藏；失败时降级显示占位带（tooltip 带原因）。
 *  父级 `.panel-toolbar-left` 容器仍留在 App（flex 布局），本组件只渲染两条互斥带。 */
defineProps<{
  panelMetricsVisible: boolean;
  panelMetricsFailed: boolean;
  panelCpuPercent: number | null;
  panelMemPercent: number | null;
  panelNetRates: { rx: number; tx: number } | null;
  panelMetricsErrorTitle: string;
  formatRate: (value: number) => string;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();
</script>

<template>
  <div v-if="panelMetricsVisible" class="panel-metrics" :title="t('metrics')">
    <span>CPU <b>{{ panelCpuPercent ?? "–" }}%</b></span>
    <span>MEM <b>{{ panelMemPercent ?? "–" }}%</b></span>
    <span v-if="panelNetRates">↓ <b>{{ formatRate(panelNetRates.rx) }}</b> ↑ <b>{{ formatRate(panelNetRates.tx) }}</b></span>
  </div>
  <div v-else-if="panelMetricsFailed" class="panel-metrics panel-metrics-degraded" :title="panelMetricsErrorTitle">
    <span>CPU <b>–%</b></span>
    <span>MEM <b>–%</b></span>
  </div>
</template>
