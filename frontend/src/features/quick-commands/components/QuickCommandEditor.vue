<script setup lang="ts">
// 快速命令编辑器（新建/编辑共用）：名称 + 分组选择 + 命令正文。
// 只维护草稿与校验，保存意图经 save 上抛（数据面在 App）。
import { computed, reactive, ref } from "vue";
import { ArrowLeft } from "@lucide/vue";
import {
  groupQuickCommands,
  QUICK_COMMAND_GROUP_NAME_MAX_LENGTH,
  QUICK_COMMAND_NAME_MAX_LENGTH,
  QUICK_COMMAND_TEXT_MAX_LENGTH,
  type QuickCommand,
  type QuickCommandGroup,
} from "../lib/quickCommands";
import { randomUUID } from "../../../shared/lib/uuid";

const props = defineProps<{
  commands: QuickCommand[];
  groups: QuickCommandGroup[];
  saving: boolean;
  limit: number;
  /** 打开时的种子：编辑既有条目，或从分组菜单新建时预选分组。 */
  initial: { item?: QuickCommand; groupId?: string };
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "save", command: { id?: string; name: string; command: string; groupId?: string; groupName?: string }): void;
  (e: "cancel"): void;
}>();

const GROUP_CHOICE_UNGROUPED = "";
const GROUP_CHOICE_NEW = "__new__";
const groupChoice = ref(props.initial.item?.groupId ?? props.initial.groupId ?? GROUP_CHOICE_UNGROUPED);
const newGroupName = ref("");
const draft = reactive({
  id: props.initial.item?.id,
  name: props.initial.item?.name ?? "",
  command: props.initial.item?.command ?? "",
});

const realGroups = computed(() => groupQuickCommands(props.commands, props.groups).filter((group) => group.id));

function saveCommand() {
  const command = draft.command.trim();
  if (!command || props.saving) return;
  if (!draft.id && props.commands.length >= props.limit) return;
  let groupId: string | undefined;
  let groupName: string | undefined;
  if (groupChoice.value === GROUP_CHOICE_NEW) {
    const name = newGroupName.value.trim();
    if (name) {
      groupId = randomUUID();
      groupName = name;
    }
  } else if (groupChoice.value) {
    groupId = groupChoice.value;
    groupName = realGroups.value.find((group) => group.id === groupChoice.value)?.name || groupChoice.value;
  }
  emit("save", { id: draft.id, name: draft.name.trim(), command, groupId, groupName });
}
</script>

<template>
  <header class="qc-view-head">
    <button class="qc-back" :title="t('cancel')" :aria-label="t('cancel')" @click="emit('cancel')"><ArrowLeft /></button>
    <div class="qc-view-title">
      <h3>{{ draft.id ? t("quickCommandsEdit") : t("quickCommandsNewCommand") }}</h3>
      <p>{{ t("quickCommandsEditorHint") }}</p>
    </div>
  </header>
  <div class="quick-command-editor qc-form">
    <label class="qc-field">
      <span>{{ t("quickCommandsName") }}</span>
      <input v-model="draft.name" :placeholder="t('quickCommandsName')" :maxlength="QUICK_COMMAND_NAME_MAX_LENGTH" autofocus />
    </label>
    <label class="qc-field">
      <span>{{ t("quickCommandsGroup") }}</span>
      <select v-model="groupChoice" class="qc-select">
        <option :value="GROUP_CHOICE_UNGROUPED">{{ t("quickCommandsUngrouped") }}</option>
        <option v-for="group in realGroups" :key="group.id" :value="group.id">{{ group.name }}</option>
        <option :value="GROUP_CHOICE_NEW">{{ t("quickCommandsGroupNew") }}</option>
      </select>
    </label>
    <label v-if="groupChoice === GROUP_CHOICE_NEW" class="qc-field">
      <span>{{ t("quickCommandsGroupName") }}</span>
      <input v-model="newGroupName" :placeholder="t('quickCommandsGroupName')" :maxlength="QUICK_COMMAND_GROUP_NAME_MAX_LENGTH" />
    </label>
    <label class="qc-field qc-field-stack">
      <span>{{ t("quickCommandsCommand") }}</span>
      <textarea v-model="draft.command" class="mono" rows="5" :placeholder="t('quickCommandsCommand')" :maxlength="QUICK_COMMAND_TEXT_MAX_LENGTH" @keydown.ctrl.enter="saveCommand" />
    </label>
    <div class="quick-command-editor-actions">
      <button class="primary-button" :disabled="saving || !draft.command.trim() || (!draft.id && commands.length >= limit)" @click="saveCommand">{{ draft.id ? t("save") : t("quickCommandsAdd") }}</button>
      <button @click="emit('cancel')">{{ t("cancel") }}</button>
      <span class="qc-limit">{{ commands.length }} / {{ limit }}</span>
    </div>
  </div>
</template>

<style scoped>
.qc-back { display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; flex: 0 0 28px; border: 1px solid var(--border); border-radius: 8px; padding: 0; background: var(--background); color: var(--foreground); cursor: pointer; }
.qc-back:hover { background: var(--accent); }
.qc-back svg { width: 14px; height: 14px; }
.qc-view-head { display: flex; align-items: flex-start; gap: 8px; }
.qc-view-title { display: flex; min-width: 0; flex: 1; flex-direction: column; gap: 2px; }
.qc-view-title h3 { margin: 0; font-size: 13px; }
.qc-view-title p { margin: 0; color: var(--muted-foreground); font-size: 10.5px; line-height: 1.5; }
.qc-form { display: flex; flex-direction: column; gap: 5px; border: 1px solid var(--border); border-radius: 10px; padding: 12px; background: var(--card-surface); }
.qc-field { display: grid; grid-template-columns: 58px minmax(0, 1fr); align-items: center; gap: 8px; }
.qc-field > span { color: var(--muted-foreground); font-size: 11px; }
.qc-field-stack { grid-template-columns: minmax(0, 1fr); align-items: stretch; gap: 4px; }
.qc-field input, .qc-field textarea, .qc-select { width: 100%; border: 1px solid var(--border); border-radius: 7px; padding: 5px 8px; background: var(--background); color: var(--foreground); font-size: 12px; }
.qc-field input, .qc-select { height: 28px; }
.qc-field textarea { min-height: 104px; resize: vertical; line-height: 1.55; }
.qc-field input:focus, .qc-field textarea:focus, .qc-select:focus { border-color: color-mix(in srgb, var(--primary) 65%, var(--border)); outline: none; }
.quick-command-editor-actions { display: flex; align-items: center; gap: 6px; }
.qc-limit { margin-left: auto; flex: 0 0 auto; border-radius: 8px; padding: 2px 7px; background: var(--muted); color: var(--muted-foreground); font-size: 10px; line-height: 15px; }
</style>
