<script setup lang="ts">
// 快速命令管理器的单实例上下文菜单：右键行、行内 ⋯、工具栏 ⋯ 共用。
// 固定定位渲染在组件所在弹层内（不进 body），因此仍处于设置弹窗的图层，
// 菜单项可点击且不被弹窗的 outside-dismiss 判定为外部。
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import {
  ClipboardPaste,
  Copy,
  FileDown,
  FilePlus2,
  FileUp,
  FolderPlus,
  Pencil,
  Play,
  Trash2,
} from "@lucide/vue";
import type {
  QuickCommandExportChannel,
  QuickCommandsMenuAction,
  QuickCommandsMenuTarget,
} from "../lib/quickCommandMenu";
import type { QuickCommand, QuickCommandGroupView } from "../lib/quickCommands";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  connected: boolean;
  hasCommands: boolean;
  selectedCommand?: QuickCommand;
  selectedGroup?: QuickCommandGroupView;
}>();

const emit = defineEmits<{
  (e: "action", action: QuickCommandsMenuAction): void;
}>();

const menu = ref<{ target: QuickCommandsMenuTarget; x: number; y: number } | null>(null);
const menuEl = ref<HTMLElement | null>(null);
const menuTrigger = ref<HTMLElement | null>(null);

const menuGroup = computed(() => (menu.value?.target.kind === "group" ? menu.value.target.group : undefined));
const menuCommand = computed(() => (menu.value?.target.kind === "command" ? menu.value.target.command : undefined));

async function open(target: QuickCommandsMenuTarget, x: number, y: number, trigger?: HTMLElement | null) {
  menuTrigger.value = trigger ?? null;
  menu.value = { target, x, y };
  // 先以点击点定位，渲染后按实际尺寸把菜单收回视口内。
  await nextTick();
  const rect = menuEl.value?.getBoundingClientRect();
  if (!rect || !menu.value) return;
  const margin = 8;
  const left = Math.min(menu.value.x, window.innerWidth - rect.width - margin);
  const top = Math.min(menu.value.y, window.innerHeight - rect.height - margin);
  menu.value = { ...menu.value, x: Math.max(margin, left), y: Math.max(margin, top) };
  await nextTick();
  menuEl.value?.querySelector<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')?.focus();
}

function close(restoreFocus = false) {
  menu.value = null;
  if (restoreFocus) menuTrigger.value?.focus();
  menuTrigger.value = null;
}

function act(action: QuickCommandsMenuAction) {
  close();
  emit("action", action);
}

type ExportAction =
  | { type: "export-group"; group: QuickCommandGroupView }
  | { type: "export-command"; command: QuickCommand }
  | { type: "export-current-command" }
  | { type: "export-current-group" }
  | { type: "export-all" };

function exportChannel(action: ExportAction, channel: QuickCommandExportChannel) {
  act({ ...action, channel });
}

function onMenuKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") {
    event.preventDefault();
    close(true);
    return;
  }
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
  event.preventDefault();
  const items = [...(menuEl.value?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? [])];
  if (!items.length) return;
  const index = items.indexOf(document.activeElement as HTMLButtonElement);
  const next = event.key === "ArrowDown" ? (index + 1) % items.length : (index - 1 + items.length) % items.length;
  items[next]?.focus();
}

function onDocumentPointerDown(event: PointerEvent) {
  if (!menu.value) return;
  const target = event.target as Element | null;
  if (target?.closest(".qc-context-menu")) return;
  close();
}

function onDocumentKeydown(event: KeyboardEvent) {
  if (event.key === "Escape" && menu.value) close(true);
}

function onWindowResize() {
  if (menu.value) close();
}

onMounted(() => {
  document.addEventListener("pointerdown", onDocumentPointerDown, true);
  document.addEventListener("keydown", onDocumentKeydown);
  window.addEventListener("resize", onWindowResize);
});
onBeforeUnmount(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown, true);
  document.removeEventListener("keydown", onDocumentKeydown);
  window.removeEventListener("resize", onWindowResize);
});

defineExpose({ open, close });
</script>

<template>
  <div
    v-if="menu"
    ref="menuEl"
    class="qc-context-menu"
    :style="{ left: `${menu.x}px`, top: `${menu.y}px` }"
    role="menu"
    :data-menu="menu.target.kind"
    @keydown="onMenuKeydown"
    @contextmenu.prevent
  >
    <template v-if="menu.target.kind === 'group'">
      <button class="qc-menu-item" role="menuitem" @click="act({ type: 'new-command', group: menuGroup })"><FilePlus2 />{{ t("quickCommandsNewCommand") }}</button>
      <button class="qc-menu-item" role="menuitem" @click="act({ type: 'new-group' })"><FolderPlus />{{ t("quickCommandsGroupNew") }}</button>
      <button class="qc-menu-item" role="menuitem" :disabled="!menuGroup?.id" @click="menuGroup && act({ type: 'rename-group', group: menuGroup })"><Pencil />{{ t("quickCommandsEdit") }}</button>
      <div class="qc-menu-sep" />
      <button class="qc-menu-item" role="menuitem" :disabled="!menuGroup?.commands.length" @click="menuGroup && exportChannel({ type: 'export-group', group: menuGroup }, 'clipboard')"><Copy />{{ t("quickCommandsExportGroupClipboard") }}</button>
      <button class="qc-menu-item" role="menuitem" :disabled="!menuGroup?.commands.length" @click="menuGroup && exportChannel({ type: 'export-group', group: menuGroup }, 'file')"><FileDown />{{ t("quickCommandsExportGroupFile") }}</button>
      <div class="qc-menu-sep" />
      <button class="qc-menu-item destructive" role="menuitem" :disabled="!menuGroup?.id" @click="menuGroup && act({ type: 'delete-group', group: menuGroup })"><Trash2 />{{ t("delete") }}</button>
    </template>
    <template v-else-if="menu.target.kind === 'command'">
      <button class="qc-menu-item" role="menuitem" :disabled="!connected" @click="menuCommand && act({ type: 'run', command: menuCommand })"><Play />{{ t("quickCommandRun") }}</button>
      <button class="qc-menu-item" role="menuitem" :disabled="!connected" @click="menuCommand && act({ type: 'paste', command: menuCommand })"><ClipboardPaste />{{ t("quickCommandPaste") }}</button>
      <button class="qc-menu-item" role="menuitem" @click="menuCommand && act({ type: 'copy', command: menuCommand })"><Copy />{{ t("quickCommandsCopy") }}</button>
      <div class="qc-menu-sep" />
      <button class="qc-menu-item" role="menuitem" @click="menuCommand && act({ type: 'edit', command: menuCommand })"><Pencil />{{ t("quickCommandsEdit") }}</button>
      <button class="qc-menu-item" role="menuitem" @click="menuCommand && exportChannel({ type: 'export-command', command: menuCommand }, 'clipboard')"><Copy />{{ t("quickCommandsExportCommandClipboard") }}</button>
      <button class="qc-menu-item" role="menuitem" @click="menuCommand && exportChannel({ type: 'export-command', command: menuCommand }, 'file')"><FileDown />{{ t("quickCommandsExportCommandFile") }}</button>
      <div class="qc-menu-sep" />
      <button class="qc-menu-item destructive" role="menuitem" @click="menuCommand && act({ type: 'delete-command', command: menuCommand })"><Trash2 />{{ t("delete") }}</button>
    </template>
    <template v-else>
      <button class="qc-menu-item" role="menuitem" @click="act({ type: 'import' })"><FileUp />{{ t("quickCommandsImportAction") }}</button>
      <div class="qc-menu-sep" />
      <button class="qc-menu-item" role="menuitem" :disabled="!selectedCommand" @click="exportChannel({ type: 'export-current-command' }, 'clipboard')">{{ t("quickCommandsExportCurrentCommandClipboard") }}</button>
      <button class="qc-menu-item" role="menuitem" :disabled="!selectedCommand" @click="exportChannel({ type: 'export-current-command' }, 'file')">{{ t("quickCommandsExportCurrentCommandFile") }}</button>
      <button class="qc-menu-item" role="menuitem" :disabled="!selectedGroup" @click="exportChannel({ type: 'export-current-group' }, 'clipboard')">{{ t("quickCommandsExportCurrentGroupClipboard") }}</button>
      <button class="qc-menu-item" role="menuitem" :disabled="!selectedGroup" @click="exportChannel({ type: 'export-current-group' }, 'file')">{{ t("quickCommandsExportCurrentGroupFile") }}</button>
      <div class="qc-menu-sep" />
      <button class="qc-menu-item" role="menuitem" :disabled="!hasCommands" @click="exportChannel({ type: 'export-all' }, 'clipboard')">{{ t("quickCommandsExportAllClipboard") }}</button>
      <button class="qc-menu-item" role="menuitem" :disabled="!hasCommands" @click="exportChannel({ type: 'export-all' }, 'file')">{{ t("quickCommandsExportAllFile") }}</button>
    </template>
  </div>
</template>

<style scoped>
.qc-context-menu { position: fixed; z-index: 80; display: flex; width: 232px; flex-direction: column; gap: 1px; border: 1px solid var(--border); border-radius: 10px; padding: 5px; background: var(--popover, var(--card-surface)); color: var(--foreground); box-shadow: 0 14px 38px color-mix(in srgb, #000 42%, transparent); }
.qc-menu-item { display: flex; align-items: center; gap: 8px; height: 30px; border: 0; border-radius: 7px; padding: 0 8px; background: transparent; color: var(--foreground); font-size: 12px; text-align: left; cursor: pointer; }
.qc-menu-item svg { width: 14px; height: 14px; flex: 0 0 14px; color: var(--muted-foreground); }
.qc-menu-item:hover:not(:disabled), .qc-menu-item:focus-visible { background: var(--accent); outline: none; }
.qc-menu-item:disabled { opacity: .42; cursor: default; }
.qc-menu-item.destructive { color: var(--destructive); }
.qc-menu-item.destructive svg { color: var(--destructive); }
.qc-menu-item.destructive:hover:not(:disabled), .qc-menu-item.destructive:focus-visible { background: color-mix(in srgb, var(--destructive) 14%, transparent); }
.qc-menu-sep { height: 1px; margin: 4px 6px; background: var(--border); }
</style>
