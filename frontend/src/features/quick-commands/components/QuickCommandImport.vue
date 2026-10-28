<script setup lang="ts">
// 快速命令批量导入：文件/粘贴 JSON → 预览计数 → 上抛 accepted 条目。
import { computed, ref } from "vue";
import { ArrowLeft, FileUp } from "@lucide/vue";
import { mergeQuickCommandImport, parseQuickCommandImport } from "../lib/quickCommandImport";
import type { QuickCommand } from "../lib/quickCommands";

const props = defineProps<{
  commands: QuickCommand[];
  importing: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "import", items: Array<{ name: string; command: string; groupName?: string }>): void;
  (e: "cancel"): void;
  (e: "error", cause: unknown): void;
}>();

const importText = ref("");
const importFileName = ref("");
const importPlan = computed(() => parseQuickCommandImport(importText.value));
const importMerge = computed(() => mergeQuickCommandImport(props.commands, importPlan.value));

async function onImportFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file) return;
  try {
    importText.value = await file.text();
    importFileName.value = file.name;
  } catch (cause) {
    emit("error", cause);
  }
}

function confirmImport() {
  const merge = importMerge.value;
  if (!merge.accepted.length || props.importing) return;
  emit("import", merge.accepted);
}
</script>

<template>
  <header class="qc-view-head">
    <button class="qc-back" :title="t('cancel')" :aria-label="t('cancel')" @click="emit('cancel')"><ArrowLeft /></button>
    <div class="qc-view-title">
      <h3>{{ t("quickCommandsImport") }}</h3>
      <p>{{ t("quickCommandsImportPolicy") }}</p>
    </div>
  </header>
  <div class="quick-command-editor qc-form">
    <label class="quick-import-file">
      <FileUp />
      <span>{{ importFileName || t("quickCommandsImportFile") }}</span>
      <input type="file" accept=".json,application/json,text/plain" @change="onImportFile" />
    </label>
    <textarea v-model="importText" class="mono" rows="7" :placeholder="t('quickCommandsImportPlaceholder')" spellcheck="false" />
    <p v-if="importText.trim()" class="muted quick-import-summary">
      <template v-if="importPlan.invalid < 0">{{ t("quickCommandsImportInvalid") }}</template>
      <template v-else>{{ t("quickCommandsImportSummary", { accepted: importMerge.accepted.length, skipped: importMerge.skippedExisting, dup: importPlan.duplicates, invalid: importPlan.invalid, overflow: importMerge.overflow }) }}</template>
    </p>
    <div class="quick-command-editor-actions">
      <button class="primary-button" :disabled="importing || !importMerge.accepted.length" @click="confirmImport">{{ t("quickCommandsImportConfirm", { count: importMerge.accepted.length }) }}</button>
      <button @click="emit('cancel')">{{ t("cancel") }}</button>
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
.quick-import-file { display: flex; align-items: center; gap: 6px; height: 28px; border: 1px dashed var(--border); border-radius: 7px; padding: 0 8px; color: var(--muted-foreground); font-size: 11px; cursor: pointer; }
.quick-import-file:hover { border-color: color-mix(in srgb, var(--primary) 55%, var(--border)); background: var(--accent); }
.quick-import-file svg { width: 13px; height: 13px; flex: none; }
.quick-import-file span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.quick-import-file input[type="file"] { display: none; }
.quick-import-summary { margin: 0; font-size: 10.5px; line-height: 1.5; }
.quick-command-editor-actions { display: flex; align-items: center; gap: 6px; }
textarea { width: 100%; resize: vertical; border: 1px solid var(--border); border-radius: 7px; padding: 6px 8px; background: var(--background); color: var(--foreground); font-size: 12px; line-height: 1.5; }
</style>
