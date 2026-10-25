import { nextTick, ref } from "vue";

export interface TooltipState {
  text: string;
  x: number;
  y: number;
  above: boolean;
  arrowOffset: number;
}

/**
 * 自定义 tooltip（对齐 DBX 宿主的气泡提示）：全局接管 title 属性——悬停或
 * 键盘聚焦时把值挪到 data-tooltip（抑制原生慢速灰框），350ms 后显示主题化
 * 气泡；下方空间不足翻到上方。模板无需改动，所有现有和未来的 title 自动生效。
 */
export function useTerminalTooltip() {
  const tooltip = ref<TooltipState | null>(null);
  const tooltipBubble = ref<HTMLElement>();
  let tooltipEl: HTMLElement | null = null;
  let tooltipTimer = 0;

  function hideTooltip() {
    window.clearTimeout(tooltipTimer);
    tooltipTimer = 0;
    tooltip.value = null;
    tooltipEl = null;
  }

  function scheduleTooltip(target: HTMLElement) {
    // title → data-tooltip：Vue 绑定只在值变化时重写 title，下次触发再挪一次。
    const title = target.getAttribute("title");
    if (title !== null) {
      target.setAttribute("data-tooltip", title);
      target.removeAttribute("title");
      // title 挪走后可访问名称会丢（VoiceOver/NVDA 读不出按钮）：回填 aria-label
      // 兜底；已有 aria-label 的元素以它为准，不覆盖。
      if (!target.hasAttribute("aria-label")) target.setAttribute("aria-label", title);
    }
    const text = target.getAttribute("data-tooltip")?.trim();
    if (!text) return;
    tooltipEl = target;
    tooltipTimer = window.setTimeout(() => {
      tooltipTimer = 0;
      if (!target.isConnected) return;
      const rect = target.getBoundingClientRect();
      const above = rect.bottom + 34 > window.innerHeight && rect.top > 34;
      const center = rect.left + rect.width / 2;
      tooltip.value = { text, x: center, y: above ? rect.top - 6 : rect.bottom + 6, above, arrowOffset: 0 };
      // 量出气泡实际宽度后重新钳位，保证整框（而非仅中心点）落在视口内；
      // 小三角按钳位偏差反向偏移，继续对准触发元素。
      void nextTick(() => {
        const bubble = tooltipBubble.value;
        const current = tooltip.value;
        if (!bubble || !current) return;
        const half = bubble.offsetWidth / 2;
        const clamped = Math.max(half + 8, Math.min(current.x, window.innerWidth - half - 8));
        if (clamped !== current.x) {
          tooltip.value = { ...current, x: clamped, arrowOffset: center - clamped };
        }
      });
    }, 350);
  }

  function onTooltipOver(event: MouseEvent) {
    const target = (event.target as HTMLElement | null)?.closest?.("[title], [data-tooltip]") as HTMLElement | null;
    if (target === tooltipEl) return;
    hideTooltip();
    if (!target) return;
    scheduleTooltip(target);
  }

  function onTooltipOut(event: MouseEvent) {
    if (!tooltipEl) return;
    const related = event.relatedTarget as HTMLElement | null;
    if (related && tooltipEl.contains(related)) return;
    hideTooltip();
  }

  // 键盘可达性：focusin/focusout 走同一套气泡（focus 锚定元素本身，
  // 不依赖指针位置），blur 时收起。
  function onTooltipFocusIn(event: FocusEvent) {
    const target = (event.target as HTMLElement | null)?.closest?.("[title], [data-tooltip]") as HTMLElement | null;
    if (target === tooltipEl) return;
    hideTooltip();
    if (!target) return;
    scheduleTooltip(target);
  }

  function onTooltipFocusOut(event: FocusEvent) {
    if (!tooltipEl) return;
    const related = event.relatedTarget as HTMLElement | null;
    if (related && tooltipEl.contains(related)) return;
    hideTooltip();
  }

  return { tooltip, tooltipBubble, hideTooltip, onTooltipOver, onTooltipOut, onTooltipFocusIn, onTooltipFocusOut };
}
