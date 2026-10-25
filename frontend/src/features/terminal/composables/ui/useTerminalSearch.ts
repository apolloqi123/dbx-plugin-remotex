import { ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import type { SearchAddon, ISearchOptions } from "@xterm/addon-search";
import { pluginStore } from "../../../../shared/lib/pluginStore";
import {
  sanitizeSearchOptions,
  TERMINAL_SEARCH_OPTIONS_KEY,
  terminalSearchSeedFromSelection,
  type TerminalSearchOptions,
} from "../../lib/behavior/terminalInteraction";

export type TerminalSearchMatchState = "idle" | "match" | "no-match";

/**
 * 主终端搜索面板：状态与搜索动作收口在这里；addon 由终端初始化阶段注入，
 * 因此用 getter 读取，组件卸载时调用方负责 dispose。
 */
export function useTerminalSearch(options: {
  terminal: () => Terminal | undefined;
  searchAddon: () => SearchAddon | undefined;
  closeMenu?: () => void;
}) {
  const searchOpen = ref(false);
  // 打开搜索面板时的种子状态：选区首行预填 + 持久化的选项开关。
  const searchSeedQuery = ref("");
  const searchSeedOptions = ref<TerminalSearchOptions>(sanitizeSearchOptions(null));
  const searchMatchState = ref<TerminalSearchMatchState>("idle");
  const searchResultIndex = ref(0);
  const searchResultCount = ref(0);

  function resetSearchResults() {
    searchMatchState.value = "idle";
    searchResultIndex.value = 0;
    searchResultCount.value = 0;
  }

  function openTerminalSearch() {
    const terminal = options.terminal();
    if (!terminal) return;
    options.closeMenu?.();
    // iTerm2 风格：打开搜索时用当前选区首行预填查询，并带入持久化的选项开关。
    searchSeedQuery.value = terminalSearchSeedFromSelection(terminal.getSelection() || "");
    searchSeedOptions.value = sanitizeSearchOptions(pluginStore.getItem(TERMINAL_SEARCH_OPTIONS_KEY));
    searchOpen.value = true;
  }

  function closeTerminalSearch() {
    searchOpen.value = false;
    resetSearchResults();
    options.searchAddon()?.clearDecorations();
    options.terminal()?.focus();
  }

  function clearTerminalSearch() {
    resetSearchResults();
    options.searchAddon()?.clearDecorations();
  }

  function handleSearchResults({ resultCount, resultIndex }: { resultCount: number; resultIndex: number }) {
    if (!searchOpen.value) return;
    searchResultCount.value = resultCount;
    searchResultIndex.value = resultCount > 0 && resultIndex >= 0 ? resultIndex + 1 : 0;
    searchMatchState.value = resultCount > 0 ? "match" : "no-match";
  }

  function runTerminalSearch(query: string, searchOptions: { caseSensitive: boolean; regex: boolean; wholeWord: boolean }, direction: "next" | "prev") {
    const addon = options.searchAddon();
    if (!addon || !query) return;
    const runtimeOptions: ISearchOptions = {
      caseSensitive: searchOptions.caseSensitive,
      regex: searchOptions.regex,
      wholeWord: searchOptions.wholeWord,
      decorations: {
        matchBackground: "#64748b55",
        matchOverviewRuler: "#64748b",
        activeMatchBackground: "#3b82f655",
        activeMatchColorOverviewRuler: "#3b82f6",
      },
    };
    if (direction === "prev") addon.findPrevious(query, runtimeOptions);
    else addon.findNext(query, runtimeOptions);
  }

  return {
    searchOpen,
    searchSeedQuery,
    searchSeedOptions,
    searchMatchState,
    searchResultIndex,
    searchResultCount,
    openTerminalSearch,
    closeTerminalSearch,
    clearTerminalSearch,
    resetSearchResults,
    handleSearchResults,
    runTerminalSearch,
  };
}
