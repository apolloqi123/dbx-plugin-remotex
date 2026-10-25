<script setup lang="ts">
// 设置·配色方案分栏（对标 Tabby「Color scheme」页，从「外观」里拆出）：
// 主题快照、实时预览、深浅两槽配色方案、终端背景来源、自定义方案导入。
// 权威态在 App：本组件只读 props + 上抛意图（应用/保存/删除主题、导入/移除方案）。
import { computed, ref } from "vue";
import { Check, FolderOpen, RotateCcw, Trash2, Upload } from "@lucide/vue";
import TerminalAppearancePreview from "../../terminal/components/TerminalAppearancePreview.vue";
import TerminalSchemePicker from "../../terminal/components/TerminalSchemePicker.vue";
import {
  CUSTOM_SCHEME_LIMIT,
  TERMINAL_APPEARANCE_PRESETS,
  type TerminalAppearanceProfile,
  type TerminalAppearanceSettings,
  type TerminalAppearanceState,
} from "../../terminal/lib/appearance/terminalAppearance";
import {
  BUILTIN_TERMINAL_SCHEMES,
  parseSchemeImport,
  type TerminalColorScheme,
  type TerminalThemeLike,
} from "../../terminal/lib/appearance/terminalScheme";
import { confirmDialog } from "../../../shared/lib/confirmDialog";
import { settingsErrorOf } from "../lib/settingsModel";
import { useAppearancePreview } from "../composables/useAppearancePreview";

const props = defineProps<{
  /** 终端外观偏好（权威态在 App）。 */
  appearance: TerminalAppearanceState;
  /** 用户保存的主题快照（内置预设由 lib 常量提供）。 */
  customThemes: TerminalAppearanceProfile[];
  /** 当前配置命中的主题 id（null = 已改动，不再等于任何主题）。 */
  activeThemeId: string | null;
  /** 宿主派生的基础终端主题（未启用配色方案时的最终结果）。 */
  hostTheme: TerminalThemeLike;
  hostColorScheme: "light" | "dark";
  hostFontFamily: string;
  terminalFontSize: number;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "update-appearance", patch: Partial<TerminalAppearanceSettings>): void;
  (e: "apply-theme", theme: TerminalAppearanceProfile): void;
  (e: "save-theme", name: string): void;
  (e: "delete-theme", id: string): void;
  (e: "add-schemes", schemes: Array<Omit<TerminalColorScheme, "id" | "source">>): void;
  (e: "remove-scheme", id: string): void;
}>();

const { appearanceSettings, appearanceProfiles, previewTheme, previewOptions, previewFontFamily, previewContrast } = useAppearancePreview({
  appearance: () => props.appearance,
  customThemes: () => props.customThemes,
  hostTheme: () => props.hostTheme,
  hostColorScheme: () => props.hostColorScheme,
  hostFontFamily: () => props.hostFontFamily,
});

const allSchemes = computed<readonly TerminalColorScheme[]>(() => [...BUILTIN_TERMINAL_SCHEMES, ...props.appearance.customSchemes]);
const themeNameDraft = ref("");
const importText = ref("");
const importError = ref("");
const importOpen = ref(false);
const schemeFileInput = ref<HTMLInputElement>();

function updateAppearance(patch: Partial<TerminalAppearanceSettings>) {
  emit("update-appearance", patch);
}

function applyTheme(theme: TerminalAppearanceProfile) {
  emit("apply-theme", theme);
}

function resetAppearance() {
  // 复用「跟随 DBX 宿主」预设：设置重置与字体回跟随宿主走同一条链路，
  // 不另写一份默认值展开，避免两处默认值漂移。
  emit("apply-theme", TERMINAL_APPEARANCE_PRESETS[0]);
}

function saveCurrentTheme() {
  const name = themeNameDraft.value.trim();
  if (!name) return;
  emit("save-theme", name);
  themeNameDraft.value = "";
}

async function deleteTheme(theme: TerminalAppearanceProfile) {
  if (!(await confirmDialog(props.t("terminalAppearance.deleteThemeConfirm", { name: props.t(theme.name) })))) return;

  emit("delete-theme", theme.id);
}

async function removeCustomScheme(scheme: TerminalColorScheme) {
  if (!(await confirmDialog(props.t("terminalAppearance.importRemoveConfirm", { name: scheme.name })))) return;

  emit("remove-scheme", scheme.id);
}

// 导入：粘贴文本或选文件。解析是纯逻辑（terminalScheme.parseSchemeImport），
// 分配 id / 持久化 / 提示交给 App（权威态在 App）。
function importSchemesFromText(text: string, fallbackName: string) {
  const result = parseSchemeImport(text, fallbackName);
  if (!result.schemes.length) {
    importError.value = props.t("terminalAppearance.importEmpty");
    return;
  }
  importError.value = "";
  importText.value = "";
  emit("add-schemes", result.schemes);
}

function submitImport() {
  if (!importText.value.trim()) return;
  importSchemesFromText(importText.value, "Imported scheme");
}

function pickSchemeFile() {
  schemeFileInput.value?.click();
}

async function onSchemeFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  try {
    // iTerm2/.itermcolors 与 Windows Terminal 导出都是文本，File API 读原文即可，
    // 不依赖宿主 fileTransfer（web/docker 模式同样可用）。
    const text = await file.text();
    importSchemesFromText(text, file.name.replace(/\.[^.]+$/, ""));
  } catch (cause) {
    importError.value = settingsErrorOf(cause);
  } finally {
    // 清空 value：同一个文件连续选两次也要能再次触发 change。
    input.value = "";
  }
}

// 单选组在模板里迭代（写死 `as const` 字面量在模板表达式中不被支持）。
const SCHEME_SOURCES: Array<{ value: TerminalAppearanceSettings["schemeSource"]; label: string }> = [
  { value: "host", label: "terminalAppearance.modeFollowHost" },
  { value: "custom", label: "terminalAppearance.modeCustom" },
];
const BACKGROUND_SOURCES: Array<{ value: TerminalAppearanceSettings["backgroundSource"]; label: string }> = [
  { value: "scheme", label: "terminalAppearance.backgroundScheme" },
  { value: "host", label: "terminalAppearance.backgroundHost" },
];
</script>

<template>
  <div class="settings-pane">
    <h3 class="settings-section-title">{{ t("terminalAppearance.themeSection") }}</h3>
    <p class="muted settings-note">{{ t("terminalAppearance.themeHint") }}</p>
    <div class="theme-chips">
      <button
        v-for="theme in appearanceProfiles"
        :key="theme.id"
        type="button"
        class="theme-chip"
        :class="{ active: theme.id === activeThemeId }"
        :aria-pressed="theme.id === activeThemeId"
        @click="applyTheme(theme)"
      >
        <Check v-if="theme.id === activeThemeId" class="theme-chip-icon" aria-hidden="true" />
        <span class="theme-chip-label">{{ t(theme.name) }}</span>
        <span
          v-if="!theme.builtin"
          class="theme-chip-delete"
          role="button"
          tabindex="0"
          :title="t('terminalAppearance.deleteTheme')"
          :aria-label="t('terminalAppearance.deleteTheme')"
          @click.stop="deleteTheme(theme)"
          @keydown.enter.stop="deleteTheme(theme)"
        ><Trash2 /></span>
      </button>
      <span v-if="!activeThemeId" class="theme-chip theme-chip--dirty">{{ t("terminalAppearance.themeCustom") }}</span>
    </div>
    <div class="theme-save-row">
      <input v-model="themeNameDraft" spellcheck="false" :placeholder="t('terminalAppearance.themeNamePlaceholder')" />
      <button type="button" :disabled="!themeNameDraft.trim()" @click="saveCurrentTheme">{{ t("terminalAppearance.saveTheme") }}</button>
      <button type="button" @click="resetAppearance"><RotateCcw />{{ t("terminalAppearance.reset") }}</button>
    </div>

    <h3 class="settings-section-title">{{ t("terminalAppearance.previewTitle") }}</h3>
    <TerminalAppearancePreview
      :theme="previewTheme"
      :font-family="previewFontFamily"
      :font-size="terminalFontSize"
      :font-weight="previewOptions.fontWeight"
      :font-weight-bold="previewOptions.fontWeightBold"
      :line-height="previewOptions.lineHeight"
      :letter-spacing="previewOptions.letterSpacing"
      :cursor-style="previewOptions.cursorStyle"
      :cursor-blink="previewOptions.cursorBlink"
      :contrast-ratio="previewContrast"
      :t="t"
    />

    <h3 class="settings-section-title">{{ t("terminalAppearance.schemeSection") }}</h3>
    <p class="muted settings-note">{{ t("terminalAppearance.schemeHint") }}</p>
    <label v-for="option in SCHEME_SOURCES" :key="option.value" class="settings-field settings-radio-row">
      <input
        type="radio"
        name="terminal-scheme-source"
        :value="option.value"
        :checked="appearanceSettings.schemeSource === option.value"
        @change="updateAppearance({ schemeSource: option.value })"
      />
      <span>{{ t(option.label) }}</span>
    </label>
    <TerminalSchemePicker
      v-if="appearanceSettings.schemeSource === 'custom'"
      :dark-scheme-id="appearanceSettings.darkSchemeId"
      :light-scheme-id="appearanceSettings.lightSchemeId"
      :custom-schemes="appearance.customSchemes"
      :schemes="allSchemes"
      :host-follow-label="t('terminalAppearance.modeFollowHost')"
      :t="t"
      @pick="(payload) => updateAppearance(payload.slot === 'dark' ? { darkSchemeId: payload.id } : { lightSchemeId: payload.id })"
    />
    <template v-if="appearanceSettings.schemeSource === 'custom'">
      <h4 class="settings-section-title">{{ t("terminalAppearance.backgroundSection") }}</h4>
      <label v-for="option in BACKGROUND_SOURCES" :key="option.value" class="settings-field settings-radio-row">
        <input
          type="radio"
          name="terminal-background-source"
          :value="option.value"
          :checked="appearanceSettings.backgroundSource === option.value"
          @change="updateAppearance({ backgroundSource: option.value })"
        />
        <span>{{ t(option.label) }}</span>
      </label>
      <p class="muted settings-note">{{ t("terminalAppearance.backgroundHint") }}</p>
    </template>

    <h4 class="settings-section-title">{{ t("terminalAppearance.importSection") }}</h4>
    <p class="muted settings-note">
      {{ t("terminalAppearance.importHint") }}
      <button class="link-button" type="button" :aria-expanded="importOpen" @click="importOpen = !importOpen">{{ t("terminalAppearance.importAction") }}</button>
    </p>
    <template v-if="importOpen">
      <label class="settings-field">
        <span>{{ t("terminalAppearance.importSection") }}</span>
        <textarea v-model="importText" rows="4" class="mono" spellcheck="false" :placeholder="t('terminalAppearance.importPlaceholder')" />
      </label>
      <div class="appearance-actions">
        <button type="button" :disabled="!importText.trim()" @click="submitImport"><Upload />{{ t("terminalAppearance.importAction") }}</button>
        <button type="button" @click="pickSchemeFile"><FolderOpen />{{ t("downloadSettings.browse") }}</button>
        <input
          ref="schemeFileInput"
          type="file"
          class="hidden-file-input"
          accept=".itermcolors,.json,.xresources,.yaml,.yml,.conf,.txt"
          @change="onSchemeFile"
        />
      </div>
      <p v-if="importError" class="task-error" role="alert">{{ importError }}</p>
    </template>
    <ul v-if="appearance.customSchemes.length" class="settings-list">
      <li v-for="scheme in appearance.customSchemes" :key="scheme.id">
        <div class="settings-list-main">
          <strong>{{ scheme.name }}</strong>
          <span class="scheme-mini-swatch" aria-hidden="true"><i v-for="(color, index) in scheme.colors.slice(0, 16)" :key="index" :style="{ background: color }" /></span>
        </div>
        <button class="icon-button" :title="t('terminalAppearance.importRemove')" :aria-label="t('terminalAppearance.importRemove')" @click="removeCustomScheme(scheme)"><Trash2 /></button>
      </li>
    </ul>
    <p class="muted">{{ t("profilesLimit", { count: appearance.customSchemes.length, limit: CUSTOM_SCHEME_LIMIT }) }}</p>
  </div>
</template>
