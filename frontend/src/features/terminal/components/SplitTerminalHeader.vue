<script setup lang="ts">
/**
 * Chrome header for one split/tab terminal pane.
 *
 * The parent owns the session state and the close action; this component only
 * renders the thin title bar (icon, title, connecting/error status, close
 * button) so its scoped styles travel with the markup.
 */
import { Loader2, Terminal as TerminalIcon, X } from "@lucide/vue";
import type { SplitTerminalStatus } from "../composables/useSplitTerminal";

interface Props {
  status: SplitTerminalStatus;
  errorMessage: string;
  closeLabel: string;
  t: (key: string, values?: Record<string, string | number>) => string;
}

defineProps<Props>();

const emit = defineEmits<{
  close: [];
}>();
</script>

<template>
  <header class="split-terminal-header">
    <span class="split-terminal-title">
      <TerminalIcon aria-hidden="true" />
      <span class="split-terminal-title-text">{{ t("terminalSplit.title") }}</span>
    </span>
    <span class="split-terminal-spacer" aria-hidden="true" />
    <span v-if="status === 'connecting'" class="split-terminal-status" role="status">
      <Loader2 class="spinning" aria-hidden="true" />
      <span>{{ t("sessionStatus.connecting") }}</span>
    </span>
    <span v-else-if="status === 'error'" class="split-terminal-status is-error" :title="errorMessage">
      {{ errorMessage }}
    </span>
    <button
      type="button"
      class="split-terminal-close"
      :title="closeLabel"
      :aria-label="closeLabel"
      @click="emit('close')"
    >
      <X aria-hidden="true" />
    </button>
  </header>
</template>

<style scoped>
.split-terminal-header {
  display: flex;
  height: 24px;
  flex: 0 0 24px;
  align-items: center;
  gap: 6px;
  border-bottom: 1px solid var(--border);
  padding: 0 4px 0 8px;
  background: var(--background);
  color: var(--muted-foreground);
  font-size: 11px;
  line-height: 1;
  user-select: none;
}

.split-terminal-title {
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 5px;
  overflow: hidden;
  font-weight: 600;
  white-space: nowrap;
}

.split-terminal-title-text {
  overflow: hidden;
  text-overflow: ellipsis;
}

.split-terminal-title svg {
  width: 12px;
  height: 12px;
  flex: 0 0 auto;
  stroke-width: 1.8;
}

.split-terminal-spacer {
  flex: 1 1 auto;
}

.split-terminal-status {
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 4px;
  overflow: hidden;
  color: var(--muted-foreground);
  font-size: 10.5px;
  white-space: nowrap;
}

.split-terminal-status > span {
  overflow: hidden;
  text-overflow: ellipsis;
}

.split-terminal-status.is-error {
  color: var(--destructive);
}

.split-terminal-status svg {
  width: 11px;
  height: 11px;
  flex: 0 0 auto;
}

.split-terminal-close {
  display: inline-grid;
  width: 18px;
  height: 18px;
  flex: 0 0 18px;
  place-items: center;
  border: 0;
  border-radius: 4px;
  padding: 0;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
}

.split-terminal-close:hover {
  background: var(--accent);
  color: var(--accent-foreground);
}

.split-terminal-close svg {
  width: 12px;
  height: 12px;
  stroke-width: 1.8;
}
</style>
