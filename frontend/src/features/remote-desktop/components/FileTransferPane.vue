<script setup lang="ts">
import { ref, watch } from "vue";
import { ArrowUp, File, Folder, Home, Loader2, RefreshCw } from "@lucide/vue";
import { formatBytes } from "../../../shared/lib/format";
import type { TransferEntry, TransferPaneState } from "../composables/useRemoteFileTransfer";

const props = defineProps<{
  t: (key: string, values?: Record<string, string | number>) => string;
  title: string;
  subtitle?: string;
  tone: "host" | "remote";
  pane: TransferPaneState;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  navigate: [path: string];
  up: [];
  enter: [entry: TransferEntry];
  toggle: [entry: TransferEntry];
}>();

const draft = ref(props.pane.path || "");
watch(
  () => props.pane.path,
  (path) => {
    draft.value = path;
  },
);

function openEntry(entry: TransferEntry) {
  if (props.disabled) return;
  if (entry.kind === "directory") emit("enter", entry);
}

function toggle(entry: TransferEntry) {
  if (props.disabled) return;
  emit("toggle", entry);
}
</script>

<template>
  <section class="file-transfer-pane" :class="`tone-${tone}`">
    <header class="pane-head">
      <span class="pane-title">{{ title }}</span>
      <span v-if="subtitle" class="pane-subtitle" :title="subtitle">{{ subtitle }}</span>
    </header>
    <div class="pane-path">
      <button
        type="button"
        class="icon-button pane-nav"
        :title="t('remoteFiles.up')"
        :aria-label="t('remoteFiles.up')"
        :disabled="disabled || !pane.parent"
        @click="emit('up')"
      ><ArrowUp :size="13" /></button>
      <button
        type="button"
        class="icon-button pane-nav"
        :title="t('remoteFiles.home')"
        :aria-label="t('remoteFiles.home')"
        :disabled="disabled || !pane.home"
        @click="pane.home && emit('navigate', pane.home)"
      ><Home :size="13" /></button>
      <button
        type="button"
        class="icon-button pane-nav"
        :title="t('remoteFiles.refresh')"
        :aria-label="t('remoteFiles.refresh')"
        :disabled="disabled || !pane.path"
        @click="pane.path && emit('navigate', pane.path)"
      ><RefreshCw :size="13" /></button>
      <input
        v-model="draft"
        class="mono pane-path-input"
        spellcheck="false"
        :disabled="disabled"
        :placeholder="t('remoteFiles.path')"
        @keydown.enter.prevent="emit('navigate', draft)"
      />
    </div>
    <div class="pane-list" role="listbox" aria-multiselectable="true">
      <div v-if="pane.loading" class="pane-empty">
        <Loader2 :size="14" class="pane-spin" />{{ t("remoteFiles.loading") }}
      </div>
      <div v-else-if="pane.error" class="pane-empty is-error">{{ pane.error }}</div>
      <div v-else-if="!pane.entries.length" class="pane-empty">{{ t("remoteFiles.empty") }}</div>
      <template v-else>
        <button
          v-for="entry in pane.entries"
          :key="entry.path"
          type="button"
          class="pane-row"
          :class="{ 'is-selected': pane.selected.includes(entry.path), 'is-hidden': entry.hidden, 'is-dir': entry.kind === 'directory' }"
          role="option"
          :aria-selected="pane.selected.includes(entry.path)"
          :disabled="disabled"
          @click="toggle(entry)"
          @dblclick="openEntry(entry)"
        >
          <Folder v-if="entry.kind === 'directory'" :size="14" class="pane-icon" aria-hidden="true" />
          <File v-else :size="14" class="pane-icon" aria-hidden="true" />
          <span class="pane-name" :title="entry.name">{{ entry.name }}</span>
          <span v-if="entry.kind !== 'directory'" class="pane-size mono">{{ formatBytes(entry.size) }}</span>
        </button>
      </template>
    </div>
    <footer class="pane-foot">
      <span v-if="pane.selected.length">{{ t("remoteFiles.selected", { count: pane.selected.length }) }}</span>
      <span v-else>{{ t("remoteFiles.entries", { count: pane.entries.length }) }}</span>
    </footer>
  </section>
</template>

<style scoped>
.file-transfer-pane {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--background);
}
.pane-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 10px 6px;
}
.pane-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--foreground);
}
.tone-remote .pane-title { color: var(--primary); }
.pane-subtitle {
  overflow: hidden;
  font-size: 10px;
  color: var(--muted-foreground);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pane-path {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 0 6px 6px;
}
.pane-nav { width: 22px; height: 22px; border-radius: 6px; }
.pane-path-input {
  flex: 1;
  min-width: 0;
  height: 24px;
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 0 7px;
  background: var(--surface);
  color: var(--foreground);
  font-size: 11px;
}
.pane-path-input:focus { border-color: color-mix(in srgb, var(--primary) 70%, var(--border)); outline: none; }
.pane-list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  border-top: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  padding: 3px;
}
.pane-row {
  display: flex;
  align-items: center;
  gap: 7px;
  width: 100%;
  border: 0;
  border-radius: 6px;
  padding: 4px 7px;
  background: transparent;
  color: var(--foreground);
  font-size: 11px;
  text-align: left;
  cursor: pointer;
}
.pane-row:hover:not(:disabled) { background: color-mix(in srgb, var(--foreground) 6%, transparent); }
.pane-row.is-selected { background: color-mix(in srgb, var(--primary) 14%, transparent); color: var(--primary); }
.pane-row.is-hidden { opacity: .55; }
.pane-row.is-dir .pane-name { font-weight: 550; }
.pane-icon { flex: 0 0 14px; color: var(--muted-foreground); }
.pane-row.is-dir .pane-icon { color: var(--warning, #d97706); }
.pane-row.is-selected .pane-icon { color: currentColor; }
.pane-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pane-size { flex: 0 0 auto; font-size: 10px; color: var(--muted-foreground); }
.pane-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  min-height: 120px;
  padding: 12px;
  color: var(--muted-foreground);
  font-size: 11px;
  text-align: center;
}
.pane-empty.is-error { color: var(--danger, #dc2626); }
.pane-spin { animation: pane-spin 1s linear infinite; }
@keyframes pane-spin { to { transform: rotate(360deg); } }
.pane-foot {
  display: flex;
  justify-content: flex-end;
  padding: 5px 10px;
  font-size: 10px;
  color: var(--muted-foreground);
}
</style>
