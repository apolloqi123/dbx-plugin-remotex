import type { Ref } from "vue";
import type { SshWorkbenchPaneOrder } from "../../lib/workbenchLayout";

/** 分栏比例的读写端：splitRatio / dockerSplitRatio 留在 App，经 getter/setter 注入。 */
export interface SplitRatioAccess {
  get: () => number;
  set: (value: number) => void;
}

/**
 * 工作台分栏把手：指针拖拽与键盘方向键微调。两个比例 ref 的所有权仍在 App
 * （模板样式/持久化同源消费），这里只按「当前拖的是哪一栏」选读写端。
 */
export function useSplitDivider(options: {
  paneContainer: Ref<HTMLElement | undefined>;
  paneLayout: Ref<{ direction: "column" | "row" }>;
  paneOrder: Ref<SshWorkbenchPaneOrder>;
  dockerPanelOpen: Ref<boolean>;
  splitRatio: SplitRatioAccess;
  dockerSplitRatio: SplitRatioAccess;
  scheduleFit: () => void;
  persistState: () => void;
}) {
  const {
    paneContainer, paneLayout, paneOrder, dockerPanelOpen,
    splitRatio, dockerSplitRatio, scheduleFit, persistState,
  } = options;

  function startDividerDrag(event: PointerEvent) {
    const container = paneContainer.value;
    if (!container) return;
    const pointerId = event.pointerId;
    // 纵向布局（强制上下 / 窄屏断点 / Docker 底栏）把手沿 Y 轴拖拽：
    // 按下时定轴，拖动中不变；Docker 使用独立比例，避免改动 SFTP 分栏。
    const vertical = paneLayout.value.direction === "column";
    const targetRatio = dockerPanelOpen.value ? dockerSplitRatio : splitRatio;
    const move = (next: PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      const bounds = container.getBoundingClientRect();
      if (vertical) {
        // splitRatio 是终端占比；纵向终端恒在上，把手下移 = 终端变高。
        const fromTop = ((next.clientY - bounds.top) / bounds.height) * 100;
        targetRatio.set(Math.max(35, Math.min(80, fromTop)));
      } else {
        const fromLeft = ((next.clientX - bounds.left) / bounds.width) * 100;
        const terminalPercent = paneOrder.value === "terminal-left" ? fromLeft : 100 - fromLeft;
        targetRatio.set(Math.max(35, Math.min(80, terminalPercent)));
      }
      scheduleFit();
    };
    // pointercancel 触发时指针已被移出 active 集合：releasePointerCapture 对非
    // active 指针会抛 NotFoundError，必须先摘监听、再守卫释放，否则监听器泄漏、
    // 容器内后续鼠标移动会持续重算分栏（触屏拖动被浏览器接管时必现）。
    const release = (next: PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      container.removeEventListener("pointermove", move);
      container.removeEventListener("pointerup", release);
      container.removeEventListener("pointercancel", release);
      if (container.hasPointerCapture(pointerId)) {
        try { container.releasePointerCapture(pointerId); } catch { /* 已被隐式释放 */ }
      }
      persistState();
    };
    container.setPointerCapture(pointerId);
    container.addEventListener("pointermove", move);
    container.addEventListener("pointerup", release);
    container.addEventListener("pointercancel", release);
  }

  // 分栏把手键盘通道（pointer 拖拽对键盘用户不可达）：横向布局左右方向键
  // ±16px，纵向布局上下方向键 ±16px，Shift 加速 ×3。`delta > 0` 表示「把手
  // 向右/向下移」（终端占比随之增大/减小由 nudgeSplitDivider 按轴解释）。
  const DIVIDER_KEY_STEP_PX = 16;
  function onDividerKeydown(event: KeyboardEvent, nudge: (deltaPx: number) => void) {
    const vertical = paneLayout.value.direction === "column";
    const horizontalKeys = event.key === "ArrowLeft" || event.key === "ArrowRight";
    const verticalKeys = event.key === "ArrowUp" || event.key === "ArrowDown";
    if (vertical ? !verticalKeys : !horizontalKeys) return;
    event.preventDefault();
    const magnitude = event.shiftKey ? DIVIDER_KEY_STEP_PX * 3 : DIVIDER_KEY_STEP_PX;
    const forward = vertical ? event.key === "ArrowDown" : event.key === "ArrowRight";
    nudge(forward ? magnitude : -magnitude);
  }

  function nudgeSplitDivider(deltaPx: number) {
    const container = paneContainer.value;
    if (!container) return;
    const bounds = container.getBoundingClientRect();
    const targetRatio = dockerPanelOpen.value ? dockerSplitRatio : splitRatio;
    // splitRatio 是终端占比：横向 terminal-left 时把手右移 = 终端变宽（sftp-left
    // 反向）；纵向终端恒在上，把手下移 = 终端变高。
    if (paneLayout.value.direction === "column") {
      if (!bounds.height) return;
      targetRatio.set(Math.max(35, Math.min(80, targetRatio.get() + (deltaPx / bounds.height) * 100)));
    } else {
      if (!bounds.width) return;
      const sign = paneOrder.value === "terminal-left" ? 1 : -1;
      targetRatio.set(Math.max(35, Math.min(80, targetRatio.get() + (deltaPx / bounds.width) * 100 * sign)));
    }
    scheduleFit();
    persistState();
  }

  return {
    startDividerDrag,
    onDividerKeydown,
    nudgeSplitDivider,
  };
}
