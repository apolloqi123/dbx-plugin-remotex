import { computed, ref, type Ref } from "vue";
import type { Terminal } from "@xterm/xterm";
import {
  buildSearchUrl,
  DEFAULT_CTX_SEARCH_ENGINES_TEXT,
  parseCtxSearchEnginesText,
  type CtxSearchEngine,
} from "../../features/terminal/components/TerminalContextMenu.vue";

/**
 * 右键菜单「在线搜索」：引擎表以原始文本持久化（sidecar preferences，
 * allowlist 键 ctx_search_engines，每行 name|url 模板）；解析收口在
 * TerminalContextMenu.vue 的纯函数，非法行静默丢弃、解析永不失败（空表只是
 * 隐藏 Search online 项）。分屏/页签终端复用主终端的引擎表与复制提示。
 */
export function useOnlineSearch(options: {
  terminal: () => Terminal | undefined;
  terminalMenuOpen: Ref<boolean>;
  copyTextToClipboard: (value: string, noticeKey: string, values?: Record<string, string | number>) => void;
  persistPrefs: (patch: Record<string, unknown>) => void;
}) {
  const { terminal, terminalMenuOpen, copyTextToClipboard, persistPrefs } = options;

  const ctxSearchEnginesText = ref(DEFAULT_CTX_SEARCH_ENGINES_TEXT);
  const ctxSearchEngines = computed(() => parseCtxSearchEnginesText(ctxSearchEnginesText.value));

  function updateCtxSearchEngines(text: string) {
    ctxSearchEnginesText.value = text;
    void persistPrefs({ ctx_search_engines: text });
  }

  function searchSelectionOnline(engine: CtxSearchEngine) {
    terminalMenuOpen.value = false;
    searchTextOnline(engine, terminal()?.getSelection() || "");
    terminal()?.focus();
  }

  function searchTextOnline(engine: CtxSearchEngine, text: string) {
    const query = text.trim();
    if (!query) return;
    const url = buildSearchUrl(engine, query);
    if (!url) return;
    // TODO(host): 宿主尚未提供 openExternal；待宿主开放后改为直接唤起系统浏览器。
    // 当前兜底：把搜索链接复制进剪贴板并提示（失败走 sftp 错误条）。
    copyTextToClipboard(url, "ctxSearch.linkCopied");
  }

  return {
    ctxSearchEnginesText,
    ctxSearchEngines,
    updateCtxSearchEngines,
    searchSelectionOnline,
    searchTextOnline,
  };
}
