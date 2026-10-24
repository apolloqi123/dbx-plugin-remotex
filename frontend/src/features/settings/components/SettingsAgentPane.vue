<script setup lang="ts">
// 设置·智能体分栏（会话级，仅 SSH 会话可见）：agent 终端同步档位 + 已记住的
// 免审批命令列表（可逐条移除）。草稿由弹窗持有，主「保存」统一落库。
import { computed } from "vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { AGENT_MODES } from "../../agent/lib/agentTerminal";
import type { ConnectionSettingsDraft } from "../composables/useConnectionSettings";

const props = defineProps<{
  draft: ConnectionSettingsDraft;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

// AI 终端同步模式下拉随档位变化的说明文案（off/auto/strict 三键 hint）。
const agentTerminalModeHint = computed(() => props.t(
  props.draft.agentTerminalMode === "auto" ? "agentTerminalAutoHint"
  : props.draft.agentTerminalMode === "strict" ? "agentTerminalStrictHint"
  : "agentTerminalOffHint",
));
</script>

<template>
  <div class="settings-pane">
    <h3 class="settings-section-title">{{ t("agentTerminalSection") }}</h3>
    <label class="settings-field">
      <span>{{ t("agentTerminalMode") }}</span>
      <Select :model-value="draft.agentTerminalMode" @update:model-value="(v) => (draft.agentTerminalMode = String(v))">
        <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="mode in AGENT_MODES" :key="mode" :value="mode">{{ t(`agentTerminal${mode === "off" ? "Off" : mode === "auto" ? "Auto" : "Strict"}`) }}</SelectItem>
        </SelectContent>
      </Select>
    </label>
    <p class="muted settings-note">{{ agentTerminalModeHint }}</p>
    <div class="settings-remembered">
      <h4 class="settings-section-title">{{ t("settingsRemembered.section") }}</h4>
      <label class="settings-field"><span>{{ t("settingsRemembered.label") }}</span></label>
      <p v-if="!draft.rememberedCommands.length" class="muted settings-note">{{ t("settingsRemembered.empty") }}</p>
      <ul v-else class="remembered-list">
        <li v-for="(line, index) in draft.rememberedCommands" :key="`${index}-${line}`" class="remembered-row">
          <code class="mono remembered-line">{{ line }}</code>
          <button class="link-button" type="button" @click="draft.rememberedCommands.splice(index, 1)">{{ t("settingsRemembered.remove") }}</button>
        </li>
      </ul>
      <p class="muted settings-note">{{ t("settingsRemembered.hint") }}</p>
    </div>
  </div>
</template>
