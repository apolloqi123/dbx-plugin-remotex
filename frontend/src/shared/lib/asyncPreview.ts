// 预览类异步面板（TextPreview / JsonPreviewPanel）的统一懒加载兜底。
//
// 为什么不用 defineAsyncComponent：后者的 errorComponent 拿不到可用的 retry
// 回调（retry 只在 onError 同步调用期内有效），也无法从失败态重新拉起已终止
// 的加载链。这里用最小自定义异步宿主：
// - 动态 import 只在组件首次渲染（预览弹窗打开）时执行，保持按需分包；
// - 150ms 内不显示加载态（避免快速加载闪烁，语义对齐 defineAsyncComponent delay）；
// - 失败渲染说明 + 重试；重试重新执行 loader（代际号丢弃过期结果）；
// - 属性/事件经 inheritAttrs:false + h(comp, attrs) 原样转发给真实面板。
//
// 文案经 labels() 闭包按当前 locale 现取，避免在模块求值期固化语言。

import {
  defineComponent,
  h,
  onBeforeUnmount,
  ref,
  shallowRef,
  type AsyncComponentLoader,
  type Component,
} from "vue";
import PreviewPaneFallback from "../ui/PreviewPaneFallback.vue";

export interface PreviewAsyncLabels {
  loading: string;
  failed: string;
  retry: string;
}

type PreviewAsyncState = "loading" | "ready" | "error";

/** 构建带加载/失败兜底的预览异步组件（首次渲染才执行动态 import）。 */
export function createPreviewAsyncComponent(
  loader: AsyncComponentLoader,
  labels: () => PreviewAsyncLabels,
) {
  return defineComponent({
    name: "PreviewAsyncPanel",
    inheritAttrs: false,
    setup(_props, { attrs }) {
      const resolved = shallowRef<Component | null>(null);
      const state = ref<PreviewAsyncState>("loading");
      const showLoading = ref(false);
      let delayTimer: ReturnType<typeof setTimeout> | undefined;
      // 代际号：卸载或重试后丢弃仍在飞行中的旧加载结果。
      let generation = 0;

      const start = () => {
        const current = ++generation;
        state.value = "loading";
        showLoading.value = false;
        clearTimeout(delayTimer);
        delayTimer = setTimeout(() => {
          if (current === generation && state.value === "loading") showLoading.value = true;
        }, 150);
        void Promise.resolve()
          .then(loader)
          .then((module) => {
            if (current !== generation) return;
            const component = (module as { default?: Component }).default ?? (module as Component);
            resolved.value = component;
            state.value = "ready";
          })
          .catch(() => {
            if (current === generation) state.value = "error";
          })
          .finally(() => {
            if (current === generation) clearTimeout(delayTimer);
          });
      };

      onBeforeUnmount(() => {
        generation += 1;
        clearTimeout(delayTimer);
      });
      start();

      return () => {
        if (state.value === "ready" && resolved.value) return h(resolved.value, attrs);
        if (state.value === "error") {
          return h(PreviewPaneFallback, {
            state: "error",
            errorLabel: labels().failed,
            retryLabel: labels().retry,
            onRetry: start,
          });
        }
        // delay 内返回空占位（与 defineAsyncComponent 的 delay 语义一致）。
        return showLoading.value
          ? h(PreviewPaneFallback, { state: "loading", loadingLabel: labels().loading })
          : null;
      };
    },
  });
}
