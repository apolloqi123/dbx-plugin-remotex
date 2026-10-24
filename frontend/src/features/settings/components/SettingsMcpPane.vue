<script setup lang="ts">
// 设置·MCP 分栏：读/传/下载限额（MiB）+ 权限档 + 连接作用域。草稿与
// mcp/settings/* RPC 在弹窗（主「保存」串行链调用 saveMcpSettings），
// 本组件只读草稿 + 上抛刷新意图。
import { Loader2 } from "@lucide/vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";

/** MCP 限额/权限草稿（弹窗主保存链读取同一份响应式对象）。 */
interface McpDraft {
  readMiB: string;
  uploadMiB: string;
  downloadMiB: string;
  permissionMode: string;
  connectionScope: string;
}

defineProps<{
  draft: McpDraft;
  loading: boolean;
  error: string;
  inputsValid: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "refresh"): void;
}>();
</script>

<template>
  <div class="settings-pane">
    <h3 class="settings-section-title">{{ t("mcpLimits.title") }}</h3>
    <div v-if="loading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
    <template v-else>
      <div class="mcp-limits">
        <label class="settings-field">
          <span>{{ t("mcpLimits.read") }}</span>
          <input v-model="draft.readMiB" type="number" min="1" step="1" inputmode="numeric" />
        </label>
        <label class="settings-field">
          <span>{{ t("mcpLimits.upload") }}</span>
          <input v-model="draft.uploadMiB" type="number" min="1" step="1" inputmode="numeric" />
        </label>
        <label class="settings-field">
          <span>{{ t("mcpLimits.download") }}</span>
          <input v-model="draft.downloadMiB" type="number" min="1" step="1" inputmode="numeric" />
        </label>
      </div>
      <label class="settings-field">
        <span>{{ t("mcpSettings.permissionMode") }}</span>
        <Select :model-value="draft.permissionMode" @update:model-value="(v) => (draft.permissionMode = String(v))">
          <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="autonomous">{{ t("mcpSettings.permissionModeAutonomous") }}</SelectItem>
            <SelectItem value="confirm">{{ t("mcpSettings.permissionModeConfirm") }}</SelectItem>
          </SelectContent>
        </Select>
      </label>
      <p v-if="draft.permissionMode === 'confirm'" class="muted settings-note">{{ t("mcpSettings.permissionModeConfirmHint") }}</p>
      <label class="settings-field">
        <span>{{ t("mcpSettings.connectionScope") }}</span>
        <textarea v-model="draft.connectionScope" rows="3" class="mono" spellcheck="false" :placeholder="t('mcpSettings.connectionScopeHint')" />
      </label>
      <p v-if="!inputsValid" class="task-error">{{ t("mcpLimits.invalid") }}</p>
      <p v-if="error" class="task-error">{{ error }} <button class="link-button" @click="emit('refresh')">{{ t("refresh") }}</button></p>
      <!-- 独立「保存」链接已并入底部主「保存」串行链（saveSettings）。 -->
    </template>
  </div>
</template>
