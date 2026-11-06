<script setup lang="ts">
// 设置弹窗（从 App.vue 抽出的独立组件）：承载设置主弹窗与 quick sudo 配置档
// 管理弹窗。连接级设置（ssh/settings/get|set）、配置档（sudo/profiles/*）、
// 已知主机（ssh/knownHosts/*）、本机密钥发现（keys/discover）、MCP 限速
// （mcp/settings/*）经 shared/lib/rpc 的 rpc() 直调（桥缺失时统一
// RpcUnavailableError，而非 TypeError）；下载偏好与终端偏好
// （WebGL/选中复制/终端字体）的权威态在宿主 App——下载偏好经 downloadPrefs
// 适配器读写，终端偏好经 props 下发 + emits 上抛。
//
// 本组件是编排者：props/emits、分类导航、分类切换与模板留在这里；数据/加载/
// 保存模型（连接级设置、sudo 配置档、传输/建议草稿、已知主机/密钥/MCP、X11/
// 自动录制/RDP 开关、主保存链与 Esc 分层退出）拆到
// composables/useSettingsDialogModel，各分类面板拆到 components/Settings*Pane.vue，
// props 契约下沉到 lib/settingsDialogProps。
import { computed } from "vue";
import { Loader2, Search, Trash2, X } from "@lucide/vue";
import { CATEGORY_ICONS } from "../lib/settingsNavIcons";
import { SETTINGS_CATEGORIES } from "../lib/settingsNavCategories";
import { useSettingsNav } from "../composables/useSettingsNav";
import { Dialog, DialogContent, DialogTitle } from "../../../shared/ui/dialog/index";
import { Tabs, TabsList, TabsTrigger } from "../../../shared/ui/tabs/index";
import TerminalHotkeyEditor from "../../terminal/components/TerminalHotkeyEditor.vue";
import HighlightRulesSection from "../../terminal/components/HighlightRulesSection.vue";
import QuickCommandsSection from "../../quick-commands/components/QuickCommandsSection.vue";
import { HIGHLIGHT_RULES_LIMIT, type HighlightDisplayMode, type HighlightRuleView } from "../../terminal/lib/decorations/keywordHighlight";
import { QUICK_COMMANDS_LIMIT, type QuickCommand } from "../../quick-commands/lib/quickCommands";
import type { TerminalColorScheme } from "../../terminal/lib/appearance/terminalScheme";
import type { TerminalAppearanceProfile, TerminalAppearanceSettings } from "../../terminal/lib/appearance/terminalAppearance";
import type { TerminalBehaviorSettings } from "../../terminal/lib/appearance/terminalBehavior";
import type { TerminalHotkeyBindings } from "../../terminal/lib/behavior/terminalHotkeys";
import type { PaneDirectionPref } from "../../../shared/lib/paneLayout";
import type { AiSettings } from "../../agent/lib/aiSettings";
import { type ActionLinkMatcherToggles } from "../../terminal/lib/decorations/actionLinksMatcher";
import type { SettingsDialogProps } from "../lib/settingsDialogProps";
import { useSettingsDialogModel } from "../composables/useSettingsDialogModel";
import SettingsSchemePane from "./SettingsSchemePane.vue";
import SettingsAppearancePane from "./SettingsAppearancePane.vue";
import SettingsSudoPane from "./SettingsSudoPane.vue";
import SettingsAgentPane from "./SettingsAgentPane.vue";
import SettingsTransferPane from "./SettingsTransferPane.vue";
import SettingsTerminalPane from "./SettingsTerminalPane.vue";
import SettingsSuggestionsPane from "./SettingsSuggestionsPane.vue";
import SettingsAiPane from "./SettingsAiPane.vue";
import SettingsSecurityPane from "./SettingsSecurityPane.vue";
import SettingsMcpPane from "./SettingsMcpPane.vue";
import ProfilesManagerDialog from "./ProfilesManagerDialog.vue";

const props = defineProps<SettingsDialogProps>();

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  (e: "update:profilesOpen", value: boolean): void;
  (e: "notice", message: string): void;
  (e: "error", cause: unknown): void;
  (e: "browse-download-dir"): void;
  (e: "update:webgl", value: boolean): void;
  /** 本地终端默认 Shell 改动：App 落 localShell 偏好（local/preferences/set）。 */
  (e: "set-local-shell", program: string): void;
  /** 行内 ghost 自动建议开关（权威态在 settingsPreferences store；写穿后上抛 App 做即时副作用）。 */
  (e: "update:ghostSuggest", value: boolean): void;
  /** 历史面板功能开关（issue #169，权威态在 settingsPreferences store）。 */
  (e: "update:historyPanelEnabled", value: boolean): void;
  /** 历史显示条数（issue #192，权威态在 settingsPreferences store，App 收紧各桶）。 */
  (e: "update:commandHistoryLimit", value: number): void;
  (e: "update:ghostTabAccept", value: boolean): void;
  /** AI 助手设置增量（Warp AI 对齐批）：App 归一化 + pluginStore 持久化。 */
  (e: "update-ai-settings", patch: Partial<AiSettings>): void;
  /** 清空 AI 修复「不再询问」记忆位。 */
  (e: "reset-ai-consent"): void;
  /** 空提示符快捷键引导条开关（权威态在 App；App 侧重开时会顺带清消散旗标）。 */
  (e: "update:promptHintsEnabled", value: boolean): void;
  /** 行为设置局部增量：App 侧会归一化 + 持久化 + 即时落地到 xterm 选项。 */
  (e: "update-behavior", patch: Partial<TerminalBehaviorSettings>): void;
  /** 分栏布局方向改动（issue #187）：App 侧持久化 + 重排终端。 */
  (e: "update:paneDirection", value: PaneDirectionPref): void;
  /** 快捷键整表替换（编辑器内部管理增删改，只上抛最终结果）。 */
  (e: "update-hotkeys", bindings: TerminalHotkeyBindings): void;
  (e: "apply-font", payload: { family: string | null; size: number }): void;
  (e: "update-appearance", patch: Partial<TerminalAppearanceSettings>): void;
  /** 动作链接设置增量：App 侧归一化 + sidecar 持久化 + 即时挂/摘 link provider。 */
  (e: "update:actionLinks", patch: { enabled?: boolean; matchers?: Partial<ActionLinkMatcherToggles> }): void;
  /** gutter 设置增量：App 侧归一化 + sidecar 持久化 + 即时挂/摘。 */
  (e: "update:gutter", patch: { showLineNumbers?: boolean; showTimestamps?: boolean; timestampFormat?: string }): void;
  /** 在线搜索引擎表整表替换：App 侧解析 + sidecar 持久化。 */
  (e: "update:ctxSearchEngines", value: string): void;
  /** 背景图：开关/透明度上抛增量；图片经本地读取 base64 后交 App 走 sidecar。 */
  (e: "update:wallpaperEnabled", value: boolean): void;
  (e: "update:wallpaperOpacity", value: number): void;
  (e: "set-wallpaper-image", image: { base64: string; mime: string }): void;
  (e: "clear-wallpaper"): void;
  (e: "apply-theme", theme: TerminalAppearanceProfile): void;
  (e: "save-theme", name: string): void;
  (e: "delete-theme", id: string): void;
  (e: "add-schemes", schemes: Array<Omit<TerminalColorScheme, "id" | "source">>): void;
  (e: "remove-scheme", id: string): void;
  /** 高亮规则增量（M32-A2）：数据面 RPC 留在 App，这里只转发 section 的意图。 */
  (e: "save-highlight-rule", rule: { id?: string; pattern: string; color: string; isRegex: boolean; caseSensitive: boolean }): void;
  (e: "delete-highlight-rule", id: string): void;
  (e: "toggle-highlight-rule", item: HighlightRuleView): void;
  /** 显示方式增量（全局偏好）：同样只转发 section 的意图，落 pluginStore 在 App。 */
  (e: "update:highlightDisplayMode", mode: HighlightDisplayMode): void;
  /** 快速命令增量（M32-A3；issue #201 分组字段随命令透传）：import 由 App 循环逐条 save。 */
  (e: "save-quick-command", command: { id?: string; name: string; command: string; groupId?: string; groupName?: string }): void;
  (e: "delete-quick-command", id: string): void;
  (e: "import-quick-commands", items: Array<{ name: string; command: string; groupName?: string }>): void;
  /** 分组新建 / 改名 / 删除（删除 = 组内命令移回未分组）：数据面在 App。 */
  (e: "create-quick-command-group", name: string): void;
  (e: "rename-quick-command-group", payload: { groupId: string; name: string }): void;
  (e: "dissolve-quick-command-group", groupId: string): void;
  /** 管理页直接执行/粘贴（与工具条弹层同一 PTY 写入语义）。 */
  (e: "run-quick-command", item: QuickCommand): void;
  (e: "paste-quick-command", item: QuickCommand): void;
}>();

const t = props.t;

// 数据/加载/保存模型（连接级设置、sudo 配置档、传输/建议草稿、已知主机/密钥/
// MCP、X11/自动录制/RDP 开关、主保存链与 Esc 分层退出）。解构出的绑定名与
// 拆分前一致，模板无需改动。
const {
  connection,
  sudo,
  font,
  settingsSaving,
  transferDrafts,
  suggestionDrafts,
  commandHistoryLimitDraft,
  knownHosts,
  knownHostsLoading,
  knownHostsError,
  localKeys,
  localKeysLoading,
  localKeysError,
  mcpDraft,
  mcpLoading,
  mcpError,
  mcpInputsValid,
  x11Enabled,
  x11ReadOnlyNote,
  rdpExperimentalEnabled,
  autoRecordEnabled,
  localShellOptions,
  isLocalSession,
  reloadSettings,
  loadKnownHosts,
  loadLocalKeys,
  removeKnownHost,
  loadMcpSettings,
  saveSettings,
  clearStoredSecrets,
  consumeInlineEsc,
  setDownloadDirDraft,
  setDownloadUseDefaultDraft,
  updateHotkeys,
  setX11Enabled,
  setRdpExperimentalEnabled,
  setAutoRecordEnabled,
} = useSettingsDialogModel({
  props,
  emitNotice: (message) => emit("notice", message),
  emitError: (cause) => emit("error", cause),
  emitApplyFont: (payload) => emit("apply-font", payload),
  emitHotkeys: (bindings) => emit("update-hotkeys", bindings),
});

// 导航（分类选择、分类搜索、面板滚动复位、Tabs a11y）全部下沉到 composables/
// useSettingsNav：本组件只负责 props/emits、13 个 pane 的模板与保存编排。
const {
  activeCategory,
  activeCategoryCounter,
  categorySearchInput,
  filteredSettingsCategories,
  focusCategorySearch,
  navEl,
  onDialogKeydown,
  onSearchEscape,
  onSettingsCategoryChange,
  panesEl,
  settingsCategory,
  settingsCategoryQuery,
  settingsPanelId,
} = useSettingsNav({
  isLocalSession: () => isLocalSession.value,
  t,
});

defineExpose({ consumeInlineEsc, setDownloadDirDraft, setDownloadUseDefaultDraft });
</script>

<template>
    <!-- 设置主弹窗 -->
    <Dialog :open="open" @update:open="(value) => emit('update:open', value)">
      <DialogContent class="modal settings-modal settings-nav-modal" @escape-key-down.prevent @keydown="onDialogKeydown">
        <header class="settings-header">
          <div class="settings-heading">
            <span class="settings-eyebrow" aria-hidden="true">{{ t("pluginName") }}</span>
            <DialogTitle class="settings-title">{{ t("settings") }}</DialogTitle>
          </div>
          <div class="settings-header-tools">
            <label class="settings-search">
              <Search class="settings-search-icon" aria-hidden="true" />
              <input
                ref="categorySearchInput"
                v-model="settingsCategoryQuery"
                class="settings-search-input"
                type="search"
                autocomplete="off"
                spellcheck="false"
                :placeholder="t('settingsSearchPlaceholder')"
                :aria-label="t('settingsSearchPlaceholder')"
                @keydown.esc="onSearchEscape"
              />
              <kbd class="settings-search-kbd" aria-hidden="true">/</kbd>
            </label>
            <button :title="t('close')" :aria-label="t('close')" class="icon-button settings-close" @click="emit('update:open', false)"><X /></button>
          </div>
        </header>
        <div class="settings-body">
          <div v-if="connection.state.loading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
          <div v-else-if="connection.state.loadFailed" class="task-error" role="alert">
            {{ t("settingsLoadFailed") }}
            <button class="link-button" @click="reloadSettings">{{ t("refresh") }}</button>
          </div>
          <template v-else>
          <div class="settings-layout">
            <nav ref="navEl" class="settings-nav" aria-label="settings categories">
              <Tabs :model-value="settingsCategory" orientation="vertical" class="settings-nav-tabs" @update:model-value="onSettingsCategoryChange">
                <TabsList class="settings-nav-list">
                  <TabsTrigger v-for="cat in filteredSettingsCategories" :key="cat.id" :value="cat.id" :title="t(cat.labelKey)" :data-category="cat.id" :data-index="String(SETTINGS_CATEGORIES.findIndex((entry) => entry.id === cat.id) + 1).padStart(2, '0')" class="settings-nav-item">
                    <component :is="CATEGORY_ICONS[cat.id]" class="settings-nav-icon" />
                    <span class="settings-nav-label">{{ t(cat.labelKey) }}</span>
                  </TabsTrigger>
                </TabsList>
              </Tabs>
              <p v-if="!filteredSettingsCategories.length" class="settings-nav-empty">{{ t("settingsSearchEmpty") }}</p>
            </nav>
            <div class="settings-content">
              <div class="settings-pane-hero" :data-category="settingsCategory">
                <span class="settings-hero-icon" aria-hidden="true"><component :is="CATEGORY_ICONS[settingsCategory]" /></span>
                <div class="settings-hero-text">
                  <span class="settings-hero-kicker">{{ activeCategoryCounter }}</span>
                  <h2 :key="settingsCategory" class="settings-hero-title">{{ t(activeCategory.labelKey) }}</h2>
                </div>
              </div>
              <div ref="panesEl" class="settings-panes">
            <!-- 配色方案（对标 Tabby「Color scheme」页，从原「外观」里拆出）：主题
                 快照、实时预览、深浅两槽配色方案、终端背景来源、自定义方案导入。 -->
            <SettingsSchemePane
              v-show="settingsCategory === 'scheme'"
              role="tabpanel"
              :id="settingsPanelId('scheme')"
              :appearance="appearance"
              :custom-themes="customThemes"
              :active-theme-id="activeThemeId"
              :host-theme="hostTheme"
              :host-color-scheme="hostColorScheme"
              :host-font-family="hostFontFamily"
              :terminal-font-size="terminalFontSize"
              :t="t"
              @update-appearance="(patch) => emit('update-appearance', patch)"
              @apply-theme="(theme) => emit('apply-theme', theme)"
              @save-theme="(name) => emit('save-theme', name)"
              @delete-theme="(id) => emit('delete-theme', id)"
              @add-schemes="(schemes) => emit('add-schemes', schemes)"
              @remove-scheme="(id) => emit('remove-scheme', id)"
            />

            <!-- 外观（对标 Tabby「Appearance」页）：字体与字号、字重/行高/字间距/内边距、
                 光标、渲染细项，末尾再放一次实时预览以便边调边看排版效果。 -->
            <SettingsAppearancePane
              v-show="settingsCategory === 'appearance'"
              role="tabpanel"
              :id="settingsPanelId('appearance')"
              :appearance="appearance"
              :custom-themes="customThemes"
              :host-theme="hostTheme"
              :host-color-scheme="hostColorScheme"
              :host-font-family="hostFontFamily"
              :terminal-font-size="terminalFontSize"
              :font="font"
              :wallpaper-enabled="wallpaperEnabled"
              :wallpaper-opacity="wallpaperOpacity"
              :wallpaper-session-only="wallpaperSessionOnly"
              :t="t"
              @update-appearance="(patch) => emit('update-appearance', patch)"
              @set-wallpaper-enabled="(value) => emit('update:wallpaperEnabled', value)"
              @set-wallpaper-opacity="(value) => emit('update:wallpaperOpacity', value)"
              @set-wallpaper-image="(image) => emit('set-wallpaper-image', image)"
              @clear-wallpaper="emit('clear-wallpaper')"
              @error="(cause) => emit('error', cause)"
            />

            <SettingsSudoPane
              v-show="settingsCategory === 'sudo'"
              role="tabpanel"
              :id="settingsPanelId('sudo')"
              :sudo="sudo"
              :draft="connection.state.draft"
              :meta="connection.state.meta"
              :t="t"
            />

            <SettingsAgentPane
              v-show="settingsCategory === 'agent'"
              role="tabpanel"
              :id="settingsPanelId('agent')"
              :draft="connection.state.draft"
              :t="t"
            />

            <SettingsTransferPane
              v-show="settingsCategory === 'transfer'"
              role="tabpanel"
              :id="settingsPanelId('transfer')"
              :drafts="transferDrafts"
              :local-download-dir="localDownloadDir"
              :local-can-save="localCanSave"
              :t="t"
              @browse-download-dir="emit('browse-download-dir')"
            />

            <!-- 终端（对标 Tabby「Terminal」页）：渲染 / 键盘 / 鼠标 / 剪贴板 / 声音五组。
                 只上抛增量，归一化与落地在 App；每项默认值都复现改动前的行为。 -->
            <SettingsTerminalPane
              v-show="settingsCategory === 'terminal'"
              role="tabpanel"
              :id="settingsPanelId('terminal')"
              :connection-id="connectionId"
              :startup="connection"
              :pane-direction="paneDirection"
              :webgl-enabled="webglEnabled"
              :terminal-behavior="terminalBehavior"
              :local-shell="localShell"
              :local-shell-options="localShellOptions"
              :action-links="actionLinks"
              :gutter="gutter"
              :ctx-search-engines="ctxSearchEngines"
              :x11-enabled="x11Enabled"
              :x11-read-only-note="x11ReadOnlyNote"
              :rdp-experimental-enabled="rdpExperimentalEnabled"
              :auto-record-enabled="autoRecordEnabled"
              :t="t"
              @set-pane-direction="(value) => emit('update:paneDirection', value)"
              @set-webgl="(value) => emit('update:webgl', value)"
              @update-behavior="(patch) => emit('update-behavior', patch)"
              @set-local-shell="(program) => emit('set-local-shell', program)"
              @update-action-links="(patch) => emit('update:actionLinks', patch)"
              @update-gutter="(patch) => emit('update:gutter', patch)"
              @set-ctx-search-engines="(value) => emit('update:ctxSearchEngines', value)"
              @set-x11-enabled="setX11Enabled"
              @set-rdp-experimental-enabled="setRdpExperimentalEnabled"
              @set-auto-record-enabled="setAutoRecordEnabled"
            />
            <!-- 命令建议独立分栏：从 Terminal 分栏拆出（#138 反馈——建议/补全/ghost
                 配置挤在终端页难找）。历史建议 + 结构化补全 + 行内 ghost 三组
                 配置与忽略列表、接受键换绑在此集中管理。 -->
            <SettingsSuggestionsPane
              v-show="settingsCategory === 'suggestions'"
              role="tabpanel"
              :id="settingsPanelId('suggestions')"
              v-model:command-history-limit-draft="commandHistoryLimitDraft"
              :drafts="suggestionDrafts"
              :ghost-tab-accept="ghostTabAccept"
              :prompt-hints-enabled="promptHintsEnabled"
              :t="t"
              @set-ghost-enabled="(value) => emit('update:ghostSuggest', value)"
              @set-history-panel-enabled="(value) => emit('update:historyPanelEnabled', value)"
              @apply-command-history-limit="(value) => emit('update:commandHistoryLimit', value)"
              @set-ghost-tab-accept="(value) => emit('update:ghostTabAccept', value)"
              @set-prompt-hints-enabled="(value) => emit('update:promptHintsEnabled', value)"
            />

            <!-- AI 助手独立分栏（Warp AI 对齐批）：三条体验开关 + 能力缺失横幅
                 + AI 修复「不再询问」重置。权威态在 App（pluginStore 单键 JSON），
                 本组件只读 + 上抛增量；能力位来自宿主桥 capabilities.ai。 -->
            <SettingsAiPane
              v-show="settingsCategory === 'ai'"
              role="tabpanel"
              :id="settingsPanelId('ai')"
              :ai-settings="aiSettings"
              :ai-bridge-ready="aiBridgeReady"
              :t="t"
              @update-ai-settings="(patch) => emit('update-ai-settings', patch)"
              @reset-ai-consent="emit('reset-ai-consent')"
            />


            <!-- 关键词高亮独立分栏：从 Terminal 分栏拆出（规则管理器与终端行为
                 设置关联性弱），规则增删改/启停在此管理，终端渲染效果实时生效
                 （数据面 RPC 与权威态在 App）。 -->
            <div v-show="settingsCategory === 'highlight'"
              role="tabpanel"
              :id="settingsPanelId('highlight')" class="settings-pane">
            <HighlightRulesSection
              :rules="highlightRules"
              :saving="highlightSaving"
              :limit="HIGHLIGHT_RULES_LIMIT"
              :display-mode="highlightDisplayMode"
              :t="t"
              @save="(rule) => emit('save-highlight-rule', rule)"
              @delete="(id) => emit('delete-highlight-rule', id)"
              @toggle="(item) => emit('toggle-highlight-rule', item)"
              @update:display-mode="(mode) => emit('update:highlightDisplayMode', mode)"
            />
            </div>

            <!-- 快速命令独立分栏：从 Terminal 分栏拆出（片段管理器）。执行仍留在
                 工具条弹层（高频），新建/编辑/导入在此管理。 -->
            <div v-show="settingsCategory === 'commands'"
              role="tabpanel"
              :id="settingsPanelId('commands')" class="settings-pane">
            <QuickCommandsSection
              :commands="quickCommands"
              :groups="quickGroups"
              :connected="connected"
              :saving="quickSaving"
              :importing="quickImporting"
              :limit="QUICK_COMMANDS_LIMIT"
              :t="t"
              @save="(command) => emit('save-quick-command', command)"
              @delete="(id) => emit('delete-quick-command', id)"
              @import="(items) => emit('import-quick-commands', items)"
              @create-group="(name) => emit('create-quick-command-group', name)"
              @rename-group="(payload) => emit('rename-quick-command-group', payload)"
              @dissolve-group="(groupId) => emit('dissolve-quick-command-group', groupId)"
              @run="(item) => emit('run-quick-command', item)"
              @paste="(item) => emit('paste-quick-command', item)"
              @notice="(message) => emit('notice', message)"
              @error="(cause) => emit('error', cause)"
            />
            </div>

            <!-- 快捷键（对标 Tabby「Hotkeys」页）：注册表编辑器，逐动作增删改 + 冲突提示 + 单项/整体复位。 -->
            <div v-show="settingsCategory === 'hotkeys'"
              role="tabpanel"
              :id="settingsPanelId('hotkeys')" class="settings-pane">
            <h3 class="settings-section-title">{{ t("terminalHotkeys.sectionTitle") }}</h3>
            <p class="muted settings-note">{{ t("terminalHotkeys.hint") }}</p>
            <TerminalHotkeyEditor :bindings="terminalHotkeys" :apple-platform="applePlatform" :t="t" @update="updateHotkeys" />
            </div>

          <SettingsSecurityPane
            v-show="settingsCategory === 'security'"
              role="tabpanel"
              :id="settingsPanelId('security')"
            :known-hosts="knownHosts"
            :known-hosts-loading="knownHostsLoading"
            :known-hosts-error="knownHostsError"
            :local-keys="localKeys"
            :local-keys-loading="localKeysLoading"
            :local-keys-error="localKeysError"
            :t="t"
            @refresh-known-hosts="loadKnownHosts"
            @refresh-local-keys="loadLocalKeys"
            @remove-known-host="removeKnownHost"
          />

          <SettingsMcpPane
            v-show="settingsCategory === 'mcp'"
              role="tabpanel"
              :id="settingsPanelId('mcp')"
            :draft="mcpDraft"
            :loading="mcpLoading"
            :error="mcpError"
            :inputs-valid="mcpInputsValid"
            :t="t"
            @refresh="loadMcpSettings"
          />
              </div>
            </div>
          </div>
          </template>

        </div>
        <!-- 存储迁移（IMPL_PLAN_STORAGE_SYNC）：可同步配置经宿主 host.storage 进
             DBX secrets 加密同步；说明与操作放在同一行 footer，压缩底部留白。 -->
        <footer class="settings-footer">
          <p class="settings-footer-note"><span class="settings-footer-pulse" aria-hidden="true"></span><span class="settings-footer-text">{{ t("settingsSyncNote") }}</span></p>
          <div class="settings-footer-actions">
            <button :disabled="connection.state.loading || connection.state.loadFailed || settingsSaving || (!connection.state.meta?.sudoPasswordSet && !connection.state.meta?.totpConfigured)" @click="clearStoredSecrets"><Trash2 />{{ t("settingsClearSecrets") }}</button>
            <button @click="emit('update:open', false)">{{ t("close") }}</button>
            <button class="primary-button" :disabled="connection.state.loading || connection.state.loadFailed || settingsSaving || (!isLocalSession && !connection.state.meta)" @click="saveSettings"><Loader2 v-if="settingsSaving" class="spinning" />{{ t("settingsSave") }}</button>
          </div>
        </footer>
      </DialogContent>
    </Dialog>

    <!-- quick sudo 配置档管理弹窗（工具栏 KeyRound 入口；与内联 section 共用草稿状态） -->
    <ProfilesManagerDialog
      :open="profilesOpen"
      :sudo="sudo"
      :t="t"
      @update:open="(value) => emit('update:profilesOpen', value)"
    />
</template>
