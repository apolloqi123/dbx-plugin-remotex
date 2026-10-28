<script setup lang="ts">
// 快速命令管理页（设置 · 快速命令）：对标命令管理器的树形管理视图。
// 分组行（空组可持久化）+ 命令行 + 搜索 + 右键上下文菜单 + 分组/命令/全部导出。
// 数据面（pluginStore + PTY 写入）仍在 App：本组件只维护视图态并经 emits 上抛。
import { computed, nextTick, ref, watch } from "vue";
import {
  ChevronDown,
  ChevronRight,
  Folder,
  FolderOpen,
  FolderPlus,
  MoreHorizontal,
  Play,
  Plus,
  Search,
  Terminal,
  X,
} from "@lucide/vue";
import {
  groupQuickCommands,
  quickCommandExportFileName,
  quickCommandGroupCount,
  QUICK_COMMAND_GROUP_NAME_MAX_LENGTH,
  serializeQuickCommands,
  type QuickCommand,
  type QuickCommandGroup,
  type QuickCommandGroupView,
} from "../lib/quickCommands";
import type { QuickCommandsMenuAction, QuickCommandsMenuTarget } from "../lib/quickCommandMenu";
import { confirmDialog } from "../../../shared/lib/confirmDialog";
import { writeClipboardText, type ClipboardDeps } from "../../../shared/lib/clipboardBridge";
import QuickCommandEditor from "./QuickCommandEditor.vue";
import QuickCommandImport from "./QuickCommandImport.vue";
import QuickCommandsMenu from "./QuickCommandsMenu.vue";

const props = defineProps<{
  commands: QuickCommand[];
  /** 显式分组定义（含空组）。 */
  groups: QuickCommandGroup[];
  /** 终端会话可执行：未连接时禁用执行/粘贴。 */
  connected: boolean;
  /** 单条 save 在途态（App 侧）。 */
  saving: boolean;
  /** 批量导入在途态（App 侧）。 */
  importing: boolean;
  limit: number;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "save", command: { id?: string; name: string; command: string; groupId?: string; groupName?: string }): void;
  (e: "delete", id: string): void;
  (e: "import", items: Array<{ name: string; command: string; groupName?: string }>): void;
  (e: "create-group", name: string): void;
  (e: "rename-group", payload: { groupId: string; name: string }): void;
  (e: "dissolve-group", groupId: string): void;
  (e: "run", item: QuickCommand): void;
  (e: "paste", item: QuickCommand): void;
  (e: "notice", message: string): void;
  (e: "error", cause: unknown): void;
}>();

const t = props.t;

// —— 树视图：分组派生 / 搜索 / 折叠 / 选中 ——
const groups = computed<QuickCommandGroupView[]>(() => groupQuickCommands(props.commands, props.groups));
const search = ref("");
const searching = computed(() => search.value.trim().length > 0);
const collapsedGroups = ref<string[]>([]);
const selected = ref<{ kind: "group" | "command"; id: string } | null>(null);

const selectedGroup = computed(() => (selected.value?.kind === "group" ? groups.value.find((group) => group.id === selected.value?.id) : undefined));
const selectedCommand = computed(() => (selected.value?.kind === "command" ? props.commands.find((item) => item.id === selected.value?.id) : undefined));

const summary = computed(() => t("quickCommandsGroupSummary", { groups: quickCommandGroupCount(props.commands, props.groups), count: props.commands.length }));

function matches(item: QuickCommand, needle: string): boolean {
  return (
    item.name.toLowerCase().includes(needle) ||
    item.command.toLowerCase().includes(needle) ||
    (item.groupName ?? "").toLowerCase().includes(needle)
  );
}

/** 搜索时按组名整体命中（组内命令全保留），否则保留名/命令命中的条目；空组仅在组名命中时显示。 */
const visibleGroups = computed<QuickCommandGroupView[]>(() => {
  const needle = search.value.trim().toLowerCase();
  if (!needle) return groups.value;
  return groups.value.flatMap((group) => {
    const label = group.id ? group.name : t("quickCommandsUngrouped");
    if (label.toLowerCase().includes(needle)) return [group];
    const commands = group.commands.filter((item) => matches(item, needle));
    return commands.length ? [{ ...group, commands }] : [];
  });
});

const hasRealGroups = computed(() => groups.value.some((group) => group.id));
const isEmpty = computed(() => !props.commands.length && !props.groups.length);

function isCollapsed(group: QuickCommandGroupView): boolean {
  return !searching.value && collapsedGroups.value.includes(group.id);
}

function toggleGroup(group: QuickCommandGroupView) {
  selected.value = { kind: "group", id: group.id };
  const index = collapsedGroups.value.indexOf(group.id);
  if (index >= 0) collapsedGroups.value.splice(index, 1);
  else collapsedGroups.value.push(group.id);
}

function groupLabel(group: QuickCommandGroupView): string {
  return group.id ? group.name : t("quickCommandsUngrouped");
}

const GROUP_TONES = ["ui-tone-emerald", "ui-tone-blue", "ui-tone-amber", "ui-tone-violet", "ui-tone-cyan", "ui-tone-rose"];
function groupTone(group: QuickCommandGroupView): string {
  if (!group.id) return "qc-tone-muted";
  // 未分组不占色位：第一个真实分组用绿色（对齐命令管理器截图的 common/k8s 配色）。
  const index = groups.value.filter((item) => item.id).findIndex((item) => item.id === group.id);
  return GROUP_TONES[Math.max(0, index) % GROUP_TONES.length];
}

// —— 上下文菜单：单实例子组件，右键 / 行内 ⋯ / 工具栏 ⋯ 共用 ——
const menuRef = ref<InstanceType<typeof QuickCommandsMenu> | null>(null);

function selectCommand(item: QuickCommand) {
  selected.value = { kind: "command", id: item.id };
}

function openGroupMenu(group: QuickCommandGroupView, event: MouseEvent) {
  selected.value = { kind: "group", id: group.id };
  void menuRef.value?.open({ kind: "group", group }, event.clientX, event.clientY);
}

function openCommandMenu(item: QuickCommand, event: MouseEvent) {
  selected.value = { kind: "command", id: item.id };
  void menuRef.value?.open({ kind: "command", command: item }, event.clientX, event.clientY);
}

function openMenuFromButton(target: QuickCommandsMenuTarget, event: MouseEvent) {
  event.stopPropagation();
  const button = event.currentTarget as HTMLElement;
  const rect = button.getBoundingClientRect();
  if (target.kind === "group") selected.value = { kind: "group", id: target.group.id };
  if (target.kind === "command") selected.value = { kind: "command", id: target.command.id };
  void menuRef.value?.open(target, rect.right - 4, rect.bottom + 4, button);
}

function openOverflowMenu(event: MouseEvent) {
  const button = event.currentTarget as HTMLElement;
  const rect = button.getBoundingClientRect();
  void menuRef.value?.open({ kind: "overflow" }, rect.right, rect.bottom + 4, button);
}

function onMenuAction(action: QuickCommandsMenuAction) {
  switch (action.type) {
    case "new-command":
      openNewCommand(action.group);
      break;
    case "new-group":
      openCreateGroup();
      break;
    case "rename-group":
      startGroupRename(action.group);
      break;
    case "export-group":
      void exportScope({ kind: "group", group: action.group }, action.channel);
      break;
    case "delete-group":
      void deleteGroup(action.group);
      break;
    case "run":
      runCommand(action.command);
      break;
    case "paste":
      pasteCommand(action.command);
      break;
    case "copy":
      void copyCommand(action.command);
      break;
    case "edit":
      openEditor(action.command);
      break;
    case "export-command":
      void exportScope({ kind: "command", item: action.command }, action.channel);
      break;
    case "delete-command":
      void deleteCommand(action.command);
      break;
    case "import":
      openImport();
      break;
    case "export-current-command":
      exportSelected("command", action.channel);
      break;
    case "export-current-group":
      exportSelected("group", action.channel);
      break;
    case "export-all":
      void exportScope({ kind: "all" }, action.channel);
      break;
  }
}

// —— 执行 / 粘贴 / 复制 ——
function runCommand(item: QuickCommand) {
  if (!props.connected) return;
  emit("run", item);
}

function pasteCommand(item: QuickCommand) {
  if (!props.connected) return;
  emit("paste", item);
}

function clipboardDeps(): ClipboardDeps {
  return {
    bridge: window.dbxPlugin?.clipboard ?? null,
    nativeClipboard: typeof navigator !== "undefined" ? navigator.clipboard ?? null : null,
  };
}

async function copyCommand(item: QuickCommand) {
  try {
    await writeClipboardText(item.command, clipboardDeps());
    emit("notice", t("quickSelect.copied"));
  } catch {
    emit("notice", t("jsonPreview.copyFailed"));
  }
}

// —— 导出：当前命令 / 当前分组 / 全部；剪贴板或宿主落盘 ——
type ExportScope = { kind: "all" } | { kind: "group"; group: QuickCommandGroupView } | { kind: "command"; item: QuickCommand };

function scopeCommands(scope: ExportScope): QuickCommand[] {
  if (scope.kind === "all") return [...props.commands];
  if (scope.kind === "command") return [scope.item];
  return props.commands.filter((item) => (item.groupId ?? "") === scope.group.id);
}

function scopeSlug(scope: ExportScope): string | undefined {
  if (scope.kind === "all") return undefined;
  if (scope.kind === "command") return scope.item.name;
  return scope.group.id ? scope.group.name : t("quickCommandsUngrouped");
}

async function exportScope(scope: ExportScope, channel: "clipboard" | "file") {
  const payload = serializeQuickCommands(scopeCommands(scope));
  if (channel === "clipboard") {
    try {
      await writeClipboardText(payload, clipboardDeps());
      emit("notice", t("quickCommandsExportCopied", { count: scopeCommands(scope).length }));
    } catch {
      emit("notice", t("jsonPreview.copyFailed"));
    }
    return;
  }
  const api = window.dbxPlugin;
  if (!api?.saveFile) {
    emit("notice", t("quickCommandsExportUnavailable"));
    return;
  }
  try {
    const saved = await api.saveFile(
      { fileName: quickCommandExportFileName(scopeSlug(scope)), contentType: "application/json" },
      payload,
    );
    if (saved) emit("notice", t("quickCommandsExportSaved", { path: saved.path }));
  } catch (cause) {
    emit("error", cause);
  }
}

/** 溢出菜单的「当前命令/当前分组」导出：选择缺失时按钮已禁用，这里再守一次。 */
function exportSelected(kind: "command" | "group", channel: "clipboard" | "file") {
  if (kind === "command") {
    if (selectedCommand.value) void exportScope({ kind: "command", item: selectedCommand.value }, channel);
    return;
  }
  if (selectedGroup.value) void exportScope({ kind: "group", group: selectedGroup.value }, channel);
}

// —— 新建分组（行内输入） ——
const creatingGroup = ref(false);
const newGroupName = ref("");
const newGroupError = ref("");
const newGroupInput = ref<HTMLInputElement | null>(null);

function openCreateGroup() {
  creatingGroup.value = true;
  newGroupName.value = "";
  newGroupError.value = "";
  void nextTick(() => newGroupInput.value?.focus());
}

function cancelCreateGroup() {
  creatingGroup.value = false;
  newGroupName.value = "";
  newGroupError.value = "";
}

function submitCreateGroup() {
  const name = newGroupName.value.trim();
  if (!name) return;
  if (props.groups.some((group) => group.name.trim().toLowerCase() === name.toLowerCase())) {
    newGroupError.value = t("quickCommandsGroupExists");
    return;
  }
  emit("create-group", name);
  cancelCreateGroup();
}

// —— 分组改名 / 删除 ——
const renamingGroupId = ref<string | null>(null);
const renameDraft = ref("");

function startGroupRename(group: QuickCommandGroupView) {
  if (!group.id) return;
  renamingGroupId.value = group.id;
  renameDraft.value = group.name;
}

function cancelGroupRename() {
  renamingGroupId.value = null;
  renameDraft.value = "";
}

function confirmGroupRename() {
  const groupId = renamingGroupId.value;
  const name = renameDraft.value.trim();
  if (!groupId || !name) return;
  emit("rename-group", { groupId, name });
  cancelGroupRename();
}

async function deleteGroup(group: QuickCommandGroupView) {
  if (!group.id) return;
  if (!(await confirmDialog(t("quickCommandsGroupDeleteConfirm", { name: group.name || group.id })))) return;
  emit("dissolve-group", group.id);
}

// —— 子视图（树 / 编辑器 / 导入） ——
type View = "tree" | "editor" | "import";
const view = ref<View>("tree");
const editorSeed = ref<{ item?: QuickCommand; groupId?: string }>({});

function openEditor(item?: QuickCommand, groupId?: string) {
  editorSeed.value = { item, groupId };
  view.value = "editor";
}

function openNewCommand(group?: QuickCommandGroupView) {
  openEditor(undefined, group?.id);
}

function closeEditor() {
  view.value = "tree";
}

function onEditorSave(command: { id?: string; name: string; command: string; groupId?: string; groupName?: string }) {
  awaitingSaveResult = true;
  emit("save", command);
}

async function deleteCommand(item: QuickCommand) {
  if (!(await confirmDialog(t("quickCommandDeleteConfirm", { name: item.name || item.command })))) return;
  emit("delete", item.id);
}

function openImport() {
  view.value = "import";
}

function closeImport() {
  view.value = "tree";
}

function onImportConfirmed(items: Array<{ name: string; command: string; groupName?: string }>) {
  awaitingImportResult = true;
  emit("import", items);
}

// 数据面在 App：保存/导入成功后 commands 引用整体替换，据此收口子视图。
let awaitingSaveResult = false;
let awaitingImportResult = false;
watch(
  () => props.commands,
  () => {
    if (awaitingSaveResult) {
      awaitingSaveResult = false;
      closeEditor();
    }
    if (awaitingImportResult) {
      awaitingImportResult = false;
      closeImport();
    }
  },
);

// 选中项被删除/解散后清空选择，避免「导出当前…」指向不存在的对象。
watch([() => props.commands, () => props.groups], () => {
  if (selected.value?.kind === "command" && !props.commands.some((item) => item.id === selected.value?.id)) selected.value = null;
  if (selected.value?.kind === "group" && selected.value.id && !groups.value.some((group) => group.id === selected.value?.id)) selected.value = null;
});
</script>

<template>
  <div class="quick-commands-section" :class="{ 'is-tree': view === 'tree' }">
    <QuickCommandImport
      v-if="view === 'import'"
      :commands="commands"
      :importing="importing"
      :t="t"
      @import="onImportConfirmed"
      @cancel="closeImport"
      @error="(cause) => emit('error', cause)"
    />

    <QuickCommandEditor
      v-else-if="view === 'editor'"
      :commands="commands"
      :groups="groups"
      :saving="saving"
      :limit="limit"
      :initial="editorSeed"
      :t="t"
      @save="onEditorSave"
      @cancel="closeEditor"
    />

    <template v-else>
      <header class="qc-head">
        <span class="qc-head-icon"><Terminal /></span>
        <div class="qc-head-text">
          <strong>{{ t("quickCommandsManagerTitle") }}</strong>
          <span>{{ summary }}</span>
        </div>
        <span class="qc-limit">{{ commands.length }} / {{ limit }}</span>
      </header>

      <div class="qc-toolbar">
        <div class="qc-search">
          <Search />
          <input v-model="search" :placeholder="t('quickCommandsSearch')" spellcheck="false" />
          <button v-if="search" class="qc-search-clear" :title="t('cancel')" :aria-label="t('cancel')" @click="search = ''"><X /></button>
        </div>
        <button class="qc-tool" :title="t('quickCommandsNewCommand')" :aria-label="t('quickCommandsNewCommand')" :disabled="commands.length >= limit" @click="openNewCommand()"><Plus /></button>
        <button class="qc-tool" :title="t('quickCommandsGroupNew')" :aria-label="t('quickCommandsGroupNew')" @click="openCreateGroup"><FolderPlus /></button>
        <button class="qc-tool" :aria-label="t('quickCommandsMore')" @click="openOverflowMenu"><MoreHorizontal /></button>
      </div>

      <div v-if="creatingGroup" class="qc-group-create">
        <FolderPlus />
        <input
          ref="newGroupInput"
          v-model="newGroupName"
          :placeholder="t('quickCommandsGroupName')"
          :maxlength="QUICK_COMMAND_GROUP_NAME_MAX_LENGTH"
          @keydown.enter="submitCreateGroup"
          @keydown.esc="cancelCreateGroup"
        />
        <button class="qc-mini primary" :disabled="!newGroupName.trim()" @click="submitCreateGroup">{{ t("create") }}</button>
        <button class="qc-mini" @click="cancelCreateGroup">{{ t("cancel") }}</button>
        <span v-if="newGroupError" class="qc-error">{{ newGroupError }}</span>
      </div>

      <div class="qc-tree">
        <div v-if="isEmpty" class="qc-empty">
          <span class="qc-empty-icon"><Terminal /></span>
          <strong>{{ t("quickCommandsEmpty") }}</strong>
          <p>{{ t("quickCommandsManagerHint") }}</p>
          <button class="primary-button" @click="openNewCommand()">{{ t("quickCommandsNewCommand") }}</button>
        </div>
        <div v-else-if="!visibleGroups.length" class="qc-empty compact">
          <Search />
          <span>{{ t("quickCommandsNoMatch") }}</span>
        </div>
        <template v-else>
          <section v-for="group in visibleGroups" :key="group.id || '__ungrouped__'" class="qc-group" :class="{ 'is-selected': selected?.kind === 'group' && selected.id === group.id }">
            <div
              class="qc-group-row"
              :class="[groupTone(group), { active: selected?.kind === 'group' && selected.id === group.id }]"
              @contextmenu.prevent="openGroupMenu(group, $event)"
            >
              <button class="qc-caret" :aria-expanded="!isCollapsed(group)" :aria-label="groupLabel(group)" @click.stop="toggleGroup(group)">
                <ChevronDown v-if="!isCollapsed(group)" />
                <ChevronRight v-else />
              </button>
              <button class="qc-group-main" @click="toggleGroup(group)">
                <FolderOpen v-if="!isCollapsed(group)" class="qc-folder" />
                <Folder v-else class="qc-folder" />
                <span class="qc-group-name">{{ groupLabel(group) }}</span>
                <span class="qc-group-count">({{ group.commands.length }})</span>
              </button>
              <button class="qc-row-more" :aria-label="t('quickCommandsMore')" @click="openMenuFromButton({ kind: 'group', group }, $event)"><MoreHorizontal /></button>
            </div>

            <div v-if="renamingGroupId === group.id" class="qc-group-rename">
              <input v-model="renameDraft" :maxlength="QUICK_COMMAND_GROUP_NAME_MAX_LENGTH" :placeholder="t('quickCommandsGroupName')" @keydown.enter="confirmGroupRename" @keydown.esc="cancelGroupRename" />
              <button class="qc-mini primary" :disabled="!renameDraft.trim()" @click="confirmGroupRename">{{ t("save") }}</button>
              <button class="qc-mini" @click="cancelGroupRename">{{ t("cancel") }}</button>
            </div>

            <div v-if="!isCollapsed(group)" class="qc-children">
              <p v-if="!group.commands.length" class="qc-group-empty">{{ t("quickCommandsGroupEmpty") }}</p>
              <div
                v-for="item in group.commands"
                :key="item.id"
                class="qc-command-row"
                :class="{ active: selected?.kind === 'command' && selected.id === item.id }"
                :title="`${item.name}\n${item.command}`"
                @click="selectCommand(item)"
                @dblclick="runCommand(item)"
                @contextmenu.prevent="openCommandMenu(item, $event)"
              >
                <span class="qc-command-text">
                  <strong>{{ item.name }}</strong>
                  <span class="mono">{{ item.command }}</span>
                </span>
                <span class="qc-command-actions">
                  <button class="qc-row-more" :title="t('quickCommandRun')" :aria-label="t('quickCommandRun')" :disabled="!connected" @click.stop="runCommand(item)"><Play /></button>
                  <button class="qc-row-more" :aria-label="t('quickCommandsMore')" @click="openMenuFromButton({ kind: 'command', command: item }, $event)"><MoreHorizontal /></button>
                </span>
              </div>
            </div>
          </section>
        </template>
      </div>

      <div class="quick-manage-actions qc-footer">
        <button class="link-button" type="button" :disabled="commands.length >= limit" @click="openNewCommand()">{{ t("quickCommandsNewCommand") }}</button>
        <button class="link-button" type="button" @click="openImport">{{ t("quickCommandsImport") }}</button>
        <span class="qc-footer-hint">{{ hasRealGroups ? t("quickCommandsManagerHint") : t("quickCommandsGlobalHint") }}</span>
      </div>
    </template>

    <QuickCommandsMenu
      ref="menuRef"
      :t="t"
      :connected="connected"
      :has-commands="commands.length > 0"
      :selected-command="selectedCommand"
      :selected-group="selectedGroup"
      @action="onMenuAction"
    />
  </div>
</template>

<style scoped src="./QuickCommandsSection.css"></style>
