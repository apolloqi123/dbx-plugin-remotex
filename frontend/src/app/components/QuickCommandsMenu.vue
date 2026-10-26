<script setup lang="ts">
import { Braces, ChevronDown, ChevronRight, Search, Zap } from "@lucide/vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import type { QuickCommand, QuickCommandGroupView } from "../../features/quick-commands/lib/quickCommands";
import type { QuickCommandsMenuActions } from "../lib/workbenchToolbarActions";

/** 工具条快速命令下拉（T19 自 App.vue 模板迁入）：列表 + 搜索 + 执行
 *  （Termius Snippets 式）；新建/编辑/导入管理入口已归位设置·终端。
 *  命令数据面（PTY 写入/持久化）与弹层视图态仍由 App 的 composables 持有，
 *  本组件只渲染并上抛动作。 */
const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  connected: boolean;
  quickCommands: QuickCommand[];
  filteredQuickCommands: QuickCommand[];
  groupedQuickCommands: QuickCommandGroupView[];
  quickHasRealGroups: boolean;
  quickCollapsedGroups: string[];
  quickExpandedId: string | null;
  quickCommandGroupSummary: string;
  actions: QuickCommandsMenuActions;
}>();

const quickMenuOpen = defineModel<boolean>("quickMenuOpen", { required: true });
const quickSearch = defineModel<string>("quickSearch", { required: true });

const t = props.t;
// 动作名与迁移前 App 模板里的绑定名逐字一致，模板不需要改写。
const {
  toggleQuickMenu,
  sendQuickCommand,
  pasteQuickCommand,
  toggleQuickGroupCollapse,
  quickGroupName,
  toggleQuickExpand,
} = props.actions;
</script>

<template>
  <div>
    <Popover :open="quickMenuOpen" @update:open="(open) => { if (!open) quickMenuOpen = false; }">
      <PopoverAnchor as-child>
        <button class="icon-button icon-amber" :title="t('quickCommands')" :aria-label="t('quickCommands')" :disabled="!connected" @pointerdown.stop @click.stop="toggleQuickMenu"><Zap /></button>
      </PopoverAnchor>
      <PopoverContent class="popover quick-commands-popover" align="end" :side-offset="5">
      <!-- M32-A3 后工具条只留"列表 + 搜索 + 执行"（Termius Snippets 式）：
           新建/编辑/导入管理归位设置·终端（QuickCommandsSection）。 -->
      <h3>{{ t("quickCommands") }}</h3>
      <p class="quick-command-global-hint">{{ t("quickCommandsGlobalHint") }}</p>
      <div v-if="quickCommands.length" class="quick-search">
        <Search />
        <input v-model="quickSearch" :placeholder="t('quickCommandsSearch')" spellcheck="false" />
      </div>
      <div v-if="!quickCommands.length" class="empty compact">{{ t("quickCommandsEmpty") }}</div>
      <div v-else-if="!filteredQuickCommands.length" class="empty compact">{{ t("quickCommandsNoMatch") }}</div>
      <template v-else-if="quickHasRealGroups">
        <section v-for="group in groupedQuickCommands" :key="group.id || '__ungrouped__'" class="quick-group">
          <button class="quick-group-head" :aria-expanded="!quickCollapsedGroups.includes(group.id)" @click="toggleQuickGroupCollapse(group.id)">
            <ChevronDown v-if="!quickCollapsedGroups.includes(group.id)" class="quick-group-chevron" />
            <ChevronRight v-else class="quick-group-chevron" />
            <span class="quick-group-name">{{ quickGroupName(group) }}</span>
            <span class="quick-group-count">{{ group.commands.length }}</span>
          </button>
          <template v-if="!quickCollapsedGroups.includes(group.id)">
            <div v-for="item in group.commands" :key="item.id" class="quick-command-row quick-card" :class="{ expanded: quickExpandedId === item.id }">
              <button class="quick-card-main" :title="item.command" :aria-label="item.command" @click="toggleQuickExpand(item.id)">
                <Braces class="quick-card-icon" />
                <span class="quick-card-text">
                  <strong>{{ item.name }}</strong>
                  <span class="mono">{{ item.command }}</span>
                </span>
              </button>
              <div class="quick-card-actions">
                <button class="quick-action" :disabled="!connected" @click="sendQuickCommand(item)">{{ t("quickCommandRun") }}</button>
                <button class="quick-action" :disabled="!connected" @click="pasteQuickCommand(item)">{{ t("quickCommandPaste") }}</button>
              </div>
              <div v-if="quickExpandedId === item.id" class="quick-card-full mono">{{ item.command }}</div>
            </div>
          </template>
        </section>
      </template>
      <div v-for="item in filteredQuickCommands" v-else :key="item.id" class="quick-command-row quick-card" :class="{ expanded: quickExpandedId === item.id }">
        <button class="quick-card-main" :title="item.command" :aria-label="item.command" @click="toggleQuickExpand(item.id)">
          <Braces class="quick-card-icon" />
          <span class="quick-card-text">
            <strong>{{ item.name }}</strong>
            <span class="mono">{{ item.command }}</span>
          </span>
        </button>
        <div class="quick-card-actions">
          <button class="quick-action" :disabled="!connected" @click="sendQuickCommand(item)">{{ t("quickCommandRun") }}</button>
          <button class="quick-action" :disabled="!connected" @click="pasteQuickCommand(item)">{{ t("quickCommandPaste") }}</button>
        </div>
        <div v-if="quickExpandedId === item.id" class="quick-card-full mono">{{ item.command }}</div>
      </div>
      <footer class="quick-command-footer quick-command-footer--readonly">
        <span class="quick-command-limit">{{ quickCommandGroupSummary }}</span>
        <span class="quick-command-manage-hint">{{ t("quickCommands.manageHint") }}</span>
      </footer>
      </PopoverContent>
    </Popover>
  </div>
</template>

<style scoped>
/* 快速命令下拉（原 App.vue scoped 样式随 DOM 迁入）：M32-A3 后只剩
   "列表 + 搜索 + 执行"；新建/编辑/导入样式随管理视图迁入 QuickCommandsSection。 */
.quick-commands-popover h3 { margin: 2px 4px 6px; font-size: 12px; }
/* 搜索行：图标 + 无边框输入（容器边框即输入框）。 */
.quick-search { display: flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: var(--radius); margin-bottom: 4px; padding: 0 8px; background: var(--background); }
.quick-search:focus-within { border-color: color-mix(in srgb, var(--primary) 70%, var(--border)); }
.quick-search svg { width: 13px; height: 13px; flex: 0 0 13px; color: var(--muted-foreground); }
.quick-search input { min-width: 0; flex: 1; height: 26px; border: 0; padding: 0; background: transparent; color: var(--foreground); font-size: 12px; outline: none; }
/* 命令卡片（Termius Snippets 式）：{} 图标 + 名称/命令两行；动作按钮 hover
   或展开时浮现；展开时显示完整命令（自动换行）。 */
.quick-card { display: flex; flex-wrap: wrap; align-items: center; gap: 2px; border: 1px solid transparent; border-radius: var(--radius); padding: 3px 4px; }
.quick-card:hover { background: color-mix(in srgb, var(--accent) 55%, transparent); }
.quick-card.expanded { border-color: var(--border); background: color-mix(in srgb, var(--accent) 40%, transparent); }
.quick-card-main { display: flex; min-width: 0; flex: 1; align-items: center; gap: 8px; border: 0; padding: 3px; background: transparent; color: var(--foreground); text-align: left; cursor: pointer; }
.quick-card-icon { width: 15px; height: 15px; flex: 0 0 15px; color: var(--muted-foreground); }
.quick-card.expanded .quick-card-icon, .quick-card:hover .quick-card-icon { color: var(--primary); }
.quick-card-text { display: flex; min-width: 0; flex: 1; flex-direction: column; gap: 1px; }
.quick-card-text strong { max-width: 100%; overflow: hidden; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.quick-card-text .mono { max-width: 100%; overflow: hidden; color: var(--muted-foreground); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.quick-card-actions { display: flex; flex: 0 0 auto; align-items: center; gap: 3px; opacity: 0; transition: opacity 100ms ease; }
.quick-card:hover .quick-card-actions, .quick-card.expanded .quick-card-actions, .quick-card-actions:focus-within { opacity: 1; }
.quick-action { height: 22px; border: 1px solid var(--border); border-radius: var(--radius); padding: 0 8px; background: var(--background); color: var(--foreground); font-size: 10px; cursor: pointer; }
.quick-action:hover:not(:disabled) { background: var(--accent); }
.quick-card-full { flex: 1 1 100%; margin: 2px 4px 4px 27px; color: var(--foreground); font-size: 11px; line-height: 1.55; white-space: pre-wrap; overflow-wrap: anywhere; }
/* 分组头（issue #201）：可折叠 + 组名 + 条数徽标；未分组同样式。 */
.quick-group { display: flex; flex-direction: column; }
.quick-group-head { display: flex; align-items: center; gap: 5px; border: 0; padding: 3px 4px; background: transparent; color: var(--muted-foreground); text-align: left; cursor: pointer; }
.quick-group-head:hover { color: var(--foreground); }
.quick-group-chevron { width: 13px; height: 13px; flex: 0 0 13px; }
.quick-group-name { min-width: 0; flex: 1; overflow: hidden; font-size: 11px; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
.quick-group-count { flex: 0 0 auto; border-radius: 8px; padding: 0 6px; background: var(--muted); color: var(--muted-foreground); font-size: 9px; line-height: 15px; }
/* 只读 footer：限额 + "管理入口在设置" 指引，不再有编辑动作。 */
.quick-command-footer { display: flex; align-items: center; gap: 8px; border-top: 1px solid var(--border); margin-top: 4px; padding-top: 8px; }
.quick-command-footer--readonly { justify-content: space-between; }
.quick-command-limit { color: var(--muted-foreground); font-size: 10px; white-space: nowrap; }
.quick-command-manage-hint { min-width: 0; overflow: hidden; color: var(--muted-foreground); font-size: 10px; text-align: right; text-overflow: ellipsis; white-space: nowrap; }
</style>
