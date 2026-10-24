<script setup lang="ts">
// 设置·终端分栏的行为段（从 SettingsTerminalPane 拆出）：键盘、鼠标、剪贴板
// 与声音四组。行为设置只上抛增量：归一化、持久化与落地到 xterm 选项全在 App，
// 保证「权威态唯一」——本组件不持有行为设置的副本，重开弹窗也不会出现回显漂移。
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { Switch } from "../../../shared/ui/switch/index";
import {
  BELL_MODES,
  LINK_MODIFIERS,
  RIGHT_CLICK_MODES,
  WORD_SEPARATOR_MAX_LENGTH,
  type TerminalBehaviorSettings,
  type TerminalBellMode,
  type TerminalLinkModifier,
  type TerminalRightClickMode,
} from "../../terminal/lib/terminalBehavior";

defineProps<{
  terminalBehavior: TerminalBehaviorSettings;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "update-behavior", patch: Partial<TerminalBehaviorSettings>): void;
}>();

/** 终端行为枚举 → i18n key。模板按 lib 的常量数组顺序迭代，渲染单选组。 */
const RIGHT_CLICK_LABELS: Record<TerminalRightClickMode, string> = {
  off: "terminalBehavior.rightClickOff",
  menu: "terminalBehavior.rightClickMenu",
  paste: "terminalBehavior.rightClickPaste",
  clipboard: "terminalBehavior.rightClickClipboard",
};
const BELL_LABELS: Record<TerminalBellMode, string> = {
  off: "terminalBehavior.bellOff",
  visual: "terminalBehavior.bellVisual",
  audible: "terminalBehavior.bellAudible",
};
const LINK_MODIFIER_LABELS: Record<TerminalLinkModifier, string> = {
  none: "terminalBehavior.linkModifierNone",
  ctrl: "terminalBehavior.linkModifierCtrl",
  alt: "terminalBehavior.linkModifierAlt",
  shift: "terminalBehavior.linkModifierShift",
  meta: "terminalBehavior.linkModifierMeta",
};

function updateBehavior(patch: Partial<TerminalBehaviorSettings>) {
  emit("update-behavior", patch);
}

/** 文本输入框取原始串（同 numberFieldValue，避免在模板里写 as 断言）。 */
function textFieldValue(event: Event): string {
  return (event.target as HTMLInputElement).value;
}

/** reka Select 回传 AcceptableValue（含 null 与对象），这里只接受字符串形态。 */
function updateLinkModifier(value: unknown) {
  updateBehavior({ linkModifier: String(value) as TerminalLinkModifier });
}
</script>

<template>
  <h3 class="settings-section-title">{{ t("terminalBehavior.keyboardSection") }}</h3>
  <label class="settings-field settings-switch-row">
    <Switch size="sm" :model-value="terminalBehavior.altIsMeta" @update:model-value="(v) => updateBehavior({ altIsMeta: v === true })" />
    <span>{{ t("terminalBehavior.altIsMeta") }}</span>
  </label>
  <p class="muted settings-note">{{ t("terminalBehavior.altIsMetaHint") }}</p>
  <label class="settings-field settings-switch-row">
    <Switch size="sm" :model-value="terminalBehavior.scrollOnInput" @update:model-value="(v) => updateBehavior({ scrollOnInput: v === true })" />
    <span>{{ t("terminalBehavior.scrollOnInput") }}</span>
  </label>
  <p class="muted settings-note">{{ t("terminalBehavior.scrollOnInputHint") }}</p>

  <h3 class="settings-section-title">{{ t("terminalBehavior.mouseSection") }}</h3>
  <h4 class="settings-section-title">{{ t("terminalBehavior.rightClick") }}</h4>
  <label v-for="mode in RIGHT_CLICK_MODES" :key="mode" class="settings-field settings-radio-row">
    <input type="radio" name="terminal-right-click" :value="mode" :checked="terminalBehavior.rightClick === mode" @change="updateBehavior({ rightClick: mode })" />
    <span>{{ t(RIGHT_CLICK_LABELS[mode]) }}</span>
  </label>
  <p class="muted settings-note">{{ t("terminalBehavior.rightClickHint") }}</p>
  <label class="settings-field settings-switch-row">
    <Switch size="sm" :model-value="terminalBehavior.pasteOnMiddleClick" @update:model-value="(v) => updateBehavior({ pasteOnMiddleClick: v === true })" />
    <span>{{ t("terminalBehavior.pasteOnMiddleClick") }}</span>
  </label>
  <label class="settings-field">
    <span>{{ t("terminalBehavior.wordSeparator") }}</span>
    <input class="mono" spellcheck="false" :maxlength="WORD_SEPARATOR_MAX_LENGTH" :value="terminalBehavior.wordSeparator" @change="updateBehavior({ wordSeparator: textFieldValue($event) })" />
  </label>
  <p class="muted settings-note">{{ t("terminalBehavior.wordSeparatorHint") }}</p>
  <label class="settings-field">
    <span>{{ t("terminalBehavior.linkModifier") }}</span>
    <Select :model-value="terminalBehavior.linkModifier" @update:model-value="updateLinkModifier">
      <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
      <SelectContent>
        <SelectItem v-for="modifier in LINK_MODIFIERS" :key="modifier" :value="modifier">{{ t(LINK_MODIFIER_LABELS[modifier]) }}</SelectItem>
      </SelectContent>
    </Select>
  </label>
  <p class="muted settings-note">{{ t("terminalBehavior.linkModifierHint") }}</p>

  <h3 class="settings-section-title">{{ t("terminalBehavior.clipboardSection") }}</h3>
  <label class="settings-field settings-switch-row">
    <Switch size="sm" :model-value="terminalBehavior.copyOnSelect" @update:model-value="(v) => updateBehavior({ copyOnSelect: v === true })" />
    <span>{{ t("terminalSelectCopy.label") }}</span>
  </label>
  <p class="muted settings-note">{{ t("terminalSelectCopy.hint") }}</p>
  <label class="settings-field settings-switch-row">
    <Switch size="sm" :model-value="terminalBehavior.bracketedPaste" @update:model-value="(v) => updateBehavior({ bracketedPaste: v === true })" />
    <span>{{ t("terminalBehavior.bracketedPaste") }}</span>
  </label>
  <p class="muted settings-note">{{ t("terminalBehavior.bracketedPasteHint") }}</p>
  <label class="settings-field settings-switch-row">
    <Switch size="sm" :model-value="terminalBehavior.warnOnMultilinePaste" @update:model-value="(v) => updateBehavior({ warnOnMultilinePaste: v === true })" />
    <span>{{ t("terminalBehavior.warnOnMultilinePaste") }}</span>
  </label>
  <p class="muted settings-note">{{ t("terminalBehavior.warnOnMultilinePasteHint") }}</p>
  <label class="settings-field settings-switch-row">
    <Switch size="sm" :model-value="terminalBehavior.replaceNewlinesWithSpaces" @update:model-value="(v) => updateBehavior({ replaceNewlinesWithSpaces: v === true })" />
    <span>{{ t("terminalBehavior.replaceNewlines") }}</span>
  </label>
  <label class="settings-field settings-switch-row">
    <Switch size="sm" :model-value="terminalBehavior.trimWhitespaceOnPaste" @update:model-value="(v) => updateBehavior({ trimWhitespaceOnPaste: v === true })" />
    <span>{{ t("terminalBehavior.trimWhitespace") }}</span>
  </label>

  <h3 class="settings-section-title">{{ t("terminalBehavior.soundSection") }}</h3>
  <h4 class="settings-section-title">{{ t("terminalBehavior.bell") }}</h4>
  <label v-for="mode in BELL_MODES" :key="mode" class="settings-field settings-radio-row">
    <input type="radio" name="terminal-bell" :value="mode" :checked="terminalBehavior.bell === mode" @change="updateBehavior({ bell: mode })" />
    <span>{{ t(BELL_LABELS[mode]) }}</span>
  </label>
  <p class="muted settings-note">{{ t("terminalBehavior.bellHint") }}</p>
</template>
