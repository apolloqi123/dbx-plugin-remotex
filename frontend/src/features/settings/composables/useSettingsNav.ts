// 设置导航的全部状态与交互：分类选择、分类搜索（姓名 + 关键词表）、面板滚动
// 复位、Tabs 可访问性补全。
//
// 抽出来的原因有二：SettingsDialog.vue 是编排者，行数预算（structure.spec 600 行）
// 要留给 props/emits 与 13 个 pane 的模板；这里全是与 pane 内容无关的导航细节，
// 单测也更好写（不需要 mount 整个弹窗）。
import { computed, nextTick, ref, watch } from "vue";
import {
  categoryMatchesQuery,
  SETTINGS_CATEGORIES,
  SESSION_CATEGORIES,
  type SettingsCategory,
} from "../lib/settingsNavCategories";

export interface UseSettingsNavOptions {
  /** 会话级分类（sudo / 智能体）只在 SSH 会话下出现；本地终端没有 sessionId。 */
  isLocalSession: () => boolean;
  /** 当前语言的取词函数，用于匹配本地化分类名。 */
  t: (key: string) => string;
}

export function useSettingsNav(options: UseSettingsNavOptions) {
  const { isLocalSession, t } = options;

  const settingsCategory = ref<SettingsCategory>("appearance");

  const visibleSettingsCategories = computed(() =>
    SETTINGS_CATEGORIES.filter(
      (cat) => !isLocalSession() || !SESSION_CATEGORIES.includes(cat.id),
    ),
  );

  // 导航搜索：纯前端过滤（分类只有 13 个，不引入额外状态与持久化）；按 “/” 聚焦、
  // Esc 清空，点击结果后自动复位搜索词，避免过滤视图长期停留在半空状态。
  // 匹配规则（分类名 + 关键词表）见 lib/settingsNavCategories。
  const settingsCategoryQuery = ref("");
  const filteredSettingsCategories = computed(() => {
    const query = settingsCategoryQuery.value.trim().toLowerCase();
    if (!query) {
      return visibleSettingsCategories.value;
    }
    return visibleSettingsCategories.value.filter((cat) =>
      categoryMatchesQuery(cat.id, t(cat.labelKey), query),
    );
  });

  const categorySearchInput = ref<HTMLInputElement | null>(null);

  // 过滤后若当前分类不在结果里，跳到第一个命中项：避免“导航只剩一项、右侧还是旧分类”
  // 的错位。清空搜索词时不做跳转，保留用户原来的分类。
  watch(filteredSettingsCategories, (categories) => {
    if (!settingsCategoryQuery.value.trim() || categories.length === 0) {
      return;
    }
    if (!categories.some((cat) => cat.id === settingsCategory.value)) {
      settingsCategory.value = categories[0].id;
    }
  });

  const activeCategory = computed(
    () =>
      SETTINGS_CATEGORIES.find((cat) => cat.id === settingsCategory.value) ??
      SETTINGS_CATEGORIES[0],
  );
  const activeCategoryIndex = computed(
    () => SETTINGS_CATEGORIES.findIndex((cat) => cat.id === settingsCategory.value) + 1,
  );
  // 仪表盘式序号（“03 / 13”），对齐右侧 hero 的 mono 计数；两位补零保证不跳动。
  const activeCategoryCounter = computed(() => {
    const total = String(visibleSettingsCategories.value.length).padStart(2, "0");
    return `${String(Math.max(activeCategoryIndex.value, 1)).padStart(2, "0")} / ${total}`;
  });

  function focusCategorySearch() {
    categorySearchInput.value?.focus();
    categorySearchInput.value?.select();
  }

  // Esc 在搜索框内先清空搜索词（并挡住冒泡），再按一次才关弹窗：工作台还有一条
  // document 级的 Esc 关闭链，会在 target 阶段之后照常关弹窗，所以这里必须 stopPropagation。
  function onSearchEscape(event: KeyboardEvent) {
    if (!settingsCategoryQuery.value) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    settingsCategoryQuery.value = "";
  }

  function onDialogKeydown(event: KeyboardEvent) {
    if (event.key !== "/" || event.metaKey || event.ctrlKey || event.altKey) {
      return;
    }
    const target = event.target as HTMLElement | null;
    if (target?.closest("input, textarea, select, [contenteditable='true']")) {
      return;
    }
    event.preventDefault();
    focusCategorySearch();
  }

  function onSettingsCategoryChange(value: string | number) {
    settingsCategory.value = value as SettingsCategory;
    settingsCategoryQuery.value = "";
  }

  const navEl = ref<HTMLElement | null>(null);
  const panesEl = ref<HTMLElement | null>(null);

  const settingsPanelId = (id: string) => `settings-panel-${id}`;

  // Tabs 可访问性：reka 的 TabsTrigger 只在挂了 TabsContent 时才算得出 aria-controls，
  // 而这里是 v-show 手写面板（13 个 pane 常驻 DOM），所以面板侧自带 role/id，
  // trigger→面板的 aria-controls 与面板→trigger 的 aria-labelledby 在渲染后成对补上。
  // 注：id 不能写在 <TabsTrigger> 上——reka 自己算 `reka-tabs-*-trigger-*` 并覆盖透传属性。
  function syncTabPanelA11y() {
    const nav = navEl.value;
    const panes = panesEl.value;
    if (!nav || !panes) {
      return;
    }
    for (const trigger of nav.querySelectorAll<HTMLElement>(".settings-nav-item[data-category]")) {
      const category = trigger.dataset.category;
      if (!category) {
        continue;
      }
      const panel = panes.querySelector<HTMLElement>(`#${settingsPanelId(category)}`);
      if (!panel) {
        continue;
      }
      trigger.setAttribute("aria-controls", panel.id);
      panel.setAttribute("aria-labelledby", trigger.id);
    }
  }

  // 切换分类时把面板滚动复位：13 个 pane 共用同一个滚动容器（v-show 而非卸载），
  // 不复位就会「点 Appearance 却落在上一个分类滚到的中段」，首屏标题被卷走。
  watch(settingsCategory, async () => {
    await nextTick();
    panesEl.value?.scrollTo?.({ top: 0 });
    if (panesEl.value) {
      panesEl.value.scrollTop = 0;
    }
  });

  // 导航项随搜索结果增减（v-for 重建），aria 关联每次渲染后重连一次。弹窗内容是
  // portal + v-if 惰性挂载的，onMounted 时 nav 还不存在，所以监听元素引用本身。
  watch([navEl, panesEl], () => {
    void nextTick(syncTabPanelA11y);
  });
  watch(filteredSettingsCategories, () => {
    void nextTick(syncTabPanelA11y);
  });

  return {
    activeCategory,
    activeCategoryCounter,
    categorySearchInput,
    filteredSettingsCategories,
    focusCategorySearch,
    navEl,
    onDialogKeydown,
    onSearchEscape,
    onSettingsCategoryChange,
    panesEl,
    settingsCategory,
    settingsCategoryQuery,
    settingsPanelId,
    syncTabPanelA11y,
    visibleSettingsCategories,
  };
}
