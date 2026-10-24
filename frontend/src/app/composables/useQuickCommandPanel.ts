import { computed, ref, type Ref } from "vue";
import {
  filterQuickCommands,
  groupQuickCommands,
  quickCommandGroupCount,
  type QuickCommand,
  type QuickCommandGroupView,
} from "../../features/quick-commands/lib/quickCommands";

/**
 * 快速命令弹层（工具条 Zap 按钮）的纯视图状态：搜索过滤 / 卡片展开 /
 * 分组折叠与派生视图。数据面（增删改、PTY 写入、持久化）在
 * features/quick-commands/composables/useQuickCommands；这里只承载弹层
 * 打开后的会话内 UI 态（不持久化），供模板与工具条 toggle 共用。
 */
export function useQuickCommandPanel(options: {
  quickCommands: Ref<QuickCommand[]>;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const { quickCommands, t } = options;

  const quickMenuOpen = ref(false);
  // Termius Snippets 式面板状态：搜索过滤 / 卡片展开。
  const quickSearch = ref("");
  const quickExpandedId = ref<string | null>(null);
  const filteredQuickCommands = computed(() => filterQuickCommands(quickCommands.value, quickSearch.value));
  // 分组视图（issue #201）：派生组 + 折叠态（会话内 UI 态，不持久化）。仅一条
  // 未分组时不渲染组头（等同旧扁平视图，避免噪声）；有真实分组才分组展示。
  const groupedQuickCommands = computed<QuickCommandGroupView[]>(() => groupQuickCommands(filteredQuickCommands.value));
  const quickHasRealGroups = computed(() => quickCommandGroupCount(quickCommands.value) > 0);
  const quickCollapsedGroups = ref<string[]>([]);
  function toggleQuickGroupCollapse(groupId: string) {
    const index = quickCollapsedGroups.value.indexOf(groupId);
    if (index >= 0) quickCollapsedGroups.value.splice(index, 1);
    else quickCollapsedGroups.value.push(groupId);
  }
  function quickGroupName(group: QuickCommandGroupView): string {
    return group.id ? group.name : t("quickCommandsUngrouped");
  }
  const quickCommandGroupSummary = computed(() => t("quickCommandsGroupSummary", { groups: quickCommandGroupCount(quickCommands.value), count: quickCommands.value.length }));

  function toggleQuickExpand(id: string) {
    quickExpandedId.value = quickExpandedId.value === id ? null : id;
  }

  return {
    quickMenuOpen,
    quickSearch,
    quickExpandedId,
    filteredQuickCommands,
    groupedQuickCommands,
    quickHasRealGroups,
    quickCollapsedGroups,
    quickCommandGroupSummary,
    toggleQuickGroupCollapse,
    quickGroupName,
    toggleQuickExpand,
  };
}
