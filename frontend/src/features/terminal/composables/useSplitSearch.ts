/**
 * Pane-local terminal search for the split/tab terminal pane.
 *
 * Owns the search panel state (open, seed query/options, match counters) and
 * the xterm SearchAddon wiring for one pane. The addon itself stays owned by
 * the component, which keeps its disposal order on unmount; this composable
 * only reads it through the getter and clears its decorations.
 */
import { ref } from "vue";
import type { ISearchOptions, SearchAddon } from "@xterm/addon-search";
import type { Terminal as XTerm } from "@xterm/xterm";
import { pluginStore } from "../../../shared/lib/pluginStore";
import {
  TERMINAL_SEARCH_OPTIONS_KEY,
  sanitizeSearchOptions,
  terminalSearchSeedFromSelection,
  type TerminalSearchOptions,
} from "../lib/terminalInteraction";

type SplitSearchMatchState = "idle" | "match" | "no-match";

/** Toggle state emitted by TerminalSearchPanel. */
export interface SplitSearchOptions {
  caseSensitive: boolean;
  regex: boolean;
  wholeWord: boolean;
}

export interface SplitSearchResultChange {
  resultCount: number;
  resultIndex: number;
}

interface UseSplitSearchOptions {
  getTerminal: () => XTerm | undefined;
  getSearchAddon: () => SearchAddon | undefined;
  /** 打开搜索面板前收起窗格右键菜单。 */
  closeMenu: () => void;
}

export function useSplitSearch({ getTerminal, getSearchAddon, closeMenu }: UseSplitSearchOptions) {
  // 分屏自己的搜索面板：与主终端同一组件/持久化开关，addon 绑定本窗格 xterm。
  const open = ref(false);
  const seedQuery = ref("");
  const seedOptions = ref<TerminalSearchOptions>(sanitizeSearchOptions(null));
  const matchState = ref<SplitSearchMatchState>("idle");
  const resultIndex = ref(0);
  const resultCount = ref(0);

  function resetResults() {
    matchState.value = "idle";
    resultIndex.value = 0;
    resultCount.value = 0;
  }

  /** SearchAddon.onDidChangeResults 回调：面板未打开时忽略。 */
  function handleResultsChange({ resultCount: count, resultIndex: index }: SplitSearchResultChange) {
    if (!open.value) return;
    resultCount.value = count;
    resultIndex.value = count > 0 && index >= 0 ? index + 1 : 0;
    matchState.value = count > 0 ? "match" : "no-match";
  }

  function openSearch() {
    const terminal = getTerminal();
    if (!terminal) return;
    closeMenu();
    seedQuery.value = terminalSearchSeedFromSelection(terminal.getSelection() || "");
    seedOptions.value = sanitizeSearchOptions(pluginStore.getItem(TERMINAL_SEARCH_OPTIONS_KEY));
    open.value = true;
  }

  function closeSearch() {
    open.value = false;
    resetResults();
    getSearchAddon()?.clearDecorations();
    getTerminal()?.focus();
  }

  function clearSearch() {
    resetResults();
    getSearchAddon()?.clearDecorations();
  }

  function run(query: string, options: SplitSearchOptions, direction: "next" | "prev") {
    const searchAddon = getSearchAddon();
    if (!searchAddon || !query) return;
    const searchOptions: ISearchOptions = {
      caseSensitive: options.caseSensitive,
      regex: options.regex,
      wholeWord: options.wholeWord,
      decorations: {
        matchBackground: "#64748b55",
        matchOverviewRuler: "#64748b",
        activeMatchBackground: "#3b82f655",
        activeMatchColorOverviewRuler: "#3b82f6",
      },
    };
    if (direction === "prev") searchAddon.findPrevious(query, searchOptions);
    else searchAddon.findNext(query, searchOptions);
  }

  return {
    open,
    seedQuery,
    seedOptions,
    matchState,
    resultIndex,
    resultCount,
    openSearch,
    closeSearch,
    clearSearch,
    run,
    handleResultsChange,
  };
}
