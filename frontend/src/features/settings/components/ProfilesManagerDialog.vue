<script setup lang="ts">
// quick sudo 配置档管理弹窗（工具栏 KeyRound 入口；与设置弹窗内联 section
// 共用同一份草稿状态与 CRUD）。
import { Loader2, Pencil, Plus, ShieldCheck, Trash2, X } from "@lucide/vue";
import { Dialog, DialogContent, DialogTitle } from "../../../shared/ui/dialog/index";
import PasswordField from "../../../shared/components/PasswordField.vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { Switch } from "../../../shared/ui/switch/index";
import { profileSummary, type SudoProfilesController } from "../composables/useSudoProfiles";

defineProps<{
  open: boolean;
  sudo: SudoProfilesController;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
}>();
</script>

<template>
  <Dialog :open="open" @update:open="(value) => emit('update:open', value)">
    <DialogContent class="modal settings-modal profiles-modal" @escape-key-down.prevent>
      <header><DialogTitle>{{ t("profilesTitle") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:open', false)"><X /></button></header>
      <div class="settings-body">
        <p class="muted">{{ t("profilesHint") }}</p>
        <div class="profiles-toolbar">
          <span class="profiles-count muted">{{ t("profilesLimit", { count: sudo.state.profiles.length, limit: 20 }) }}</span>
          <button v-if="!sudo.state.editing" class="primary-button profiles-add" @click="sudo.startProfileCreate"><Plus />{{ t("profilesAdd") }}</button>
        </div>
        <div class="profiles-content">
          <div v-if="sudo.state.loading && !sudo.state.profiles.length" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
          <div v-else-if="!sudo.state.profiles.length" class="profiles-empty"><ShieldCheck /><p>{{ t("profilesEmpty") }}</p></div>
          <ul v-else class="settings-list profiles-list">
            <li v-for="profile in sudo.state.profiles" :key="profile.id">
              <div class="settings-list-main">
                <strong>{{ profile.name }}</strong>
                <span class="muted">{{ profileSummary(t, profile) }}</span>
              </div>
              <span class="settings-list-actions">
                <button class="icon-button" :title="t('profilesEdit')" :aria-label="t('profilesEdit')" @click="sudo.startProfileEdit(profile)"><Pencil /></button>
                <button class="icon-button" :title="t('profilesDelete')" :aria-label="t('profilesDelete')" @click="sudo.removeProfile(profile)"><Trash2 /></button>
              </span>
            </li>
          </ul>
        </div>

        <template v-if="sudo.state.editing">
          <h3 class="settings-section-title">{{ sudo.state.draft.id ? t("profilesEdit") : t("profilesAdd") }}</h3>
          <label class="settings-field">
            <span>{{ t("profilesName") }}</span>
            <input v-model="sudo.state.draft.name" spellcheck="false" :placeholder="t('profilesNamePlaceholder')" />
          </label>
          <label class="settings-field">
            <span>{{ t("profilesPassword") }}</span>
            <PasswordField v-model="sudo.state.draft.sudoPassword" :placeholder="sudo.state.hadPassword ? t('profilesPasswordKeep') : t('settingsSudoPasswordPlaceholder')" :t="t" />
          </label>
          <label class="settings-field">
            <span>{{ t("settingsTotp") }}</span>
            <textarea v-model="sudo.state.draft.totpSecret" rows="2" spellcheck="false" :placeholder="sudo.state.hadTotp ? t('settingsConfigured') : t('settingsTotpPlaceholder')" />
          </label>
          <label class="settings-field">
            <span>{{ t("settingsFlowMode") }}</span>
            <Select :model-value="sudo.state.draft.authFlowMode" @update:model-value="(v) => (sudo.state.draft.authFlowMode = String(v))">
              <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="password_then_otp">{{ t("flowThenOtp") }}</SelectItem>
                <SelectItem value="password_plus_otp">{{ t("flowPlusOtp") }}</SelectItem>
                <SelectItem value="password_only">{{ t("flowOnly") }}</SelectItem>
              </SelectContent>
            </Select>
          </label>
          <label class="settings-field">
            <span>{{ t("settingsPasswordHint") }}</span>
            <input v-model="sudo.state.draft.passwordPromptHint" spellcheck="false" :placeholder="t('settingsHintPlaceholder')" />
          </label>
          <label class="settings-field">
            <span>{{ t("settingsTotpHint") }}</span>
            <input v-model="sudo.state.draft.totpPromptHint" spellcheck="false" :placeholder="t('settingsHintPlaceholder')" />
          </label>
          <label class="quick-sudo-control">
            <Switch v-model="sudo.state.draft.sudoUsePty" size="sm" />
            <span>{{ t("settingsUsePty") }}</span>
          </label>
          <p v-if="sudo.state.error" class="task-error">{{ sudo.state.error }}</p>
          <footer class="profiles-form-actions">
            <button @click="sudo.cancelProfileEdit">{{ t("cancel") }}</button>
            <button class="primary-button" :disabled="sudo.state.saving || !sudo.state.draft.name.trim()" @click="sudo.saveProfileDraft"><Loader2 v-if="sudo.state.saving" class="spinning" />{{ t("save") }}</button>
          </footer>
        </template>
      </div>
      <footer><button @click="emit('update:open', false)">{{ t("close") }}</button></footer>
    </DialogContent>
  </Dialog>
</template>
