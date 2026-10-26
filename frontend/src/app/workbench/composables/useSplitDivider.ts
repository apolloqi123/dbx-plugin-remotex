import type { Ref } from "vue";
import type { SshWorkbenchPaneOrder } from "../../lib/workbenchLayout";

/** 分栏比例的读写端：splitRatio / dockerSplitRatio 留在 App，经 getter/setter 注入。 */
export interface SplitRatioAccess {
  get: () => number;
  set: (value: number) => void;
}

/** 容器分屏把手：终端+编辑器占比夹取 40-80（容器页 20%-60%），与 SFTP 分栏同经 scheduleFit。 */
const DOCKER_SPLIT_MIN = 40;
const DOCKER_SPLIT_MAX = 80;
/** 键盘微调步长：16px，Shift 加速 ×3（与右侧栏把手一致）。 */
const DIVIDER_KEY_STEP_PX = 16;

/**
 * 工作台分栏把手：指针拖拽与键盘方向键微调。两个比例 ref 的所有权仍在 App
 * （模板样式/持久化同源消费），这里只按「当前拖的是哪一栏」选读写端。
 *
 * 右侧栏把手用 `paneContainer`（工作台根），容器分屏把手用 `terminalStackRef`
 * （终端列，含编辑器页签），后者比例语义是「终端+编辑器」占比。
 */
export function useSplitDivider(options: {
  paneContainer: Ref<HTMLElement | undefined>;
  terminalStackRef: Ref<HTMLElement | null>;
  paneLayout: Ref<{ direction: "column" | "row" }>;
  paneOrder: Ref<SshWorkbenchPaneOrder>;
  splitRatio: SplitRatioAccess;
  dockerSplitRatio: SplitRatioAccess;
  scheduleFit: () => void;
  persistState: () => void;
}) {
  const {
    paneContainer, terminalStackRef, paneLayout, paneOrder,
    splitRatio, dockerSplitRatio, scheduleFit, persistState,
  } = options;

  function startDividerDrag(event: PointerEvent) {
    const container = paneContainer.value;
    if (!container) return;
    const pointerId = event.pointerId;
    // 纵向布局（强制上下 / 窄屏断点）把手沿 Y 轴拖拽：按下时定轴，拖动中不变。
    const vertical = paneLayout.value.direction === "column";
    const move = (next: PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      const bounds = container.getBoundingClientRect();
      if (vertical) {
        // splitRatio 是终端占比；纵向终端恒在上，把手下移 = 终端变高。
        const fromTop = ((next.clientY - bounds.top) / bounds.height) * 100;
        splitRatio.set(Math.max(35, Math.min(80, fromTop)));
      } else {
        const fromLeft = ((next.clientX - bounds.left) / bounds.width) * 100;
        const terminalPercent = paneOrder.value === "terminal-left" ? fromLeft : 100 - fromLeft;
        splitRatio.set(Math.max(35, Math.min(80, terminalPercent)));
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
    // splitRatio 是终端占比：横向 terminal-left 时把手右移 = 终端变宽（sftp-left
    // 反向）；纵向终端恒在上，把手下移 = 终端变高。
    if (paneLayout.value.direction === "column") {
      if (!bounds.height) return;
      splitRatio.set(Math.max(35, Math.min(80, splitRatio.get() + (deltaPx / bounds.height) * 100)));
    } else {
      if (!bounds.width) return;
      const sign = paneOrder.value === "terminal-left" ? 1 : -1;
      splitRatio.set(Math.max(35, Math.min(80, splitRatio.get() + (deltaPx / bounds.width) * 100 * sign)));
    }
    scheduleFit();
    persistState();
  }

  /** 容器分屏把手：ratio = 终端列中终端+编辑器占比（40-80），不落盘。 */
  function setDockerRatioFromPointer(pointerY: number, stack: HTMLElement) {
    const bounds = stack.getBoundingClientRect();
    if (!bounds.height) return;
    const fromTop = ((pointerY - bounds.top) / bounds.height) * 100;
    dockerSplitRatio.set(Math.max(DOCKER_SPLIT_MIN, Math.min(DOCKER_SPLIT_MAX, fromTop)));
    scheduleFit();
  }

  function startDockerDividerDrag(event: PointerEvent) {
    const stack = terminalStackRef.value;
    const divider = event.currentTarget as HTMLElement | null;
    if (!stack || !divider) return;
    event.preventDefault();
    const pointerId = event.pointerId;
    const move = (next: PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      setDockerRatioFromPointer(next.clientY, stack);
    };
    // 与右侧栏把手同一释放契约：先摘监听再守卫 releasePointerCapture。
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

  function nudgeDockerDivider(deltaPx: number) {
    const stack = terminalStackRef.value;
    if (!stack) return;
    const bounds = stack.getBoundingClientRect();
    if (!bounds.height) return;
    // 把手下移（delta > 0）= 容器页变矮、终端占比变大。
    dockerSplitRatio.set(Math.max(DOCKER_SPLIT_MIN, Math.min(DOCKER_SPLIT_MAX, dockerSplitRatio.get() + (deltaPx / bounds.height) * 100)));
    scheduleFit();
  }

  /** 容器分屏把手键盘通道：仅上下方向键，Shift ×3。 */
  function onDockerDividerKeydown(event: KeyboardEvent) {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    const magnitude = event.shiftKey ? DIVIDER_KEY_STEP_PX * 3 : DIVIDER_KEY_STEP_PX;
    nudgeDockerDivider(event.key === "ArrowDown" ? magnitude : -magnitude);
  }

  return {
    startDividerDrag,
    onDividerKeydown,
    nudgeSplitDivider,
    startDockerDividerDrag,
    onDockerDividerKeydown,
    nudgeDockerDivider,
  };
}
