<script setup lang="ts">
// 设置·终端分栏的本地 Shell 段（从 SettingsTerminalPane 拆出）：下拉项来自
// local/shells/list（弹窗打开时拉一次，选项在弹窗持有）；localShell 偏好的
// 权威态在 App，这里经 set-local-shell 上抛。reka Select 不接受空串 value，
// 用哨兵表达「自动检测」。
import { computed } from "vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";

type LocalShellOption = { program: string; name: string; isDefault: boolean; isUserShell: boolean; injectable?: boolean };

const props = defineProps<{
  localShell: string;
  localShellOptions: LocalShellOption[];
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "set-local-shell", program: string): void;
}>();

const LOCAL_SHELL_AUTO = "__auto__";
const localShellModelValue = computed(() => (props.localShell ? props.localShell : LOCAL_SHELL_AUTO));
function resolveLocalShellValue(value: string): string {
  return value === LOCAL_SHELL_AUTO ? "" : value;
}
</script>

<template>
  <h3 class="settings-section-title">{{ t("terminalBehavior.localShellSection") }}</h3>
  <label class="settings-field">
    <span>{{ t("terminalBehavior.localShell") }}</span>
    <Select :model-value="localShellModelValue" @update:model-value="(value) => emit('set-local-shell', resolveLocalShellValue(String(value)))">
      <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
      <SelectContent>
        <SelectItem value="__auto__">{{ t("terminalBehavior.localShellAuto") }}</SelectItem>
        <SelectItem v-for="shell in localShellOptions" :key="shell.program" :value="shell.program" :title="shell.program">{{ shell.name === shell.program ? shell.name : `${shell.name} · ${shell.program}` }}</SelectItem>
      </SelectContent>
    </Select>
  </label>
  <p class="muted settings-note">{{ t("terminalBehavior.localShellHint") }}</p>
</template>
