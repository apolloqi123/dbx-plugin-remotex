<script setup lang="ts">
// 设置·sudo 分栏：凭据来源（全局配置档 / 本连接凭据）、内联配置档管理
// （列表 + 新增/编辑同表单状态，与独立 profiles 弹窗共用）与本连接凭据/流程。
// 会话级分类（仅 SSH 会话可见，门控在弹窗导航）；主「保存」串行提交未保存的
// profile 编辑并落库连接设置。
import { Loader2, Pencil, Trash2 } from "@lucide/vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { Switch } from "../../../shared/ui/switch/index";
import PasswordField from "../../../shared/components/PasswordField.vue";
import { profileSummary, type SudoProfilesController } from "../composables/useSudoProfiles";
import type { ConnectionSettingsDraft } from "../composables/useConnectionSettings";
import type { SshSettings } from "../lib/settingsModel";

const props = defineProps<{
  /** 全局 quick sudo 配置档状态与 CRUD（与独立配置档弹窗共用）。 */
  sudo: SudoProfilesController;
  /** 连接设置草稿（主保存链与 agent 分栏共用同一份）。 */
  draft: ConnectionSettingsDraft;
  meta: SshSettings | undefined;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

// reka Select 不接受空串 option value（空串 = 未选中占位）；空值选项用哨兵值双向映射。
const SELECT_EMPTY_SENTINEL = "__empty__";
</script>

<template>
  <div class="settings-pane">
    <label class="settings-field">
      <span>{{ t("settingsCredentialSource") }}</span>
      <span class="credential-source-row">
        <Select :model-value="draft.quickSudoProfileId || SELECT_EMPTY_SENTINEL" @update:model-value="(v) => (draft.quickSudoProfileId = v === SELECT_EMPTY_SENTINEL ? '' : String(v))">
          <SelectTrigger size="xs" class="credential-source-select">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem :value="SELECT_EMPTY_SENTINEL">{{ t("profileSourceConnection") }}</SelectItem>
            <SelectItem v-for="profile in sudo.state.profiles" :key="profile.id" :value="profile.id">{{ profile.name }}</SelectItem>
          </SelectContent>
        </Select>
        <!-- 内联管理入口：展开/收起下方配置档 section，不再跳独立弹窗（工具栏 KeyRound 仍保留独立弹窗）。 -->
        <button class="link-button" :aria-expanded="sudo.state.inlineOpen" @click="sudo.state.inlineOpen = !sudo.state.inlineOpen">{{ t("profilesManage") }}</button>
      </span>
    </label>
    <p v-if="sudo.boundProfile" class="muted settings-note">{{ t("profilesBoundSummary", { name: sudo.boundProfile.name }) }} · {{ profileSummary(t, sudo.boundProfile) }}</p>
    <!-- 内联 quick sudo 配置档管理：列表 + 新增/编辑同表单状态
         （sudoProfiles/profileDraft/... 与独立 profiles 弹窗共用），主「保存」串行提交。 -->
    <section v-if="sudo.state.inlineOpen" class="profiles-inline">
      <h3 class="settings-section-title">{{ t("profilesTitle") }}</h3>
      <p class="muted">{{ t("profilesHint") }}</p>
      <div v-if="sudo.state.loading && !sudo.state.profiles.length" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
      <div v-else-if="!sudo.state.profiles.length" class="empty compact">{{ t("profilesEmpty") }}</div>
      <ul v-else class="settings-list">
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
      <p class="muted">{{ t("profilesLimit", { count: sudo.state.profiles.length, limit: 20 }) }}</p>
      <button v-if="!sudo.state.editing" class="link-button" @click="sudo.startProfileCreate">{{ t("profilesAdd") }}</button>
      <template v-if="sudo.state.editing">
        <h4 class="settings-section-title">{{ sudo.state.draft.id ? t("profilesEdit") : t("profilesAdd") }}</h4>
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
              <SelectItem value="off">{{ t("flowOff") }}</SelectItem>
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
    </section>
    <label class="quick-sudo-control">
      <Switch v-model="draft.quickSudo" size="sm" />
      <span>{{ t("settingsQuickSudo") }}</span>
    </label>
    <template v-if="!sudo.boundProfile">
      <label class="settings-field">
        <span>{{ t("settingsSudoPassword") }}</span>
        <PasswordField v-model="draft.sudoPassword" :placeholder="meta?.sudoPasswordSet ? t('settingsConfigured') : t('settingsSudoPasswordPlaceholder')" :t="t" />
      </label>
      <label class="settings-field">
        <span>{{ t("settingsTotp") }}</span>
        <textarea v-model="draft.totpSecret" rows="2" spellcheck="false" :placeholder="meta?.totpConfigured ? t('settingsConfigured') : t('settingsTotpPlaceholder')" />
      </label>
      <label class="settings-field">
        <span>{{ t("settingsFlowMode") }}</span>
        <Select :model-value="draft.authFlowMode" @update:model-value="(v) => (draft.authFlowMode = String(v))">
          <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="off">{{ t("flowOff") }}</SelectItem>
            <SelectItem value="password_then_otp">{{ t("flowThenOtp") }}</SelectItem>
            <SelectItem value="password_plus_otp">{{ t("flowPlusOtp") }}</SelectItem>
            <SelectItem value="password_only">{{ t("flowOnly") }}</SelectItem>
          </SelectContent>
        </Select>
      </label>
      <p class="muted settings-note">{{ t("settingsFlowHint") }}</p>
      <label class="settings-field">
        <span>{{ t("settingsPasswordHint") }}</span>
        <input v-model="draft.passwordPromptHint" spellcheck="false" :placeholder="t('settingsHintPlaceholder')" />
      </label>
      <label class="settings-field">
        <span>{{ t("settingsTotpHint") }}</span>
        <input v-model="draft.totpPromptHint" spellcheck="false" :placeholder="t('settingsHintPlaceholder')" />
      </label>
      <label class="quick-sudo-control">
        <Switch v-model="draft.sudoUsePty" size="sm" />
        <span>{{ t("settingsUsePty") }}</span>
      </label>
    </template>
    <p v-if="sudo.state.error" class="task-error">{{ sudo.state.error }} <button class="link-button" @click="sudo.loadSudoProfiles">{{ t("refresh") }}</button></p>
    <p class="muted settings-note">{{ t("settingsNote") }}</p>
  </div>
</template>
