import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { SftpTreeRow } from "../lib/sftpFileTree";
import { computeWindow } from "../lib/virtualWindow";

const ROW_HEIGHT = 28;
const OVERSCAN = 8;

export interface RenderedTreeRow extends SftpTreeRow {
  index: number;
}

/** 文件树虚拟滚动：28px 标称行高 + overscan 的可见窗口计算、实测行高校准、
 *  ResizeObserver/窗口滚动监听的生命周期，以及目录跟随（贴顶 + 尾部余量）
 *  与键盘焦点行的滚动落位。行状态仍由父级持有，这里只消费行数组。 */
export function useTreeVirtualScroll(options: {
  rows: () => SftpTreeRow[];
  virtualize: () => boolean;
  followPadding: () => boolean;
  /** 滚动时收起浮层（详情卡），由父级注入。 */
  onScrollHook?: () => void;
}) {
  const { rows, virtualize, followPadding, onScrollHook } = options;

  const scrollEl = ref<HTMLElement | null>(null);
  const scrollTop = ref(0);
  const viewportHeight = ref(0);
  // 行高以实际渲染值为准：主题/缩放/样式覆盖都可能让真实行高偏离标称 28px。
  // 虚拟窗口、滚动定位和置顶余量统一用它，避免按固定像素推算导致目标行被
  // 顶掉一行（真实宿主 WKWebView 下出现过）。
  const rowHeight = ref(ROW_HEIGHT);
  let resizeObserver: ResizeObserver | undefined;

  const virtualWindow = computed(() =>
    computeWindow({
      scrollTop: scrollTop.value,
      viewportHeight: viewportHeight.value,
      rowHeight: rowHeight.value,
      total: rows().length,
      overscan: OVERSCAN,
    }),
  );

  function measureRowHeight() {
    const first = scrollEl.value?.querySelector<HTMLElement>(".sftp-tree-row");
    const measured = first?.offsetHeight ?? first?.getBoundingClientRect().height ?? 0;
    if (measured > 0 && Math.abs(measured - rowHeight.value) > 0.01) rowHeight.value = measured;
  }

  const renderedRows = computed<RenderedTreeRow[]>(() => {
    if (!virtualize()) return rows().map((row, index) => ({ ...row, index }));
    const window = virtualWindow.value;
    return rows()
      .slice(window.start, window.end)
      .map((row, offset) => ({
        ...row,
        index: window.start + offset,
      }));
  });

  // 跟随目录贴顶所需的尾部余量：只为「自然滚动到不了顶部」的靠后行补，且补
  // 最小量。浅位置目标本来就在视口上部，若仍补近一屏空白，短列表的手动滚动
  // 会全程滑过空白、看起来像卡顿/阻尼。
  const followPadPx = ref(0);

  /** 滚动内容总高；无布局环境（测试/隐藏）退回行数与实测行高推算。 */
  function scrollContentHeight(el: HTMLElement): number {
    return el.scrollHeight > 0
      ? el.scrollHeight
      : rows().length * rowHeight.value + followPadPx.value;
  }

  function onScroll(event: Event) {
    const el = event.currentTarget as HTMLElement;
    scrollTop.value = el.scrollTop;
    // 用户把滚动位置带回自然可滚动范围后，置顶预留的尾部空白就该撤掉：
    // 之后的手动滚动不再滑过一段空列表（撤销此时不改变任何行位置）。
    if (followPadPx.value > 0) {
      const naturalMax = Math.max(0, scrollContentHeight(el) - followPadPx.value - viewportHeight.value);
      if (el.scrollTop <= naturalMax) followPadPx.value = 0;
    }
    onScrollHook?.();
  }

  function measureViewport() {
    if (scrollEl.value) viewportHeight.value = scrollEl.value.clientHeight;
  }

  onMounted(() => {
    measureViewport();
    measureRowHeight();
    if (typeof ResizeObserver !== "undefined" && scrollEl.value) {
      resizeObserver = new ResizeObserver(measureViewport);
      resizeObserver.observe(scrollEl.value);
    }
    window.addEventListener("scroll", onWindowScroll, { passive: true });
  });

  onBeforeUnmount(() => {
    resizeObserver?.disconnect();
    resizeObserver = undefined;
    window.removeEventListener("scroll", onWindowScroll);
  });

  function onWindowScroll() {
    onScrollHook?.();
  }

  // 行高可能随首屏渲染/缩放落定而变化：每次窗口行集合变化后按真实 DOM 重测。
  watch(() => renderedRows.value.length, measureRowHeight, { flush: "post" });

  // 行被过滤/折叠后总高变短：把旧的 scrollTop 夹回有效范围，避免窗口
  // start 越过 total 渲染出空列表。
  watch(
    () => rows().length,
    () => {
      void nextTick(() => {
        const el = scrollEl.value;
        if (!el) return;
        const max = Math.max(0, scrollContentHeight(el) - viewportHeight.value);
        if (scrollTop.value <= max) return;
        scrollTop.value = max;
        el.scrollTop = max;
      });
    },
  );

  // 关闭目录跟随后不再保留为置顶预留的尾部余量；否则下次手动滚动会先滑过
  // 一段没有内容的空白。
  watch(
    () => followPadding(),
    (enabled) => {
      if (!enabled) followPadPx.value = 0;
    },
  );

  function rowIndexOf(node: SftpTreeRow["node"]): number {
    const exact = rows().findIndex((row) => row.node === node);
    if (exact >= 0) return exact;
    return rows().findIndex((row) => row.node.path === node.path);
  }

  function findRowElement(index: number): HTMLElement | null {
    return scrollEl.value?.querySelector<HTMLElement>(`[data-tree-index="${index}"]`) ?? null;
  }

  /** 目标行在滚动内容中的真实偏移（含容器 padding）；未渲染时返回 null。 */
  function rowContentOffset(index: number): number | null {
    const el = scrollEl.value;
    const row = findRowElement(index);
    if (!el || !row) return null;
    const rowRect = row.getBoundingClientRect();
    // happy-dom/隐藏容器没有布局：退回索引推算，避免把 0 当真实位置。
    if (rowRect.height === 0) return null;
    return rowRect.top - el.getBoundingClientRect().top + el.scrollTop;
  }

  /** 滚动落位后闭环校准：量目标行的真实屏幕位置，差多少补多少。虚拟窗口
   *  重排、行高/缩放差异都可能在首次 scrollTop 之后留下整行偏差。 */
  async function settlePinnedRow(index: number): Promise<void> {
    for (let pass = 0; pass < 3; pass += 1) {
      const el = scrollEl.value;
      const row = findRowElement(index);
      if (!el || !row) return;
      const rowRect = row.getBoundingClientRect();
      if (rowRect.height === 0) return; // 无布局（测试/隐藏容器）
      const offsetY = rowRect.top - el.getBoundingClientRect().top;
      if (Math.abs(offsetY) < 1) return;
      const max = Math.max(0, scrollContentHeight(el) - viewportHeight.value);
      const next = Math.max(0, Math.min(max, el.scrollTop + offsetY));
      if (Math.abs(next - el.scrollTop) < 1) return;
      scrollTop.value = next;
      el.scrollTop = next;
      await nextTick();
    }
  }

  /** 目录跟随：把目标目录行滚到视口顶部（调用方已展开祖先）。 */
  async function scrollPathToTop(path: string): Promise<boolean> {
    const index = rows().findIndex((row) => row.node.path === path);
    const el = scrollEl.value;
    if (index < 0 || !el) return false;
    // 先用实测行高做一次索引定位，把虚拟窗口滚到目标附近；实际行高可能因
    // 主题/缩放/样式覆盖偏离标称值，纯 index*28 会累积成“顶掉一行”。
    measureRowHeight();
    const provisional = Math.max(0, index * rowHeight.value);
    scrollTop.value = provisional;
    el.scrollTop = provisional;
    await nextTick();
    const elNow = scrollEl.value;
    if (!elNow) return false;
    // 行已在窗口内时直接量 DOM 真实位置，彻底摆脱固定行高推算。
    const paddingTop = Number.parseFloat(getComputedStyle(elNow).paddingTop) || 0;
    const measured = rowContentOffset(index);
    const desired = Math.max(0, Math.round((measured ?? index * rowHeight.value + paddingTop) - paddingTop));
    const viewport = viewportHeight.value;
    // 目标行要精确贴顶，缺多少内容就补多少余量（不多补）。目标本来就能通过
    // 自然滚动到达顶部时（包括根行）不补，避免短列表无端出现空白。
    const baseHeight = Math.max(0, scrollContentHeight(elNow) - followPadPx.value);
    const pad = followPadding() && desired + viewport > baseHeight
      ? desired + viewport - baseHeight
      : 0;
    if (followPadPx.value !== pad) {
      followPadPx.value = pad;
      // 余量先落到 DOM 里，再设 scrollTop；否则浏览器会用旧内容高度钳制。
      await nextTick();
    }
    const finalEl = scrollEl.value;
    if (!finalEl) return false;
    const maxTop = Math.max(0, scrollContentHeight(finalEl) - viewport);
    const top = Math.max(0, Math.min(maxTop, desired));
    scrollTop.value = top;
    finalEl.scrollTop = top;
    // 首次落位后闭环校准，消除窗口重排/行高差异带来的整行偏差。
    await nextTick();
    await settlePinnedRow(index);
    return true;
  }

  /** 聚焦第 index 行；目标不在虚拟窗口内时先滚动到它，下一帧再聚焦。 */
  function focusRowAt(index: number): boolean {
    if (index < 0 || index >= rows().length) return false;
    const existing = findRowElement(index);
    if (existing) {
      existing.focus();
      return true;
    }
    if (!virtualize() || !scrollEl.value) return false;
    const maxTop = Math.max(0, rows().length * rowHeight.value - viewportHeight.value);
    const top = Math.max(0, Math.min(maxTop, index * rowHeight.value - rowHeight.value));
    scrollTop.value = top;
    scrollEl.value.scrollTop = top;
    void nextTick(() => findRowElement(index)?.focus());
    return true;
  }

  return {
    scrollEl,
    scrollTop,
    renderedRows,
    virtualWindow,
    followPadPx,
    onScroll,
    measureRowHeight,
    findRowElement,
    rowIndexOf,
    focusRowAt,
    scrollPathToTop,
  };
}
