<script setup lang="ts">
// 设置·命令建议分栏（从 Terminal 分栏拆出，#138 反馈——建议/补全/ghost 配置挤在
// 终端页难找）：历史建议 + 结构化补全 + 行内 ghost + 忽略列表 + 接受键换绑 +
// 历史面板/条数 + 空提示符引导条。全局偏好的权威态在 settingsPreferences store，
// 写穿后仍按既有 emits 上抛 App 做即时副作用。
import { onMounted, ref } from "vue";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../shared/ui/select/index";
import { Switch } from "../../../shared/ui/switch/index";
import {
  COMPLETION_ENGINE_KEY,
  loadCompletionEngine,
  pluginStore,
  sanitizeCompletionEngine,
  type CompletionEngineSetting,
} from "../../../shared/lib/pluginStore";
import {
  COMMAND_HISTORY_LIMIT,
  MAX_COMMAND_HISTORY_LIMIT,
  MIN_COMMAND_HISTORY_LIMIT,
  sanitizeCommandHistoryLimit,
} from "../../terminal/lib/panels/commandHistory";
import { settingsPreferences } from "../lib/settingsPreferences";
import { clearSuggestionBlocklist, loadSuggestionBlocklist } from "../../completion/lib/suggestionBlocklist";

/** 命令建议草稿（弹窗持有并随主「保存」落库）。 */
interface SuggestionDrafts {
  enabled: boolean;
  minChars: string;
  maxChars: string;
}

const props = defineProps<{
  drafts: SuggestionDrafts;
  ghostTabAccept: boolean;
  promptHintsEnabled: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

const emit = defineEmits<{
  (e: "set-ghost-enabled", value: boolean): void;
  (e: "set-history-panel-enabled", value: boolean): void;
  (e: "apply-command-history-limit", value: number): void;
  (e: "set-ghost-tab-accept", value: boolean): void;
  (e: "set-prompt-hints-enabled", value: boolean): void;
}>();

// 建议黑名单（批 4c）：计数随每次打开设置重读（App 内行内 ✗ 即时持久化，
// 这里只读展示 + 清空上抛）。
const suggestionBlocklistCount = ref(0);
function refreshSuggestionBlocklistCount() {
  suggestionBlocklistCount.value = loadSuggestionBlocklist().length;
}
onMounted(refreshSuggestionBlocklistCount);
function clearSuggestionBlocklistFromSettings() {
  clearSuggestionBlocklist();
  refreshSuggestionBlocklistCount();
}

// 结构化补全引擎选择（FIG wave-1，契约 §2.3）：组件内自治读写 pluginStore
// （键 ssh-completion-engine，fig-safe 默认 / fig / off）——不走 props/emit，
// App 每次调度前直读同一键，无需事件同步。off = 无结构化浮层（历史/ghost
// 不受影响）；fig 与 fig-safe 批次 1 行为相同，差异自 generator 接线起。
const completionEngine = ref<CompletionEngineSetting>(loadCompletionEngine());

function setCompletionEngine(next: string) {
  const value = sanitizeCompletionEngine(next);
  completionEngine.value = value;
  try {
    pluginStore.setItem(COMPLETION_ENGINE_KEY, value);
  } catch {
    // 存储不可用（无宿主桥且 localStorage 受限）：仅当前会话生效。
  }
}

// 终端行内 ghost 自动建议开关（np8，对标 Warp/fish）：偏好权威迁入
// settingsPreferences store（与 App/useGhostSuggest 共享同一 ref）。
const ghostEnabled = settingsPreferences.ghostSuggestEnabled;
// 历史面板功能开关（issue #169）：同上，store 权威。默认开；关闭后 ↑ 与
// Ctrl+R 都归还远端 shell。
const historyPanelEnabled = settingsPreferences.historyPanelEnabled;

function setGhostEnabled(next: boolean) {
  settingsPreferences.setGhostSuggestEnabled(next);
  emit("set-ghost-enabled", next);
}

function setHistoryPanelEnabled(next: boolean) {
  settingsPreferences.setHistoryPanelEnabled(next);
  emit("set-history-panel-enabled", next);
}

// 历史显示条数（issue #192）：命令环上限（10..1000，默认 100）。draft 保留
// 输入框的非提交态；由对话框持有并跨开关保留（与拆分前一致），提交时经
// store（归一化 + 持久化）写穿，再上抛 App 收紧各桶。
const commandHistoryLimitDraft = defineModel<string>("commandHistoryLimitDraft", { required: true });

function applyCommandHistoryLimitFromDraft() {
  const next = sanitizeCommandHistoryLimit(Number.parseInt(commandHistoryLimitDraft.value, 10));
  commandHistoryLimitDraft.value = String(next);
  settingsPreferences.setCommandHistoryLimit(next);
  emit("apply-command-history-limit", next);
}
</script>

<template>
  <div class="settings-pane">
    <h3 class="settings-section-title">{{ t("suggestions.settingsTitle") }}</h3>
    <label class="settings-field settings-switch-row">
      <Switch v-model="drafts.enabled" size="sm" />
      <span>{{ t("suggestions.settingsEnabled") }}</span>
    </label>
    <p class="muted settings-note">{{ t("suggestions.settingsEnabledHint") }}</p>
    <label class="settings-field">
      <span>{{ t("suggestions.settingsMinChars") }}</span>
      <input v-model="drafts.minChars" type="number" min="1" max="16" step="1" @change="drafts.minChars = String(Math.min(16, Math.max(1, Number.parseInt(drafts.minChars, 10) || 1)))" />
    </label>
    <p class="muted settings-note">{{ t("suggestions.settingsMinCharsHint") }}</p>
    <label class="settings-field">
      <span>{{ t("suggestions.settingsMaxChars") }}</span>
      <input v-model="drafts.maxChars" type="number" min="8" max="512" step="1" @change="drafts.maxChars = String(Math.min(512, Math.max(8, Number.parseInt(drafts.maxChars, 10) || 64)))" />
    </label>
    <p class="muted settings-note">{{ t("suggestions.settingsMaxCharsHint") }}</p>
    <!-- 建议黑名单（批 4c）：计数 + 一键清空；行内 ✗ 的排除项在此恢复。 -->
    <div v-if="suggestionBlocklistCount > 0" class="settings-field suggestion-blocklist-row">
      <span class="muted settings-note">{{ t("suggestions.blocklistCount", { count: suggestionBlocklistCount }) }}</span>
      <button type="button" class="connect-card-ghost-button" @click="clearSuggestionBlocklistFromSettings">{{ t("suggestions.blocklistClear") }}</button>
    </div>
    <label class="settings-field">
      <span>{{ t("completionMenu.engine") }}</span>
      <Select :model-value="completionEngine" @update:model-value="setCompletionEngine(String($event))">
        <SelectTrigger size="xs"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value="fig-safe">{{ t("completionMenu.engineFigSafe") }}</SelectItem>
          <SelectItem value="fig">{{ t("completionMenu.engineFig") }}</SelectItem>
          <SelectItem value="off">{{ t("completionMenu.engineOff") }}</SelectItem>
        </SelectContent>
      </Select>
    </label>
    <p class="muted settings-note">{{ t("completionMenu.engineHint") }}</p>

    <!-- 行内 ghost 自动建议（np8，对标 Warp/fish）：追加于「终端行为」组末尾；
         权威态在 settingsPreferences store（键 ssh-terminal-ghost-suggest，
         弹窗/App/useGhostSuggest 共享同一 ref），写穿后即时上抛 App。 -->
    <h3 class="settings-section-title">{{ t("terminalGhost.sectionTitle") }}</h3>
    <label class="settings-field settings-switch-row">
      <Switch :model-value="ghostEnabled" size="sm" @update:model-value="setGhostEnabled(Boolean($event))" />
      <span>{{ t("terminalGhost.label") }}</span>
    </label>
    <p class="muted settings-note">{{ t("terminalGhost.hint") }}</p>
    <label class="settings-field settings-switch-row">
      <Switch :model-value="ghostTabAccept" size="sm" @update:model-value="emit('set-ghost-tab-accept', Boolean($event))" />
      <span>{{ t("terminalGhost.tabAccept") }}</span>
    </label>
    <p class="muted settings-note">{{ t("terminalGhost.tabAcceptHint") }}</p>
    <p class="muted settings-note">{{ t("terminalGhost.keysSummary") }}</p>
    <!-- 历史面板功能开关（issue #169）：关闭后 ↑ 与 Ctrl+R 都归还
         shell 原生历史/反向搜索，面板整体不可唤起。 -->
    <h3 class="settings-section-title">{{ t("historyPanelSettings.sectionTitle") }}</h3>
    <label class="settings-field settings-switch-row">
      <Switch :model-value="historyPanelEnabled" size="sm" @update:model-value="setHistoryPanelEnabled(Boolean($event))" />
      <span>{{ t("historyPanelSettings.enabledLabel") }}</span>
    </label>
    <p class="muted settings-note">{{ t("historyPanelSettings.enabledHint") }}</p>
    <!-- 历史显示条数（issue #192）：命令环上限，收紧即时裁剪生效。 -->
    <label class="settings-field">
      <span>{{ t("historyPanelSettings.limitLabel") }}</span>
      <input v-model="commandHistoryLimitDraft" type="number" :min="MIN_COMMAND_HISTORY_LIMIT" :max="MAX_COMMAND_HISTORY_LIMIT" step="10" @change="applyCommandHistoryLimitFromDraft" />
    </label>
    <p class="muted settings-note">{{ t("historyPanelSettings.limitHint", { min: MIN_COMMAND_HISTORY_LIMIT, max: MAX_COMMAND_HISTORY_LIMIT, def: COMMAND_HISTORY_LIMIT }) }}</p>
    <!-- 空提示符快捷键引导条开关：终端空行静置时的 kbd 提示（↑/Ctrl+R/
         Ctrl+Space/→），用过即散；设置里重开即清消散旗标。 -->
    <label class="settings-field settings-switch-row">
      <Switch :model-value="promptHintsEnabled" size="sm" @update:model-value="emit('set-prompt-hints-enabled', Boolean($event))" />
      <span>{{ t("terminalHints.settingsLabel") }}</span>
    </label>
    <p class="muted settings-note">{{ t("terminalHints.settingsHint") }}</p>
  </div>
</template>
