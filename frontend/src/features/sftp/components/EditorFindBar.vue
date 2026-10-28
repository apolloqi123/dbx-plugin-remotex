<script setup lang="ts">
// VS Code 风格的查找/替换浮层（纯展示 + 事件）：定位、匹配与编辑器交互
// 由 TextPreview 持有；这里只负责输入、开关与键盘转发。
import { ref } from "vue";
import {
  ArrowDown,
  ArrowUp,
  CaseSensitive,
  ChevronDown,
  ChevronRight,
  Regex,
  WholeWord,
  X,
} from "@lucide/vue";

const props = defineProps<{
  t: (key: string) => string;
  query: string;
  replacement: string;
  countLabel: string;
  invalidRegex: boolean;
  replaceOpen: boolean;
  caseSensitive: boolean;
  wholeWord: boolean;
  useRegex: boolean;
}>();

const emit = defineEmits<{
  "update:query": [value: string];
  "update:replacement": [value: string];
  "update:caseSensitive": [value: boolean];
  "update:wholeWord": [value: boolean];
  "update:useRegex": [value: boolean];
  next: [];
  prev: [];
  close: [];
  toggleReplace: [];
  replace: [];
  replaceAll: [];
}>();

const findInput = ref<HTMLInputElement>();
const replaceInput = ref<HTMLInputElement>();

function focusFind(): void {
  findInput.value?.focus();
  findInput.value?.select();
}

function focusReplace(): void {
  replaceInput.value?.focus();
}

defineExpose({ focusFind, focusReplace });

function inputValue(event: Event): string {
  return (event.target as HTMLInputElement).value;
}

function onFindKeydown(event: KeyboardEvent): void {
  if (event.key === "Enter") {
    event.preventDefault();
    if (event.shiftKey) emit("prev");
    else emit("next");
  } else if (event.key === "Escape") {
    event.preventDefault();
    emit("close");
  }
}
</script>

<template>
  <form
    class="editor-find"
    role="search"
    :aria-label="props.t('terminalSearch.fileTitle')"
    @submit.prevent="emit('next')"
    @keydown.esc.prevent="emit('close')"
  >
    <div class="editor-find-row">
      <button
        type="button"
        class="editor-find-icon editor-find-toggle"
        :class="{ 'is-active': props.replaceOpen }"
        :title="props.t('terminalSearch.toggleReplace')"
        :aria-label="props.t('terminalSearch.toggleReplace')"
        :aria-expanded="props.replaceOpen"
        @click="emit('toggleReplace')"
      >
        <ChevronDown v-if="props.replaceOpen" />
        <ChevronRight v-else />
      </button>
      <input
        ref="findInput"
        class="editor-find-input"
        :class="{ 'is-invalid': props.invalidRegex }"
        :value="props.query"
        :placeholder="props.t('terminalSearch.placeholder')"
        :aria-label="props.t('terminalSearch.fileTitle')"
        spellcheck="false"
        autocomplete="off"
        @input="emit('update:query', inputValue($event))"
        @keydown="onFindKeydown"
      />
      <span v-if="props.countLabel" class="editor-find-count">{{ props.countLabel }}</span>
      <button
        type="button"
        class="editor-find-icon"
        :class="{ 'is-active': props.caseSensitive }"
        :title="props.t('terminalSearch.caseSensitive')"
        :aria-label="props.t('terminalSearch.caseSensitive')"
        :aria-pressed="props.caseSensitive"
        @click="emit('update:caseSensitive', !props.caseSensitive)"
      >
        <CaseSensitive />
      </button>
      <button
        type="button"
        class="editor-find-icon"
        :class="{ 'is-active': props.wholeWord }"
        :title="props.t('terminalSearch.wholeWord')"
        :aria-label="props.t('terminalSearch.wholeWord')"
        :aria-pressed="props.wholeWord"
        @click="emit('update:wholeWord', !props.wholeWord)"
      >
        <WholeWord />
      </button>
      <button
        type="button"
        class="editor-find-icon"
        :class="{ 'is-active': props.useRegex }"
        :title="props.t('terminalSearch.regex')"
        :aria-label="props.t('terminalSearch.regex')"
        :aria-pressed="props.useRegex"
        @click="emit('update:useRegex', !props.useRegex)"
      >
        <Regex />
      </button>
      <button
        type="button"
        class="editor-find-icon"
        :title="props.t('terminalSearch.prev')"
        :aria-label="props.t('terminalSearch.prev')"
        @click="emit('prev')"
      >
        <ArrowUp />
      </button>
      <button
        type="button"
        class="editor-find-icon"
        :title="props.t('terminalSearch.next')"
        :aria-label="props.t('terminalSearch.next')"
        @click="emit('next')"
      >
        <ArrowDown />
      </button>
      <button
        type="button"
        class="editor-find-icon editor-find-close"
        :title="props.t('terminalSearch.close')"
        :aria-label="props.t('terminalSearch.close')"
        @click="emit('close')"
      >
        <X />
      </button>
    </div>
    <div v-if="props.replaceOpen" class="editor-find-row editor-find-replace">
      <span class="editor-find-spacer" />
      <input
        ref="replaceInput"
        class="editor-find-input"
        :value="props.replacement"
        :placeholder="props.t('terminalSearch.replacePlaceholder')"
        :aria-label="props.t('terminalSearch.replacePlaceholder')"
        spellcheck="false"
        autocomplete="off"
        @input="emit('update:replacement', inputValue($event))"
        @keydown.enter.prevent="emit('replace')"
        @keydown.esc.prevent="emit('close')"
      />
      <button type="button" class="editor-find-action" @click="emit('replace')">
        {{ props.t("terminalSearch.replace") }}
      </button>
      <button type="button" class="editor-find-action" @click="emit('replaceAll')">
        {{ props.t("terminalSearch.replaceAll") }}
      </button>
    </div>
  </form>
</template>

<style scoped>
.editor-find {
  position: absolute;
  top: 8px;
  right: 12px;
  z-index: 6;
  display: flex;
  width: min(360px, calc(100% - 24px));
  flex-direction: column;
  gap: 4px;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-control);
  padding: 4px 5px;
  background: var(--popover);
  box-shadow: var(--shadow-popover);
  color: var(--foreground);
  font-size: 11px;
}

.editor-find-row {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 2px;
}

.editor-find-input {
  min-width: 0;
  flex: 1 1 auto;
  height: 22px;
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  padding: 0 6px;
  background: var(--background);
  color: var(--foreground);
  font-size: 11px;
  outline: none;
}

.editor-find-input:focus {
  border-color: color-mix(in srgb, var(--primary) 55%, var(--border));
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--primary) 15%, transparent);
}

.editor-find-input.is-invalid {
  border-color: var(--destructive);
}

.editor-find-count {
  flex: 0 0 auto;
  min-width: 28px;
  padding: 0 3px;
  color: var(--muted-foreground);
  font-size: 10px;
  text-align: right;
  white-space: nowrap;
}

.editor-find-icon {
  display: inline-grid;
  width: 20px;
  height: 20px;
  flex: 0 0 20px;
  place-items: center;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  padding: 0;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
}

.editor-find-icon svg {
  width: 12px;
  height: 12px;
}

.editor-find-icon:hover {
  background: color-mix(in srgb, var(--foreground) 8%, transparent);
  color: var(--foreground);
}

.editor-find-icon.is-active {
  border-color: color-mix(in srgb, var(--primary) 45%, var(--border));
  background: color-mix(in srgb, var(--primary) 16%, transparent);
  color: var(--primary);
}

.editor-find-toggle {
  margin-right: 1px;
}

.editor-find-toggle.is-active {
  background: transparent;
  border-color: transparent;
  color: var(--foreground);
}

.editor-find-close:hover {
  background: var(--tone-rose-soft, color-mix(in srgb, var(--destructive) 15%, transparent));
  color: var(--destructive);
}

.editor-find-replace {
  padding-left: 23px;
}

.editor-find-spacer {
  flex: 0 0 0;
}

.editor-find-action {
  flex: 0 0 auto;
  height: 22px;
  border: 1px solid var(--card-border);
  border-radius: var(--radius-xs);
  padding: 0 7px;
  background: var(--card-surface-raised);
  color: var(--foreground);
  font-size: 11px;
  cursor: pointer;
}

.editor-find-action:hover {
  border-color: color-mix(in srgb, var(--primary) 40%, var(--card-border));
  color: var(--primary);
}
</style>
