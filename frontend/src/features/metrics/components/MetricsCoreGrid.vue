<script setup lang="ts">
// 每核 CPU 占用：左侧核号、中间分段刻度条、右侧百分比。分段感来自覆盖在
// 轨道上的 repeating-linear-gradient（与卡片底色同色），不额外增加 DOM。
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    cores: Array<{ index: number; percent?: number | null }>;
  }>(),
  { cores: () => [] },
);

const rows = computed(() =>
  props.cores.map((core) => {
    const percent = Math.max(0, Math.min(100, core.percent ?? 0));
    return {
      index: core.index,
      percent,
      text: core.percent == null ? "–" : `${core.percent.toFixed(1)}%`,
      level: percent >= 80 ? "hot" : percent >= 50 ? "warm" : "normal",
    };
  }),
);
</script>

<template>
  <div class="metric-core-grid">
    <div v-for="row in rows" :key="row.index" class="metric-core-row">
      <span class="metric-core-index">{{ row.index }}</span>
      <span class="metric-core-track">
        <span class="metric-core-fill" :class="`is-${row.level}`" :style="{ width: `${row.percent}%` }" />
      </span>
      <span class="metric-core-value" :class="{ 'is-hot': row.level === 'hot' }">{{ row.text }}</span>
    </div>
  </div>
</template>

<style scoped>
.metric-core-grid { display: flex; flex-direction: column; gap: 5px; }
.metric-core-row {
  display: grid;
  grid-template-columns: 16px minmax(0, 1fr) 46px;
  align-items: center;
  gap: 8px;
}
.metric-core-index {
  color: var(--muted-foreground);
  font-family: var(--terminal-font-family);
  font-size: 10px;
  text-align: right;
}
.metric-core-track {
  position: relative;
  display: block;
  height: 9px;
  overflow: hidden;
  border-radius: 3px;
  background: color-mix(in srgb, var(--foreground) 8%, transparent);
}
.metric-core-track::after {
  content: "";
  position: absolute;
  inset: 0;
  background: repeating-linear-gradient(
    90deg,
    transparent 0 4px,
    var(--card-surface-inset) 4px 6px
  );
}
.metric-core-fill {
  position: absolute;
  inset: 0 auto 0 0;
  border-radius: 3px;
  transition: width 400ms cubic-bezier(0.4, 0, 0.2, 1);
}
.metric-core-fill.is-normal { background: linear-gradient(90deg, var(--tone-emerald), color-mix(in srgb, var(--tone-emerald) 62%, var(--tone-amber))); }
.metric-core-fill.is-warm { background: linear-gradient(90deg, var(--tone-amber), color-mix(in srgb, var(--tone-amber) 65%, var(--tone-rose))); }
.metric-core-fill.is-hot { background: linear-gradient(90deg, var(--tone-rose), color-mix(in srgb, var(--tone-rose) 70%, var(--tone-amber))); }
.metric-core-value {
  color: var(--muted-foreground);
  font-family: var(--terminal-font-family);
  font-size: 10px;
  font-variant-numeric: tabular-nums;
  text-align: right;
}
.metric-core-value.is-hot { color: var(--tone-rose); font-weight: 600; }
</style>
