/**
 * xterm renderer for the split (secondary) terminal pane.
 *
 * Owns the terminal instance and its addons (fit/search/unicode11/web-links),
 * theme/font option application, fit scheduling, the ResizeObserver and the
 * rendering-time watchers (theme, font, tab visibility, layout epoch).
 * Session/frame/input ordering stays in useSplitTerminal, and the pane
 * component still decides the disposal interleaving through `dispose()` so the
 * addon teardown order is unchanged.
 */
import { nextTick, ref, watch } from "vue";
import { Terminal as XTerm, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { SPLIT_DEFAULT_COLS, SPLIT_DEFAULT_ROWS } from "../../lib/behavior/terminalSplit";

export interface SplitTerminalRendererOptions {
  /** xterm mount point inside the pane body. */
  getHost: () => HTMLElement | null;
  getTheme: () => Record<string, unknown>;
  getFontFamily: () => string;
  getFontSize: () => number;
  /** 页签是否可见；切回时重新 fit 并聚焦。 */
  getActive: () => boolean;
  /** 布局变化计数：分屏/关闭让宿主搬到新槽位后重新 fit。 */
  getLayoutEpoch: () => number;
  /** xterm input bytes (AI intercept, line tracking, PTY send). */
  onData: (data: string) => void;
  /** Ctrl/Cmd+F 打开窗格自己的搜索面板。 */
  onOpenSearch: () => void;
  /** SearchAddon 结果变化（面板计数）。 */
  onSearchResults: (change: { resultCount: number; resultIndex: number }) => void;
  /** PTY 尺寸同步（去重与下发在 useSplitTerminal）。 */
  onResize: (cols: number, rows: number) => void;
}

/** 分屏 xterm 渲染器：终端/附加组件/主题/尺寸适配的唯一持有者。 */
export function useSplitTerminalRenderer(options: SplitTerminalRendererOptions) {
  const hasSelection = ref(false);

  let terminal: XTerm | undefined;
  let searchAddon: SearchAddon | undefined;
  let selectionDisposable: { dispose(): void } | undefined;
  let fitAddon: FitAddon | undefined;
  let unicode11: Unicode11Addon | undefined;
  let webLinks: WebLinksAddon | undefined;
  let dataDisposable: { dispose(): void } | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let fitTimer = 0;
  let disposed = false;
  let appliedTheme: Record<string, unknown> | undefined;

  function openTerminalLink(_event: MouseEvent, uri: string) {
    // Same hardening as the main terminal: never let the opened page navigate
    // its opener (reverse tabnabbing).
    const opened = window.open();
    if (!opened) return;
    try {
      opened.opener = null;
    } catch {
      // Some desktop webviews refuse opener writes; ignore like the built-in handler.
    }
    opened.location.href = uri;
  }

  /** 选区变化后同步菜单可用态（渲染器是 hasSelection 的唯一所有者）。 */
  function syncSelection() {
    hasSelection.value = terminal?.hasSelection() ?? false;
  }

  function createTerminal() {
    const host = options.getHost();
    if (!host || terminal) return;
    appliedTheme = { ...options.getTheme() };
    terminal = new XTerm({
      convertEol: false,
      cursorBlink: true,
      fontFamily: options.getFontFamily(),
      fontSize: options.getFontSize(),
      scrollback: 5_000,
      allowProposedApi: true,
      theme: options.getTheme() as ITheme,
    });
    fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(host);
    // Same width/emoji handling as the primary terminal: load after open().
    unicode11 = new Unicode11Addon();
    terminal.loadAddon(unicode11);
    terminal.unicode.activeVersion = "11";
    webLinks = new WebLinksAddon(openTerminalLink);
    terminal.loadAddon(webLinks);
    searchAddon = new SearchAddon();
    terminal.loadAddon(searchAddon);
    searchAddon.onDidChangeResults(options.onSearchResults);
    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown" || event.altKey || event.shiftKey) return true;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        options.onOpenSearch();
        return false;
      }
      return true;
    });
    dataDisposable = terminal.onData(options.onData);
    selectionDisposable = terminal.onSelectionChange(() => syncSelection());
  }

  /** Measured spawn size. Falls back to 80x24 until the pane is laid out. */
  function measureDimensions(): { cols: number; rows: number } {
    const host = options.getHost();
    if (terminal && fitAddon && host && host.clientWidth > 0 && host.clientHeight > 0) {
      try {
        fitAddon.fit();
      } catch {
        // The iframe can be detached during a tab switch; keep xterm's size.
      }
    }
    const cols = terminal?.cols ?? 0;
    const rows = terminal?.rows ?? 0;
    return {
      cols: cols > 0 ? cols : SPLIT_DEFAULT_COLS,
      rows: rows > 0 ? rows : SPLIT_DEFAULT_ROWS,
    };
  }

  function scheduleFit() {
    if (disposed) return;
    window.clearTimeout(fitTimer);
    fitTimer = window.setTimeout(() => {
      if (disposed || !terminal || !fitAddon) return;
      const host = options.getHost();
      if (!host?.clientWidth || !host?.clientHeight) return;
      try {
        fitAddon.fit();
      } catch {
        return;
      }
      options.onResize(terminal.cols, terminal.rows);
    }, 20);
  }

  function observeSize() {
    const host = options.getHost();
    if (!host || typeof ResizeObserver === "undefined") return;
    resizeObserver = new ResizeObserver(() => scheduleFit());
    resizeObserver.observe(host);
  }

  function focus() {
    terminal?.focus();
  }

  function resetTerminal() {
    terminal?.reset();
  }

  function sameTheme(next: Record<string, unknown>, previous: Record<string, unknown> | undefined): boolean {
    if (next === previous) return true;
    if (!previous) return false;
    const keys = Object.keys(next);
    if (keys.length !== Object.keys(previous).length) return false;
    return keys.every((key) => next[key] === previous[key]);
  }

  const stopThemeWatch = watch(
    options.getTheme,
    (theme) => {
      if (!terminal || sameTheme(theme, appliedTheme)) return;
      appliedTheme = { ...theme };
      terminal.options.theme = theme as ITheme;
      scheduleFit();
    },
    { deep: true },
  );

  const stopFontWatch = watch(
    () => [options.getFontFamily(), options.getFontSize()] as const,
    ([fontFamily, fontSize]) => {
      if (!terminal) return;
      terminal.options.fontFamily = fontFamily;
      terminal.options.fontSize = fontSize;
      scheduleFit();
    },
  );

  // 页签切换：隐藏期间容器尺寸为 0，重新显示后必须 fit 一次并聚焦。
  const stopActiveWatch = watch(options.getActive, async (active) => {
    if (!active || disposed) return;
    await nextTick();
    scheduleFit();
    terminal?.focus();
  });

  // 分屏/关闭会重建叶子槽位，终端宿主被搬进新位置：重新 fit（不抢焦点）。
  const stopLayoutWatch = watch(options.getLayoutEpoch, () => {
    if (disposed) return;
    scheduleFit();
  });

  function isDisposed(): boolean {
    return disposed;
  }

  /**
   * Teardown in the pane's original order: stop timers/listeners, let the
   * caller close its PTY (`onDetach`), then dispose addons and the terminal.
   */
  function dispose(onDetach: () => void) {
    disposed = true;
    stopThemeWatch();
    stopFontWatch();
    stopActiveWatch();
    stopLayoutWatch();
    window.clearTimeout(fitTimer);
    resizeObserver?.disconnect();
    resizeObserver = undefined;
    dataDisposable?.dispose();
    dataDisposable = undefined;
    selectionDisposable?.dispose();
    selectionDisposable = undefined;
    onDetach();
    searchAddon?.dispose();
    searchAddon = undefined;
    fitAddon?.dispose();
    fitAddon = undefined;
    unicode11?.dispose();
    unicode11 = undefined;
    webLinks?.dispose();
    webLinks = undefined;
    terminal?.dispose();
    terminal = undefined;
  }

  return {
    hasSelection,
    getTerminal: () => terminal,
    getSearchAddon: () => searchAddon,
    createTerminal,
    measureDimensions,
    scheduleFit,
    observeSize,
    focus,
    resetTerminal,
    syncSelection,
    isDisposed,
    dispose,
  };
}
