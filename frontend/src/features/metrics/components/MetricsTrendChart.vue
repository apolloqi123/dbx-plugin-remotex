<script setup lang="ts">
// 指标趋势图：把 useMetricsPanel 生成的 polyline 采样串渲染成带渐变填充的
// 面积图。viewBox 保持采样时的坐标系，preserveAspectRatio=none 拉满卡片宽度，
// vector-effect 保证描边不被横向拉伸。
import { computed, useId } from "vue";

type TrendTone = "emerald" | "blue" | "cyan" | "violet" | "amber";

const props = withDefaults(
  defineProps<{
    points?: string;
    secondaryPoints?: string;
    tone?: TrendTone;
    secondaryTone?: TrendTone;
    viewWidth?: number;
    viewHeight?: number;
    /** 双线对比图（如上下行）关掉面积填充，避免两条曲线叠成色块。 */
    fill?: boolean;
    /** 纵轴刻度文案，自顶向底等距排布（如 ["100%","50%","0%"]）；不传则不画轴。 */
    yTicks?: readonly string[];
  }>(),
  {
    points: "",
    secondaryPoints: "",
    tone: "emerald",
    secondaryTone: "violet",
    viewWidth: 120,
    viewHeight: 18,
    fill: true,
    yTicks: () => [],
  },
);

const uid = useId();
const primaryFill = computed(() => `metric-trend-fill-${uid}`);

const TONE_VAR: Record<TrendTone, string> = {
  emerald: "var(--tone-emerald)",
  blue: "var(--tone-blue)",
  cyan: "var(--tone-cyan)",
  violet: "var(--tone-violet)",
  amber: "var(--tone-amber)",
};

const toneColor = computed(() => TONE_VAR[props.tone]);
const secondaryColor = computed(() => TONE_VAR[props.secondaryTone]);

const hasYAxis = computed(() => props.yTicks.length > 1);
/** 有刻度时网格线落在每条刻度（顶/中/底），与标签一一对应；否则保留中线。 */
const gridLines = computed(() =>
  hasYAxis.value
    ? [0, 0.5, 1].map((ratio) => props.viewHeight * ratio)
    : [props.viewHeight / 2],
);

function area(points: string) {
  if (!points) return "";
  return `0,${props.viewHeight} ${points} ${props.viewWidth},${props.viewHeight}`;
}
</script>

<template>
  <div class="metric-trend-frame">
    <div v-if="hasYAxis" class="metric-trend-yaxis" aria-hidden="true">
      <span v-for="(tick, index) in yTicks" :key="index">{{ tick }}</span>
    </div>
    <div class="metric-trend-plot">
      <svg
        class="metric-trend"
        :viewBox="`0 0 ${viewWidth} ${viewHeight}`"
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        <defs>
          <linearGradient :id="primaryFill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" :stop-color="toneColor" stop-opacity="0.32" />
            <stop offset="100%" :stop-color="toneColor" stop-opacity="0" />
          </linearGradient>
        </defs>
        <line
          v-for="line in gridLines"
          :key="line"
          class="metric-trend-grid"
          x1="0"
          :y1="line"
          :x2="viewWidth"
          :y2="line"
        />
        <line class="metric-trend-axis" x1="0" :y1="viewHeight - 0.5" :x2="viewWidth" :y2="viewHeight - 0.5" />
        <template v-if="points">
          <polygon v-if="fill" :points="area(points)" :fill="`url(#${primaryFill})`" />
          <polyline
            :points="points"
            fill="none"
            :style="{ stroke: toneColor }"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            vector-effect="non-scaling-stroke"
          />
        </template>
        <template v-if="secondaryPoints">
          <polyline
            :points="secondaryPoints"
            fill="none"
            :style="{ stroke: secondaryColor }"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            vector-effect="non-scaling-stroke"
          />
        </template>
      </svg>
    </div>
  </div>
</template>

<style scoped>
.metric-trend-frame { display: flex; width: 100%; height: 46px; align-items: stretch; gap: 4px; }
.metric-trend-plot { min-width: 0; flex: 1 1 auto; }
.metric-trend-yaxis {
  display: flex;
  flex: 0 0 auto;
  flex-direction: column;
  align-items: flex-end;
  justify-content: space-between;
  /* 让首/末刻度中心正好落在图表顶/底边（8px 行高的一半外扩）。 */
  margin-block: -4px;
  color: var(--muted-foreground);
  font-size: 8px;
  font-variant-numeric: tabular-nums;
  line-height: 1;
  white-space: nowrap;
}
.metric-trend { display: block; width: 100%; height: 100%; }
.metric-trend-axis { stroke: color-mix(in srgb, var(--foreground) 10%, transparent); stroke-width: 1; vector-effect: non-scaling-stroke; }
.metric-trend-grid { stroke: color-mix(in srgb, var(--foreground) 6%, transparent); stroke-width: 1; vector-effect: non-scaling-stroke; }
</style>
