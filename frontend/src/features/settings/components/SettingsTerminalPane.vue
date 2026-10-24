<script setup lang="ts">
// 设置·终端分栏（对标 Tabby「Terminal」页）：分栏布局、渲染、本地 Shell、
// 键盘/鼠标/剪贴板/声音、动作链接、gutter、在线搜索、X11/RDP/自动录制、启动
// 命令与连接级文件名编码。终端行为类只上抛增量（归一化与落地在 App）；启动
// 命令与文件名编码由连接级组合式自治读写 sidecar 偏好。
import { computed } from "vue";
import { ArrowDown, ArrowUp, Plus, Trash2 } from "@lucide/vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { Switch } from "../../../shared/ui/switch/index";
import {
  BELL_MODES,
  LINK_MODIFIERS,
  RIGHT_CLICK_MODES,
  SCROLLBACK_MAX,
  SCROLLBACK_MIN,
  WORD_SEPARATOR_MAX_LENGTH,
  type TerminalBehaviorSettings,
  type TerminalBellMode,
  type TerminalLinkModifier,
  type TerminalRightClickMode,
} from "../../terminal/lib/terminalBehavior";
import type { PaneDirectionPref } from "../../../shared/lib/paneLayout";
import { type ActionLinkMatcherToggles, type ActionLinksSettings } from "../../terminal/lib/actionLinksMatcher";
import { GUTTER_TIMESTAMP_DEFAULT_FORMAT, type GutterSettings } from "../../terminal/lib/terminalGutter";
import {
  STARTUP_COMMAND_MAX,
  STARTUP_DELAY_MAX_MS,
} from "../../connections/lib/startupCommands";
import type { ConnectionNameEncodingChoice } from "../../connections/lib/connectionNameEncoding";
import type { ConnectionSettingsController } from "../composables/useConnectionSettings";

type LocalShellOption = { program: string; name: string; isDefault: boolean; isUserShell: boolean; injectable?: boolean };

const props = defineProps<{
  /** 当前 SSH 会话的连接 id：启动命令/文件名编码按 connectionId 分桶（无会话时整区隐藏）。 */
  connectionId?: string;
  /** 连接级启动命令与文件名编码（组合式持有，自治 RPC）。 */
  startup: ConnectionSettingsController;
  paneDirection: PaneDirectionPref;
  webglEnabled: boolean;
  terminalBehavior: TerminalBehaviorSettings;
  localShell: string;
  localShellOptions: LocalShellOption[];
  actionLinks: ActionLinksSettings;
  gutter: GutterSettings;
  ctxSearchEngines: string;
  x11Enabled: boolean;
  x11ReadOnlyNote: boolean;
  rdpExperimentalEnabled: boolean;
  autoRecordEnabled: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "set-pane-direction", value: PaneDirectionPref): void;
  (e: "set-webgl", value: boolean): void;
  (e: "update-behavior", patch: Partial<TerminalBehaviorSettings>): void;
  (e: "set-local-shell", program: string): void;
  (e: "update-action-links", patch: { enabled?: boolean; matchers?: Partial<ActionLinkMatcherToggles> }): void;
  (e: "update-gutter", patch: { showLineNumbers?: boolean; showTimestamps?: boolean; timestampFormat?: string }): void;
  (e: "set-ctx-search-engines", value: string): void;
  (e: "set-x11-enabled", value: boolean): void;
  (e: "set-rdp-experimental-enabled", value: boolean): void;
  (e: "set-auto-record-enabled", value: boolean): void;
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

// 行为设置只上抛增量：归一化、持久化与落地到 xterm 选项全在 App，
// 保证「权威态唯一」——本组件不持有行为设置的副本，重开弹窗也不会出现回显漂移。
function updateBehavior(patch: Partial<TerminalBehaviorSettings>) {
  emit("update-behavior", patch);
}

/** 数字输入框 → 行为字段：空串或非数值直接忽略，避免清空输入框把值打成 NaN。 */
function updateScrollback(raw: string) {
  const trimmed = raw.trim();
  if (!trimmed) return;
  const value = Number(trimmed);
  if (!Number.isFinite(value)) return;
  updateBehavior({ scrollbackLines: value });
}

// 动作链接/gutter 与行为设置同款增量语义：本组件不持副本，重开弹窗无回显漂移。
function updateActionLinks(patch: { enabled?: boolean; matchers?: Partial<ActionLinkMatcherToggles> }) {
  emit("update-action-links", patch);
}

function updateGutter(patch: { showLineNumbers?: boolean; showTimestamps?: boolean; timestampFormat?: string }) {
  emit("update-gutter", patch);
}

/** 单个匹配器开关：reka Switch 回传 boolean | string，收紧为布尔再上抛。 */
function updateMatcher(key: keyof ActionLinkMatcherToggles, value: unknown) {
  updateActionLinks({ matchers: { [key]: value === true } });
}

/** 时间戳格式输入：空串回退默认格式（输入中途清空不落空格式）。 */
function updateGutterFormat(raw: string) {
  const trimmed = raw.trim();
  updateGutter({ timestampFormat: trimmed || GUTTER_TIMESTAMP_DEFAULT_FORMAT });
}

/** 三类匹配器开关按常量数组迭代（模板不支持 as const 字面量迭代）。 */
const ACTION_LINK_MATCHER_ROWS: Array<{ key: keyof ActionLinkMatcherToggles; label: string }> = [
  { key: "ipv4", label: "actionLinks.ipv4" },
  { key: "hostPort", label: "actionLinks.hostPort" },
  { key: "archive", label: "actionLinks.archive" },
];

/** 文本输入框取原始串（同 numberFieldValue，避免在模板里写 as 断言）。 */
function textFieldValue(event: Event): string {
  return (event.target as HTMLInputElement).value;
}

/** 数值输入框取原始串。 */
function numberFieldValue(event: Event): string {
  return (event.target as HTMLInputElement).value;
}

/** reka Select 回传 AcceptableValue（含 null 与对象），这里只接受字符串形态。 */
function updateLinkModifier(value: unknown) {
  updateBehavior({ linkModifier: String(value) as TerminalLinkModifier });
}

// 本地终端默认 Shell：下拉项来自 local/shells/list（弹窗打开时拉一次，选项在
// 弹窗持有）；localShell 偏好的权威态在 App，这里经 set-local-shell 上抛。
// reka Select 不接受空串 value，用哨兵表达「自动检测」。
const LOCAL_SHELL_AUTO = "__auto__";
const localShellModelValue = computed(() => (props.localShell ? props.localShell : LOCAL_SHELL_AUTO));
function resolveLocalShellValue(value: string): string {
  return value === LOCAL_SHELL_AUTO ? "" : value;
}
</script>

<template>
  <div class="settings-pane">
    <!-- 分栏布局方向（issue #187）：终端与 SFTP 的排布三选；auto 沿用
         窄屏断点，强制上下/左右不受视口影响。 -->
    <h3 class="settings-section-title">{{ t("workbenchLayout.sectionTitle") }}</h3>
    <label class="settings-field">
      <span>{{ t("workbenchLayout.direction") }}</span>
      <Select :model-value="paneDirection" @update:model-value="(v) => emit('set-pane-direction', String(v) as PaneDirectionPref)">
        <SelectTrigger size="sm"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value="auto">{{ t("workbenchLayout.directionAuto") }}</SelectItem>
          <SelectItem value="vertical">{{ t("workbenchLayout.directionVertical") }}</SelectItem>
          <SelectItem value="horizontal">{{ t("workbenchLayout.directionHorizontal") }}</SelectItem>
        </SelectContent>
      </Select>
    </label>
    <h3 class="settings-section-title">{{ t("terminalBehavior.renderingSection") }}</h3>
    <label class="settings-field settings-switch-row">
      <Switch size="sm" :model-value="webglEnabled" @update:model-value="(value) => emit('set-webgl', value === true)" />
      <span>{{ t("webglLabel") }}</span>
    </label>
    <p class="muted settings-note">{{ t("webglHint") }}</p>
    <label class="settings-field">
      <span>{{ t("terminalBehavior.scrollbackLines") }}</span>
      <input type="number" :min="SCROLLBACK_MIN" :max="SCROLLBACK_MAX" step="100" :value="terminalBehavior.scrollbackLines" @change="updateScrollback(numberFieldValue($event))" />
    </label>
    <p class="muted settings-note">{{ t("terminalBehavior.scrollbackHint") }}</p>

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

    <h3 class="settings-section-title">{{ t("actionLinks.sectionTitle") }}</h3>
    <label class="settings-field settings-switch-row">
      <Switch size="sm" :model-value="actionLinks.enabled" @update:model-value="(v) => updateActionLinks({ enabled: v === true })" />
      <span>{{ t("actionLinks.enabled") }}</span>
    </label>
    <p class="muted settings-note">{{ t("actionLinks.enabledHint") }}</p>
    <template v-if="actionLinks.enabled">
      <label v-for="matcher in ACTION_LINK_MATCHER_ROWS" :key="matcher.key" class="settings-field settings-switch-row">
        <Switch size="sm" :model-value="actionLinks.matchers[matcher.key]" @update:model-value="(v) => updateMatcher(matcher.key, v)" />
        <span>{{ t(matcher.label) }}</span>
      </label>
    </template>

    <h3 class="settings-section-title">{{ t("gutter.sectionTitle") }}</h3>
    <label class="settings-field settings-switch-row">
      <Switch size="sm" :model-value="gutter.showLineNumbers" @update:model-value="(v) => updateGutter({ showLineNumbers: v === true })" />
      <span>{{ t("gutter.showLineNumbers") }}</span>
    </label>
    <p class="muted settings-note">{{ t("gutter.showLineNumbersHint") }}</p>
    <label class="settings-field settings-switch-row">
      <Switch size="sm" :model-value="gutter.showTimestamps" @update:model-value="(v) => updateGutter({ showTimestamps: v === true })" />
      <span>{{ t("gutter.showTimestamps") }}</span>
    </label>
    <p class="muted settings-note">{{ t("gutter.showTimestampsHint") }}</p>
    <label class="settings-field">
      <span>{{ t("gutter.timestampFormat") }}</span>
      <input class="mono" spellcheck="false" :maxlength="64" :placeholder="GUTTER_TIMESTAMP_DEFAULT_FORMAT" :value="gutter.timestampFormat" @change="updateGutterFormat(textFieldValue($event))" />
    </label>
    <p class="muted settings-note">{{ t("gutter.timestampFormatHint") }}</p>

    <h3 class="settings-section-title">{{ t("ctxSearch.sectionTitle") }}</h3>
    <p class="muted settings-note">{{ t("ctxSearch.hint") }}</p>
    <label class="settings-field">
      <span>{{ t("ctxSearch.enginesLabel") }}</span>
      <textarea class="mono" rows="4" spellcheck="false" :value="ctxSearchEngines" @change="emit('set-ctx-search-engines', ($event.target as HTMLTextAreaElement).value)"></textarea>
    </label>

    <p class="muted settings-note">{{ t("terminalBehavior.scopeNote") }}</p>
    <p class="muted settings-note">{{ t("terminalFont.movedHint") }}</p>

    <h3 class="settings-section-title">{{ t("x11.sectionTitle") }}</h3>
    <label class="settings-field settings-switch-row">
      <Switch :model-value="x11Enabled" size="sm" @update:model-value="(v) => emit('set-x11-enabled', Boolean(v))" />
      <span>{{ t("x11.enabled") }}</span>
    </label>
    <p class="muted settings-note">{{ t("x11.enabledHint") }}</p>
    <p v-if="x11ReadOnlyNote" class="muted settings-note">{{ t("x11.readOnlyNote") }}</p>

    <h3 class="settings-section-title">{{ t("rdp.experimentalSection") }}</h3>
    <label class="settings-field settings-switch-row">
      <Switch :model-value="rdpExperimentalEnabled" size="sm" @update:model-value="(v) => emit('set-rdp-experimental-enabled', Boolean(v))" />
      <span>{{ t("rdp.experimentalEnabled") }}</span>
    </label>
    <p class="muted settings-note">{{ t("rdp.experimentalHint") }}</p>

    <!-- 会话自动录制（M14）：新会话自动挂录制器，标 iShell 录制增强。 -->
    <h3 class="settings-section-title">{{ t("autoRecord.sectionTitle") }}</h3>
    <label class="settings-field settings-switch-row">
      <Switch :model-value="autoRecordEnabled" size="sm" @update:model-value="(v) => emit('set-auto-record-enabled', Boolean(v))" />
      <span>{{ t("autoRecord.enabled") }}</span>
    </label>
    <p class="muted settings-note">{{ t("autoRecord.enabledHint") }}</p>

    <!-- 启动命令（对标 Tabby「Login scripts」）：连接建立进入 shell 后按序
         自动键入；仅 SSH 交互 shell 会话生效（RemoteCommand exec 会话跳过）。 -->
    <template v-if="connectionId">
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
  </div>
</template>
