import { watch, type Ref, type WatchStopHandle } from "vue";

/**
 * 子组件经 `defineExpose` 回传的模板 ref 需要在父级持续同步：函数 ref 只在
 * 挂载/卸载时调用一次，子组件内部根元素 `v-if` 切换时不会再次触发。
 * 以 getter 形式读取暴露值，ref 变化时写回 `target`。
 */
export function trackExposedRef<T>(
  get: () => T | undefined,
  target: Ref<T | undefined>,
): WatchStopHandle {
  return watch(
    get,
    (value) => {
      target.value = value;
    },
    { immediate: true, flush: "post" },
  );
}
