<script setup lang="ts">
// 设置·终端分栏的渲染段（从 SettingsTerminalPane 拆出）：WebGL 渲染开关与
// 回滚行数。两者同样只上抛增量，归一化/持久化/落地在 App，本组件不持副本。
import { Switch } from "../../../shared/ui/switch/index";
import {
  SCROLLBACK_MAX,
  SCROLLBACK_MIN,
  type TerminalBehaviorSettings,
} from "../../terminal/lib/appearance/terminalBehavior";

defineProps<{
  webglEnabled: boolean;
  terminalBehavior: TerminalBehaviorSettings;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "set-webgl", value: boolean): void;
  (e: "update-behavior", patch: Partial<TerminalBehaviorSettings>): void;
}>();

function updateBehavior(patch: Partial<TerminalBehaviorSettings>) {
  emit("update-behavior", patch);
}

/** 数字输入框 → 行为字段：空串或非数值直接忽略，避免清空输入框把值打成 NaN。 */
function updateScrollback(raw: string) {
  const trimmed = raw.trim();
  if (!trimmed) return;
  const value = Number(trimmed);
  if (!Number.isFinite(value)) return;
  updateBehavior({ scrollbackLines: value });
}

/** 数值输入框取原始串。 */
function numberFieldValue(event: Event): string {
  return (event.target as HTMLInputElement).value;
}
</script>

<template>
  <h3 class="settings-section-title">{{ t("terminalBehavior.renderingSection") }}</h3>
  <label class="settings-field settings-switch-row">
    <Switch size="sm" :model-value="webglEnabled" @update:model-value="(value) => emit('set-webgl', value === true)" />
    <span>{{ t("webglLabel") }}</span>
  </label>
  <p class="muted settings-note">{{ t("webglHint") }}</p>
  <label class="settings-field">
    <span>{{ t("terminalBehavior.scrollbackLines") }}</span>
    <input type="number" :min="SCROLLBACK_MIN" :max="SCROLLBACK_MAX" step="100" :value="terminalBehavior.scrollbackLines" @change="updateScrollback(numberFieldValue($event))" />
  </label>
  <p class="muted settings-note">{{ t("terminalBehavior.scrollbackHint") }}</p>
</template>
