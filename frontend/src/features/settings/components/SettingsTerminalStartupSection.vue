<script setup lang="ts">
// 设置·终端分栏的 shell 集成/启动段（从 SettingsTerminalPane 拆出）：连接级
// 启动命令与 SFTP 文件名编码。由连接级组合式（useConnectionSettings）自治
// 读写 sidecar 偏好；父组件仅在存在 SSH 会话（connectionId）时渲染本段。
import { ArrowDown, ArrowUp, Plus, Trash2 } from "@lucide/vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { Switch } from "../../../shared/ui/switch/index";
import {
  STARTUP_COMMAND_MAX,
  STARTUP_DELAY_MAX_MS,
} from "../../connections/lib/startupCommands";
import type { ConnectionNameEncodingChoice } from "../../connections/lib/connectionNameEncoding";
import type { ConnectionSettingsController } from "../composables/useConnectionSettings";

defineProps<{
  /** 连接级启动命令与文件名编码（组合式持有，自治 RPC）。 */
  startup: ConnectionSettingsController;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

/** 文本输入框取原始串（避免在模板里写 as 断言）。 */
function textFieldValue(event: Event): string {
  return (event.target as HTMLInputElement).value;
}
</script>

<template>
  <!-- 启动命令（对标 Tabby「Login scripts」）：连接建立进入 shell 后按序
       自动键入；仅 SSH 交互 shell 会话生效（RemoteCommand exec 会话跳过）。 -->
  <h3 class="settings-section-title">{{ t("startupCommands.sectionTitle") }}</h3>
  <label class="settings-field settings-switch-row">
    <Switch :model-value="startup.state.startupEnabled" size="sm" @update:model-value="(v) => startup.setStartupEnabled(Boolean(v))" />
    <span>{{ t("startupCommands.enabled") }}</span>
  </label>
  <p class="muted settings-note">{{ t("startupCommands.enabledHint") }}</p>
  <template v-if="startup.state.startupEnabled">
    <p v-if="!startup.state.startupCommands.length" class="muted settings-note">{{ t("startupCommands.empty") }}</p>
    <ul v-else class="settings-list startup-commands-list">
      <li v-for="(entry, index) in startup.state.startupCommands" :key="index">
        <input
          type="checkbox"
          :checked="entry.enabled"
          :aria-label="t('startupCommands.rowEnabled')"
          :title="t('startupCommands.rowEnabled')"
          @change="startup.updateStartupEntry(index, { enabled: ($event.target as HTMLInputElement).checked })"
        />
        <input
          class="mono startup-command-text"
          spellcheck="false"
          :value="entry.command"
          :placeholder="t('startupCommands.commandPlaceholder')"
          :aria-label="t('startupCommands.commandPlaceholder')"
          @change="startup.updateStartupEntry(index, { command: textFieldValue($event) })"
        />
        <input
          class="mono startup-command-delay"
          type="number"
          min="0"
          :max="STARTUP_DELAY_MAX_MS"
          step="50"
          :value="entry.delayMs"
          :aria-label="t('startupCommands.delay')"
          :title="t('startupCommands.delay')"
          @change="startup.updateStartupEntry(index, { delayMs: startup.clampStartupDelayInput(($event.target as HTMLInputElement).value) })"
        />
        <span class="settings-list-actions">
          <button class="icon-button" :disabled="index === 0" :title="t('startupCommands.moveUp')" :aria-label="t('startupCommands.moveUp')" @click="startup.moveStartupEntry(index, -1)"><ArrowUp /></button>
          <button class="icon-button" :disabled="index === startup.state.startupCommands.length - 1" :title="t('startupCommands.moveDown')" :aria-label="t('startupCommands.moveDown')" @click="startup.moveStartupEntry(index, 1)"><ArrowDown /></button>
          <button class="icon-button" :title="t('startupCommands.remove')" :aria-label="t('startupCommands.remove')" @click="startup.removeStartupEntry(index)"><Trash2 /></button>
        </span>
      </li>
    </ul>
    <button v-if="startup.state.startupCommands.length < STARTUP_COMMAND_MAX" class="link-button" type="button" @click="startup.addStartupEntry"><Plus />{{ t("startupCommands.add") }}</button>
    <p class="muted settings-note">{{ t("startupCommands.scopeNote") }}</p>
  </template>

  <!-- 连接级 SFTP 文件名编码（M16）：覆盖全局 `sftp_name_encoding`；
       缺省「跟随全局」（不落盘），白名单 auto/latin-1 与全局一致。 -->
  <h3 class="settings-section-title">{{ t("connNameEncoding.sectionTitle") }}</h3>
  <label class="settings-field">
    <span>{{ t("connNameEncoding.label") }}</span>
    <Select :model-value="startup.state.nameEncoding" @update:model-value="(v) => startup.setConnNameEncoding(String(v) as ConnectionNameEncodingChoice)">
      <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
      <SelectContent>
        <SelectItem value="follow">{{ t("connNameEncoding.follow") }}</SelectItem>
        <SelectItem value="auto">{{ t("transferCfg.encoding.auto") }}</SelectItem>
        <SelectItem value="latin-1">{{ t("transferCfg.encoding.latin-1") }}</SelectItem>
      </SelectContent>
    </Select>
  </label>
  <p class="muted settings-note">{{ t("connNameEncoding.hint") }}</p>
</template>
