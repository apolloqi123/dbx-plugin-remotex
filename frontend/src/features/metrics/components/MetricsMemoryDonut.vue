<script setup lang="ts">
// 内存环形图：已用 / 缓存 / 空闲三段。usedBytes 保持“总量-可用”的历史语义
// （含缓存），因此这里把 cacheBytes 从已用段里拆出来，三段之和恰好等于总量。
import { computed } from "vue";
import { formatBytes } from "../../../shared/lib/format";

const props = withDefaults(
  defineProps<{
    totalBytes: number;
    usedBytes: number;
    cacheBytes?: number;
    usedLabel: string;
    cacheLabel: string;
    freeLabel: string;
  }>(),
  { cacheBytes: 0, totalBytes: 0, usedBytes: 0 },
);

const RADIUS = 34;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

const segments = computed(() => {
  const total = Math.max(0, props.totalBytes);
  const cache = Math.max(0, Math.min(props.cacheBytes, props.usedBytes, total));
  const used = Math.max(0, Math.min(props.usedBytes - cache, total - cache));
  const free = Math.max(0, total - used - cache);
  const denominator = used + cache + free || 1;
  const share = (value: number) => (value / denominator) * CIRCUMFERENCE;
  const usedLength = share(used);
  const cacheLength = share(cache);
  const freeLength = Math.max(0, CIRCUMFERENCE - usedLength - cacheLength);
  return [
    { key: "used", label: props.usedLabel, bytes: used, length: usedLength, offset: 0 },
    { key: "cache", label: props.cacheLabel, bytes: cache, length: cacheLength, offset: usedLength },
    { key: "free", label: props.freeLabel, bytes: free, length: freeLength, offset: usedLength + cacheLength },
  ];
});

// 中心百分比跟随蓝色“已用”扇区（缓存单独成段），避免环上 32% 与中心 68% 打架。
const usedPercent = computed(() => {
  if (!props.totalBytes) return "–";
  const used = Math.max(0, props.usedBytes - props.cacheBytes);
  return String(Math.round((used / props.totalBytes) * 100));
});
</script>

<template>
  <div class="metric-memory">
    <svg class="metric-memory-ring" viewBox="0 0 88 88" role="img" :aria-label="usedLabel">
      <circle class="metric-memory-track" cx="44" cy="44" :r="RADIUS" />
      <circle
        v-for="segment in segments"
        :key="segment.key"
        class="metric-memory-seg"
        :class="`is-${segment.key}`"
        cx="44"
        cy="44"
        :r="RADIUS"
        transform="rotate(-90 44 44)"
        :stroke-dasharray="`${segment.length} ${Math.max(0, CIRCUMFERENCE - segment.length)}`"
        :stroke-dashoffset="-segment.offset"
      />
      <text class="metric-memory-percent" x="44" y="42" text-anchor="middle">{{ usedPercent }}<tspan class="metric-memory-unit">%</tspan></text>
      <text class="metric-memory-caption" x="44" y="55" text-anchor="middle">{{ usedLabel }}</text>
    </svg>
    <ul class="metric-memory-legend">
      <li v-for="segment in segments" :key="segment.key">
        <span class="metric-memory-dot" :class="`is-${segment.key}`" />
        <span class="metric-memory-name">{{ segment.label }}</span>
        <b>{{ formatBytes(segment.bytes) }}</b>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.metric-memory {
  display: grid;
  grid-template-columns: 88px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
}
.metric-memory-ring {
  width: 88px;
  height: 88px;
}
.metric-memory-track {
  fill: none;
  stroke: color-mix(in srgb, var(--foreground) 9%, transparent);
  stroke-width: 9;
}
.metric-memory-seg {
  fill: none;
  stroke-width: 9;
  transition: stroke-dasharray 400ms cubic-bezier(0.4, 0, 0.2, 1);
}
.metric-memory-seg.is-used { stroke: var(--tone-blue); }
.metric-memory-seg.is-cache { stroke: var(--tone-violet); }
.metric-memory-seg.is-free { stroke: color-mix(in srgb, var(--foreground) 16%, transparent); }
.metric-memory-percent {
  fill: var(--foreground);
  font-family: var(--ui-font-family);
  font-size: 17px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}
.metric-memory-unit { fill: var(--muted-foreground); font-size: 9.5px; font-weight: 500; }
.metric-memory-caption {
  fill: var(--muted-foreground);
  font-family: var(--ui-font-family);
  font-size: 9px;
}
.metric-memory-legend {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 7px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.metric-memory-legend li {
  display: grid;
  grid-template-columns: 7px minmax(0, 1fr) auto;
  align-items: center;
  gap: 7px;
  font-size: 10px;
}
.metric-memory-dot { width: 7px; height: 7px; border-radius: 50%; }
.metric-memory-dot.is-used { background: var(--tone-blue); }
.metric-memory-dot.is-cache { background: var(--tone-violet); }
.metric-memory-dot.is-free { background: color-mix(in srgb, var(--foreground) 22%, transparent); }
.metric-memory-name {
  overflow: hidden;
  color: var(--muted-foreground);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.metric-memory-legend b {
  color: var(--foreground);
  font-family: var(--terminal-font-family);
  font-size: 10px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
</style>
