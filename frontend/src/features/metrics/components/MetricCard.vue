<script setup lang="ts">
// 指标仪表盘卡片外壳：统一图标底色、标题层级与右上角元信息，
// 让六块内容（系统/CPU/内存/网络/磁盘/进程）在同一节奏下排列。
import type { Component } from "vue";

defineProps<{
  icon: Component;
  title: string;
  tone?: "neutral" | "emerald" | "blue" | "cyan" | "amber" | "violet";
  meta?: string;
}>();
</script>

<template>
  <section class="metric-dash-card" :data-tone="tone ?? 'neutral'">
    <header class="metric-dash-head">
      <span class="metric-dash-icon"><component :is="icon" /></span>
      <span class="metric-dash-title">{{ title }}</span>
      <span v-if="meta" class="metric-dash-meta">{{ meta }}</span>
      <slot name="actions" />
    </header>
    <slot />
  </section>
</template>

<style scoped>
.metric-dash-card {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 10px;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-md);
  padding: 10px 12px 12px;
  background: var(--card-surface-inset);
}
.metric-dash-head {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
}
.metric-dash-icon {
  display: inline-flex;
  width: 24px;
  height: 24px;
  flex: 0 0 24px;
  align-items: center;
  justify-content: center;
  border-radius: 7px;
  background: color-mix(in srgb, var(--foreground) 7%, transparent);
  color: var(--muted-foreground);
}
.metric-dash-icon > svg { width: 14px; height: 14px; }
.metric-dash-title {
  overflow: hidden;
  color: var(--foreground);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.02em;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.metric-dash-meta {
  margin-left: auto;
  overflow: hidden;
  color: var(--muted-foreground);
  font-size: 10px;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.metric-dash-card[data-tone="emerald"] .metric-dash-icon { color: var(--tone-emerald); background: var(--tone-emerald-soft); }
.metric-dash-card[data-tone="blue"] .metric-dash-icon { color: var(--tone-blue); background: var(--tone-blue-soft); }
.metric-dash-card[data-tone="cyan"] .metric-dash-icon { color: var(--tone-cyan); background: var(--tone-cyan-soft); }
.metric-dash-card[data-tone="amber"] .metric-dash-icon { color: var(--tone-amber); background: var(--tone-amber-soft); }
.metric-dash-card[data-tone="violet"] .metric-dash-icon { color: var(--tone-violet); background: var(--tone-violet-soft); }
</style>
