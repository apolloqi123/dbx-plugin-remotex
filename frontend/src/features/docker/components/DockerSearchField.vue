<script setup lang="ts">
// 容器列表页头搜索框：只负责输入/清除的呈现，过滤逻辑留在 ContainerListPane
// （状态筛选 + 搜索在同一 computed 里叠加）。Esc 清空，非空时显示清除按钮。
import { Search, X } from "@lucide/vue";

defineProps<{
  modelValue: string;
  placeholder: string;
  clearLabel: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

function onInput(event: Event): void {
  emit("update:modelValue", (event.target as HTMLInputElement).value);
}

function clear(): void {
  emit("update:modelValue", "");
}
</script>

<template>
  <label class="docker-search">
    <Search class="docker-search-icon" aria-hidden="true" />
    <input
      :value="modelValue"
      type="text"
      class="docker-search-input"
      :placeholder="placeholder"
      :aria-label="placeholder"
      autocomplete="off"
      spellcheck="false"
      @input="onInput"
      @keydown.esc.stop.prevent="clear"
    />
    <button
      v-if="modelValue"
      type="button"
      class="docker-search-clear"
      :title="clearLabel"
      :aria-label="clearLabel"
      @click="clear"
    >
      <X />
    </button>
  </label>
</template>

<style scoped>
/* 与状态筛选同高（20px），窄列可收缩到 88px。 */
.docker-search {
  display: inline-flex;
  flex: 0 1 200px;
  min-width: 88px;
  align-items: center;
  gap: 4px;
  height: 20px;
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  padding: 0 4px 0 5px;
  background: var(--background);
  color: var(--muted-foreground);
}

/* 列表列过窄（拖窄/小窗分屏）时隐藏，优先保住页头其它按钮。 */
@container (max-width: 280px) {
  .docker-search {
    display: none;
  }
}

.docker-search:focus-within {
  border-color: color-mix(in srgb, var(--primary) 55%, var(--border));
}

.docker-search-icon {
  width: 11px;
  height: 11px;
  flex: 0 0 11px;
}

.docker-search-input {
  min-width: 0;
  flex: 1 1 auto;
  border: 0;
  padding: 0;
  background: transparent;
  color: var(--foreground);
  font: inherit;
  font-size: 10px;
  outline: none;
}

.docker-search-input::placeholder {
  color: var(--muted-foreground);
}

.docker-search-clear {
  display: inline-flex;
  width: 14px;
  height: 14px;
  flex: 0 0 14px;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 3px;
  padding: 0;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
}

.docker-search-clear:hover {
  background: var(--accent);
  color: var(--foreground);
}

.docker-search-clear svg {
  width: 9px;
  height: 9px;
}
</style>
