import { computed, ref } from "vue";
import { randomUUID } from "../../../shared/lib/uuid";

/** 一个工作台内的终端页签：main 是连接主 PTY，aux 是额外 PTY。 */
export interface TerminalTab {
  id: string;
  kind: "main" | "aux";
  /** aux 页签的展示序号（从 1 开始，关闭后不复用）。 */
  index: number;
}

export const TERMINAL_TAB_LIMIT = 8;
const MAIN_TAB_ID = "main";

/** 工作台内终端页签状态机：新建/切换/关闭，关闭当前页签时落到右邻或左邻。 */
export function useTerminalTabs(options: { limit?: number; createId?: () => string } = {}) {
  const limit = Math.max(2, Math.floor(options.limit ?? TERMINAL_TAB_LIMIT));
  const createId = options.createId ?? randomUUID;
  let nextIndex = 0;

  const tabs = ref<TerminalTab[]>([{ id: MAIN_TAB_ID, kind: "main", index: 0 }]);
  const activeId = ref(MAIN_TAB_ID);
  const auxTabs = computed(() => tabs.value.filter((tab) => tab.kind === "aux"));
  const canAdd = computed(() => tabs.value.length < limit);

  function create(): TerminalTab | undefined {
    if (!canAdd.value) return undefined;
    nextIndex += 1;
    const tab: TerminalTab = { id: createId(), kind: "aux", index: nextIndex };
    tabs.value.push(tab);
    activeId.value = tab.id;
    return tab;
  }

  function activate(id: string) {
    if (tabs.value.some((tab) => tab.id === id)) activeId.value = id;
  }

  function close(id: string): boolean {
    const index = tabs.value.findIndex((tab) => tab.id === id);
    if (index < 0 || tabs.value[index].kind === "main") return false;
    tabs.value.splice(index, 1);
    if (activeId.value === id) {
      const next = tabs.value[index] ?? tabs.value[index - 1] ?? tabs.value[0];
      activeId.value = next.id;
    }
    return true;
  }

  function reset() {
    tabs.value = [{ id: MAIN_TAB_ID, kind: "main", index: 0 }];
    activeId.value = MAIN_TAB_ID;
  }

  function tabLabelKey(tab: TerminalTab): "terminalTab.main" | "terminalTab.index" {
    return tab.kind === "main" ? "terminalTab.main" : "terminalTab.index";
  }

  return {
    tabs,
    activeId,
    auxTabs,
    canAdd,
    limit,
    create,
    activate,
    close,
    reset,
    tabLabelKey,
  };
}
