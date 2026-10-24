<script setup lang="ts">
// 设置·AI 助手分栏（Warp AI 对齐批）：四条体验开关 + 能力缺失横幅 + AI 修复
// 「不再询问」重置。权威态在 App（pluginStore 单键 JSON），本组件只读 + 上抛
// 增量；能力位来自宿主桥 capabilities.ai。
import { Switch } from "../../../shared/ui/switch/index";
import type { AiSettings } from "../../agent/lib/aiSettings";

defineProps<{
  aiSettings: AiSettings;
  aiBridgeReady: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "update-ai-settings", patch: Partial<AiSettings>): void;
  (e: "reset-ai-consent"): void;
}>();
</script>

<template>
  <div class="settings-pane">
    <h3 class="settings-section-title">{{ t("aiSettings.sectionTitle") }}</h3>
    <p v-if="!aiBridgeReady" class="muted settings-note" role="note">{{ t("aiSettings.capabilityMissing") }}</p>
    <label class="settings-field settings-switch-row">
      <Switch :model-value="aiSettings.search" size="sm" :disabled="!aiBridgeReady" @update:model-value="emit('update-ai-settings', { search: Boolean($event) })" />
      <span>{{ t("aiSettings.searchLabel") }}</span>
    </label>
    <p class="muted settings-note">{{ t("aiSettings.searchHint") }}</p>
    <label class="settings-field settings-switch-row">
      <Switch :model-value="aiSettings.fix" size="sm" :disabled="!aiBridgeReady" @update:model-value="emit('update-ai-settings', { fix: Boolean($event) })" />
      <span>{{ t("aiSettings.fixLabel") }}</span>
    </label>
    <p class="muted settings-note">{{ t("aiSettings.fixHint") }}</p>
    <label class="settings-field settings-switch-row">
      <Switch :model-value="aiSettings.assist" size="sm" :disabled="!aiBridgeReady" @update:model-value="emit('update-ai-settings', { assist: Boolean($event) })" />
      <span>{{ t("aiSettings.assistLabel") }}</span>
    </label>
    <p class="muted settings-note">{{ t("aiSettings.assistHint") }}</p>
    <label class="settings-field settings-switch-row">
      <Switch :model-value="aiSettings.agentMode" size="sm" :disabled="!aiBridgeReady" @update:model-value="emit('update-ai-settings', { agentMode: Boolean($event) })" />
      <span>{{ t("aiSettings.agentModeLabel") }}</span>
    </label>
    <p class="muted settings-note">{{ t("aiSettings.agentModeHint") }}</p>
    <button type="button" class="connect-card-ghost-button" :disabled="!aiSettings.fixConsent" @click="emit('reset-ai-consent')">{{ t("aiSettings.resetConsent") }}</button>
  </div>
</template>
