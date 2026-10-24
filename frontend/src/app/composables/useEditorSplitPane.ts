import { nextTick, ref, watch, type Ref } from "vue";

/**
 * 终端区编辑器页签的纵向分栏：占终端区高度的比例（%），25-75 夹取，
 * 拖拽/方向键即时改，xterm 重排由调用方注入的 scheduleFit 承担。
 * 模板 ref 由本 composable 创建并随解构暴露（script setup 绑定不变）。
 */
export function useEditorSplitPane(options: {
  hasEditorTabs: Ref<boolean>;
  scheduleFit: () => void;
}) {
  const { hasEditorTabs, scheduleFit } = options;
  const editorPaneRef = ref<{ $el?: unknown } | null>(null);
  const terminalStackRef = ref<HTMLElement | null>(null);
  const EDITOR_SPLIT_MIN = 25;
  const EDITOR_SPLIT_MAX = 75;
  const editorSplitRatio = ref(45);

  watch(hasEditorTabs, () => {
    // 页签出现/清空会改变终端可用高度：等 DOM 落地后再 fit。
    void nextTick(scheduleFit);
  });
  watch(editorSplitRatio, () => {
    scheduleFit();
  });

  function startEditorDividerDrag(event: PointerEvent) {
    const divider = event.currentTarget as HTMLElement | null;
    const stack = terminalStackRef.value;
    if (!divider || !stack) return;
    event.preventDefault();
    const pointerId = event.pointerId;
    const move = (next: PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      const bounds = stack.getBoundingClientRect();
      if (!bounds.height) return;
      editorSplitRatio.value = Math.max(
        EDITOR_SPLIT_MIN,
        Math.min(EDITOR_SPLIT_MAX, ((next.clientY - bounds.top) / bounds.height) * 100),
      );
    };
    // pointercancel 时指针已被移出 active 集合，释放前必须 hasPointerCapture 守卫。
    const release = (next: PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      divider.removeEventListener("pointermove", move);
      divider.removeEventListener("pointerup", release);
      divider.removeEventListener("pointercancel", release);
      if (divider.hasPointerCapture(pointerId)) {
        try { divider.releasePointerCapture(pointerId); } catch { /* 已被隐式释放 */ }
      }
    };
    divider.setPointerCapture(pointerId);
    divider.addEventListener("pointermove", move);
    divider.addEventListener("pointerup", release);
    divider.addEventListener("pointercancel", release);
  }

  function onEditorDividerKeydown(event: KeyboardEvent) {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    const delta = event.key === "ArrowDown" ? 2 : -2;
    editorSplitRatio.value = Math.max(EDITOR_SPLIT_MIN, Math.min(EDITOR_SPLIT_MAX, editorSplitRatio.value + delta));
  }

  /** Cmd/Ctrl+S 只接管编辑器内的焦点：终端（xterm）内保持既有输入语义。 */
  function editorPaneContainsFocus() {
    const root = editorPaneRef.value?.$el;
    return root instanceof HTMLElement && root.contains(document.activeElement);
  }

  return {
    editorPaneRef,
    terminalStackRef,
    editorSplitRatio,
    startEditorDividerDrag,
    onEditorDividerKeydown,
    editorPaneContainsFocus,
  };
}
