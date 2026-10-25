<script setup lang="ts">
// 设置·终端分栏（对标 Tabby「Terminal」页）：分栏布局、渲染、本地 Shell、
// 键盘/鼠标/剪贴板/声音、动作链接、gutter、在线搜索、X11/RDP/自动录制、启动
// 命令与连接级文件名编码。终端行为类只上抛增量（归一化与落地在 App）；启动
// 命令与文件名编码由连接级组合式自治读写 sidecar 偏好。
// 分组细节已拆到同目录子组件（渲染/本地 Shell/行为/启动集成），本组件保留
// 布局方向、动作链接、gutter、在线搜索与 X11/RDP/自动录制，DOM 顺序不变。
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { Switch } from "../../../shared/ui/switch/index";
import { type TerminalBehaviorSettings } from "../../terminal/lib/appearance/terminalBehavior";
import type { PaneDirectionPref } from "../../../shared/lib/paneLayout";
import { type ActionLinkMatcherToggles, type ActionLinksSettings } from "../../terminal/lib/decorations/actionLinksMatcher";
import { GUTTER_TIMESTAMP_DEFAULT_FORMAT, type GutterSettings } from "../../terminal/lib/appearance/terminalGutter";
import type { ConnectionSettingsController } from "../composables/useConnectionSettings";
import SettingsTerminalRenderingSection from "./SettingsTerminalRenderingSection.vue";
import SettingsTerminalLocalShellSection from "./SettingsTerminalLocalShellSection.vue";
import SettingsTerminalBehaviorSection from "./SettingsTerminalBehaviorSection.vue";
import SettingsTerminalStartupSection from "./SettingsTerminalStartupSection.vue";

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

// 行为设置只上抛增量：归一化、持久化与落地到 xterm 选项全在 App，
// 保证「权威态唯一」——子组件不持有行为设置的副本，重开弹窗也不会回显漂移。
function updateBehavior(patch: Partial<TerminalBehaviorSettings>) {
  emit("update-behavior", patch);
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

/** 文本输入框取原始串（避免在模板里写 as 断言）。 */
function textFieldValue(event: Event): string {
  return (event.target as HTMLInputElement).value;
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

    <SettingsTerminalRenderingSection
      :webgl-enabled="webglEnabled"
      :terminal-behavior="terminalBehavior"
      :t="t"
      @set-webgl="(value) => emit('set-webgl', value)"
      @update-behavior="updateBehavior"
    />

    <SettingsTerminalLocalShellSection
      :local-shell="localShell"
      :local-shell-options="localShellOptions"
      :t="t"
      @set-local-shell="(program) => emit('set-local-shell', program)"
    />

    <SettingsTerminalBehaviorSection
      :terminal-behavior="terminalBehavior"
      :t="t"
      @update-behavior="updateBehavior"
    />

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

    <!-- 启动命令 + 连接级文件名编码：仅 SSH 会话（connectionId）存在时渲染。 -->
    <SettingsTerminalStartupSection
      v-if="connectionId"
      :startup="startup"
      :t="t"
    />
  </div>
</template>
