<script setup lang="ts">
// 设置·外观分栏（对标 Tabby「Appearance」页）：字体与字号、字重/行高/字间距/
// 内边距、光标、渲染细项、背景图，末尾再放一次实时预览以便边调边看排版效果。
// 权威态在 App（外观偏好）/ 弹窗（字体控件草稿）：本组件只读 + 上抛意图。
import { ref } from "vue";
import { Trash2, Upload } from "@lucide/vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { Switch } from "../../../shared/ui/switch/index";
import TerminalAppearancePreview from "../../terminal/components/TerminalAppearancePreview.vue";
import {
  type TerminalAppearanceProfile,
  type TerminalAppearanceSettings,
  type TerminalAppearanceState,
  type TerminalCursorInactiveStyle,
  type TerminalCursorStyle,
} from "../../terminal/lib/terminalAppearance";
import type { TerminalThemeLike } from "../../terminal/lib/terminalScheme";
import { TERMINAL_FONT_MAX, TERMINAL_FONT_MIN } from "../../terminal/lib/terminalZoom";
import { useAppearancePreview } from "../composables/useAppearancePreview";
import type { TerminalFontControls } from "../composables/useTerminalFontControls";

const props = defineProps<{
  /** 终端外观偏好（权威态在 App）。 */
  appearance: TerminalAppearanceState;
  customThemes: TerminalAppearanceProfile[];
  hostTheme: TerminalThemeLike;
  hostColorScheme: "light" | "dark";
  hostFontFamily: string;
  terminalFontSize: number;
  /** 字体控件草稿（弹窗持有：初始化一次，应用/恢复默认后刷新）。 */
  font: TerminalFontControls;
  wallpaperEnabled: boolean;
  wallpaperOpacity: number;
  wallpaperSessionOnly: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "update-appearance", patch: Partial<TerminalAppearanceSettings>): void;
  (e: "set-wallpaper-enabled", value: boolean): void;
  (e: "set-wallpaper-opacity", value: number): void;
  (e: "set-wallpaper-image", image: { base64: string; mime: string }): void;
  (e: "clear-wallpaper"): void;
  (e: "error", cause: unknown): void;
}>();

const { appearanceSettings, previewTheme, previewOptions, previewFontFamily, previewContrast } = useAppearancePreview({
  appearance: () => props.appearance,
  customThemes: () => props.customThemes,
  hostTheme: () => props.hostTheme,
  hostColorScheme: () => props.hostColorScheme,
  hostFontFamily: () => props.hostFontFamily,
});

// 背景图上传（P2-9）：本地 FileReader 读成 data URL，拆出 mime + 纯 base64
// 交 App 走 sidecar 落盘（web/docker 失败时 App 降级会话内存态）。客户端先做
// 8MiB 上限与 accept 类型粗校验，权威校验在 sidecar（魔数 + 大小）。
const WALLPAPER_MAX_BYTES = 8 * 1024 * 1024;
const wallpaperFileInput = ref<HTMLInputElement>();
function onWallpaperFileChange(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file) return;
  if (file.size > WALLPAPER_MAX_BYTES) {
    emit("error", new Error(props.t("wallpaper.tooLarge")));
    return;
  }
  const reader = new FileReader();
  reader.onload = () => {
    const dataUrl = typeof reader.result === "string" ? reader.result : "";
    const match = /^data:(image\/(?:png|jpeg|webp));base64,(.+)$/.exec(dataUrl);
    if (!match) {
      emit("error", new Error(props.t("wallpaper.invalidImage")));
      return;
    }
    emit("set-wallpaper-image", { base64: match[2], mime: match[1] });
  };
  reader.onerror = () => emit("error", new Error(props.t("wallpaper.invalidImage")));
  reader.readAsDataURL(file);
}

function updateAppearance(patch: Partial<TerminalAppearanceSettings>) {
  emit("update-appearance", patch);
}

/** 数值型外观字段：留空 = 恢复默认（null 回落到 xterm 既有默认），非法输入不落地。 */
function updateAppearanceNumber(key: keyof TerminalAppearanceSettings, raw: string) {
  const trimmed = raw.trim();
  if (trimmed.length === 0) {
    updateAppearance({ [key]: null } as Partial<TerminalAppearanceSettings>);
    return;
  }
  const parsed = Number(trimmed);
  if (Number.isFinite(parsed)) updateAppearance({ [key]: parsed } as Partial<TerminalAppearanceSettings>);
}

// reka Select 不接受空串 value，字重的「默认」用哨兵值双向映射。
const FONT_WEIGHT_AUTO = "__auto__";
const FONT_WEIGHT_OPTIONS = ["300", "400", "500", "600", "700", "800", "900"];

function updateFontWeight(key: "fontWeight" | "fontWeightBold", value: unknown) {
  const next = String(value);
  updateAppearance({ [key]: next === FONT_WEIGHT_AUTO ? null : Number(next) });
}

const CURSOR_STYLES: readonly TerminalCursorStyle[] = ["bar", "block", "underline"];
const CURSOR_INACTIVE_STYLES: readonly TerminalCursorInactiveStyle[] = ["outline", "block", "bar", "underline", "none"];
const CURSOR_STYLE_LABELS: Record<TerminalCursorStyle, string> = {
  bar: "terminalAppearance.cursorBar",
  block: "terminalAppearance.cursorBlock",
  underline: "terminalAppearance.cursorUnderline",
};
const CURSOR_INACTIVE_LABELS: Record<TerminalCursorInactiveStyle, string> = {
  outline: "terminalAppearance.inactiveOutline",
  block: "terminalAppearance.inactiveBlock",
  bar: "terminalAppearance.inactiveBar",
  underline: "terminalAppearance.inactiveUnderline",
  none: "terminalAppearance.inactiveNone",
};

// reka Select 回传 AcceptableValue（含 null 与对象），这里只接受字符串形态。
function updateCursorStyle(value: unknown) {
  updateAppearance({ cursorStyle: String(value) as TerminalCursorStyle });
}

function updateCursorInactiveStyle(value: unknown) {
  updateAppearance({ cursorInactiveStyle: String(value) as TerminalCursorInactiveStyle });
}

/** 数值输入框取原始串（模板里避免写 as 断言，TS 模板表达式支持有限）。 */
function numberFieldValue(event: Event): string {
  return (event.target as HTMLInputElement).value;
}
</script>

<template>
  <div class="settings-pane">
    <h4 class="settings-section-title">{{ t("terminalAppearance.typographySection") }}</h4>
    <label class="settings-field">
      <span>{{ t("terminalFont.family") }}</span>
      <Select :model-value="font.state.familyChoice" @update:model-value="(v) => font.onTerminalFontFamilyChoice(String(v))">
        <SelectTrigger size="sm"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem :value="font.followHost">{{ t("terminalFont.followHost") }}</SelectItem>
          <SelectItem v-for="preset in font.presets" :key="preset.value" :value="preset.value" class="terminal-font-option" :style="{ fontFamily: preset.value }">{{ preset.label }}</SelectItem>
          <SelectItem :value="font.custom">{{ t("terminalFont.custom") }}</SelectItem>
        </SelectContent>
      </Select>
    </label>
    <label v-if="font.state.familyChoice === font.custom" class="settings-field">
      <span>{{ t("terminalFont.custom") }}</span>
      <input v-model="font.state.customDraft" class="mono" spellcheck="false" :placeholder="t('terminalFont.customPlaceholder')" @change="font.applyTerminalFontFromControls()" />
    </label>
    <label class="settings-field">
      <span>{{ t("terminalFont.size") }}</span>
      <input v-model="font.state.sizeDraft" type="number" :min="TERMINAL_FONT_MIN" :max="TERMINAL_FONT_MAX" step="1" @change="font.applyTerminalFontFromControls()" />
    </label>
    <div class="appearance-number-grid">
      <label class="settings-field">
        <span>{{ t("terminalAppearance.fontWeight") }}</span>
        <Select :model-value="appearanceSettings.fontWeight == null ? FONT_WEIGHT_AUTO : String(appearanceSettings.fontWeight)" @update:model-value="(v) => updateFontWeight('fontWeight', v)">
          <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem :value="FONT_WEIGHT_AUTO">{{ t("terminalAppearance.valueAuto") }}</SelectItem>
            <SelectItem v-for="weight in FONT_WEIGHT_OPTIONS" :key="weight" :value="weight">{{ weight }}</SelectItem>
          </SelectContent>
        </Select>
      </label>
      <label class="settings-field">
        <span>{{ t("terminalAppearance.fontWeightBold") }}</span>
        <Select :model-value="appearanceSettings.fontWeightBold == null ? FONT_WEIGHT_AUTO : String(appearanceSettings.fontWeightBold)" @update:model-value="(v) => updateFontWeight('fontWeightBold', v)">
          <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem :value="FONT_WEIGHT_AUTO">{{ t("terminalAppearance.valueAuto") }}</SelectItem>
            <SelectItem v-for="weight in FONT_WEIGHT_OPTIONS" :key="weight" :value="weight">{{ weight }}</SelectItem>
          </SelectContent>
        </Select>
      </label>
      <label class="settings-field">
        <span>{{ t("terminalAppearance.lineHeight") }}</span>
        <input type="number" min="1" max="3" step="0.05" :value="appearanceSettings.lineHeight ?? ''" :placeholder="t('terminalAppearance.valueAuto')" @change="updateAppearanceNumber('lineHeight', numberFieldValue($event))" />
      </label>
      <label class="settings-field">
        <span>{{ t("terminalAppearance.letterSpacing") }}</span>
        <input type="number" min="-5" max="10" step="1" :value="appearanceSettings.letterSpacing ?? ''" :placeholder="t('terminalAppearance.valueAuto')" @change="updateAppearanceNumber('letterSpacing', numberFieldValue($event))" />
      </label>
      <label class="settings-field">
        <span>{{ t("terminalAppearance.paddingX") }}</span>
        <input type="number" min="0" max="32" step="1" :value="appearanceSettings.paddingX ?? ''" :placeholder="t('terminalAppearance.valueAuto')" @change="updateAppearanceNumber('paddingX', numberFieldValue($event))" />
      </label>
      <label class="settings-field">
        <span>{{ t("terminalAppearance.paddingY") }}</span>
        <input type="number" min="0" max="32" step="1" :value="appearanceSettings.paddingY ?? ''" :placeholder="t('terminalAppearance.valueAuto')" @change="updateAppearanceNumber('paddingY', numberFieldValue($event))" />
      </label>
    </div>
    <p class="muted settings-note">{{ t("terminalAppearance.paddingHint") }}</p>

    <h4 class="settings-section-title">{{ t("terminalAppearance.cursorSection") }}</h4>
    <label class="settings-field">
      <span>{{ t("terminalAppearance.cursorStyle") }}</span>
      <Select :model-value="appearanceSettings.cursorStyle" @update:model-value="updateCursorStyle">
        <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="style in CURSOR_STYLES" :key="style" :value="style">{{ t(CURSOR_STYLE_LABELS[style]) }}</SelectItem>
        </SelectContent>
      </Select>
    </label>
    <label class="quick-sudo-control">
      <Switch size="sm" :model-value="appearanceSettings.cursorBlink" @update:model-value="(v) => updateAppearance({ cursorBlink: v === true })" />
      <span>{{ t("terminalAppearance.cursorBlink") }}</span>
    </label>
    <label class="settings-field">
      <span>{{ t("terminalAppearance.cursorInactive") }}</span>
      <Select :model-value="appearanceSettings.cursorInactiveStyle" @update:model-value="updateCursorInactiveStyle">
        <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="style in CURSOR_INACTIVE_STYLES" :key="style" :value="style">{{ t(CURSOR_INACTIVE_LABELS[style]) }}</SelectItem>
        </SelectContent>
      </Select>
    </label>

    <h4 class="settings-section-title">{{ t("terminalAppearance.renderSection") }}</h4>
    <label class="quick-sudo-control">
      <Switch size="sm" :model-value="appearanceSettings.drawBoldTextInBrightColors" @update:model-value="(v) => updateAppearance({ drawBoldTextInBrightColors: v === true })" />
      <span>{{ t("terminalAppearance.drawBoldInBright") }}</span>
    </label>
    <p class="muted settings-note">{{ t("terminalAppearance.drawBoldInBrightHint") }}</p>
    <label class="settings-field">
      <span>{{ t("terminalAppearance.minimumContrast") }}</span>
      <input type="number" min="1" max="21" step="0.5" :value="appearanceSettings.minimumContrastRatio" @change="updateAppearanceNumber('minimumContrastRatio', numberFieldValue($event))" />
    </label>
    <p class="muted settings-note">{{ t("terminalAppearance.minimumContrastHint") }}</p>

    <h4 class="settings-section-title">{{ t("wallpaper.sectionTitle") }}</h4>
    <label class="quick-sudo-control">
      <Switch size="sm" :model-value="wallpaperEnabled" @update:model-value="(v) => emit('set-wallpaper-enabled', v === true)" />
      <span>{{ t("wallpaper.enabled") }}</span>
    </label>
    <p class="muted settings-note">{{ t("wallpaper.enabledHint") }}</p>
    <template v-if="wallpaperEnabled">
      <div class="appearance-actions">
        <button type="button" @click="wallpaperFileInput?.click()"><Upload />{{ t("wallpaper.upload") }}</button>
        <button type="button" @click="emit('clear-wallpaper')"><Trash2 />{{ t("wallpaper.clear") }}</button>
      </div>
      <input ref="wallpaperFileInput" class="hidden-file-input" type="file" accept="image/png,image/jpeg,image/webp" @change="onWallpaperFileChange" />
      <label class="settings-field">
        <span>{{ t("wallpaper.opacity") }}</span>
        <input type="range" min="10" max="90" step="5" :value="wallpaperOpacity" @change="emit('set-wallpaper-opacity', Number(($event.target as HTMLInputElement).value))" />
      </label>
      <p class="muted settings-note">{{ t("wallpaper.limitHint") }}</p>
      <p v-if="wallpaperSessionOnly" class="muted settings-note">{{ t("wallpaper.sessionOnly") }}</p>
    </template>

    <h4 class="settings-section-title">{{ t("terminalAppearance.previewTitle") }}</h4>
    <!-- 与「配色方案」页共用同一个纯展示预览组件：两处 props 必须保持一致，
         它是无状态无 id 的纯 DOM 复刻，实例化两次没有额外副作用。 -->
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
  </div>
</template>
