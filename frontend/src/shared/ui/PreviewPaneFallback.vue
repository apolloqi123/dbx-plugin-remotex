<script setup lang="ts">
// 异步预览面板（TextPreview / JsonPreviewPanel）的加载/失败兜底。
//
// 背景：预览面板经 defineAsyncComponent 懒加载（CodeMirror + language-data
// 不参与首屏求值）。旧实现没有 loading/error 组件：动态 import 失败（宿主
// 资源被拦截、构建产物缺块、网络中断）时 Vue 只渲染注释占位，弹窗正文空白，
// 用户既看不到原因也无法重试。本组件沿用预览区 .preview-editor 的布局语义，
// 失败态提供原文无关的说明 + 重试按钮（点击经 errorComponent 的 retry 回调
// 重新触发动态 import）。
import { RefreshCw, TriangleAlert } from "@lucide/vue";

withDefaults(
  defineProps<{
    state: "loading" | "error";
    loadingLabel?: string;
    errorLabel?: string;
    retryLabel?: string;
  }>(),
  {
    loadingLabel: "Loading...",
    errorLabel: "Preview failed to load.",
    retryLabel: "Retry",
  },
);

const emit = defineEmits<{
  retry: [];
}>();
</script>

<template>
  <div
    class="preview-editor preview-fallback"
    :class="`preview-fallback--${state}`"
    :role="state === 'error' ? 'alert' : 'status'"
    aria-live="polite"
  >
    <span v-if="state === 'loading'" class="preview-fallback-spinner" aria-hidden="true" />
    <TriangleAlert v-else class="preview-fallback-icon" aria-hidden="true" />
    <p>{{ state === "loading" ? loadingLabel : errorLabel }}</p>
    <button v-if="state === 'error'" type="button" class="preview-fallback-retry" @click="emit('retry')">
      <RefreshCw aria-hidden="true" />
      {{ retryLabel }}
    </button>
  </div>
</template>

<style scoped>
.preview-fallback {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 24px;
  color: var(--muted-foreground);
  font-size: 12px;
  text-align: center;
}

.preview-fallback p {
  max-width: 46ch;
  margin: 0;
  line-height: 1.5;
}

.preview-fallback-icon {
  width: 26px;
  height: 26px;
  opacity: 0.8;
}

/* 纯 CSS 旋转占位：避免为兜底态再引入图标组件依赖。 */
.preview-fallback-spinner {
  width: 22px;
  height: 22px;
  border: 2px solid color-mix(in srgb, currentColor 24%, transparent);
  border-top-color: currentColor;
  border-radius: 999px;
  animation: preview-fallback-spin 0.9s linear infinite;
}

.preview-fallback-retry {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.preview-fallback-retry svg {
  width: 14px;
  height: 14px;
}

@keyframes preview-fallback-spin {
  to {
    transform: rotate(360deg);
  }
}

@media (prefers-reduced-motion: reduce) {
  .preview-fallback-spinner {
    animation: none;
  }
}
</style>
