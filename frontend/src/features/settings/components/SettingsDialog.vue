<script setup lang="ts">
// 设置弹窗（从 App.vue 抽出的独立组件）：承载设置主弹窗与 quick sudo 配置档
// 管理弹窗。连接级设置（ssh/settings/get|set）、配置档（sudo/profiles/*）、
// 已知主机（ssh/knownHosts/*）、本机密钥发现（keys/discover）、MCP 限速
// （mcp/settings/*）经 shared/lib/rpc 的 rpc() 直调（桥缺失时统一
// RpcUnavailableError，而非 TypeError）；下载偏好与终端偏好
// （WebGL/选中复制/终端字体）的权威态在宿主 App——下载偏好经 downloadPrefs
// 适配器读写，终端偏好经 props 下发 + emits 上抛。
//
// 本组件是编排者：导航、弹窗外壳、主保存链、Esc 分层退出与共享草稿留在这里；
// 各分类面板拆到 components/Settings*Pane.vue，连接级偏好与 sudo 配置档状态
// 拆到 composables/useConnectionSettings、useSudoProfiles，字体控件拆到
// composables/useTerminalFontControls。
import { computed, reactive, ref, watch } from "vue";
import { Loader2, Trash2, X } from "@lucide/vue";
import { Dialog, DialogContent, DialogTitle } from "../../../shared/ui/dialog/index";
import { Tabs, TabsList, TabsTrigger } from "../../../shared/ui/tabs/index";
import TerminalHotkeyEditor from "../../terminal/components/TerminalHotkeyEditor.vue";
import HighlightRulesSection from "../../terminal/components/HighlightRulesSection.vue";
import QuickCommandsSection from "../../quick-commands/components/QuickCommandsSection.vue";
import { HIGHLIGHT_RULES_LIMIT, type HighlightDisplayMode, type HighlightRuleView } from "../../terminal/lib/decorations/keywordHighlight";
import { settingsPreferences } from "../lib/settingsPreferences";
import { QUICK_COMMANDS_LIMIT, type QuickCommand } from "../../quick-commands/lib/quickCommands";
import { SSH_AGENT_APPROVED_STORE_KEY, SSH_AGENT_MODES_STORE_KEY, sanitizeAgentTerminalMode, sanitizeRememberedCommands } from "../../agent/lib/agentTerminal";
import { saveConnectionEntry } from "../../connections/lib/connectionSettingMirror";
import { confirmDialog } from "../../../shared/lib/confirmDialog";
import { rpc } from "../../../shared/lib/rpc";
import { MIB, mibField, persistMcpSettingsMirror, settingsErrorOf, type DiscoveredKey, type KnownHostEntry, type McpSizeSettings, type SshSettings } from "../lib/settingsModel";
import type { DownloadConflictPolicy } from "../../sftp/lib/downloadPrefs";
import type { EditorConfig, KnownEditor } from "../../sftp/lib/editorRules";
import type { TransferDuplicatePolicy } from "../../transfers/lib/transferQueue";
import type { SftpNameEncoding } from "../../sftp/lib/sftpName";
import {
  type TerminalAppearanceProfile,
  type TerminalAppearanceSettings,
  type TerminalAppearanceState,
} from "../../terminal/lib/appearance/terminalAppearance";
import type { TerminalColorScheme, TerminalThemeLike } from "../../terminal/lib/appearance/terminalScheme";
import type { TerminalBehaviorSettings } from "../../terminal/lib/appearance/terminalBehavior";
import type { TerminalHotkeyBindings } from "../../terminal/lib/behavior/terminalHotkeys";
import type { PaneDirectionPref } from "../../../shared/lib/paneLayout";
import type { AiSettings } from "../../agent/lib/aiSettings";
import { type ActionLinkMatcherToggles, type ActionLinksSettings } from "../../terminal/lib/decorations/actionLinksMatcher";
import { rdpExperimentalEnabled as parseRdpExperimentalEnabled } from "../../remote-desktop/lib/rdpExperimental";
import type { GutterSettings } from "../../terminal/lib/appearance/terminalGutter";
import { useConnectionSettings } from "../composables/useConnectionSettings";
import { useSudoProfiles } from "../composables/useSudoProfiles";
import { useTerminalFontControls } from "../composables/useTerminalFontControls";
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

const props = defineProps<{
  open: boolean;
  profilesOpen: boolean;
  sessionId?: string;
  /** 当前 SSH 会话的连接 id：启动命令偏好按 connectionId 分桶（无会话时整区隐藏）。 */
  connectionId?: string;
  /** 终端当前生效字号（缩放链路在 App，设置页字号草稿以它为初始回显）。 */
  terminalFontSize: number;
  /** 宿主主题终端基准字号（「恢复默认」回到该值）。 */
  hostFontSize: number;
  /** 宿主终端字体族（用户未单独设置字体时预览用它）。 */
  hostFontFamily: string;
  localDownloadDir: string;
  localCanSave: boolean;
  webglEnabled: boolean;
  /** 本地终端默认 Shell（localShell 偏好，权威态在 App）：只读 + 上抛改动。 */
  localShell: string;
  /** 终端行为偏好（对标 Tabby「Terminal」页）：权威态在 App，本组件只读 + 上抛增量。 */
  terminalBehavior: TerminalBehaviorSettings;
  /** 分栏布局方向偏好（issue #187，权威态在 App）：只读 + 上抛改动。 */
  paneDirection: PaneDirectionPref;
  /** ghost 接受键（批 4e/反馈迭代）：权威态在 App，本组件只读 + 上抛改动。 */
  ghostTabAccept: boolean;
  /** AI 助手设置（Warp AI 对齐批）：权威态在 App（pluginStore 单键 JSON），
   *  本组件只读 + 上抛增量；bridgeReady 决定能力缺失横幅。 */
  aiSettings: AiSettings;
  aiBridgeReady: boolean;
  /** 空提示符快捷键引导条开关（权威态在 App）：只读 + 上抛改动。 */
  promptHintsEnabled: boolean;
  /** 终端快捷键绑定（对标 Tabby「Hotkeys」页）：权威态在 App。 */
  terminalHotkeys: TerminalHotkeyBindings;
  /** 是否 Apple 平台：决定快捷键修饰键的显示符号与默认键位口径。 */
  applePlatform: boolean;
  /** 动作链接偏好（权威态在 App，sidecar preferences 持久化）：只读 + 上抛增量。 */
  actionLinks: ActionLinksSettings;
  /** 行号/时间戳 gutter 偏好（权威态在 App）：只读 + 上抛增量。 */
  gutter: GutterSettings;
  /** 右键「在线搜索」引擎表原始文本（每行 name|url；权威态在 App，sidecar 持久化）。 */
  ctxSearchEngines: string;
  /** 背景图偏好（权威态在 App）：开关 / 透明度（10..=90 百分比）/ 会话内存态标记。 */
  wallpaperEnabled: boolean;
  wallpaperOpacity: number;
  wallpaperSessionOnly: boolean;
  /** 终端外观偏好（权威态在 App）：本组件只读 + 经 emits 上抛改动意图。 */
  appearance: TerminalAppearanceState;
  /** 用户保存的主题快照（内置预设由 lib 常量提供，不需经 props）。 */
  customThemes: TerminalAppearanceProfile[];
  /** 当前配置命中的主题 id（null = 已改动，不再等于任何主题）。 */
  activeThemeId: string | null;
  /** 宿主派生的基础终端主题（未启用配色方案时的最终结果）。 */
  hostTheme: TerminalThemeLike;
  hostColorScheme: "light" | "dark";
  /** 下载偏好的读写适配器（权威态与 sidecar preferences 同步在 App）。 */
  downloadPrefs: {
    loadDir(): string;
    loadUseDefault(): boolean;
    loadConflict(): DownloadConflictPolicy;
    persistDir(value: string): void;
    persistUseDefault(value: boolean): void;
    persistConflict(value: DownloadConflictPolicy): void;
  };
  /** 上传并发 / 重复目标策略的读写适配器（权威态在 App，sidecar preferences 同步）。 */
  transferPrefs: {
    loadConcurrency(): number;
    loadDuplicatePolicy(): TransferDuplicatePolicy;
    persistConcurrency(value: number): void;
    persistDuplicatePolicy(value: TransferDuplicatePolicy): void;
    /** M14-B：会话级并发深度 / 兼容模式 / 文件名编码。 */
    loadMaxActive(): number;
    loadCompatMode(): boolean;
    loadNameEncoding(): SftpNameEncoding;
    persistMaxActive(value: number): void;
    persistCompatMode(value: boolean): void;
    persistNameEncoding(value: SftpNameEncoding): void;
    /** Issue #66：下载限速（KiB/s，0=不限速）。 */
    loadDownloadLimit(): number;
    persistDownloadLimit(value: number): void;
    /** M33：压缩传输策略（auto/on/off）与生效阈值（MiB，0=不限下限）。 */
    loadCompressMode(): string;
    loadCompressThreshold(): number;
    persistCompressMode(value: string): void;
    persistCompressThreshold(value: number): void;
  };
  /** 命令输入建议（开关 + 查询长度上下限）的读写适配器，权威态同在 App。 */
  suggestionPrefs: {
    loadEnabled(): boolean;
    loadMinChars(): number;
    loadMaxChars(): number;
    persistEnabled(value: boolean): void;
    persistMinChars(value: number): void;
    persistMaxChars(value: number): void;
  };
  /** 外部编辑器配置（扩展名关联/默认编辑器/回传策略）的读写适配器 + 知名
   * 编辑器目录（权威态与探测在 App：pluginStore 单键 + local/editors/list）。 */
  editorPrefs: {
    loadConfig(): EditorConfig;
    persistConfig(value: EditorConfig): void;
    loadEditors(): KnownEditor[];
    reloadEditors(): Promise<KnownEditor[]>;
  };
  /** 关键词高亮规则（M32-A2：权威态 + RPC 在 App，本组件只读 + 上抛增量）。 */
  highlightRules: HighlightRuleView[];
  highlightSaving: boolean;
  /** 关键词显示方式（全局：背景色块 / 字体颜色；权威态在 App）。 */
  highlightDisplayMode: HighlightDisplayMode;
  /** 快速命令（M32-A3：权威态 + RPC 在 App，本组件只读 + 上抛增量）。 */
  quickCommands: QuickCommand[];
  quickSaving: boolean;
  quickImporting: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}>();

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
  (e: "import-quick-commands", items: Array<{ name: string; command: string }>): void;
  /** 分组改名 / 删除分组（删除 = 组内命令移回未分组）：数据面在 App。 */
  (e: "rename-quick-command-group", payload: { groupId: string; name: string }): void;
  (e: "dissolve-quick-command-group", groupId: string): void;
}>();

const t = props.t;


// 分类顺序对齐 Tabby 的设置页优先级：外观 / 配色方案 / 终端 / 快捷键 四个
// 终端相关分类排在最前（Tabby 把 Appearance 与 Color scheme 标为 prioritized），
// 之后才是本插件特有的 sudo / 智能体 / 传输 / 安全 / MCP。
const SETTINGS_CATEGORIES = [
  { id: "appearance", labelKey: "settingsNav.appearance" },
  { id: "scheme", labelKey: "settingsNav.scheme" },
  { id: "terminal", labelKey: "settingsNav.terminal" },
  // 从 Terminal 分栏拆出（#138 反馈：建议/补全/ghost 配置挤在终端页难找）：
  // 历史建议 + 结构化补全引擎 + 行内 ghost + 忽略列表 + 接受键换绑集中在此。
  { id: "suggestions", labelKey: "settingsNav.suggestions" },
  // 从 Terminal 分栏拆出（内容过大且与终端行为关联性弱）：高亮规则是规则
  // 管理器、快速命令是片段管理器，各自独立成栏，导航标签复用现有键。
  { id: "highlight", labelKey: "highlightRules.title" },
  { id: "commands", labelKey: "quickCommands" },
  // AI 助手（Warp AI 对齐批）：# 命令搜索 / 失败修复 / 唤起助手三开关 +
  // 能力缺失横幅 + 「不再询问」重置；权威态在 App（pluginStore 单键 JSON）。
  { id: "ai", labelKey: "settingsNav.ai" },
  { id: "hotkeys", labelKey: "settingsNav.hotkeys" },
  { id: "sudo", labelKey: "settingsNav.sudo" },
  { id: "agent", labelKey: "agentTerminalSection" },
  { id: "transfer", labelKey: "downloadSettings.title" },
  { id: "security", labelKey: "settingsNav.security" },
  { id: "mcp", labelKey: "mcpLimits.title" },
] as const;
type SettingsCategory = (typeof SETTINGS_CATEGORIES)[number]["id"];
const settingsCategory = ref<SettingsCategory>("appearance");
// 会话级分类（sudo / 智能体）只对 SSH 会话有意义：本地终端没有 sessionId，
// 加载/保存/清密钥/摘要刷新四处门控都会跳过，面板沦为可编辑却永不可保存的
// 死 UI——直接从导航隐藏。本地判定收敛成一处，新增会话级门控一律引用
// isLocalSession，别再手写 `!props.sessionId`（漏写会重新制造「横幅替换
// 整条弹窗」的原始 bug）。
const isLocalSession = computed(() => !props.sessionId);
const visibleSettingsCategories = computed(() =>
  SETTINGS_CATEGORIES.filter(
    (cat) => !isLocalSession.value || (cat.id !== "sudo" && cat.id !== "agent"),
  ),
);

function onSettingsCategoryChange(value: string | number) {
  settingsCategory.value = value as SettingsCategory;
}

// 连接级设置（启动命令 / 文件名编码 / agent 镜像）与会话级加载路径：
// settingsMeta/settingsDraft 仍由本组件的主保存链直接读取。
const connection = useConnectionSettings({
  connectionId: () => props.connectionId,
  sessionId: () => props.sessionId,
  isLocalSession: () => isLocalSession.value,
  isOpen: () => props.open,
  emitError: (cause) => emit("error", cause),
});

// 全局 quick sudo 配置档：列表与编辑表单状态（密钥只在提交时发送）。设置弹窗
// 内联 section 与独立 profiles 弹窗共存复用同一份状态。
const sudo = useSudoProfiles({
  t,
  settingsDraft: connection.state.draft,
  isProfilesOpen: () => props.profilesOpen,
  refreshSettingsMeta: connection.refreshSettingsMeta,
  emitNotice: (message) => emit("notice", message),
});

// 终端字体控件（issue #31）：预设等宽字体 + 跟随宿主 + 自定义；初始化一次，
// 应用/恢复默认后刷新，外观分栏只负责渲染。
const font = useTerminalFontControls({
  terminalFontSize: () => props.terminalFontSize,
  hostFontSize: () => props.hostFontSize,
  emitApplyFont: (payload) => emit("apply-font", payload),
});

const settingsSaving = ref(false);
// 下载 / 传输 / 编辑器 / 命令建议草稿：随主「保存」落库，重开弹窗（reloadSettings）
// 从适配器回填。
const transferDrafts = reactive({
  downloadDir: "",
  downloadUseDefault: props.downloadPrefs.loadUseDefault(),
  downloadConflict: "rename" as DownloadConflictPolicy,
  editorConfig: { associations: [], customEditors: [], uploadPolicy: "auto" } as EditorConfig,
  editorAssociationPattern: "",
  editorAssociationTarget: "",
  editorDefault: "",
  editorKnownEditors: [] as KnownEditor[],
  concurrency: String(props.transferPrefs.loadConcurrency()),
  duplicatePolicy: props.transferPrefs.loadDuplicatePolicy() as TransferDuplicatePolicy,
  maxActive: String(props.transferPrefs.loadMaxActive()),
  downloadLimit: String(props.transferPrefs.loadDownloadLimit()),
  compatMode: props.transferPrefs.loadCompatMode(),
  nameEncoding: props.transferPrefs.loadNameEncoding() as SftpNameEncoding,
  compressMode: props.transferPrefs.loadCompressMode(),
  compressThreshold: String(props.transferPrefs.loadCompressThreshold()),
});
const suggestionDrafts = reactive({
  enabled: props.suggestionPrefs.loadEnabled(),
  minChars: String(props.suggestionPrefs.loadMinChars()),
  maxChars: String(props.suggestionPrefs.loadMaxChars()),
});
// 历史条数输入框草稿由对话框持有：关闭再打开时保留用户输入的非提交态
// （拆分前语义），提交才归一化写穿 store。
const commandHistoryLimitDraft = ref(String(settingsPreferences.commandHistoryLimit.value));

function editorDefaultValue(config: EditorConfig): string {
  return config.defaultCustomId
    ? `custom:${config.defaultCustomId}`
    : config.defaultEditorId
      ? `known:${config.defaultEditorId}`
      : /* reka Select 不接受空串 value，用 "system" 哨兵表示系统默认 */ "system";
}

const knownHosts = ref<KnownHostEntry[]>([]);
const knownHostsLoading = ref(false);
const knownHostsError = ref("");
const localKeys = ref<DiscoveredKey[]>([]);
const localKeysLoading = ref(false);
const localKeysError = ref("");
const mcpDraft = reactive({ readMiB: "", uploadMiB: "", downloadMiB: "", permissionMode: "autonomous", connectionScope: "" });
const mcpLoading = ref(false);
const mcpError = ref("");
const mcpSaving = ref(false);
const mcpInputsValid = computed(() => [mcpDraft.readMiB, mcpDraft.uploadMiB, mcpDraft.downloadMiB]
  .every((value) => /^\d+$/.test(value.trim()) && Number.parseInt(value.trim(), 10) > 0));

/** X11 转发偏好（P3-3）：组件内自治读写 sidecar 偏好——即时生效语义
 * （新会话才启用），不走 props/emit（App 无需感知）。 */
const x11Enabled = ref(false);
const x11ReadOnlyNote = ref(false);

async function loadX11Preference() {
  try {
    const prefs = await window.dbxPlugin?.invoke<{ x11_forwarding?: unknown }>("local/preferences/get", {});
    x11Enabled.value = prefs?.x11_forwarding === true;
  } catch {
    x11Enabled.value = false;
  }
}

async function setX11Enabled(next: boolean) {
  x11Enabled.value = next;
  try {
    await window.dbxPlugin?.invoke("local/preferences/set", { x11_forwarding: next });
  } catch {
    x11Enabled.value = !next;
  }
}

/** 会话自动录制偏好（M14，对标 iShell）：与 X11 同款「组件内自治读写
 * sidecar 偏好」模式，即时生效语义（之后 open 的会话才自动录制）。 */
const autoRecordEnabled = ref(false);

/** RDP 仍处实验阶段：默认关闭，必须由用户在设置中显式启用。 */
const rdpExperimentalEnabled = ref(false);

async function loadRdpExperimentalPreference() {
  try {
    const prefs = await window.dbxPlugin?.invoke<{ rdp_experimental_enabled?: unknown }>("local/preferences/get", {});
    rdpExperimentalEnabled.value = parseRdpExperimentalEnabled(prefs?.rdp_experimental_enabled);
  } catch {
    rdpExperimentalEnabled.value = false;
  }
}

async function setRdpExperimentalEnabled(next: boolean) {
  rdpExperimentalEnabled.value = next;
  try {
    // 开关持久化到 sidecar 偏好即可生效（后端 rdp/start 前置检查）；
    // App 不再持有内存门副本（M32-A 后工具条无 RDP 入口）。
    await window.dbxPlugin?.invoke("local/preferences/set", { rdp_experimental_enabled: next });
  } catch {
    rdpExperimentalEnabled.value = !next;
  }
}

async function loadAutoRecordPreference() {
  try {
    const prefs = await window.dbxPlugin?.invoke<{ auto_record?: unknown }>("local/preferences/get", {});
    autoRecordEnabled.value = prefs?.auto_record === true;
  } catch {
    autoRecordEnabled.value = false;
  }
}

async function setAutoRecordEnabled(next: boolean) {
  autoRecordEnabled.value = next;
  try {
    await window.dbxPlugin?.invoke("local/preferences/set", { auto_record: next });
  } catch {
    autoRecordEnabled.value = !next;
  }
}

void loadAutoRecordPreference();
void loadRdpExperimentalPreference();

void loadX11Preference();

async function reloadSettings() {
  transferDrafts.downloadDir = props.downloadPrefs.loadDir();
  transferDrafts.downloadUseDefault = props.downloadPrefs.loadUseDefault();
  transferDrafts.downloadConflict = props.downloadPrefs.loadConflict();
  transferDrafts.concurrency = String(props.transferPrefs.loadConcurrency());
  transferDrafts.duplicatePolicy = props.transferPrefs.loadDuplicatePolicy();
  transferDrafts.maxActive = String(props.transferPrefs.loadMaxActive());
  transferDrafts.downloadLimit = String(props.transferPrefs.loadDownloadLimit());
  transferDrafts.compatMode = props.transferPrefs.loadCompatMode();
  transferDrafts.nameEncoding = props.transferPrefs.loadNameEncoding();
  transferDrafts.compressMode = props.transferPrefs.loadCompressMode();
  transferDrafts.compressThreshold = String(props.transferPrefs.loadCompressThreshold());
  suggestionDrafts.enabled = props.suggestionPrefs.loadEnabled();
  suggestionDrafts.minChars = String(props.suggestionPrefs.loadMinChars());
  suggestionDrafts.maxChars = String(props.suggestionPrefs.loadMaxChars());
  transferDrafts.editorConfig = props.editorPrefs.loadConfig();
  transferDrafts.editorDefault = editorDefaultValue(transferDrafts.editorConfig);
  void props.editorPrefs.reloadEditors().then((list) => { transferDrafts.editorKnownEditors = list; });
  if (connection.state.loading || settingsSaving.value) return;
  // 每次打开都回到收起态，并丢弃上次遗留的内联编辑草稿：
  // 主「保存」会串行提交未保存的 profile 编辑，不能把陈旧草稿静默入库。
  sudo.state.inlineOpen = false;
  sudo.cancelProfileEdit();
  void loadKnownHosts();
  void loadLocalKeys();
  void loadMcpSettings();
  void sudo.loadSudoProfiles();
  await connection.load();
}

// 本地终端默认 Shell（设置·终端）：下拉项来自 local/shells/list（弹窗打开时
// 拉一次）；localShell 偏好的权威态在 App，这里经 set-local-shell 上抛。
const localShellOptions = ref<Array<{ program: string; name: string; isDefault: boolean; isUserShell: boolean; injectable?: boolean }>>([]);
const localShellOptionsLoaded = ref(false);
async function ensureLocalShellOptions() {
  if (localShellOptionsLoaded.value) return;
  localShellOptionsLoaded.value = true;
  try {
    const result = await window.dbxPlugin?.invoke<{ shells: typeof localShellOptions.value }>("local/shells/list", {}, { timeoutMs: 10_000 });
    localShellOptions.value = result?.shells ?? [];
  } catch {
    // 旧 sidecar 无 shell 发现能力：下拉只剩自动检测项（与工具条选择器同降级）。
    localShellOptions.value = [];
  }
}

watch(() => props.open, (open) => {
  if (open) {
    void reloadSettings();
    void ensureLocalShellOptions();
  }
});

// 弹窗开着时活跃会话切到另一连接：启动命令区按新 connectionId 重新回显。
watch(() => props.connectionId, () => {
  if (props.open) {
    void connection.loadStartupCommands();
    void connection.loadConnNameEncoding();
  }
});

/** 快捷键整表替换：编辑器内部管草稿，只把最终结果上抛给 App 持久化。 */
function updateHotkeys(bindings: TerminalHotkeyBindings) {
  emit("update-hotkeys", bindings);
}

async function loadKnownHosts() {
  knownHostsLoading.value = true;
  knownHostsError.value = "";
  try {
    const result = await rpc<{ entries: KnownHostEntry[] }>("ssh/knownHosts/list", {});
    knownHosts.value = result.entries;
  } catch (cause) {
    knownHosts.value = [];
    knownHostsError.value = settingsErrorOf(cause);
  } finally {
    knownHostsLoading.value = false;
  }
}

async function removeKnownHost(entry: KnownHostEntry) {
  if (!(await confirmDialog(t("knownHosts.removeConfirm", { host: `${entry.host}:${entry.port}` })))) return;

  try {
    await rpc("ssh/knownHosts/remove", { host: entry.host, port: entry.port });
    emit("notice", t("knownHosts.removed", { host: `${entry.host}:${entry.port}` }));
  } catch (cause) {
    knownHostsError.value = settingsErrorOf(cause);
  } finally {
    await loadKnownHosts();
  }
}

async function loadLocalKeys() {
  localKeysLoading.value = true;
  localKeysError.value = "";
  try {
    const result = await rpc<{ keys: DiscoveredKey[] }>("keys/discover", {});
    localKeys.value = result.keys.map((key) => ({ ...key, hasPassphrase: key.hasPassphrase ?? key.has_passphrase === true }));
  } catch (cause) {
    localKeys.value = [];
    localKeysError.value = settingsErrorOf(cause);
  } finally {
    localKeysLoading.value = false;
  }
}

async function loadMcpSettings() {
  mcpLoading.value = true;
  mcpError.value = "";
  try {
    const result = await rpc<McpSizeSettings>("mcp/settings/get", {});
    mcpDraft.readMiB = mibField(result.maxReadBytes);
    mcpDraft.uploadMiB = mibField(result.maxUploadBytes);
    mcpDraft.downloadMiB = mibField(result.maxDownloadBytes);
    // §1.3 新字段：旧 sidecar 不回时用默认（autonomous / 空=不限）。
    mcpDraft.permissionMode = result.execPermissionMode === "confirm" ? "confirm" : "autonomous";
    mcpDraft.connectionScope = Array.isArray(result.connectionScope) ? result.connectionScope.join("\n") : "";
  } catch (cause) {
    mcpError.value = settingsErrorOf(cause);
  } finally {
    mcpLoading.value = false;
  }
}

async function saveMcpSettings() {
  if (!mcpInputsValid.value || mcpSaving.value) return;
  mcpSaving.value = true;
  mcpError.value = "";
  try {
    // 可同步子集一次成形：RPC（sidecar 即时权威）与 pluginStore 镜像（云同
    // 步载荷）双写同一份；localTransferRoot 不在保存参数里，由 sidecar 部分
    // 更新语义保留（存储迁移批 1，IMPL_PLAN_STORAGE_SYNC）。
    const updates = {
      maxReadBytes: Number.parseInt(mcpDraft.readMiB.trim(), 10) * MIB,
      maxUploadBytes: Number.parseInt(mcpDraft.uploadMiB.trim(), 10) * MIB,
      maxDownloadBytes: Number.parseInt(mcpDraft.downloadMiB.trim(), 10) * MIB,
      // §1.3 MCP 权限档 + 连接作用域（每行一条，trim 去空后提交；旧 sidecar
      // 不识别新字段时整体报错，经 mcpError 容错展示）。
      execPermissionMode: mcpDraft.permissionMode === "confirm" ? "confirm" : "autonomous",
      connectionScope: mcpDraft.connectionScope.split("\n").map((line) => line.trim()).filter(Boolean),
    };
    await rpc("mcp/settings/set", updates);
    persistMcpSettingsMirror({ ...updates });
    emit("notice", t("mcpLimits.saved"));
  } catch (cause) {
    mcpError.value = settingsErrorOf(cause);
  } finally {
    mcpSaving.value = false;
  }
}

/**
 * 一次保存链（设置弹窗主按钮）：① 未保存的 profile 编辑 → ② 连接设置 →
 * ③ MCP 限速。各步独立容错——saveProfileDraft/saveMcpSettings 内部已把失败
 * 写入 sudoProfilesError/mcpError 并展示，单步失败不阻断其余步骤；
 * MCP 表单非法时保持现有校验提示、静默跳过提交。
 */
async function saveSettings() {
  // 本地 Shell 工作台没有 SSH 会话（settingsMeta 恒缺）：全局偏好面板必须
  // 可保存，只隔离会话级段（sudo/2FA/agent 更新依赖 sessionId）。
  if (settingsSaving.value || connection.state.loading || connection.state.loadFailed) return;
  if (!isLocalSession.value && !connection.state.meta) return;
  settingsSaving.value = true;
  try {
    props.downloadPrefs.persistDir(transferDrafts.downloadDir);
    props.downloadPrefs.persistUseDefault(transferDrafts.downloadUseDefault);
    props.downloadPrefs.persistConflict(transferDrafts.downloadConflict);
    props.transferPrefs.persistConcurrency(Number.parseInt(transferDrafts.concurrency, 10) || 3);
    props.transferPrefs.persistDuplicatePolicy(transferDrafts.duplicatePolicy);
    props.transferPrefs.persistMaxActive(Number.parseInt(transferDrafts.maxActive, 10) || 3);
    props.transferPrefs.persistDownloadLimit(Math.max(0, Number.parseInt(transferDrafts.downloadLimit, 10) || 0));
    props.transferPrefs.persistCompatMode(transferDrafts.compatMode);
    props.transferPrefs.persistNameEncoding(transferDrafts.nameEncoding);
    props.transferPrefs.persistCompressMode(transferDrafts.compressMode);
    props.transferPrefs.persistCompressThreshold(Math.min(65536, Math.max(0, Number.parseInt(transferDrafts.compressThreshold, 10) || 0)));
    props.suggestionPrefs.persistEnabled(suggestionDrafts.enabled);
    props.suggestionPrefs.persistMinChars(Number.parseInt(suggestionDrafts.minChars, 10) || 1);
    props.suggestionPrefs.persistMaxChars(Number.parseInt(suggestionDrafts.maxChars, 10) || 64);
    props.editorPrefs.persistConfig({
      ...transferDrafts.editorConfig,
      defaultEditorId: transferDrafts.editorDefault.startsWith("known:") ? transferDrafts.editorDefault.slice("known:".length) : undefined,
      defaultCustomId: transferDrafts.editorDefault.startsWith("custom:") ? transferDrafts.editorDefault.slice("custom:".length) : undefined,
      // editorDefaultDraft === "system"（或意外值）时不落默认编辑器字段。
    });
    if (sudo.state.editing) await sudo.saveProfileDraft();
    // 会话级段：本地 Shell 工作台整段跳过（分类已隐藏，无表单可改）。
    if (!isLocalSession.value) {
    const rememberedCommands = sanitizeRememberedCommands(connection.state.draft.rememberedCommands);
    const updates: Record<string, unknown> = {
      quickSudo: connection.state.draft.quickSudo,
      sudoUsePty: connection.state.draft.sudoUsePty,
      authFlowMode: connection.state.draft.authFlowMode,
      passwordPromptHint: connection.state.draft.passwordPromptHint,
      totpPromptHint: connection.state.draft.totpPromptHint,
      quickSudoProfileId: connection.state.draft.quickSudoProfileId,
      agentTerminalMode: connection.state.draft.agentTerminalMode,
      rememberedCommands,
    };
    if (connection.state.draft.sudoPassword) updates.sudoPassword = connection.state.draft.sudoPassword;
    if (connection.state.draft.totpSecret.trim()) updates.totpSecret = connection.state.draft.totpSecret;
    const meta = await rpc<SshSettings>("ssh/settings/set", { sessionId: props.sessionId, ...updates });
    connection.state.meta = meta;
    // 存储迁移批 2：保存成功即镜像连接级 agent 两域（云同步载荷）。
    if (props.connectionId) {
      saveConnectionEntry(SSH_AGENT_MODES_STORE_KEY, props.connectionId, sanitizeAgentTerminalMode(connection.state.draft.agentTerminalMode));
      saveConnectionEntry(SSH_AGENT_APPROVED_STORE_KEY, props.connectionId, rememberedCommands);
    }
    connection.state.draft.sudoPassword = "";
    connection.state.draft.totpSecret = "";
    }
    await saveMcpSettings();
    emit("notice", t("settingsSaved"));
  } catch (cause) {
    emit("error", cause);
  } finally {
    settingsSaving.value = false;
  }
}

async function clearStoredSecrets() {
  if (isLocalSession.value || settingsSaving.value || connection.state.loading || connection.state.loadFailed || !connection.state.meta) return;
  try {
    const meta = await rpc<SshSettings>("ssh/settings/set", {
      sessionId: props.sessionId,
      sudoPassword: "",
      totpSecret: "",
    });
    connection.state.meta = meta;
    emit("notice", t("settingsSecretsCleared"));
  } catch (cause) {
    emit("error", cause);
  }
}

/// Esc 分层退出：先关编辑表单，再收起配置档 section；返回 false 表示已到
/// 最底层，调用方（App Esc 链）应关闭整个弹窗。
function consumeInlineEsc(): boolean {
  if (sudo.state.inlineOpen && sudo.state.editing) {
    sudo.cancelProfileEdit();
    return true;
  }
  if (sudo.state.inlineOpen) {
    sudo.state.inlineOpen = false;
    return true;
  }
  return false;
}


/// 下载询问弹窗勾选「设为默认」/目录选择器回填后，App 同步设置页草稿
/// （弹窗开着也能立即看到）。
function setDownloadDirDraft(dir: string) {
  transferDrafts.downloadDir = dir;
}

function setDownloadUseDefaultDraft(value: boolean) {
  transferDrafts.downloadUseDefault = value;
}

defineExpose({ consumeInlineEsc, setDownloadDirDraft, setDownloadUseDefaultDraft });
</script>

<template>
    <!-- 设置主弹窗 -->
    <Dialog :open="open" @update:open="(value) => emit('update:open', value)">
      <DialogContent class="modal settings-modal settings-nav-modal" @escape-key-down.prevent>
        <header><DialogTitle>{{ t("settings") }}</DialogTitle><button :title="t('close')" :aria-label="t('close')" class="icon-button" @click="emit('update:open', false)"><X /></button></header>
        <div class="settings-body">
          <div v-if="connection.state.loading" class="empty compact"><Loader2 class="spinning" />{{ t("loading") }}</div>
          <div v-else-if="connection.state.loadFailed" class="task-error" role="alert">
            {{ t("settingsLoadFailed") }}
            <button class="link-button" @click="reloadSettings">{{ t("refresh") }}</button>
          </div>
          <template v-else>
          <div class="settings-layout">
            <nav class="settings-nav" aria-label="settings categories">
              <Tabs :model-value="settingsCategory" orientation="vertical" class="settings-nav-tabs" @update:model-value="onSettingsCategoryChange">
                <TabsList class="settings-nav-list">
                  <TabsTrigger v-for="cat in visibleSettingsCategories" :key="cat.id" :value="cat.id" class="settings-nav-item">{{ t(cat.labelKey) }}</TabsTrigger>
                </TabsList>
              </Tabs>
            </nav>
            <div class="settings-content">
            <!-- 配色方案（对标 Tabby「Color scheme」页，从原「外观」里拆出）：主题
                 快照、实时预览、深浅两槽配色方案、终端背景来源、自定义方案导入。 -->
            <SettingsSchemePane
              v-show="settingsCategory === 'scheme'"
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
              :sudo="sudo"
              :draft="connection.state.draft"
              :meta="connection.state.meta"
              :t="t"
            />

            <SettingsAgentPane
              v-show="settingsCategory === 'agent'"
              :draft="connection.state.draft"
              :t="t"
            />

            <SettingsTransferPane
              v-show="settingsCategory === 'transfer'"
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
              :ai-settings="aiSettings"
              :ai-bridge-ready="aiBridgeReady"
              :t="t"
              @update-ai-settings="(patch) => emit('update-ai-settings', patch)"
              @reset-ai-consent="emit('reset-ai-consent')"
            />


            <!-- 关键词高亮独立分栏：从 Terminal 分栏拆出（规则管理器与终端行为
                 设置关联性弱），规则增删改/启停在此管理，终端渲染效果实时生效
                 （数据面 RPC 与权威态在 App）。 -->
            <div v-show="settingsCategory === 'highlight'" class="settings-pane">
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
            <div v-show="settingsCategory === 'commands'" class="settings-pane">
            <QuickCommandsSection
              :commands="quickCommands"
              :saving="quickSaving"
              :importing="quickImporting"
              :limit="QUICK_COMMANDS_LIMIT"
              :t="t"
              @save="(command) => emit('save-quick-command', command)"
              @delete="(id) => emit('delete-quick-command', id)"
              @import="(items) => emit('import-quick-commands', items)"
              @rename-group="(payload) => emit('rename-quick-command-group', payload)"
              @dissolve-group="(groupId) => emit('dissolve-quick-command-group', groupId)"
              @error="(cause) => emit('error', cause)"
            />
            </div>

            <!-- 快捷键（对标 Tabby「Hotkeys」页）：注册表编辑器，逐动作增删改 + 冲突提示 + 单项/整体复位。 -->
            <div v-show="settingsCategory === 'hotkeys'" class="settings-pane">
            <h3 class="settings-section-title">{{ t("terminalHotkeys.sectionTitle") }}</h3>
            <p class="muted settings-note">{{ t("terminalHotkeys.hint") }}</p>
            <TerminalHotkeyEditor :bindings="terminalHotkeys" :apple-platform="applePlatform" :t="t" @update="updateHotkeys" />
            </div>

          <SettingsSecurityPane
            v-show="settingsCategory === 'security'"
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
            :draft="mcpDraft"
            :loading="mcpLoading"
            :error="mcpError"
            :inputs-valid="mcpInputsValid"
            :t="t"
            @refresh="loadMcpSettings"
          />
            </div>
          </div>
          </template>

        </div>
        <!-- 存储迁移（IMPL_PLAN_STORAGE_SYNC）：可同步配置经宿主 host.storage 进
             DBX secrets 加密同步；生效前提在 DBX 侧，这里向用户交代边界。 -->
        <p class="muted settings-note">{{ t("settingsSyncNote") }}</p>
        <footer>
          <button :disabled="connection.state.loading || connection.state.loadFailed || settingsSaving || (!connection.state.meta?.sudoPasswordSet && !connection.state.meta?.totpConfigured)" @click="clearStoredSecrets"><Trash2 />{{ t("settingsClearSecrets") }}</button>
          <button @click="emit('update:open', false)">{{ t("close") }}</button>
          <button class="primary-button" :disabled="connection.state.loading || connection.state.loadFailed || settingsSaving || (!isLocalSession && !connection.state.meta)" @click="saveSettings"><Loader2 v-if="settingsSaving" class="spinning" />{{ t("settingsSave") }}</button>
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
