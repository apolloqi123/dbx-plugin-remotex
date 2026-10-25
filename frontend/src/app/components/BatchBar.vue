<script setup lang="ts">
import { Braces, ChevronDown, ChevronRight, ListChecks, Loader2, Save, Search, Send, X } from "@lucide/vue";
import { Popover, PopoverAnchor, PopoverContent } from "../../shared/ui/popover/index";
import { batchTargetLabel, type BatchSendTarget } from "../../features/terminal/lib/transfer/batchSend";
import { QUICK_COMMANDS_LIMIT, type QuickCommand, type QuickCommandGroupView } from "../../features/quick-commands/lib/quickCommands";

/** 批量发送命令条（Electerm quick-command bar）：贴终端底部，回车即发送。
 *  目标选择/快速命令切换/保存为快速命令均在条上完成；状态与 sidecar 接线
 *  仍由 App 的 useBatchSend 持有，本组件只渲染并上抛动作。 */
const props = defineProps<{
  batchTargetsOpen: boolean;
  batchLoading: boolean;
  batchSending: boolean;
  batchTargets: BatchSendTarget[];
  batchSelected: string[];
  batchDraft: string;
  batchQuickOpen: boolean;
  batchQuickSearch: string;
  filteredBatchQuickCommands: QuickCommand[];
  groupedBatchQuickCommands: QuickCommandGroupView[];
  batchQuickPickName: string;
  batchSaveMode: boolean;
  batchSaveName: string;
  batchSaving: boolean;
  quickCommands: QuickCommand[];
  quickHasRealGroups: boolean;
  quickCollapsedGroups: string[];
  quickCommandGroupSummary: string;
  quickGroupName: (group: QuickCommandGroupView) => string;
  sessionId: string;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "update:batchTargetsOpen", value: boolean): void;
  (e: "update:batchQuickOpen", value: boolean): void;
  (e: "update:batchQuickSearch", value: string): void;
  (e: "update:batchSaveName", value: string): void;
  (e: "update:batchDraft", value: string): void;
  (e: "toggle-targets"): void;
  (e: "pick-targets", mode: "connected" | "all"): void;
  (e: "refresh-targets"): void;
  (e: "toggle-target", sessionId: string): void;
  (e: "toggle-quick"): void;
  (e: "pick-quick", item: QuickCommand): void;
  (e: "toggle-group", groupId: string): void;
  (e: "broadcast"): void;
  (e: "browse-history", direction: "up" | "down"): void;
  (e: "send"): void;
  (e: "open-save"): void;
  (e: "confirm-save"): void;
  (e: "cancel-save"): void;
}>();

const t = props.t;

function onDraftInput(event: Event) {
  emit("update:batchDraft", (event.target as HTMLInputElement).value);
  emit("broadcast");
}
</script>

<template>
  <section class="batch-bar" @contextmenu.stop @mousedown.stop @click.stop>
    <div>
      <!-- 底部条上的 popover 向上展开（reka side="top"，旧绝对定位会被裁切）。 -->
      <Popover :open="batchTargetsOpen" @update:open="(value) => emit('update:batchTargetsOpen', value)">
        <PopoverAnchor as-child>
          <button class="batch-bar-targets" :title="t('batchSendTitle')" :aria-label="t('batchSendTitle')" @pointerdown.stop @click.stop="emit('toggle-targets')"><ListChecks /><span>{{ t("batchSendTargets", { count: batchSelected.length, total: batchTargets.length }) }}</span></button>
        </PopoverAnchor>
        <PopoverContent class="popover batch-targets-popover" side="top" align="start" :side-offset="6">
        <p class="muted batch-send-hint">{{ t("batchSendHint") }}</p>
        <div class="command-history-header">
          <span>{{ t("batchSendTargets", { count: batchSelected.length, total: batchTargets.length }) }}</span>
          <span class="batch-target-actions">
            <button class="link-button" @click="emit('pick-targets', 'connected')">{{ t("batchSendConnected") }}</button>
            <button class="link-button" @click="emit('pick-targets', 'all')">{{ t("batchSendAll") }}</button>
            <button class="link-button" :disabled="batchLoading" @click="emit('refresh-targets')">{{ t("refresh") }}</button>
          </span>
        </div>
        <div v-if="batchLoading && !batchTargets.length" class="empty compact"><Loader2 class="spinning" /><span>{{ t("batchSendLoading") }}</span></div>
        <div v-else-if="!batchTargets.length" class="empty compact">{{ t("batchSendNoSessions") }}</div>
        <div v-else class="batch-target-list">
          <label v-for="target in batchTargets" :key="target.sessionId" class="batch-target-row" :class="{ offline: target.connected === false }">
            <input type="checkbox" :checked="batchSelected.includes(target.sessionId)" @change="emit('toggle-target', target.sessionId)" />
            <span class="mono">{{ batchTargetLabel(target) }}</span>
            <span v-if="target.sessionId === sessionId" class="batch-badge">{{ t("batchSendCurrent") }}</span>
            <span v-if="target.connected === false" class="batch-badge batch-badge-warn">{{ t("batchSendDisconnected") }}</span>
            <span v-if="target.readOnly" class="read-only-badge">{{ t("readOnly") }}</span>
          </label>
        </div>
        </PopoverContent>
      </Popover>
    </div>
    <!-- 快速命令弹层：与右上角面板同形态（搜索 + 可折叠分组 + 命令卡片，
         复用面板样式类与折叠态），点击卡片即回填批量输入框（发送仍由
         回车/发送按钮触发）。无真实分组时保持平铺，避免单条未分组的噪声组头。 -->
    <Popover v-if="!batchSaveMode && quickCommands.length" :open="batchQuickOpen" @update:open="(value) => emit('update:batchQuickOpen', value)">
      <PopoverAnchor as-child>
        <button class="batch-bar-quick batch-bar-targets" :title="t('batchSendQuickPick')" :aria-label="t('batchSendQuickPick')" @pointerdown.stop @click.stop="emit('toggle-quick')">
          <Braces />
          <span>{{ batchQuickPickName || t("batchSendQuickPick") }}</span>
        </button>
      </PopoverAnchor>
      <PopoverContent class="popover batch-quick-popover" side="top" align="start" :side-offset="6">
        <div class="quick-search">
          <Search />
          <input :value="batchQuickSearch" :placeholder="t('quickCommandsSearch')" spellcheck="false" @input="emit('update:batchQuickSearch', ($event.target as HTMLInputElement).value)" />
        </div>
        <div v-if="!filteredBatchQuickCommands.length" class="empty compact">{{ t("quickCommandsNoMatch") }}</div>
        <template v-else-if="quickHasRealGroups">
          <section v-for="group in groupedBatchQuickCommands" :key="group.id || '__ungrouped__'" class="quick-group">
            <button class="quick-group-head" :aria-expanded="!quickCollapsedGroups.includes(group.id)" @click="emit('toggle-group', group.id)">
              <ChevronDown v-if="!quickCollapsedGroups.includes(group.id)" class="quick-group-chevron" />
              <ChevronRight v-else class="quick-group-chevron" />
              <span class="quick-group-name">{{ quickGroupName(group) }}</span>
              <span class="quick-group-count">{{ group.commands.length }}</span>
            </button>
            <template v-if="!quickCollapsedGroups.includes(group.id)">
              <button v-for="item in group.commands" :key="item.id" class="quick-command-row quick-card" :title="item.command" :aria-label="item.command" @click="emit('pick-quick', item)">
                <Braces class="quick-card-icon" />
                <span class="quick-card-text">
                  <strong>{{ item.name }}</strong>
                  <span class="mono">{{ item.command }}</span>
                </span>
              </button>
            </template>
          </section>
        </template>
        <button v-for="item in filteredBatchQuickCommands" v-else :key="item.id" class="quick-command-row quick-card" :title="item.command" :aria-label="item.command" @click="emit('pick-quick', item)">
          <Braces class="quick-card-icon" />
          <span class="quick-card-text">
            <strong>{{ item.name }}</strong>
            <span class="mono">{{ item.command }}</span>
          </span>
        </button>
        <footer class="quick-command-footer quick-command-footer--readonly">
          <span class="quick-command-limit">{{ quickCommandGroupSummary }}</span>
        </footer>
      </PopoverContent>
    </Popover>
    <input
      v-if="batchSaveMode"
      :value="batchSaveName"
      class="batch-bar-input"
      :placeholder="t('quickCommandsName')"
      :maxlength="60"
      autofocus
      :disabled="batchSaving"
      @input="emit('update:batchSaveName', ($event.target as HTMLInputElement).value)"
      @keydown.enter="emit('confirm-save')"
    />
    <input
      v-else
      :value="batchDraft"
      class="batch-bar-input mono"
      spellcheck="false"
      :placeholder="t('batchSendPlaceholder')"
      :disabled="batchSending"
      @input="onDraftInput"
      @keydown.up.prevent="emit('browse-history', 'up')"
      @keydown.down.prevent="emit('browse-history', 'down')"
      @keydown.enter="emit('send')"
    />
    <template v-if="batchSaveMode">
      <button class="icon-button icon-emerald" :title="t('save')" :aria-label="t('save')" :disabled="batchSaving" @click="emit('confirm-save')"><Save /></button>
      <button class="icon-button" :title="t('cancel')" :aria-label="t('cancel')" :disabled="batchSaving" @click="emit('cancel-save')"><X /></button>
    </template>
    <button
      v-else
      class="icon-button"
      :title="quickCommands.length >= QUICK_COMMANDS_LIMIT ? t('quickCommandsLimit', { count: quickCommands.length, limit: QUICK_COMMANDS_LIMIT }) : t('batchBarSave')" :aria-label="quickCommands.length >= QUICK_COMMANDS_LIMIT ? t('quickCommandsLimit', { count: quickCommands.length, limit: QUICK_COMMANDS_LIMIT }) : t('batchBarSave')"
      :disabled="!batchDraft.trim() || quickCommands.length >= QUICK_COMMANDS_LIMIT"
      @click="emit('open-save')"
    ><Save /></button>
    <button class="icon-button icon-emerald batch-bar-send" :title="t('batchSendSend')" :aria-label="t('batchSendSend')" :disabled="batchSending || !batchDraft.trim() || !batchSelected.length" @click="emit('send')">
      <Loader2 v-if="batchSending" class="spinning" />
      <Send v-else />
    </button>
  </section>
</template>

<style scoped>
/* 快速命令列表与批量条快速弹层共用同一组卡片/分组样式（与 App 内联
   快速命令弹层一致；App 侧同名规则仍保留给工具条弹层）。 */
.command-history-header { display: flex; align-items: center; justify-content: space-between; color: var(--muted-foreground); font-size: 10px; letter-spacing: .04em; text-transform: uppercase; }
.quick-search { display: flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: var(--radius); margin-bottom: 4px; padding: 0 8px; background: var(--background); }
.quick-search:focus-within { border-color: color-mix(in srgb, var(--primary) 70%, var(--border)); }
.quick-search svg { width: 13px; height: 13px; flex: 0 0 13px; color: var(--muted-foreground); }
.quick-search input { min-width: 0; flex: 1; height: 26px; border: 0; padding: 0; background: transparent; color: var(--foreground); font-size: 12px; outline: none; }
.quick-card { display: flex; flex-wrap: wrap; align-items: center; gap: 2px; border: 1px solid transparent; border-radius: var(--radius); padding: 3px 4px; }
.quick-card:hover { background: color-mix(in srgb, var(--accent) 55%, transparent); }
.quick-card.expanded { border-color: var(--border); background: color-mix(in srgb, var(--accent) 40%, transparent); }
.quick-card-icon { width: 15px; height: 15px; flex: 0 0 15px; color: var(--muted-foreground); }
.quick-card.expanded .quick-card-icon, .quick-card:hover .quick-card-icon { color: var(--primary); }
.quick-card-text { display: flex; min-width: 0; flex: 1; flex-direction: column; gap: 1px; }
.quick-card-text strong { max-width: 100%; overflow: hidden; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.quick-card-text .mono { max-width: 100%; overflow: hidden; color: var(--muted-foreground); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.quick-group { display: flex; flex-direction: column; }
.quick-group-head { display: flex; align-items: center; gap: 5px; border: 0; padding: 3px 4px; background: transparent; color: var(--muted-foreground); text-align: left; cursor: pointer; }
.quick-group-head:hover { color: var(--foreground); }
.quick-group-chevron { width: 13px; height: 13px; flex: 0 0 13px; }
.quick-group-name { min-width: 0; flex: 1; overflow: hidden; font-size: 11px; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
.quick-group-count { flex: 0 0 auto; border-radius: 8px; padding: 0 6px; background: var(--muted); color: var(--muted-foreground); font-size: 9px; line-height: 15px; }
.quick-command-footer { display: flex; align-items: center; gap: 8px; border-top: 1px solid var(--border); margin-top: 4px; padding-top: 8px; }
.quick-command-footer--readonly { justify-content: space-between; }
.quick-command-limit { color: var(--muted-foreground); font-size: 10px; white-space: nowrap; }
</style>
