<script setup lang="ts">
import { computed } from "vue";
import { WORKBENCH_GLYPHS, type WorkbenchGlyphName } from "./glyphs";

/** 工作台自有图标（几何见 ./glyphs.ts）。尺寸/线宽由全局 .icon-button svg
 *  规则统一为 14px / 1.7；这里只保留 SVG 语义与无障碍隐藏。 */
const props = defineProps<{
  name: WorkbenchGlyphName;
  size?: number;
}>();

const shapes = computed(() => WORKBENCH_GLYPHS[props.name]);
</script>

<template>
  <svg
    class="workbench-glyph"
    :width="size ?? 14"
    :height="size ?? 14"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
  >
    <template v-for="(shape, index) in shapes" :key="index">
      <path v-if="shape.tag === 'path'" :d="shape.d" :fill="shape.fill ?? 'none'" />
      <circle v-else-if="shape.tag === 'circle'" :cx="shape.cx" :cy="shape.cy" :r="shape.r" :fill="shape.fill ?? 'none'" />
      <line v-else :x1="shape.x1" :y1="shape.y1" :x2="shape.x2" :y2="shape.y2" />
    </template>
  </svg>
</template>
